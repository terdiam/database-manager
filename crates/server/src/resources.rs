//! Database servers, destinations and backup configurations.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use dbm_shared::{e2ee, jobs::COMPRESSIONS, jobs::valid_database_name};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AppState, Error, audit,
    auth::{Auth, Ip},
    jobs::{NewJob, enqueue},
    schedule,
};
use time::OffsetDateTime;

/// Sealed secrets are an encapsulated key plus a small JSON document.
fn sealed_bytes(text: &str) -> Result<Vec<u8>, Error> {
    e2ee::decode(text)
        .ok()
        .filter(|bytes| (48..=8192).contains(&bytes.len()))
        .ok_or(Error::Invalid("secret"))
}

fn named(value: &str, code: &'static str) -> Result<String, Error> {
    let value = value.trim();
    if value.is_empty() || value.len() > 64 {
        return Err(Error::Invalid(code));
    }
    Ok(value.to_owned())
}

fn field(value: &str, code: &'static str) -> Result<(), Error> {
    if value.len() > 255 {
        Err(Error::Invalid(code))
    } else {
        Ok(())
    }
}

async fn require_agent_key(app: &AppState, agent: Uuid) -> Result<(), Error> {
    let has: Option<bool> =
        sqlx::query_scalar("SELECT public_key IS NOT NULL FROM agents WHERE id = $1")
            .bind(agent)
            .fetch_optional(&app.db)
            .await?;
    match has {
        Some(true) => Ok(()),
        _ => Err(Error::Invalid("agent")),
    }
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Server {
    id: Uuid,
    name: String,
    engine: String,
    agent_id: Uuid,
    agent: String,
    host: String,
    port: i32,
    username: String,
}

pub async fn list_servers(
    State(app): State<AppState>,
    Auth(_): Auth,
) -> Result<Json<Vec<Server>>, Error> {
    let rows = sqlx::query_as(
        "SELECT s.id, s.name, s.engine, s.agent_id, a.name AS agent, s.host, s.port, s.username
         FROM database_servers s JOIN agents a ON a.id = s.agent_id ORDER BY s.name LIMIT 500",
    )
    .fetch_all(&app.db)
    .await?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
pub struct NewServer {
    /// Chosen by the browser: the secret is sealed to it before the row exists.
    id: Uuid,
    name: String,
    agent_id: Uuid,
    host: String,
    port: u16,
    username: String,
    secret: String,
}

pub async fn create_server(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Json(body): Json<NewServer>,
) -> Result<StatusCode, Error> {
    me.require_operator()?;
    let name = named(&body.name, "name")?;
    field(&body.host, "host")?;
    field(&body.username, "username")?;
    let secret = sealed_bytes(&body.secret)?;
    require_agent_key(&app, body.agent_id).await?;
    sqlx::query(
        "INSERT INTO database_servers (id, name, engine, agent_id, host, port, username, secret)
         VALUES ($1, $2, 'postgresql', $3, $4, $5, $6, $7)",
    )
    .bind(body.id)
    .bind(&name)
    .bind(body.agent_id)
    .bind(body.host.trim())
    .bind(i32::from(body.port))
    .bind(body.username.trim())
    .bind(secret)
    .execute(&app.db)
    .await?;
    audit::record(
        &app.db,
        Some(&me),
        "create",
        &format!("database {name}"),
        &ip,
        true,
    )
    .await?;
    Ok(StatusCode::CREATED)
}

pub async fn delete_server(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    me.require_operator()?;
    let name: String =
        sqlx::query_scalar("DELETE FROM database_servers WHERE id = $1 RETURNING name")
            .bind(id)
            .fetch_optional(&app.db)
            .await?
            .ok_or(Error::NotFound)?;
    audit::record(
        &app.db,
        Some(&me),
        "delete",
        &format!("database {name}"),
        &ip,
        true,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct Queued {
    pub job_id: Uuid,
}

pub async fn test_server(
    State(app): State<AppState>,
    Auth(me): Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<Queued>, Error> {
    me.require_operator()?;
    let (name, agent): (String, Uuid) =
        sqlx::query_as("SELECT name, agent_id FROM database_servers WHERE id = $1")
            .bind(id)
            .fetch_optional(&app.db)
            .await?
            .ok_or(Error::NotFound)?;
    let job = NewJob {
        server_id: Some(id),
        ..NewJob::new("test_database", agent, name, &me.name)
    };
    Ok(Json(Queued {
        job_id: enqueue(&app.db, job).await?,
    }))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Destination {
    id: Uuid,
    name: String,
    kind: String,
    agent_id: Uuid,
    agent: String,
    endpoint: String,
    bucket: String,
    prefix: String,
}

pub async fn list_destinations(
    State(app): State<AppState>,
    Auth(_): Auth,
) -> Result<Json<Vec<Destination>>, Error> {
    let rows = sqlx::query_as(
        "SELECT d.id, d.name, d.kind, d.agent_id, a.name AS agent, d.endpoint, d.bucket, d.prefix
         FROM destinations d JOIN agents a ON a.id = d.agent_id ORDER BY d.name LIMIT 500",
    )
    .fetch_all(&app.db)
    .await?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
pub struct NewDestination {
    id: Uuid,
    name: String,
    agent_id: Uuid,
    endpoint: String,
    bucket: String,
    prefix: String,
    secret: String,
}

pub async fn create_destination(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Json(body): Json<NewDestination>,
) -> Result<StatusCode, Error> {
    me.require_operator()?;
    let name = named(&body.name, "name")?;
    field(&body.endpoint, "endpoint")?;
    field(&body.bucket, "bucket")?;
    field(&body.prefix, "prefix")?;
    let secret = sealed_bytes(&body.secret)?;
    require_agent_key(&app, body.agent_id).await?;
    sqlx::query(
        "INSERT INTO destinations (id, name, kind, agent_id, endpoint, bucket, prefix, secret)
         VALUES ($1, $2, 's3', $3, $4, $5, $6, $7)",
    )
    .bind(body.id)
    .bind(&name)
    .bind(body.agent_id)
    .bind(body.endpoint.trim())
    .bind(body.bucket.trim())
    .bind(body.prefix.trim())
    .bind(secret)
    .execute(&app.db)
    .await?;
    audit::record(
        &app.db,
        Some(&me),
        "create",
        &format!("destination {name}"),
        &ip,
        true,
    )
    .await?;
    Ok(StatusCode::CREATED)
}

pub async fn delete_destination(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    me.require_operator()?;
    let name: String = sqlx::query_scalar("DELETE FROM destinations WHERE id = $1 RETURNING name")
        .bind(id)
        .fetch_optional(&app.db)
        .await?
        .ok_or(Error::NotFound)?;
    audit::record(
        &app.db,
        Some(&me),
        "delete",
        &format!("destination {name}"),
        &ip,
        true,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn test_destination(
    State(app): State<AppState>,
    Auth(me): Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<Queued>, Error> {
    me.require_operator()?;
    let (name, agent): (String, Uuid) =
        sqlx::query_as("SELECT name, agent_id FROM destinations WHERE id = $1")
            .bind(id)
            .fetch_optional(&app.db)
            .await?
            .ok_or(Error::NotFound)?;
    let job = NewJob {
        destination_id: Some(id),
        ..NewJob::new("test_destination", agent, name, &me.name)
    };
    Ok(Json(Queued {
        job_id: enqueue(&app.db, job).await?,
    }))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Backup {
    id: Uuid,
    name: String,
    server_id: Uuid,
    server: String,
    database_name: String,
    destination_id: Uuid,
    destination: String,
    compression: String,
    level: i32,
    schedule: Option<String>,
    timezone: String,
    #[serde(with = "time::serde::rfc3339::option")]
    next_run_at: Option<OffsetDateTime>,
    retention_unit: Option<String>,
    retention_value: Option<i32>,
    keep_daily: Option<i32>,
    keep_weekly: Option<i32>,
    keep_monthly: Option<i32>,
}

pub async fn list_backups(
    State(app): State<AppState>,
    Auth(_): Auth,
) -> Result<Json<Vec<Backup>>, Error> {
    let rows = sqlx::query_as(
        "SELECT b.id, b.name, b.server_id, s.name AS server, b.database_name, b.destination_id,
                d.name AS destination, b.compression, b.level, b.schedule, b.timezone, b.next_run_at,
                b.retention_unit, b.retention_value, b.keep_daily, b.keep_weekly, b.keep_monthly
         FROM backup_configs b
         JOIN database_servers s ON s.id = b.server_id
         JOIN destinations d ON d.id = b.destination_id
         ORDER BY b.name LIMIT 500",
    )
    .fetch_all(&app.db)
    .await?;
    Ok(Json(rows))
}

/// When a backup entry runs and how long its snapshots are kept.
#[derive(Deserialize, Default)]
pub struct Plan {
    /// Five-field cron pattern; absent means manual only.
    schedule: Option<String>,
    #[serde(default)]
    timezone: String,
    /// `days`, `months`, `gfs`, or absent to keep everything.
    retention_unit: Option<String>,
    retention_value: Option<i32>,
    keep_daily: Option<i32>,
    keep_weekly: Option<i32>,
    keep_monthly: Option<i32>,
}

struct Planned {
    timezone: String,
    next_run_at: Option<OffsetDateTime>,
    value: Option<i32>,
    keep: [Option<i32>; 3],
}

impl Plan {
    fn check(&self) -> Result<Planned, Error> {
        let timezone = if self.timezone.is_empty() {
            "UTC"
        } else {
            &self.timezone
        };
        if timezone.parse::<chrono_tz::Tz>().is_err() {
            return Err(Error::Invalid("timezone"));
        }
        let next_run_at = self
            .schedule
            .as_deref()
            .map(|pattern| schedule::next_after(pattern, timezone, OffsetDateTime::now_utc()))
            .transpose()?;
        let bad = Error::Invalid("retention");
        let (value, keep) = match self.retention_unit.as_deref() {
            None => (None, [None; 3]),
            Some(unit @ ("days" | "months")) => {
                let max = if unit == "days" { 3650 } else { 120 };
                match self.retention_value {
                    Some(value) if (1..=max).contains(&value) => (Some(value), [None; 3]),
                    _ => return Err(bad),
                }
            }
            Some("gfs") => {
                let keep = [self.keep_daily, self.keep_weekly, self.keep_monthly]
                    .map(|k| Some(k.unwrap_or(0)));
                let all = keep.iter().flatten();
                if all.clone().any(|k| !(0..=1000).contains(k)) || all.sum::<i32>() == 0 {
                    return Err(bad);
                }
                (None, keep)
            }
            Some(_) => return Err(bad),
        };
        Ok(Planned {
            timezone: timezone.to_owned(),
            next_run_at,
            value,
            keep,
        })
    }
}

#[derive(Deserialize)]
pub struct NewBackup {
    name: String,
    server_id: Uuid,
    database_name: String,
    destination_id: Uuid,
    compression: String,
    level: i32,
    #[serde(flatten)]
    plan: Plan,
}

pub async fn create_backup(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Json(body): Json<NewBackup>,
) -> Result<StatusCode, Error> {
    me.require_operator()?;
    let name = named(&body.name, "name")?;
    if !valid_database_name(&body.database_name) {
        return Err(Error::Invalid("database_name"));
    }
    let max_level = if body.compression == "zstd" { 19 } else { 9 };
    if !COMPRESSIONS.contains(&body.compression.as_str()) || !(1..=max_level).contains(&body.level)
    {
        return Err(Error::Invalid("compression"));
    }
    let planned = body.plan.check()?;
    // The agent that dumps is the one that uploads, so both must belong to it.
    let same: Option<bool> = sqlx::query_scalar(
        "SELECT s.agent_id = d.agent_id FROM database_servers s, destinations d WHERE s.id = $1 AND d.id = $2",
    )
    .bind(body.server_id)
    .bind(body.destination_id)
    .fetch_optional(&app.db)
    .await?;
    match same {
        Some(true) => {}
        Some(false) => return Err(Error::Invalid("different_agent")),
        None => return Err(Error::NotFound),
    }
    sqlx::query(
        "INSERT INTO backup_configs (name, server_id, database_name, destination_id, compression, level,
             schedule, timezone, next_run_at, retention_unit, retention_value, keep_daily, keep_weekly,
             keep_monthly)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
    )
    .bind(&name)
    .bind(body.server_id)
    .bind(&body.database_name)
    .bind(body.destination_id)
    .bind(&body.compression)
    .bind(body.level)
    .bind(&body.plan.schedule)
    .bind(planned.timezone)
    .bind(planned.next_run_at)
    .bind(&body.plan.retention_unit)
    .bind(planned.value)
    .bind(planned.keep[0])
    .bind(planned.keep[1])
    .bind(planned.keep[2])
    .execute(&app.db)
    .await?;
    audit::record(
        &app.db,
        Some(&me),
        "create",
        &format!("backup {name}"),
        &ip,
        true,
    )
    .await?;
    Ok(StatusCode::CREATED)
}

/// Changes the schedule and retention. Snapshots already made keep their own expiry.
pub async fn update_backup(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Path(id): Path<Uuid>,
    Json(plan): Json<Plan>,
) -> Result<StatusCode, Error> {
    me.require_operator()?;
    let planned = plan.check()?;
    let name: String = sqlx::query_scalar(
        "UPDATE backup_configs SET schedule = $2, timezone = $3, next_run_at = $4,
             retention_since = CASE WHEN (retention_unit, retention_value, keep_daily, keep_weekly, keep_monthly)
                 IS DISTINCT FROM ($5::text, $6::integer, $7::integer, $8::integer, $9::integer)
                 THEN now() ELSE retention_since END,
             retention_unit = $5, retention_value = $6, keep_daily = $7, keep_weekly = $8, keep_monthly = $9
         WHERE id = $1 RETURNING name",
    )
    .bind(id)
    .bind(&plan.schedule)
    .bind(planned.timezone)
    .bind(planned.next_run_at)
    .bind(&plan.retention_unit)
    .bind(planned.value)
    .bind(planned.keep[0])
    .bind(planned.keep[1])
    .bind(planned.keep[2])
    .fetch_optional(&app.db)
    .await?
    .ok_or(Error::NotFound)?;
    let target = format!("backup {name} schedule and retention");
    audit::record(&app.db, Some(&me), "update", &target, &ip, true).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_backup(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    me.require_operator()?;
    let name: String =
        sqlx::query_scalar("DELETE FROM backup_configs WHERE id = $1 RETURNING name")
            .bind(id)
            .fetch_optional(&app.db)
            .await?
            .ok_or(Error::NotFound)?;
    audit::record(
        &app.db,
        Some(&me),
        "delete",
        &format!("backup {name}"),
        &ip,
        true,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
