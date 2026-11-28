//! The job queue: what users see, and what agents claim, report and finish.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use dbm_shared::{
    e2ee,
    jobs::{
        Claim, Finish, Increment, Job, LEASE_SECS, LogLine, MAX_LOG_LINES, Report, ReportReply,
        Sealed, Task,
    },
};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, PgPool, Postgres, QueryBuilder};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState, Error, agents, audit,
    auth::{Auth, Ip},
    keys, snapshots,
};

const PAGE: i64 = 100;

pub struct NewJob {
    pub kind: &'static str,
    pub agent_id: Uuid,
    pub target: String,
    pub created_by: String,
    pub backup_config_id: Option<Uuid>,
    pub server_id: Option<Uuid>,
    pub destination_id: Option<Uuid>,
    pub snapshot_id: Option<Uuid>,
    pub database_name: Option<String>,
    pub object_key: Option<String>,
    pub compression: Option<String>,
    pub level: Option<i32>,
    pub key_id: Option<i32>,
    pub ssh_config_id: Option<Uuid>,
    /// For an incremental backup: the snapshot it continues.
    pub parent_id: Option<Uuid>,
    /// For a restore of a chain: stop replaying here.
    pub restore_until: Option<OffsetDateTime>,
}

impl NewJob {
    pub fn new(kind: &'static str, agent_id: Uuid, target: String, created_by: &str) -> Self {
        Self {
            kind,
            agent_id,
            target,
            created_by: created_by.to_owned(),
            backup_config_id: None,
            server_id: None,
            destination_id: None,
            snapshot_id: None,
            database_name: None,
            object_key: None,
            compression: None,
            level: None,
            key_id: None,
            ssh_config_id: None,
            parent_id: None,
            restore_until: None,
        }
    }
}

pub async fn enqueue(db: &PgPool, job: NewJob) -> Result<Uuid, Error> {
    let id = sqlx::query_scalar(
        "INSERT INTO jobs (kind, agent_id, target, created_by, backup_config_id, server_id, destination_id,
                           snapshot_id, database_name, object_key, compression, level, key_id, ssh_config_id,
                           parent_id, restore_until)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16) RETURNING id",
    )
    .bind(job.kind)
    .bind(job.agent_id)
    .bind(job.target)
    .bind(job.created_by)
    .bind(job.backup_config_id)
    .bind(job.server_id)
    .bind(job.destination_id)
    .bind(job.snapshot_id)
    .bind(job.database_name)
    .bind(job.object_key)
    .bind(job.compression)
    .bind(job.level)
    .bind(job.key_id)
    .bind(job.ssh_config_id)
    .bind(job.parent_id)
    .bind(job.restore_until)
    .fetch_one(db)
    .await?;
    Ok(id)
}

/// Jobs of agents that went silent. They are not retried: a half-done restore must be looked at.
pub async fn expire_stale(db: &PgPool) -> Result<(), Error> {
    sqlx::query(
        "UPDATE jobs SET status = 'failed', error = 'the agent stopped responding', finished_at = now(), lease = NULL
         WHERE status = 'running' AND lease_expires_at < now()",
    )
    .execute(db)
    .await?;
    Ok(())
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Row {
    id: Uuid,
    kind: String,
    status: String,
    target: String,
    agent: String,
    bytes: i64,
    error: Option<String>,
    created_by: String,
    cancel_requested: bool,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    started_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    finished_at: Option<OffsetDateTime>,
}

const SELECT: &str = "SELECT j.id, j.kind, j.status, j.target, a.name AS agent, j.bytes, j.error, j.created_by,
    j.cancel_requested, j.created_at, j.started_at, j.finished_at FROM jobs j JOIN agents a ON a.id = j.agent_id";

#[derive(Deserialize)]
pub struct Filter {
    #[serde(default, with = "time::serde::rfc3339::option")]
    from: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    to: Option<OffsetDateTime>,
    status: Option<String>,
    before: Option<Uuid>,
}

#[derive(Serialize)]
pub struct Page {
    items: Vec<Row>,
    next: Option<Uuid>,
}

pub async fn list(
    State(app): State<AppState>,
    Auth(_): Auth,
    Query(filter): Query<Filter>,
) -> Result<Json<Page>, Error> {
    expire_stale(&app.db).await?;
    let mut sql = QueryBuilder::<Postgres>::new(SELECT);
    sql.push(" WHERE true");
    if let Some(from) = filter.from {
        sql.push(" AND j.created_at >= ").push_bind(from);
    }
    if let Some(to) = filter.to {
        sql.push(" AND j.created_at < ").push_bind(to);
    }
    if let Some(status) = filter.status.filter(|s| !s.is_empty()) {
        sql.push(" AND j.status = ").push_bind(status);
    }
    if let Some(before) = filter.before {
        sql.push(" AND (j.created_at, j.id) < (SELECT created_at, id FROM jobs WHERE id = ")
            .push_bind(before)
            .push(")");
    }
    sql.push(" ORDER BY j.created_at DESC, j.id DESC LIMIT ")
        .push_bind(PAGE + 1);
    let mut items: Vec<Row> = sql.build_query_as().fetch_all(&app.db).await?;
    let more = items.len() as i64 > PAGE;
    items.truncate(PAGE as usize);
    let next = items.last().filter(|_| more).map(|row| row.id);
    Ok(Json(Page { items, next }))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Line {
    id: i64,
    level: String,
    message: String,
}

#[derive(Serialize)]
pub struct Detail {
    #[serde(flatten)]
    job: Row,
    logs: Vec<Line>,
}

#[derive(Deserialize)]
pub struct After {
    #[serde(default)]
    after: i64,
}

/// Polled while a job runs: only lines newer than `after` are returned.
pub async fn get(
    State(app): State<AppState>,
    Auth(_): Auth,
    Path(id): Path<Uuid>,
    Query(query): Query<After>,
) -> Result<Json<Detail>, Error> {
    expire_stale(&app.db).await?;
    let mut sql = QueryBuilder::<Postgres>::new(SELECT);
    sql.push(" WHERE j.id = ").push_bind(id);
    let job: Row = sql
        .build_query_as()
        .fetch_optional(&app.db)
        .await?
        .ok_or(Error::NotFound)?;
    let logs = sqlx::query_as(
        "SELECT id, level, message FROM job_logs WHERE job_id = $1 AND id > $2 ORDER BY id LIMIT 500",
    )
    .bind(id)
    .bind(query.after)
    .fetch_all(&app.db)
    .await?;
    Ok(Json(Detail { job, logs }))
}

pub async fn cancel(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    me.require_operator()?;
    let target: String = sqlx::query_scalar(
        "UPDATE jobs SET cancel_requested = true,
             status = CASE WHEN status = 'pending' THEN 'cancelled' ELSE status END,
             finished_at = CASE WHEN status = 'pending' THEN now() ELSE finished_at END
         WHERE id = $1 AND status IN ('pending', 'running') RETURNING target",
    )
    .bind(id)
    .fetch_optional(&app.db)
    .await?
    .ok_or(Error::NotFound)?;
    audit::record(
        &app.db,
        Some(&me),
        "update",
        &format!("cancel job for {target}"),
        &ip,
        true,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(sqlx::FromRow)]
struct Claimed {
    id: Uuid,
    lease: Uuid,
    kind: String,
    server_id: Option<Uuid>,
    destination_id: Option<Uuid>,
    snapshot_id: Option<Uuid>,
    database_name: Option<String>,
    object_key: Option<String>,
    compression: Option<String>,
    level: Option<i32>,
    key_id: Option<i32>,
    ssh_config_id: Option<Uuid>,
    backup_config_id: Option<Uuid>,
    parent_id: Option<Uuid>,
    restore_until: Option<OffsetDateTime>,
}

#[derive(sqlx::FromRow)]
struct Link {
    object_key: String,
    sha256: String,
    compression: String,
    key_id: i32,
    manifest: Option<String>,
    database_name: String,
    position_from: Option<String>,
    position_to: Option<String>,
}

/// The form the database's own log tools take a time in; the agent runs them in UTC.
fn utc_text(at: OffsetDateTime) -> String {
    let at = at.to_offset(time::UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second()
    )
}

async fn sealed_from(db: &PgPool, sql: &'static str, id: Option<Uuid>) -> Result<Sealed, Error> {
    let id = id.ok_or(Error::Conflict("record_removed"))?;
    let secret: Vec<u8> = sqlx::query_scalar(sql)
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or(Error::Conflict("record_removed"))?;
    Ok(Sealed {
        id: id.to_string(),
        version: 1,
        data: e2ee::encode(&secret),
    })
}

/// Secrets are read at claim time and go only to the agent that owns the job.
async fn task(app: &AppState, agent: Uuid, job: &Claimed) -> Result<Task, Error> {
    let database = || {
        sealed_from(
            &app.db,
            "SELECT secret FROM database_servers WHERE id = $1",
            job.server_id,
        )
    };
    let destination = || {
        sealed_from(
            &app.db,
            "SELECT secret FROM destinations WHERE id = $1",
            job.destination_id,
        )
    };
    // The tunnel of the database this job talks to, if it has one.
    let tunnel = || async {
        let id: Option<Uuid> =
            sqlx::query_scalar("SELECT ssh_config_id FROM database_servers WHERE id = $1")
                .bind(job.server_id)
                .fetch_optional(&app.db)
                .await?
                .flatten();
        match id {
            Some(id) => sealed_from(
                &app.db,
                "SELECT secret FROM ssh_configs WHERE id = $1",
                Some(id),
            )
            .await
            .map(Some),
            None => Ok(None),
        }
    };
    let missing = || Error::Conflict("record_removed");
    Ok(match job.kind.as_str() {
        "backup" => Task::Backup {
            ssh: tunnel().await?,
            database: database().await?,
            destination: destination().await?,
            name: job.database_name.clone().ok_or_else(missing)?,
            compression: job.compression.clone().ok_or_else(missing)?,
            level: job.level.ok_or_else(missing)?,
            recipients: keys::recipients(app, job.key_id.ok_or_else(missing)?).await?,
            object_key: job.object_key.clone().ok_or_else(missing)?,
            since: match job.parent_id {
                Some(parent) => Some(
                    sqlx::query_scalar::<_, Option<String>>(
                        "SELECT position_to FROM snapshots WHERE id = $1",
                    )
                    .bind(parent)
                    .fetch_optional(&app.db)
                    .await?
                    .flatten()
                    .ok_or_else(missing)?,
                ),
                None => None,
            },
            // Only a backup that others follow needs the extra rights of reading positions.
            track: job.parent_id.is_none()
                && sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM backup_configs WHERE base_id = $1)",
                )
                .bind(job.backup_config_id)
                .fetch_one(&app.db)
                .await?,
        },
        "restore" => {
            // The chain from its full backup to the chosen one; a plain snapshot is a chain
            // of one.
            let mut chain: Vec<Link> = sqlx::query_as(
                "WITH RECURSIVE chain AS (
                     SELECT s.*, 0 AS depth FROM snapshots s WHERE s.id = $1
                     UNION ALL
                     SELECT s.*, c.depth + 1 FROM snapshots s JOIN chain c ON s.id = c.parent_id
                 )
                 SELECT object_key, sha256, compression, key_id, manifest, database_name,
                        position_from, position_to FROM chain ORDER BY depth DESC",
            )
            .bind(job.snapshot_id.ok_or_else(missing)?)
            .fetch_all(&app.db)
            .await?;
            if chain.is_empty() {
                return Err(missing());
            }
            let full = chain.remove(0);
            let mut increments = Vec::with_capacity(chain.len());
            for link in chain {
                increments.push(Increment {
                    identity: keys::seal_identity(app, link.key_id, agent, job.id).await?,
                    object_key: link.object_key,
                    sha256: link.sha256,
                    manifest: link.manifest,
                    compression: link.compression,
                    from: link.position_from.ok_or_else(missing)?,
                    to: link.position_to.ok_or_else(missing)?,
                });
            }
            Task::Restore {
                ssh: tunnel().await?,
                database: database().await?,
                destination: destination().await?,
                identity: keys::seal_identity(app, full.key_id, agent, job.id).await?,
                object_key: full.object_key,
                sha256: full.sha256,
                manifest: full.manifest,
                compression: full.compression,
                name: job.database_name.clone().ok_or_else(missing)?,
                source: full.database_name,
                position: full.position_to,
                increments,
                until: job.restore_until.map(utc_text),
            }
        }
        "test_database" => Task::TestDatabase {
            database: database().await?,
            ssh: tunnel().await?,
        },
        "test_ssh" => Task::TestSsh {
            ssh: sealed_from(
                &app.db,
                "SELECT secret FROM ssh_configs WHERE id = $1",
                job.ssh_config_id,
            )
            .await?,
        },
        "test_destination" => Task::TestDestination {
            destination: destination().await?,
        },
        "delete_snapshot" => Task::DeleteSnapshot {
            destination: destination().await?,
            object_key: job.object_key.clone().ok_or_else(missing)?,
        },
        "presign_snapshot" => Task::PresignSnapshot {
            destination: destination().await?,
            object_key: job.object_key.clone().ok_or_else(missing)?,
        },
        other => return Err(Error::Internal(format!("unknown job kind {other}"))),
    })
}

/// What the job list shows when a claimed job cannot be put together. Internal details
/// stay in the server log.
fn not_prepared(error: &Error) -> &'static str {
    match error {
        Error::Conflict("record_removed") => "a record this job needs is gone",
        Error::Conflict("agent_without_key") => {
            "the agent has no encryption key: revoke it and link it again"
        }
        _ => "the server could not prepare this job, see the server log",
    }
}

pub async fn claim(State(app): State<AppState>, headers: HeaderMap) -> Result<Json<Claim>, Error> {
    let agent = agents::authenticate(&app, &headers).await?;
    expire_stale(&app.db).await?;
    // SKIP LOCKED: two claims never receive the same row.
    let claimed: Option<Claimed> = sqlx::query_as(
        "UPDATE jobs SET status = 'running', lease = gen_random_uuid(), started_at = now(),
                lease_expires_at = now() + make_interval(secs => $2)
         WHERE id = (SELECT id FROM jobs WHERE agent_id = $1 AND status = 'pending'
                     ORDER BY created_at LIMIT 1 FOR UPDATE SKIP LOCKED)
         RETURNING id, lease, kind, server_id, destination_id, snapshot_id, database_name, object_key,
                   compression, level, key_id, ssh_config_id, backup_config_id, parent_id, restore_until",
    )
    .bind(agent)
    .bind(f64::from(LEASE_SECS))
    .fetch_optional(&app.db)
    .await?;
    let Some(claimed) = claimed else {
        return Ok(Json(Claim { job: None }));
    };
    match task(&app, agent, &claimed).await {
        Ok(task) => Ok(Json(Claim {
            job: Some(Job {
                id: claimed.id,
                lease: claimed.lease,
                task,
            }),
        })),
        Err(error) => {
            sqlx::query(
                "UPDATE jobs SET status = 'failed', error = $2, finished_at = now(), lease = NULL
                 WHERE id = $1",
            )
            .bind(claimed.id)
            .bind(not_prepared(&error))
            .execute(&app.db)
            .await?;
            tracing::warn!("job {} not prepared: {}", claimed.id, error.detail());
            Err(error)
        }
    }
}

async fn append_logs(tx: &mut PgConnection, job: Uuid, logs: &[LogLine]) -> Result<(), Error> {
    if logs.is_empty() {
        return Ok(());
    }
    let mut sql = QueryBuilder::<Postgres>::new("INSERT INTO job_logs (job_id, level, message) ");
    sql.push_values(logs.iter().take(MAX_LOG_LINES), |mut row, line| {
        let level = match line.level.as_str() {
            "error" | "warn" | "ok" => line.level.clone(),
            _ => "info".into(),
        };
        let message: String = line.message.chars().take(2000).collect();
        row.push_bind(job).push_bind(level).push_bind(message);
    });
    sql.build().execute(tx).await?;
    Ok(())
}

pub async fn report(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Report>,
) -> Result<Json<ReportReply>, Error> {
    let agent = agents::authenticate(&app, &headers).await?;
    let mut tx = app.db.begin().await?;
    // The lease fences out an attempt that was already declared dead.
    let cancel: bool = sqlx::query_scalar(
        "UPDATE jobs SET bytes = $4, lease_expires_at = now() + make_interval(secs => $5)
         WHERE id = $1 AND agent_id = $2 AND lease = $3 AND status = 'running' RETURNING cancel_requested",
    )
    .bind(body.id)
    .bind(agent)
    .bind(body.lease)
    .bind(body.bytes.max(0))
    .bind(f64::from(LEASE_SECS))
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(Error::Conflict("lease_lost"))?;
    append_logs(&mut tx, body.id, &body.logs).await?;
    tx.commit().await?;
    Ok(Json(ReportReply { cancel }))
}

pub async fn finish(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Finish>,
) -> Result<StatusCode, Error> {
    let agent = agents::authenticate(&app, &headers).await?;
    let failed = body.error.is_some();
    let sha256 = body
        .sha256
        .as_deref()
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()));
    let mut tx = app.db.begin().await?;
    let (kind, cancelled, snapshot_id): (String, bool, Option<Uuid>) = sqlx::query_as(
        "UPDATE jobs SET bytes = $4, error = $5, finished_at = now(), lease = NULL,
             status = CASE WHEN $6 AND cancel_requested THEN 'cancelled' WHEN $6 THEN 'failed' ELSE 'completed' END
         WHERE id = $1 AND agent_id = $2 AND lease = $3 AND status = 'running'
         RETURNING kind, cancel_requested, snapshot_id",
    )
    .bind(body.id)
    .bind(agent)
    .bind(body.lease)
    .bind(body.bytes.max(0))
    .bind(body.error.as_deref().map(|e| e.chars().take(2000).collect::<String>()))
    .bind(failed)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(Error::Conflict("lease_lost"))?;
    append_logs(&mut tx, body.id, &body.logs).await?;

    if !failed && kind == "backup" {
        let sha256 = sha256.ok_or(Error::Invalid("sha256"))?;
        let manifest = body.manifest.as_deref().filter(|m| m.len() <= 128);
        let position = |p: &Option<String>| p.clone().filter(|p| p.len() <= 255);
        // An increment joins its parent's chain; a full backup with a position starts one.
        sqlx::query(
            "INSERT INTO snapshots (id, job_id, backup_config_id, server_name, database_name, destination_id,
                                    object_key, size, sha256, compression, key_id, manifest, expires_at, engine,
                                    parent_id, chain_id, position_from, position_to)
             SELECT new.id, j.id, j.backup_config_id, j.target, j.database_name, j.destination_id, j.object_key,
                    j.bytes, $2, j.compression, j.key_id, $3,
                    CASE c.retention_unit
                        WHEN 'days' THEN now() + make_interval(days => c.retention_value)
                        WHEN 'months' THEN now() + make_interval(months => c.retention_value)
                    END,
                    coalesce(s.engine, 'postgresql'),
                    j.parent_id,
                    CASE WHEN j.parent_id IS NOT NULL
                             THEN (SELECT p.chain_id FROM snapshots p WHERE p.id = j.parent_id)
                         WHEN $5::text IS NOT NULL THEN new.id
                    END,
                    $4, $5
             FROM (SELECT uuidv7() AS id) new, jobs j LEFT JOIN backup_configs c ON c.id = j.backup_config_id
             LEFT JOIN database_servers s ON s.id = j.server_id WHERE j.id = $1",
        )
        .bind(body.id)
        .bind(sha256)
        .bind(manifest)
        .bind(position(&body.position_from))
        .bind(position(&body.position_to))
        .execute(&mut *tx)
        .await?;
    }
    if !failed && !cancelled && kind == "presign_snapshot" {
        // The link grants access to the file, so it is stored encrypted and only briefly.
        let url = body
            .result
            .as_deref()
            .filter(|u| u.len() <= 4096 && (u.starts_with("https://") || u.starts_with("http://")))
            .ok_or(Error::Invalid("result"))?;
        let sealed = app
            .master
            .seal("jobs", "result", &body.id.to_string(), url.as_bytes())
            .map_err(Error::Internal)?;
        sqlx::query("UPDATE jobs SET result = $2 WHERE id = $1")
            .bind(body.id)
            .bind(sealed)
            .execute(&mut *tx)
            .await?;
    }
    if !failed && !cancelled && kind == "delete_snapshot" {
        sqlx::query("DELETE FROM snapshots WHERE id = $1")
            .bind(snapshot_id)
            .execute(&mut *tx)
            .await?;
    }
    // The log the increment needed is gone: take a full backup now, so the chain starts over.
    let restart: Option<Uuid> = if failed && !cancelled && body.chain_broken && kind == "backup" {
        sqlx::query_scalar(
            "SELECT c.base_id FROM jobs j JOIN backup_configs c ON c.id = j.backup_config_id WHERE j.id = $1",
        )
        .bind(body.id)
        .fetch_optional(&mut *tx)
        .await?
        .flatten()
    } else {
        None
    };
    tx.commit().await?;
    if let Some(base) = restart
        && let Err(error) = snapshots::queue_backup(&app, base, "chain restart").await
    {
        tracing::warn!("could not start a new chain: {}", error.detail());
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_that_cannot_be_prepared_says_why_without_internal_details() {
        assert_eq!(
            not_prepared(&Error::Conflict("record_removed")),
            "a record this job needs is gone"
        );
        assert!(not_prepared(&Error::Conflict("agent_without_key")).contains("no encryption key"));
        let internal = not_prepared(&Error::Internal("password authentication failed".into()));
        assert!(internal.contains("server log") && !internal.contains("password"));
    }
}
