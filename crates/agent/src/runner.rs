//! Carries out one job.

use std::time::Duration;

use dbm_shared::{
    e2ee::{self, SecretContext},
    jobs::{DatabaseSecret, Job, Sealed, SshSecret, StorageSecret, Task},
};
use serde::de::DeserializeOwned;
use tokio::{runtime::Handle, task};

use crate::{
    Result, engines, manifest, pipeline, progress::Progress, ssh::Tunnel, state::State, storage,
};

/// Opens a secret sealed to this agent. A server cannot read it, and cannot move it
/// to another record without the check failing.
fn open(state: &State, sealed: &Sealed, kind: &str) -> Result<Vec<u8>> {
    let context = SecretContext {
        agent_id: &state.agent_id,
        secret_id: &sealed.id,
        kind,
        version: sealed.version,
    };
    let private = e2ee::decode(&state.private_key)?;
    e2ee::open_secret(&private, &context, &e2ee::decode(&sealed.data)?)
        .map_err(|_| format!("cannot open the {kind} secret: enter it again for this agent").into())
}

fn open_json<T: DeserializeOwned>(state: &State, sealed: &Sealed, kind: &str) -> Result<T> {
    Ok(serde_json::from_slice(&open(state, sealed, kind)?)?)
}

/// The database to talk to, going through its SSH tunnel when it has one. The tunnel
/// stays open as long as it is held.
async fn database(
    state: &State,
    sealed: &Sealed,
    ssh: Option<&Sealed>,
    progress: &Progress,
) -> Result<(DatabaseSecret, Option<Tunnel>)> {
    let mut secret: DatabaseSecret = open_json(state, sealed, "database")?;
    let Some(sealed) = ssh else {
        return Ok((secret, None));
    };
    let login: SshSecret = open_json(state, sealed, "ssh")?;
    if secret.engine == "sqlite" {
        return Err("SQLite files cannot be reached through SSH".into());
    }
    let tunnel = Tunnel::open(&sealed.id, &login, &secret.host, secret.port).await?;
    progress.log("info", format!("SSH tunnel through {} is open", login.host));
    // Tools talk to the near end; the far end resolves the database host.
    secret.host = "127.0.0.1".into();
    secret.port = tunnel.port;
    Ok((secret, Some(tunnel)))
}

async fn store(state: &State, sealed: &Sealed) -> Result<(storage::Store, StorageSecret)> {
    let secret: StorageSecret = open_json(state, sealed, "destination")?;
    Ok((storage::open(&secret, &sealed.id).await?, secret))
}

/// Blocking work off the async thread, so reports keep flowing.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> std::result::Result<T, String> + Send + 'static,
) -> Result<T> {
    Ok(task::spawn_blocking(work).await??)
}

fn text(error: Box<dyn std::error::Error>) -> String {
    error.to_string()
}

/// What a successful job hands back to the server.
#[derive(Default)]
pub struct Output {
    pub sha256: Option<String>,
    pub manifest: Option<String>,
    pub result: Option<String>,
}

/// Old snapshots carry no signature; restoring one has to be allowed on the agent itself.
fn unsigned_allowed() -> bool {
    std::env::var("DBM_ALLOW_UNSIGNED_RESTORE").is_ok_and(|v| v == "true" || v == "1")
}

pub async fn run(state: &State, job: Job, progress: Progress) -> Result<Output> {
    match job.task {
        Task::TestSsh { ssh } => {
            let login: SshSecret = open_json(state, &ssh, "ssh")?;
            // Forwarding to the SSH port itself proves the login and that forwarding is allowed.
            let _tunnel = Tunnel::open(&ssh.id, &login, "127.0.0.1", login.port).await?;
            progress.log(
                "ok",
                format!("signed in to {} and opened a tunnel", login.host),
            );
            Ok(Output::default())
        }
        Task::TestDatabase {
            database: sealed,
            ssh,
        } => {
            let (secret, _tunnel) = database(state, &sealed, ssh.as_ref(), &progress).await?;
            let version = blocking(move || engines::test(&secret).map_err(text)).await?;
            progress.log("ok", format!("connected: {version}"));
            Ok(Output::default())
        }
        Task::TestDestination { destination } => {
            store(state, &destination).await?.0.test().await?;
            progress.log("ok", "wrote, read and deleted a test file");
            Ok(Output::default())
        }
        Task::Backup {
            database: sealed,
            destination,
            name,
            compression,
            level,
            recipients,
            object_key,
            ssh,
        } => {
            let (database, _tunnel) = database(state, &sealed, ssh.as_ref(), &progress).await?;
            let (store, _) = store(state, &destination).await?;
            progress.log("info", format!("dumping {name}, compression {compression}"));
            let method = compression.clone();

            let mut sink = store.sink(&object_key).await?;
            let handle = Handle::current();
            let shared = progress.clone();
            // The pipeline blocks, so it gets its own thread and the reports keep flowing.
            let (sink, result) = task::spawn_blocking(move || {
                let result = engines::dump(&database, &name)
                    .and_then(|dump| {
                        pipeline::backup(
                            dump,
                            &compression,
                            level,
                            &recipients,
                            &mut sink,
                            handle,
                            &shared,
                        )
                    })
                    .map_err(text);
                (sink, result)
            })
            .await?;
            match result {
                Ok(sha256) => {
                    sink.commit().await?;
                    progress.log("ok", format!("stored {object_key}"));
                    Ok(Output {
                        manifest: Some(manifest::sign(state, &object_key, &sha256, &method)?),
                        sha256: Some(sha256),
                        result: None,
                    })
                }
                Err(error) => {
                    if !matches!(
                        tokio::time::timeout(Duration::from_secs(30), sink.abort()).await,
                        Ok(Ok(()))
                    ) {
                        progress.log("warn", "could not remove the unfinished upload");
                    }
                    Err(error.into())
                }
            }
        }
        Task::Restore {
            database: sealed,
            destination,
            identity,
            object_key,
            sha256,
            manifest,
            compression,
            name,
            source,
            ssh,
        } => {
            // Checked before anything is downloaded or touched.
            match &manifest {
                Some(tag) => {
                    manifest::verify(state, &object_key, &sha256, &compression, tag)?;
                    progress.log("ok", "backup signature verified");
                }
                None if unsigned_allowed() => {
                    progress.log("warn", "restoring a backup without a signature");
                }
                None => {
                    return Err("this backup has no signature: set DBM_ALLOW_UNSIGNED_RESTORE=true on the agent to restore it".into());
                }
            }
            let identity = String::from_utf8(open(state, &identity, "backup-key")?)?;
            let (store, _) = store(state, &destination).await?;

            progress.log("info", format!("downloading {object_key}"));
            let temporary = tempfile::NamedTempFile::new()?;
            let mut file = tokio::fs::File::from_std(temporary.reopen()?);
            if store.download(&object_key, &mut file, &progress).await? != sha256 {
                return Err(
                    "checksum mismatch: the stored file is not the one that was backed up".into(),
                );
            }
            progress.log("ok", "checksum verified");

            let (database, _tunnel) = database(state, &sealed, ssh.as_ref(), &progress).await?;
            let file = temporary.reopen()?;
            let shared = progress.clone();
            blocking(move || {
                let (tool, created) = engines::restore(&database, &name, &source).map_err(text)?;
                if created {
                    shared.log("info", format!("created {name}"));
                }
                shared.log("info", format!("restoring into {name}"));
                pipeline::restore(tool, &compression, &identity, file, &shared).map_err(text)
            })
            .await?;
            progress.log("ok", "restore completed");
            Ok(Output::default())
        }
        Task::DeleteSnapshot {
            destination,
            object_key,
        } => {
            store(state, &destination)
                .await?
                .0
                .delete(&object_key)
                .await?;
            progress.log("ok", format!("deleted {object_key}"));
            Ok(Output::default())
        }
        Task::PresignSnapshot {
            destination,
            object_key,
        } => {
            let secret: StorageSecret = open_json(state, &destination, "destination")?;
            let link = storage::link(&secret, &object_key).await?;
            progress.log("ok", "download link created, valid for 5 minutes");
            Ok(Output {
                result: Some(link),
                ..Output::default()
            })
        }
    }
}
