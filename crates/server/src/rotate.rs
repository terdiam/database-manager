//! Replacing the master key or the transport key, from the server console while the server
//! is stopped. Whoever can run this already holds the database and the old key.

use dbm_shared::e2ee::{self, Keypair};
use sqlx::{PgPool, Postgres, Transaction};

use crate::{MasterKey, audit};

/// A running server holds this lock shared, so neither can happen under the other.
const RUNNING: &str = "SELECT pg_advisory_lock_shared(hashtext('dbm-server-running'))";
const EXCLUSIVE: &str = "SELECT pg_try_advisory_xact_lock(hashtext('dbm-server-running'))";

/// Marks the database as in use for as long as the returned connection is held.
pub async fn hold_running_lock(
    db: &PgPool,
) -> Result<sqlx::pool::PoolConnection<Postgres>, sqlx::Error> {
    let mut connection = db.acquire().await?;
    sqlx::query(RUNNING).execute(&mut *connection).await?;
    Ok(connection)
}

fn fail(error: sqlx::Error) -> String {
    error.to_string()
}

async fn stopped(db: &PgPool) -> Result<Transaction<'static, Postgres>, String> {
    let mut tx = db.begin().await.map_err(fail)?;
    let alone: bool = sqlx::query_scalar(EXCLUSIVE)
        .fetch_one(&mut *tx)
        .await
        .map_err(fail)?;
    if !alone {
        return Err("stop the server first: it is still using this database".into());
    }
    Ok(tx)
}

async fn done(mut tx: Transaction<'static, Postgres>, what: &str) -> Result<(), String> {
    audit::record(&mut *tx, None, "update", what, "console", true)
        .await
        .map_err(|_| "could not write the audit log")?;
    tx.commit().await.map_err(fail)
}

/// Every column encrypted with the master key: table, the column that names the row in the
/// encryption context, and how to find the row again.
const SEALED: [(&str, &str, &str, &str); 4] = [
    (
        "server_keys",
        "secret",
        "SELECT name, secret FROM server_keys FOR UPDATE",
        "UPDATE server_keys SET secret = $2 WHERE name = $1",
    ),
    (
        "backup_keys",
        "secret",
        "SELECT recipient, secret FROM backup_keys FOR UPDATE",
        "UPDATE backup_keys SET secret = $2 WHERE recipient = $1",
    ),
    (
        "notification_channels",
        "secret",
        "SELECT id::text, secret FROM notification_channels FOR UPDATE",
        "UPDATE notification_channels SET secret = $2 WHERE id = $1::uuid",
    ),
    (
        "jobs",
        "result",
        "SELECT id::text, result FROM jobs WHERE result IS NOT NULL FOR UPDATE",
        "UPDATE jobs SET result = $2 WHERE id = $1::uuid",
    ),
];

/// Re-encrypts every value under `new`. All of them or none: a wrong old key changes
/// nothing. Returns how many values there were.
pub async fn rotate_master_key(
    db: &PgPool,
    old: &MasterKey,
    new: &MasterKey,
) -> Result<u64, String> {
    let mut tx = stopped(db).await?;
    let mut count = 0;
    for (table, column, read, write) in SEALED {
        let rows: Vec<(String, Vec<u8>)> = sqlx::query_as(read)
            .fetch_all(&mut *tx)
            .await
            .map_err(fail)?;
        for (row, sealed) in rows {
            let plain = old
                .open(table, column, &row, &sealed)
                .map_err(|e| format!("{table}: {e}; nothing was changed"))?;
            let sealed = new.seal(table, column, &row, &plain)?;
            sqlx::query(write)
                .bind(&row)
                .bind(sealed)
                .execute(&mut *tx)
                .await
                .map_err(fail)?;
            count += 1;
        }
    }
    done(tx, "master key rotated from the server console").await?;
    Ok(count)
}

/// A new key for payload encryption; returns its fingerprint, which agents need.
pub async fn rotate_transport_key(db: &PgPool, master: &MasterKey) -> Result<String, String> {
    let mut tx = stopped(db).await?;
    let key = Keypair::generate();
    let sealed = master.seal("server_keys", "secret", "transport", &key.private)?;
    let old: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT secret FROM server_keys WHERE name = 'transport' FOR UPDATE")
            .fetch_optional(&mut *tx)
            .await
            .map_err(fail)?;
    // A wrong master key would leave a key the server cannot read.
    if let Some(old) = old {
        master.open("server_keys", "secret", "transport", &old)?;
    }
    sqlx::query(
        "INSERT INTO server_keys (name, public_key, secret) VALUES ('transport', $1, $2)
         ON CONFLICT (name) DO UPDATE SET public_key = excluded.public_key, secret = excluded.secret",
    )
    .bind(&key.public)
    .bind(sealed)
    .execute(&mut *tx)
    .await
    .map_err(fail)?;
    done(tx, "transport key rotated from the server console").await?;
    Ok(e2ee::fingerprint(&key.public))
}
