//! Finished backups: listing, starting backups and restores, deleting.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use dbm_shared::jobs::{dump_extension, extension, valid_target};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, QueryBuilder};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState, Error, audit,
    auth::{Auth, Ip},
    jobs::{NewJob, enqueue},
    keys,
    resources::Queued,
};

const PAGE: i64 = 100;

/// Keeps object keys predictable whatever the server was named.
fn slug(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

pub async fn run_backup(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Path(id): Path<Uuid>,
) -> Result<Json<Queued>, Error> {
    me.require_operator()?;
    let (job_id, name) = queue_backup(&app, id, &me.name).await?;
    audit::record(&app.db, Some(&me), "backup", &name, &ip, true).await?;
    Ok(Json(Queued { job_id }))
}

/// Queues one run of a backup entry; returns the job and the entry's name.
pub async fn queue_backup(app: &AppState, id: Uuid, by: &str) -> Result<(Uuid, String), Error> {
    let (name, server_id, server, agent, database, destination_id, compression, level, engine): (
        String, Uuid, String, Uuid, String, Uuid, String, i32, String,
    ) = sqlx::query_as(
        "SELECT b.name, s.id, s.name, s.agent_id, b.database_name, b.destination_id, b.compression, b.level, s.engine
         FROM backup_configs b JOIN database_servers s ON s.id = b.server_id WHERE b.id = $1",
    )
    .bind(id)
    .fetch_optional(&app.db)
    .await?
    .ok_or(Error::NotFound)?;

    let now = OffsetDateTime::now_utc();
    let stamp = format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
    let unique = &Uuid::new_v4().simple().to_string()[..8];
    let object_key = format!(
        "{}/{database}/{stamp}-{unique}{}{}.age",
        slug(&server),
        dump_extension(&engine),
        extension(&compression)
    );
    let job = NewJob {
        backup_config_id: Some(id),
        server_id: Some(server_id),
        destination_id: Some(destination_id),
        database_name: Some(database),
        object_key: Some(object_key),
        compression: Some(compression),
        level: Some(level),
        key_id: Some(keys::current(app).await?),
        // The server's name: it becomes the snapshot's source.
        ..NewJob::new("backup", agent, server, by)
    };
    match enqueue(&app.db, job).await {
        Ok(job_id) => Ok((job_id, name)),
        Err(Error::Conflict("duplicate")) => Err(Error::Conflict("already_running")),
        Err(other) => Err(other),
    }
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Snapshot {
    id: Uuid,
    server_name: String,
    database_name: String,
    destination: String,
    agent_id: Uuid,
    object_key: String,
    size: i64,
    sha256: String,
    compression: String,
    engine: String,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    /// `None` when kept until rotated or removed by hand.
    #[serde(with = "time::serde::rfc3339::option")]
    expires_at: Option<OffsetDateTime>,
}

const SELECT: &str = "SELECT s.id, s.server_name, s.database_name, d.name AS destination, d.agent_id, s.object_key,
    s.size, s.sha256, s.compression, s.engine, s.created_at, s.expires_at FROM snapshots s JOIN destinations d ON d.id = s.destination_id";

#[derive(Deserialize)]
pub struct Filter {
    #[serde(default, with = "time::serde::rfc3339::option")]
    from: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    to: Option<OffsetDateTime>,
    server: Option<String>,
    before: Option<Uuid>,
}

#[derive(Serialize)]
pub struct Page {
    items: Vec<Snapshot>,
    next: Option<Uuid>,
}

pub async fn list(
    State(app): State<AppState>,
    Auth(_): Auth,
    Query(filter): Query<Filter>,
) -> Result<Json<Page>, Error> {
    let mut sql = QueryBuilder::<Postgres>::new(SELECT);
    sql.push(" WHERE true");
    if let Some(from) = filter.from {
        sql.push(" AND s.created_at >= ").push_bind(from);
    }
    if let Some(to) = filter.to {
        sql.push(" AND s.created_at < ").push_bind(to);
    }
    if let Some(server) = filter.server.filter(|s| !s.is_empty()) {
        sql.push(" AND s.server_name = ").push_bind(server);
    }
    if let Some(before) = filter.before {
        sql.push(" AND (s.created_at, s.id) < (SELECT created_at, id FROM snapshots WHERE id = ")
            .push_bind(before)
            .push(")");
    }
    sql.push(" ORDER BY s.created_at DESC, s.id DESC LIMIT ")
        .push_bind(PAGE + 1);
    let mut items: Vec<Snapshot> = sql.build_query_as().fetch_all(&app.db).await?;
    let more = items.len() as i64 > PAGE;
    items.truncate(PAGE as usize);
    let next = items.last().filter(|_| more).map(|s| s.id);
    Ok(Json(Page { items, next }))
}

pub async fn get(
    State(app): State<AppState>,
    Auth(_): Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<Snapshot>, Error> {
    let mut sql = QueryBuilder::<Postgres>::new(SELECT);
    sql.push(" WHERE s.id = ").push_bind(id);
    Ok(Json(
        sql.build_query_as()
            .fetch_optional(&app.db)
            .await?
            .ok_or(Error::NotFound)?,
    ))
}

#[derive(Deserialize)]
pub struct Restore {
    server_id: Uuid,
    database_name: String,
}

pub async fn restore(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Path(id): Path<Uuid>,
    Json(body): Json<Restore>,
) -> Result<Json<Queued>, Error> {
    me.require_operator()?;
    let (destination_id, source_agent, object_key, engine): (Uuid, Uuid, String, String) =
        sqlx::query_as(
            "SELECT s.destination_id, d.agent_id, s.object_key, s.engine
         FROM snapshots s JOIN destinations d ON d.id = s.destination_id WHERE s.id = $1",
        )
        .bind(id)
        .fetch_optional(&app.db)
        .await?
        .ok_or(Error::NotFound)?;
    let (server, agent, target_engine): (String, Uuid, String) =
        sqlx::query_as("SELECT name, agent_id, engine FROM database_servers WHERE id = $1")
            .bind(body.server_id)
            .fetch_optional(&app.db)
            .await?
            .ok_or(Error::NotFound)?;
    // Only that agent can open the destination's credentials.
    if agent != source_agent {
        return Err(Error::Invalid("different_agent"));
    }
    if engine != target_engine {
        return Err(Error::Invalid("different_engine"));
    }
    if !valid_target(&engine, &body.database_name) {
        return Err(Error::Invalid("database_name"));
    }
    let job = NewJob {
        server_id: Some(body.server_id),
        destination_id: Some(destination_id),
        snapshot_id: Some(id),
        database_name: Some(body.database_name.clone()),
        object_key: Some(object_key.clone()),
        ..NewJob::new(
            "restore",
            agent,
            format!("{server}/{}", body.database_name),
            &me.name,
        )
    };
    let job_id = enqueue(&app.db, job).await?;
    let target = format!("{object_key} -> {server}/{}", body.database_name);
    audit::record(&app.db, Some(&me), "restore", &target, &ip, true).await?;
    Ok(Json(Queued { job_id }))
}

/// Asks the agent to remove the file; returns the job and the object key.
pub async fn queue_delete(app: &AppState, id: Uuid, by: &str) -> Result<(Uuid, String), Error> {
    let (server, destination_id, agent, object_key): (String, Uuid, Uuid, String) = sqlx::query_as(
        "SELECT s.server_name, s.destination_id, d.agent_id, s.object_key
         FROM snapshots s JOIN destinations d ON d.id = s.destination_id WHERE s.id = $1",
    )
    .bind(id)
    .fetch_optional(&app.db)
    .await?
    .ok_or(Error::NotFound)?;
    let job = NewJob {
        destination_id: Some(destination_id),
        snapshot_id: Some(id),
        object_key: Some(object_key.clone()),
        ..NewJob::new("delete_snapshot", agent, server, by)
    };
    Ok((enqueue(&app.db, job).await?, object_key))
}

/// The agent removes the file; the record goes once it confirms.
pub async fn delete(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Path(id): Path<Uuid>,
) -> Result<Json<Queued>, Error> {
    me.require_admin()?;
    let (job_id, object_key) = queue_delete(&app, id, &me.name).await?;
    let target = format!("snapshot {object_key}");
    audit::record(&app.db, Some(&me), "delete", &target, &ip, true).await?;
    Ok(Json(Queued { job_id }))
}

/// The agent makes a short-lived link to the (still encrypted) file in the storage.
pub async fn download(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Path(id): Path<Uuid>,
) -> Result<Json<Queued>, Error> {
    me.require_operator()?;
    let (server, destination_id, agent, object_key): (String, Uuid, Uuid, String) = sqlx::query_as(
        "SELECT s.server_name, s.destination_id, d.agent_id, s.object_key
         FROM snapshots s JOIN destinations d ON d.id = s.destination_id WHERE s.id = $1",
    )
    .bind(id)
    .fetch_optional(&app.db)
    .await?
    .ok_or(Error::NotFound)?;
    let job = NewJob {
        destination_id: Some(destination_id),
        snapshot_id: Some(id),
        object_key: Some(object_key.clone()),
        ..NewJob::new("presign_snapshot", agent, server, &me.name)
    };
    let job_id = enqueue(&app.db, job).await?;
    let target = format!("snapshot {object_key}");
    audit::record(&app.db, Some(&me), "download", &target, &ip, true).await?;
    Ok(Json(Queued { job_id }))
}

#[derive(Serialize)]
pub struct Link {
    url: String,
}

/// Hands the link out once.
pub async fn download_link(
    State(app): State<AppState>,
    Auth(me): Auth,
    Path(job): Path<Uuid>,
) -> Result<Json<Link>, Error> {
    me.require_operator()?;
    let sealed: Vec<u8> = sqlx::query_scalar(
        "UPDATE jobs j SET result = NULL
         FROM (SELECT id, result FROM jobs WHERE id = $1 AND created_by = $2 FOR UPDATE) old
         WHERE j.id = old.id AND old.result IS NOT NULL RETURNING old.result",
    )
    .bind(job)
    .bind(&me.name)
    .fetch_optional(&app.db)
    .await?
    .ok_or(Error::NotFound)?;
    let url = app
        .master
        .open("jobs", "result", &job.to_string(), &sealed)
        .map_err(Error::Internal)?;
    let url = String::from_utf8(url).map_err(|e| Error::Internal(e.to_string()))?;
    Ok(Json(Link { url }))
}
