//! Carries out one job.

use std::time::Duration;

use dbm_shared::{
    e2ee::{self, SecretContext},
    jobs::{DatabaseSecret, Job, Sealed, SshSecret, StorageSecret, Task},
};
use serde::de::DeserializeOwned;
use tokio::{runtime::Handle, task};

use crate::{
    Result,
    binlog::{self, Position},
    engines, manifest,
    oplog::{self, Timestamp},
    pipeline,
    progress::Progress,
    ssh::Tunnel,
    state::State,
    storage,
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

/// An error carried across threads without losing that the chain is broken.
enum Kept {
    Broken(binlog::ChainBroken),
    Other(String),
}

impl From<Box<dyn std::error::Error>> for Kept {
    fn from(error: Box<dyn std::error::Error>) -> Self {
        match error.downcast::<binlog::ChainBroken>() {
            Ok(broken) => Kept::Broken(*broken),
            Err(other) => Kept::Other(other.to_string()),
        }
    }
}

impl Kept {
    fn into_error(self) -> Box<dyn std::error::Error> {
        match self {
            Kept::Broken(broken) => broken.into(),
            Kept::Other(other) => other.into(),
        }
    }
}

/// What a successful job hands back to the server.
#[derive(Default)]
pub struct Output {
    pub sha256: Option<String>,
    pub manifest: Option<String>,
    pub result: Option<String>,
    pub position_from: Option<String>,
    pub position_to: Option<String>,
}

/// What an incremental backup takes instead of a dump.
enum Take {
    /// MySQL and MariaDB binary log files.
    Logs(Vec<String>),
    /// MongoDB oplog entries between two positions.
    Oplog { since: String, to: String },
}

/// Where a backup sits in the database's log.
#[derive(Default)]
struct Chain {
    /// Set for an incremental backup; a full one dumps the database.
    take: Option<Take>,
    from: Option<String>,
    to: Option<String>,
    /// The server a full MySQL backup is dumped from; its position is read from the dump.
    server: Option<String>,
}

/// Looked up before the dump starts.
fn chain(secret: &DatabaseSecret, name: &str, track: bool, since: Option<&str>) -> Result<Chain> {
    if !track && since.is_none() {
        return Ok(Chain::default());
    }
    if !dbm_shared::jobs::can_follow(&secret.engine, name) {
        return Err("this backup cannot be followed by incremental ones".into());
    }
    let mongo = secret.engine == "mongodb";
    if let Some(since) = since {
        let (take, to) = if mongo {
            let to = oplog::span(secret, Timestamp::parse(since)?)?.to_string();
            let take = Take::Oplog {
                since: since.into(),
                to: to.clone(),
            };
            (take, to)
        } else {
            let span = binlog::span(secret, &Position::parse(since)?)?;
            (Take::Logs(span.files), span.to.to_string())
        };
        return Ok(Chain {
            take: Some(take),
            from: Some(since.into()),
            to: Some(to),
            server: None,
        });
    }
    if mongo {
        // Replaying an entry twice changes nothing, so starting before the dump is safe.
        return Ok(Chain {
            to: Some(oplog::current(secret)?.to_string()),
            ..Chain::default()
        });
    }
    binlog::check(secret)?;
    Ok(Chain {
        server: Some(binlog::server(secret)?),
        ..Chain::default()
    })
}

/// Checks the signature before anything is downloaded or touched.
fn trusted(
    state: &State,
    progress: &Progress,
    object_key: &str,
    sha256: &str,
    compression: &str,
    positions: manifest::Positions<'_>,
    manifest: Option<&str>,
) -> Result<()> {
    match manifest {
        Some(tag) => {
            manifest::verify(state, object_key, sha256, compression, positions, tag)?;
            progress.log("ok", "backup signature verified");
        }
        None if unsigned_allowed() && positions.is_none() => {
            progress.log("warn", "restoring a backup without a signature");
        }
        None => {
            return Err("this backup has no signature: set DBM_ALLOW_UNSIGNED_RESTORE=true on the agent to restore it".into());
        }
    }
    Ok(())
}

/// Downloads a stored file whole and checks it, so a database never sees part of a bad one.
async fn fetch(
    store: &storage::Store,
    object_key: &str,
    sha256: &str,
    progress: &Progress,
) -> Result<tempfile::NamedTempFile> {
    progress.log("info", format!("downloading {object_key}"));
    let temporary = tempfile::NamedTempFile::new()?;
    let mut file = tokio::fs::File::from_std(temporary.reopen()?);
    if store.download(object_key, &mut file, progress).await? != sha256 {
        return Err("checksum mismatch: the stored file is not the one that was backed up".into());
    }
    progress.log("ok", "checksum verified");
    Ok(temporary)
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
            track,
            since,
        } => {
            let (database, _tunnel) = database(state, &sealed, ssh.as_ref(), &progress).await?;
            let (store, _) = store(state, &destination).await?;
            let database = std::sync::Arc::new(database);
            let looked = database.clone();
            let named = name.clone();
            let mut chain = task::spawn_blocking(move || {
                chain(&looked, &named, track, since.as_deref()).map_err(Kept::from)
            })
            .await?
            .map_err(Kept::into_error)?;
            match &chain.take {
                Some(_) => progress.log(
                    "info",
                    format!(
                        "taking the log of {name} since the last backup, compression {compression}"
                    ),
                ),
                None => progress.log("info", format!("dumping {name}, compression {compression}")),
            }
            let method = compression.clone();
            let take = chain.take.take();

            let mut sink = store.sink(&object_key).await?;
            let handle = Handle::current();
            let shared = progress.clone();
            // The pipeline blocks, so it gets its own thread and the reports keep flowing.
            let (sink, result) = task::spawn_blocking(move || {
                let tool = match &take {
                    Some(Take::Logs(files)) => engines::dump_increment(&database, files),
                    Some(Take::Oplog { since, to }) => engines::dump_oplog(&database, since, to),
                    None => engines::dump(&database, &name, track),
                };
                let result = tool
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
            let finished = result.and_then(|(sha256, head)| {
                // The dump itself says where in the log it was taken.
                if let Some(server) = chain.server.take() {
                    let (file, offset) = binlog::dump_position(&head)
                        .ok_or("the dump does not say where in the log it was taken")?;
                    chain.to = Some(
                        Position {
                            server,
                            file,
                            offset,
                        }
                        .to_string(),
                    );
                }
                Ok(sha256)
            });
            match finished {
                Ok(sha256) => {
                    sink.commit().await?;
                    progress.log("ok", format!("stored {object_key}"));
                    let positions = chain
                        .to
                        .as_deref()
                        .map(|to| (chain.from.as_deref().unwrap_or_default(), to));
                    Ok(Output {
                        manifest: Some(manifest::sign(
                            state,
                            &object_key,
                            &sha256,
                            &method,
                            positions,
                        )?),
                        sha256: Some(sha256),
                        result: None,
                        position_from: chain.from,
                        position_to: chain.to,
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
            position,
            increments,
            until,
        } => {
            // Every signature first, and that each increment continues the one before.
            trusted(
                state,
                &progress,
                &object_key,
                &sha256,
                &compression,
                position.as_deref().map(|to| ("", to)),
                manifest.as_deref(),
            )?;
            let mut reached = position;
            for link in &increments {
                trusted(
                    state,
                    &progress,
                    &link.object_key,
                    &link.sha256,
                    &link.compression,
                    Some((&link.from, &link.to)),
                    link.manifest.as_deref(),
                )?;
                if reached.as_deref() != Some(link.from.as_str()) {
                    return Err("the chain has a gap: not restoring".into());
                }
                reached = Some(link.to.clone());
            }
            let identity = String::from_utf8(open(state, &identity, "backup-key")?)?;
            let (store, _) = store(state, &destination).await?;

            let temporary = fetch(&store, &object_key, &sha256, &progress).await?;
            let mut replays = Vec::with_capacity(increments.len());
            for link in increments {
                let key = String::from_utf8(open(state, &link.identity, "backup-key")?)?;
                let file = fetch(&store, &link.object_key, &link.sha256, &progress).await?;
                replays.push((link, key, file));
            }

            let (database, _tunnel) = database(state, &sealed, ssh.as_ref(), &progress).await?;
            let file = temporary.reopen()?;
            let shared = progress.clone();
            blocking(move || {
                // MongoDB replaces what the backup holds; replaying SQL needs a clean start.
                if !replays.is_empty() && database.engine != "mongodb" {
                    engines::require_empty(&database, &name)
                        // A database that does not exist yet is as empty as it gets.
                        .map_err(text)?;
                }
                let (tool, created) = engines::restore(&database, &name, &source).map_err(text)?;
                if created {
                    shared.log("info", format!("created {name}"));
                }
                shared.log("info", format!("restoring into {name}"));
                pipeline::restore(tool, &compression, &identity, file, &shared).map_err(text)?;
                let until = until.unwrap_or_default();
                let count = replays.len();
                for (at, (link, key, file)) in replays.into_iter().enumerate() {
                    shared.log("info", format!("replaying increment {} of {count}", at + 1));
                    let tool = engines::restore_increment(
                        &database, &name, &source, &link.from, &link.to, &until,
                    )
                    .map_err(text)?;
                    pipeline::restore(
                        tool,
                        &link.compression,
                        &key,
                        file.reopen().map_err(|e| e.to_string())?,
                        &shared,
                    )
                    .map_err(text)?;
                }
                if !until.is_empty() && count > 0 {
                    shared.log("info", format!("replayed up to {until} UTC"));
                }
                Ok(())
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
