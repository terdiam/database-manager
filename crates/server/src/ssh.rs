//! SSH servers databases are reached through.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AppState, Error, audit,
    auth::{Auth, Ip},
    jobs::{NewJob, enqueue},
    resources::{Queued, field, named, require_agent_key, sealed_bytes},
};

#[derive(Serialize, sqlx::FromRow)]
pub struct SshConfig {
    id: Uuid,
    name: String,
    agent_id: Uuid,
    agent: String,
    host: String,
    port: i32,
    username: String,
    auth: String,
    /// How many databases go through it.
    used: i64,
}

pub async fn list(
    State(app): State<AppState>,
    Auth(_): Auth,
) -> Result<Json<Vec<SshConfig>>, Error> {
    let rows = sqlx::query_as(
        "SELECT c.id, c.name, c.agent_id, a.name AS agent, c.host, c.port, c.username, c.auth,
                (SELECT count(*) FROM database_servers d WHERE d.ssh_config_id = c.id) AS used
         FROM ssh_configs c JOIN agents a ON a.id = c.agent_id ORDER BY c.name LIMIT 500",
    )
    .fetch_all(&app.db)
    .await?;
    Ok(Json(rows))
}

/// Host, user and credentials are sealed together, so changing any of them needs all.
#[derive(Deserialize)]
pub struct Login {
    host: String,
    port: u16,
    username: String,
    auth: String,
    secret: String,
}

impl Login {
    fn check(&self) -> Result<Vec<u8>, Error> {
        for (value, code) in [(&self.host, "host"), (&self.username, "username")] {
            field(value, code)?;
            if value.trim().is_empty() || value.trim().starts_with('-') {
                return Err(Error::Invalid(code));
            }
        }
        if self.port == 0 || !matches!(self.auth.as_str(), "password" | "key") {
            return Err(Error::Invalid("secret"));
        }
        // A private key is larger than a password.
        sealed_bytes(&self.secret)
    }
}

#[derive(Deserialize)]
pub struct NewConfig {
    /// Chosen by the browser: the secret is sealed to it before the row exists.
    id: Uuid,
    name: String,
    agent_id: Uuid,
    #[serde(flatten)]
    login: Login,
}

pub async fn create(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Json(body): Json<NewConfig>,
) -> Result<StatusCode, Error> {
    me.require_operator()?;
    let name = named(&body.name, "name")?;
    let secret = body.login.check()?;
    require_agent_key(&app, body.agent_id).await?;
    sqlx::query(
        "INSERT INTO ssh_configs (id, name, agent_id, host, port, username, auth, secret)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(body.id)
    .bind(&name)
    .bind(body.agent_id)
    .bind(body.login.host.trim())
    .bind(i32::from(body.login.port))
    .bind(body.login.username.trim())
    .bind(&body.login.auth)
    .bind(secret)
    .execute(&app.db)
    .await?;
    let target = format!("SSH tunnel {name}");
    audit::record(&app.db, Some(&me), "create", &target, &ip, true).await?;
    Ok(StatusCode::CREATED)
}

#[derive(Deserialize)]
pub struct Edit {
    name: String,
    connection: Option<Login>,
}

pub async fn update(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Path(id): Path<Uuid>,
    Json(body): Json<Edit>,
) -> Result<StatusCode, Error> {
    me.require_operator()?;
    let name = named(&body.name, "name")?;
    let mut tx = app.db.begin().await?;
    let found = sqlx::query("UPDATE ssh_configs SET name = $2 WHERE id = $1")
        .bind(id)
        .bind(&name)
        .execute(&mut *tx)
        .await?;
    if found.rows_affected() == 0 {
        return Err(Error::NotFound);
    }
    if let Some(login) = &body.connection {
        let secret = login.check()?;
        sqlx::query(
            "UPDATE ssh_configs SET host = $2, port = $3, username = $4, auth = $5, secret = $6
             WHERE id = $1",
        )
        .bind(id)
        .bind(login.host.trim())
        .bind(i32::from(login.port))
        .bind(login.username.trim())
        .bind(&login.auth)
        .bind(secret)
        .execute(&mut *tx)
        .await?;
    }
    let what = if body.connection.is_some() {
        " and connection"
    } else {
        ""
    };
    let target = format!("SSH tunnel {name}{what}");
    audit::record(&mut *tx, Some(&me), "update", &target, &ip, true).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    me.require_operator()?;
    let name: String = sqlx::query_scalar("DELETE FROM ssh_configs WHERE id = $1 RETURNING name")
        .bind(id)
        .fetch_optional(&app.db)
        .await?
        .ok_or(Error::NotFound)?;
    let target = format!("SSH tunnel {name}");
    audit::record(&app.db, Some(&me), "delete", &target, &ip, true).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn test(
    State(app): State<AppState>,
    Auth(me): Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<Queued>, Error> {
    me.require_operator()?;
    let (name, agent): (String, Uuid) =
        sqlx::query_as("SELECT name, agent_id FROM ssh_configs WHERE id = $1")
            .bind(id)
            .fetch_optional(&app.db)
            .await?
            .ok_or(Error::NotFound)?;
    let job = NewJob {
        ssh_config_id: Some(id),
        ..NewJob::new("test_ssh", agent, name, &me.name)
    };
    Ok(Json(Queued {
        job_id: enqueue(&app.db, job).await?,
    }))
}
