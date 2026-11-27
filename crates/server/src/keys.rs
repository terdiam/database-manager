//! age keypairs that backup files are encrypted to. Kept by the server, encrypted under
//! the master key, so restores work without asking anyone for a key.

use age::{secrecy::ExposeSecret, x25519};
use axum::{Json, extract::State};
use dbm_shared::{
    e2ee::{self, SecretContext},
    jobs::Sealed,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AppState, Error, audit,
    auth::{self, Auth, Ip},
};

const TABLE: &str = "backup_keys";

/// The key new backups use; created on first need.
pub async fn current(app: &AppState) -> Result<i32, Error> {
    let mut tx = app.db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('dbm-backup-keys'))")
        .execute(&mut *tx)
        .await?;
    let existing: Option<i32> =
        sqlx::query_scalar("SELECT id FROM backup_keys ORDER BY id DESC LIMIT 1")
            .fetch_optional(&mut *tx)
            .await?;
    let id = match existing {
        Some(id) => id,
        None => {
            let identity = x25519::Identity::generate();
            let recipient = identity.to_public().to_string();
            // The recipient names the row for the cipher: it is unique and known before the insert.
            let sealed = app
                .master
                .seal(
                    TABLE,
                    "secret",
                    &recipient,
                    identity.to_string().expose_secret().as_bytes(),
                )
                .map_err(Error::Internal)?;
            sqlx::query_scalar(
                "INSERT INTO backup_keys (recipient, secret) VALUES ($1, $2) RETURNING id",
            )
            .bind(recipient)
            .bind(sealed)
            .fetch_one(&mut *tx)
            .await?
        }
    };
    tx.commit().await?;
    Ok(id)
}

async fn load(app: &AppState, id: i32) -> Result<(String, String), Error> {
    let (recipient, sealed): (String, Vec<u8>) =
        sqlx::query_as("SELECT recipient, secret FROM backup_keys WHERE id = $1")
            .bind(id)
            .fetch_optional(&app.db)
            .await?
            .ok_or(Error::Conflict("record_removed"))?;
    let identity = app
        .master
        .open(TABLE, "secret", &recipient, &sealed)
        .map_err(Error::Internal)?;
    let identity = String::from_utf8(identity).map_err(|e| Error::Internal(e.to_string()))?;
    Ok((recipient, identity))
}

/// The server's recipient, plus the offline one from the configuration if any.
pub async fn recipients(app: &AppState, id: i32) -> Result<Vec<String>, Error> {
    let recipient: String = sqlx::query_scalar("SELECT recipient FROM backup_keys WHERE id = $1")
        .bind(id)
        .fetch_optional(&app.db)
        .await?
        .ok_or(Error::Conflict("record_removed"))?;
    Ok(std::iter::once(recipient)
        .chain(app.config.extra_recipient.clone())
        .collect())
}

/// The private key for one restore job, readable only by that job's agent.
pub async fn seal_identity(
    app: &AppState,
    key: i32,
    agent: Uuid,
    job: Uuid,
) -> Result<Sealed, Error> {
    let (_, identity) = load(app, key).await?;
    let public: Vec<u8> =
        sqlx::query_scalar::<_, Option<Vec<u8>>>("SELECT public_key FROM agents WHERE id = $1")
            .bind(agent)
            .fetch_one(&app.db)
            .await?
            .ok_or(Error::Conflict("agent_without_key"))?;
    let (agent, job) = (agent.to_string(), job.to_string());
    let context = SecretContext {
        agent_id: &agent,
        secret_id: &job,
        kind: "backup-key",
        version: 1,
    };
    let data = e2ee::seal_secret(&public, &context, identity.as_bytes())
        .map_err(|e| Error::Internal(e.to_string()))?;
    Ok(Sealed {
        id: job.clone(),
        version: 1,
        data: e2ee::encode(&data),
    })
}

#[derive(Serialize)]
pub struct Exported {
    id: i32,
    recipient: String,
    identity: String,
}

#[derive(Deserialize)]
pub struct Export {
    password: String,
}

/// For disaster recovery: without these keys, backups cannot be read if the server is lost.
pub async fn export(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Json(body): Json<Export>,
) -> Result<Json<Vec<Exported>>, Error> {
    me.require_admin()?;
    // A stolen session alone is not enough to take the keys.
    auth::confirm_password(&app, &me, &ip, body.password).await?;
    current(&app).await?;
    let ids: Vec<i32> = sqlx::query_scalar("SELECT id FROM backup_keys ORDER BY id")
        .fetch_all(&app.db)
        .await?;
    let mut keys = Vec::new();
    for id in ids {
        let (recipient, identity) = load(&app, id).await?;
        keys.push(Exported {
            id,
            recipient,
            identity,
        });
    }
    audit::record(
        &app.db,
        Some(&me),
        "download",
        "backup encryption keys",
        &ip,
        true,
    )
    .await?;
    Ok(Json(keys))
}
