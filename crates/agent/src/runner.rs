//! Carries out one job.

use std::time::Duration;

use dbm_shared::{
    e2ee::{self, SecretContext},
    jobs::{DatabaseSecret, Job, S3Secret, Sealed, Task},
};
use serde::de::DeserializeOwned;
use tokio::{runtime::Handle, task};

use crate::{Result, pipeline, postgres, progress::Progress, state::State, storage};

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

/// Blocking work off the async thread, so reports keep flowing.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> std::result::Result<T, String> + Send + 'static,
) -> Result<T> {
    Ok(task::spawn_blocking(work).await??)
}

fn text(error: Box<dyn std::error::Error>) -> String {
    error.to_string()
}

/// Returns the SHA-256 of the stored file for backups.
pub async fn run(state: &State, job: Job, progress: Progress) -> Result<Option<String>> {
    match job.task {
        Task::TestDatabase { database } => {
            let secret: DatabaseSecret = open_json(state, &database, "database")?;
            let version = blocking(move || postgres::server_version(&secret).map_err(text)).await?;
            progress.log("ok", format!("connected, PostgreSQL {version}"));
            Ok(None)
        }
        Task::TestDestination { destination } => {
            let secret: S3Secret = open_json(state, &destination, "destination")?;
            storage::test(&storage::open(&secret)?).await?;
            progress.log("ok", "wrote, read and deleted a test file");
            Ok(None)
        }
        Task::Backup {
            database,
            destination,
            name,
            compression,
            level,
            recipients,
            object_key,
        } => {
            let database: DatabaseSecret = open_json(state, &database, "database")?;
            let destination: S3Secret = open_json(state, &destination, "destination")?;
            let operator = storage::open(&destination)?;
            progress.log("info", format!("dumping {name}, compression {compression}"));

            let mut writer = storage::Upload::start(&operator, &object_key).await?;
            let handle = Handle::current();
            let shared = progress.clone();
            let (writer, result) = task::spawn_blocking(move || {
                let result = postgres::dump(&database, &name)
                    .and_then(|dump| {
                        pipeline::backup(
                            dump,
                            &compression,
                            level,
                            &recipients,
                            &mut writer,
                            handle,
                            &shared,
                        )
                    })
                    .map_err(text);
                (writer, result)
            })
            .await?;
            match result {
                Ok(sha256) => {
                    progress.log("ok", format!("stored {object_key}"));
                    Ok(Some(sha256))
                }
                Err(error) => {
                    let mut writer = writer;
                    if tokio::time::timeout(Duration::from_secs(30), writer.abort())
                        .await
                        .is_err()
                    {
                        progress.log("warn", "could not abort the unfinished upload");
                    }
                    Err(error.into())
                }
            }
        }
        Task::Restore {
            database,
            destination,
            identity,
            object_key,
            sha256,
            compression,
            name,
        } => {
            let database: DatabaseSecret = open_json(state, &database, "database")?;
            let destination: S3Secret = open_json(state, &destination, "destination")?;
            let identity = String::from_utf8(open(state, &identity, "backup-key")?)?;
            let operator = storage::open(&destination)?;

            progress.log("info", format!("downloading {object_key}"));
            let temporary = tempfile::NamedTempFile::new()?;
            let mut file = tokio::fs::File::from_std(temporary.reopen()?);
            if storage::download(&operator, &object_key, &mut file, &progress).await? != sha256 {
                return Err(
                    "checksum mismatch: the stored file is not the one that was backed up".into(),
                );
            }
            progress.log("ok", "checksum verified");

            let file = temporary.reopen()?;
            let shared = progress.clone();
            blocking(move || {
                if postgres::ensure_database(&database, &name).map_err(text)? {
                    shared.log("info", format!("created database {name}"));
                }
                shared.log("info", format!("restoring into {name}"));
                let command = postgres::restore(&database, &name).map_err(text)?;
                pipeline::restore(command, &compression, &identity, file, &shared).map_err(text)
            })
            .await?;
            progress.log("ok", "restore completed");
            Ok(None)
        }
        Task::DeleteSnapshot {
            destination,
            object_key,
        } => {
            let secret: S3Secret = open_json(state, &destination, "destination")?;
            storage::open(&secret)?.delete(&object_key).await?;
            progress.log("ok", format!("deleted {object_key}"));
            Ok(None)
        }
    }
}
