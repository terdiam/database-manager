//! Database servers, destinations and backup configurations.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use dbm_shared::{
    e2ee,
    jobs::{COMPRESSIONS, ENGINES, supports_incremental, valid_target},
};
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
pub fn sealed_bytes(text: &str) -> Result<Vec<u8>, Error> {
    e2ee::decode(text)
        .ok()
        .filter(|bytes| (48..=8192).contains(&bytes.len()))
        .ok_or(Error::Invalid("secret"))
}

pub fn named(value: &str, code: &'static str) -> Result<String, Error> {
    let value = value.trim();
    if value.is_empty() || value.len() > 64 {
        return Err(Error::Invalid(code));
    }
    Ok(value.to_owned())
}

pub fn field(value: &str, code: &'static str) -> Result<(), Error> {
    if value.len() > 255 {
        Err(Error::Invalid(code))
    } else {
        Ok(())
    }
}

pub async fn require_agent_key(app: &AppState, agent: Uuid) -> Result<(), Error> {
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
    ssl_mode: Option<String>,
    ssh_config_id: Option<Uuid>,
    ssh: Option<String>,
}

pub async fn list_servers(
    State(app): State<AppState>,
    Auth(_): Auth,
) -> Result<Json<Vec<Server>>, Error> {
    let rows = sqlx::query_as(
        "SELECT s.id, s.name, s.engine, s.agent_id, a.name AS agent, s.host, s.port, s.username, s.ssl_mode,
                s.ssh_config_id, c.name AS ssh
         FROM database_servers s JOIN agents a ON a.id = s.agent_id
         LEFT JOIN ssh_configs c ON c.id = s.ssh_config_id ORDER BY s.name LIMIT 500",
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
    #[serde(default = "postgresql")]
    engine: String,
    host: String,
    port: u16,
    username: String,
    #[serde(default)]
    ssl_mode: Option<String>,
    #[serde(default)]
    ssh_config_id: Option<Uuid>,
    secret: String,
}

/// A tunnel belongs to one agent, and SQLite is a file: nothing to forward.
async fn check_tunnel(
    app: &AppState,
    tunnel: Option<Uuid>,
    agent: Uuid,
    engine: &str,
) -> Result<(), Error> {
    let Some(tunnel) = tunnel else {
        return Ok(());
    };
    if engine == "sqlite" {
        return Err(Error::Invalid("ssh_sqlite"));
    }
    let owner: Option<Uuid> = sqlx::query_scalar("SELECT agent_id FROM ssh_configs WHERE id = $1")
        .bind(tunnel)
        .fetch_optional(&app.db)
        .await?;
    match owner {
        Some(owner) if owner == agent => Ok(()),
        Some(_) => Err(Error::Invalid("different_agent")),
        None => Err(Error::NotFound),
    }
}

fn postgresql() -> String {
    "postgresql".into()
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
    if !ENGINES.contains(&body.engine.as_str()) {
        return Err(Error::Invalid("engine"));
    }
    let secret = sealed_bytes(&body.secret)?;
    require_agent_key(&app, body.agent_id).await?;
    check_tunnel(&app, body.ssh_config_id, body.agent_id, &body.engine).await?;
    sqlx::query(
        "INSERT INTO database_servers (id, name, engine, agent_id, host, port, username, secret, ssl_mode,
             ssh_config_id)
         VALUES ($1, $2, $9, $3, $4, $5, $6, $7, $8, $10)",
    )
    .bind(body.id)
    .bind(&name)
    .bind(body.agent_id)
    .bind(body.host.trim())
    .bind(i32::from(body.port))
    .bind(body.username.trim())
    .bind(secret)
    .bind(ssl_mode(body.ssl_mode.as_deref())?)
    .bind(&body.engine)
    .bind(body.ssh_config_id)
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

fn ssl_mode(value: Option<&str>) -> Result<Option<&str>, Error> {
    match value {
        None | Some("disable" | "prefer" | "require" | "verify-full") => Ok(value),
        Some(_) => Err(Error::Invalid("ssl_mode")),
    }
}

/// Host, user and password are sealed together, so changing any of them needs all of them again.
#[derive(Deserialize)]
pub struct Connection {
    host: String,
    port: u16,
    username: String,
    ssl_mode: String,
    secret: String,
}

#[derive(Deserialize)]
pub struct ServerEdit {
    name: String,
    connection: Option<Connection>,
    /// Always the full choice: `null` connects directly.
    #[serde(default)]
    ssh_config_id: Option<Uuid>,
}

pub async fn update_server(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Path(id): Path<Uuid>,
    Json(body): Json<ServerEdit>,
) -> Result<StatusCode, Error> {
    me.require_operator()?;
    let name = named(&body.name, "name")?;
    let (agent, engine): (Uuid, String) =
        sqlx::query_as("SELECT agent_id, engine FROM database_servers WHERE id = $1")
            .bind(id)
            .fetch_optional(&app.db)
            .await?
            .ok_or(Error::NotFound)?;
    check_tunnel(&app, body.ssh_config_id, agent, &engine).await?;
    let mut tx = app.db.begin().await?;
    sqlx::query("UPDATE database_servers SET name = $2, ssh_config_id = $3 WHERE id = $1")
        .bind(id)
        .bind(&name)
        .bind(body.ssh_config_id)
        .execute(&mut *tx)
        .await?;
    if let Some(connection) = &body.connection {
        field(&connection.host, "host")?;
        field(&connection.username, "username")?;
        sqlx::query(
            "UPDATE database_servers SET host = $2, port = $3, username = $4, ssl_mode = $5, secret = $6
             WHERE id = $1",
        )
        .bind(id)
        .bind(connection.host.trim())
        .bind(i32::from(connection.port))
        .bind(connection.username.trim())
        .bind(ssl_mode(Some(&connection.ssl_mode))?)
        .bind(sealed_bytes(&connection.secret)?)
        .execute(&mut *tx)
        .await?;
    }
    let what = if body.connection.is_some() {
        " and connection"
    } else {
        ""
    };
    let target = format!("database {name}{what}");
    audit::record(&mut *tx, Some(&me), "update", &target, &ip, true).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
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
    region: Option<String>,
    virtual_host: Option<bool>,
    public_endpoint: Option<String>,
    username: Option<String>,
}

pub async fn list_destinations(
    State(app): State<AppState>,
    Auth(_): Auth,
) -> Result<Json<Vec<Destination>>, Error> {
    let rows = sqlx::query_as(
        "SELECT d.id, d.name, d.kind, d.agent_id, a.name AS agent, d.endpoint, d.bucket, d.prefix,
                d.region, d.virtual_host, d.public_endpoint, d.username
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
    #[serde(default = "s3")]
    kind: String,
    #[serde(flatten)]
    options: Options,
    secret: String,
}

fn s3() -> String {
    "s3".into()
}

#[derive(Deserialize, Default)]
pub struct Options {
    #[serde(default)]
    region: Option<String>,
    #[serde(default)]
    virtual_host: Option<bool>,
    #[serde(default)]
    public_endpoint: Option<String>,
    /// SFTP, FTP and SMB.
    #[serde(default)]
    username: Option<String>,
}

impl Options {
    fn check(&self) -> Result<(), Error> {
        field(self.region.as_deref().unwrap_or_default(), "endpoint")?;
        field(self.username.as_deref().unwrap_or_default(), "username")?;
        field(
            self.public_endpoint.as_deref().unwrap_or_default(),
            "endpoint",
        )
    }
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
    body.options.check()?;
    if !["s3", "sftp", "ftp", "smb"].contains(&body.kind.as_str()) {
        return Err(Error::Invalid("kind"));
    }
    let secret = sealed_bytes(&body.secret)?;
    require_agent_key(&app, body.agent_id).await?;
    sqlx::query(
        "INSERT INTO destinations (id, name, kind, agent_id, endpoint, bucket, prefix, secret, region,
             virtual_host, public_endpoint, username)
         VALUES ($1, $2, $12, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(body.id)
    .bind(&name)
    .bind(body.agent_id)
    .bind(body.endpoint.trim())
    .bind(body.bucket.trim())
    .bind(body.prefix.trim())
    .bind(secret)
    .bind(&body.options.region)
    .bind(body.options.virtual_host)
    .bind(&body.options.public_endpoint)
    .bind(&body.options.username)
    .bind(&body.kind)
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

#[derive(Deserialize)]
pub struct Storage {
    endpoint: String,
    bucket: String,
    prefix: String,
    #[serde(flatten)]
    options: Options,
    secret: String,
}

#[derive(Deserialize)]
pub struct DestinationEdit {
    name: String,
    connection: Option<Storage>,
}

pub async fn update_destination(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Path(id): Path<Uuid>,
    Json(body): Json<DestinationEdit>,
) -> Result<StatusCode, Error> {
    me.require_operator()?;
    let name = named(&body.name, "name")?;
    let mut tx = app.db.begin().await?;
    let (bucket, prefix): (String, String) =
        sqlx::query_as("UPDATE destinations SET name = $2 WHERE id = $1 RETURNING bucket, prefix")
            .bind(id)
            .bind(&name)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::NotFound)?;
    if let Some(storage) = &body.connection {
        field(&storage.endpoint, "endpoint")?;
        field(&storage.bucket, "bucket")?;
        field(&storage.prefix, "prefix")?;
        storage.options.check()?;
        // Stored backups are found by their place: moving it would orphan them.
        if storage.bucket.trim() != bucket || storage.prefix.trim() != prefix {
            let used: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM snapshots WHERE destination_id = $1)
                     OR EXISTS (SELECT 1 FROM jobs WHERE destination_id = $1 AND status IN ('pending', 'running'))",
            )
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
            if used {
                return Err(Error::Conflict("has_snapshots"));
            }
        }
        sqlx::query(
            "UPDATE destinations SET endpoint = $2, bucket = $3, prefix = $4, secret = $5, region = $6,
                 virtual_host = $7, public_endpoint = $8, username = $9
             WHERE id = $1",
        )
        .bind(id)
        .bind(storage.endpoint.trim())
        .bind(storage.bucket.trim())
        .bind(storage.prefix.trim())
        .bind(sealed_bytes(&storage.secret)?)
        .bind(&storage.options.region)
        .bind(storage.options.virtual_host)
        .bind(&storage.options.public_endpoint)
        .bind(&storage.options.username)
        .execute(&mut *tx)
        .await?;
    }
    let what = if body.connection.is_some() {
        " and connection"
    } else {
        ""
    };
    let target = format!("destination {name}{what}");
    audit::record(&mut *tx, Some(&me), "update", &target, &ip, true).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
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
    /// Set on incremental entries: the full entry they follow, and its name.
    base_id: Option<Uuid>,
    base: Option<String>,
}

pub async fn list_backups(
    State(app): State<AppState>,
    Auth(_): Auth,
) -> Result<Json<Vec<Backup>>, Error> {
    let rows = sqlx::query_as(
        "SELECT b.id, b.name, b.server_id, s.name AS server, b.database_name, b.destination_id,
                d.name AS destination, b.compression, b.level, b.schedule, b.timezone, b.next_run_at,
                b.retention_unit, b.retention_value, b.keep_daily, b.keep_weekly, b.keep_monthly,
                b.base_id, (SELECT f.name FROM backup_configs f WHERE f.id = b.base_id) AS base
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

/// An incremental entry chooses only its name and when it runs.
#[derive(Deserialize)]
pub struct NewIncrement {
    name: String,
    base_id: Uuid,
    schedule: Option<String>,
    #[serde(default)]
    timezone: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum BackupBody {
    Increment(NewIncrement),
    Full(NewBackup),
}

/// Everything the followers take from their full entry.
const INHERIT: &str =
    "UPDATE backup_configs c SET server_id = f.server_id, database_name = f.database_name,
    destination_id = f.destination_id, compression = f.compression, level = f.level,
    retention_unit = f.retention_unit, retention_value = f.retention_value
    FROM backup_configs f WHERE c.base_id = f.id";

/// Creates an incremental entry, or changes one when `id` is given.
async fn save_increment(
    app: &AppState,
    body: &NewIncrement,
    id: Option<Uuid>,
) -> Result<String, Error> {
    let name = named(&body.name, "name")?;
    let (engine, unit, nested): (String, Option<String>, bool) = sqlx::query_as(
        "SELECT s.engine, c.retention_unit, c.base_id IS NOT NULL FROM backup_configs c
         JOIN database_servers s ON s.id = c.server_id WHERE c.id = $1",
    )
    .bind(body.base_id)
    .fetch_optional(&app.db)
    .await?
    .ok_or(Error::Invalid("base"))?;
    if nested || !supports_incremental(&engine) {
        return Err(Error::Invalid("base"));
    }
    // Chains expire as a whole, which rotation by day, week and month cannot express.
    if unit.as_deref() == Some("gfs") {
        return Err(Error::Invalid("retention_chain"));
    }
    let planned = Plan {
        schedule: body.schedule.clone(),
        timezone: body.timezone.clone(),
        ..Plan::default()
    }
    .check()?;
    let mut tx = app.db.begin().await?;
    let sql = match id {
        None => {
            "INSERT INTO backup_configs (name, base_id, schedule, timezone, next_run_at, server_id,
                 database_name, destination_id, compression, level)
             SELECT $1, f.id, $3, $4, $5, f.server_id, f.database_name, f.destination_id, f.compression, f.level
             FROM backup_configs f WHERE f.id = $2 AND $6::uuid IS NULL RETURNING id"
        }
        // The kind of an entry never changes.
        Some(_) => {
            "UPDATE backup_configs SET name = $1, base_id = $2, schedule = $3, timezone = $4, next_run_at = $5
             WHERE id = $6 AND base_id IS NOT NULL RETURNING id"
        }
    };
    let saved: Uuid = sqlx::query_scalar(sql)
        .bind(&name)
        .bind(body.base_id)
        .bind(&body.schedule)
        .bind(planned.timezone)
        .bind(planned.next_run_at)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error::NotFound)?;
    sqlx::query(sqlx::AssertSqlSafe(format!("{INHERIT} AND c.id = $1")))
        .bind(saved)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(name)
}

/// The rules a backup entry must meet, new or edited.
async fn check_backup(app: &AppState, body: &NewBackup) -> Result<(String, Planned), Error> {
    let name = named(&body.name, "name")?;
    let engine: String = sqlx::query_scalar("SELECT engine FROM database_servers WHERE id = $1")
        .bind(body.server_id)
        .fetch_optional(&app.db)
        .await?
        .ok_or(Error::NotFound)?;
    if !valid_target(&engine, &body.database_name) {
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
        Some(true) => Ok((name, planned)),
        Some(false) => Err(Error::Invalid("different_agent")),
        None => Err(Error::NotFound),
    }
}

pub async fn create_backup(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Json(body): Json<BackupBody>,
) -> Result<StatusCode, Error> {
    me.require_operator()?;
    let body = match body {
        BackupBody::Increment(body) => {
            let name = save_increment(&app, &body, None).await?;
            let target = format!("incremental backup {name}");
            audit::record(&app.db, Some(&me), "create", &target, &ip, true).await?;
            return Ok(StatusCode::CREATED);
        }
        BackupBody::Full(body) => body,
    };
    let (name, planned) = check_backup(&app, &body).await?;
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
    let target = format!("backup {name}");
    audit::record(&app.db, Some(&me), "create", &target, &ip, true).await?;
    Ok(StatusCode::CREATED)
}

/// Snapshots already made keep their own expiry, and rotation only judges those made after
/// the retention or what is backed up last changed.
pub async fn update_backup(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Path(id): Path<Uuid>,
    Json(body): Json<BackupBody>,
) -> Result<StatusCode, Error> {
    me.require_operator()?;
    let body = match body {
        BackupBody::Increment(body) => {
            let name = save_increment(&app, &body, Some(id)).await?;
            let target = format!("incremental backup {name}");
            audit::record(&app.db, Some(&me), "update", &target, &ip, true).await?;
            return Ok(StatusCode::NO_CONTENT);
        }
        BackupBody::Full(body) => body,
    };
    let (name, planned) = check_backup(&app, &body).await?;
    let (incremental, followed): (bool, bool) = sqlx::query_as(
        "SELECT base_id IS NOT NULL, EXISTS (SELECT 1 FROM backup_configs f WHERE f.base_id = c.id)
         FROM backup_configs c WHERE c.id = $1",
    )
    .bind(id)
    .fetch_optional(&app.db)
    .await?
    .ok_or(Error::NotFound)?;
    if incremental {
        return Err(Error::Invalid("base"));
    }
    if followed {
        let engine: String =
            sqlx::query_scalar("SELECT engine FROM database_servers WHERE id = $1")
                .bind(body.server_id)
                .fetch_one(&app.db)
                .await?;
        if !supports_incremental(&engine) {
            return Err(Error::Invalid("base"));
        }
        if body.plan.retention_unit.as_deref() == Some("gfs") {
            return Err(Error::Invalid("retention_chain"));
        }
    }
    let found = sqlx::query(
        "UPDATE backup_configs SET
             retention_since = CASE WHEN (server_id, database_name, destination_id, retention_unit,
                     retention_value, keep_daily, keep_weekly, keep_monthly)
                 IS DISTINCT FROM ($3::uuid, $4::text, $5::uuid, $11::text, $12::integer, $13::integer,
                     $14::integer, $15::integer)
                 THEN now() ELSE retention_since END,
             name = $2, server_id = $3, database_name = $4, destination_id = $5, compression = $6,
             level = $7, schedule = $8, timezone = $9, next_run_at = $10, retention_unit = $11,
             retention_value = $12, keep_daily = $13, keep_weekly = $14, keep_monthly = $15
         WHERE id = $1",
    )
    .bind(id)
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
    if found.rows_affected() == 0 {
        return Err(Error::NotFound);
    }
    sqlx::query(sqlx::AssertSqlSafe(format!("{INHERIT} AND f.id = $1")))
        .bind(id)
        .execute(&app.db)
        .await?;
    let target = format!("backup {name}");
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
