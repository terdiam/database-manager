use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
};
use dbm_shared::{EnrollRequest, EnrollResponse, HeartbeatRequest, OFFLINE_AFTER_SECS, e2ee};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState, Error, audit,
    auth::{Auth, Ip, new_token, token_hash},
};

const TOKEN_MINUTES: i32 = 60;

#[derive(Serialize, sqlx::FromRow)]
pub struct Agent {
    id: Uuid,
    name: String,
    version: String,
    #[serde(with = "time::serde::rfc3339")]
    last_seen_at: OffsetDateTime,
    online: bool,
    #[serde(skip)]
    public_key: Option<Vec<u8>>,
    /// The browser seals secrets to this.
    #[sqlx(skip)]
    key: Option<String>,
    /// To compare with the one the agent prints; absent for agents enrolled before keys existed.
    #[sqlx(skip)]
    fingerprint: Option<String>,
}

pub async fn list(State(app): State<AppState>, Auth(_): Auth) -> Result<Json<Vec<Agent>>, Error> {
    let mut agents: Vec<Agent> = sqlx::query_as(
        "SELECT id, name, version, public_key, last_seen_at, last_seen_at > now() - make_interval(secs => $1) AS online
         FROM agents ORDER BY name LIMIT 500",
    )
    .bind(OFFLINE_AFTER_SECS as f64)
    .fetch_all(&app.db)
    .await?;
    for agent in &mut agents {
        agent.fingerprint = agent.public_key.as_deref().map(e2ee::fingerprint);
        agent.key = agent.public_key.as_deref().map(e2ee::encode);
    }
    Ok(Json(agents))
}

#[derive(Deserialize)]
pub struct NewToken {
    name: String,
}

#[derive(Serialize)]
pub struct Token {
    token: String,
    expires_in_minutes: i32,
}

/// One-time token an administrator hands to a new agent.
pub async fn create_token(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Json(body): Json<NewToken>,
) -> Result<Json<Token>, Error> {
    me.require_admin()?;
    let name = body.name.trim();
    if name.is_empty() || name.len() > 64 {
        return Err(Error::Invalid("name"));
    }
    let taken: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM agents WHERE name = $1)")
        .bind(name)
        .fetch_one(&app.db)
        .await?;
    if taken {
        return Err(Error::Conflict("agent_name_taken"));
    }
    let token = new_token();
    sqlx::query("DELETE FROM enrollment_tokens WHERE expires_at <= now() OR used_at IS NOT NULL")
        .execute(&app.db)
        .await?;
    sqlx::query("INSERT INTO enrollment_tokens (token_hash, agent_name, expires_at) VALUES ($1, $2, now() + make_interval(mins => $3))")
        .bind(token_hash(&token))
        .bind(name)
        .bind(TOKEN_MINUTES)
        .execute(&app.db)
        .await?;
    audit::record(
        &app.db,
        Some(&me),
        "create",
        &format!("enrollment token for agent {name}"),
        &ip,
        true,
    )
    .await?;
    Ok(Json(Token {
        token,
        expires_in_minutes: TOKEN_MINUTES,
    }))
}

#[derive(Deserialize)]
pub struct Rename {
    name: String,
}

pub async fn rename(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Path(id): Path<Uuid>,
    Json(body): Json<Rename>,
) -> Result<StatusCode, Error> {
    me.require_admin()?;
    let name = body.name.trim();
    if name.is_empty() || name.len() > 64 {
        return Err(Error::Invalid("name"));
    }
    let found = sqlx::query("UPDATE agents SET name = $2 WHERE id = $1")
        .bind(id)
        .bind(name)
        .execute(&app.db)
        .await
        .map_err(|e| match Error::from(e) {
            Error::Conflict("duplicate") => Error::Conflict("agent_name_taken"),
            other => other,
        })?;
    if found.rows_affected() == 0 {
        return Err(Error::NotFound);
    }
    let target = format!("agent {name}");
    audit::record(&app.db, Some(&me), "update", &target, &ip, true).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Removing an agent revokes its credential.
pub async fn remove(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    me.require_admin()?;
    let name: String = sqlx::query_scalar("DELETE FROM agents WHERE id = $1 RETURNING name")
        .bind(id)
        .fetch_optional(&app.db)
        .await?
        .ok_or(Error::NotFound)?;
    audit::record(
        &app.db,
        Some(&me),
        "delete",
        &format!("agent {name}"),
        &ip,
        true,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn clipped(version: &str) -> String {
    version.chars().take(32).collect()
}

/// Ends an agent's credential at once and returns a one-time token that gives the real
/// agent a new one. Its databases, destinations and backups stay.
pub async fn revoke(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Path(id): Path<Uuid>,
) -> Result<Json<Token>, Error> {
    me.require_admin()?;
    let mut tx = app.db.begin().await?;
    // A hash nothing hashes to, until the agent is linked again.
    let name: String =
        sqlx::query_scalar("UPDATE agents SET token_hash = $2 WHERE id = $1 RETURNING name")
            .bind(id)
            .bind(token_hash(&new_token()))
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::NotFound)?;
    // Whoever held the credential may be working on these.
    sqlx::query(
        "UPDATE jobs SET status = 'failed', error = 'the agent credential was revoked', finished_at = now(), lease = NULL
         WHERE agent_id = $1 AND status IN ('pending', 'running')",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    let token = new_token();
    sqlx::query("DELETE FROM enrollment_tokens WHERE agent_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO enrollment_tokens (token_hash, agent_name, agent_id, expires_at)
         VALUES ($1, $2, $3, now() + make_interval(mins => $4))",
    )
    .bind(token_hash(&token))
    .bind(&name)
    .bind(id)
    .bind(TOKEN_MINUTES)
    .execute(&mut *tx)
    .await?;
    let target = format!("agent {name} credential revoked");
    audit::record(&mut *tx, Some(&me), "update", &target, &ip, true).await?;
    tx.commit().await?;
    Ok(Json(Token {
        token,
        expires_in_minutes: TOKEN_MINUTES,
    }))
}

pub async fn enroll(
    State(app): State<AppState>,
    Ip(ip): Ip,
    Json(body): Json<EnrollRequest>,
) -> Result<Json<EnrollResponse>, Error> {
    let public_key = e2ee::decode(&body.public_key)
        .ok()
        .filter(|key| key.len() == 32)
        .ok_or(Error::Invalid("public_key"))?;

    let mut tx = app.db.begin().await?;
    // The UPDATE claims the token atomically, so it works exactly once.
    let (name, existing): (String, Option<Uuid>) = sqlx::query_as(
        "UPDATE enrollment_tokens SET used_at = now()
         WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now() RETURNING agent_name, agent_id",
    )
    .bind(token_hash(&body.token))
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(Error::Unauthorized)?;

    let secret = new_token();
    if let Some(agent_id) = existing {
        // Secrets are sealed to its key, so only the agent holding that key is linked again.
        let linked = sqlx::query(
            "UPDATE agents SET token_hash = $2, version = $3 WHERE id = $1 AND public_key = $4",
        )
        .bind(agent_id)
        .bind(token_hash(&secret))
        .bind(clipped(&body.version))
        .bind(public_key)
        .execute(&mut *tx)
        .await?;
        if linked.rows_affected() == 0 {
            return Err(Error::Invalid("public_key"));
        }
        let target = format!("agent {name} linked again");
        audit::record(&mut *tx, None, "update", &target, &ip, true).await?;
        tx.commit().await?;
        return Ok(Json(EnrollResponse { agent_id, secret }));
    }
    let agent_id: Uuid = sqlx::query_scalar(
        "INSERT INTO agents (name, token_hash, version, public_key) VALUES ($1, $2, $3, $4)
         ON CONFLICT (name) DO NOTHING RETURNING id",
    )
    .bind(&name)
    .bind(token_hash(&secret))
    .bind(clipped(&body.version))
    .bind(public_key)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(Error::Conflict("agent_name_taken"))?;
    audit::record(
        &mut *tx,
        None,
        "create",
        &format!("agent {name} enrolled"),
        &ip,
        true,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(EnrollResponse { agent_id, secret }))
}

/// The agent behind a bearer credential. Every agent route starts here.
pub async fn authenticate(app: &AppState, headers: &HeaderMap) -> Result<Uuid, Error> {
    let secret = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(Error::Unauthorized)?;
    let id: Uuid = sqlx::query_scalar("SELECT id FROM agents WHERE token_hash = $1")
        .bind(token_hash(secret))
        .fetch_optional(&app.db)
        .await?
        .ok_or(Error::Unauthorized)?;
    // Any call proves the agent is alive; written at most every 20 seconds.
    sqlx::query("UPDATE agents SET last_seen_at = now() WHERE id = $1 AND last_seen_at < now() - interval '20 seconds'")
        .bind(id)
        .execute(&app.db)
        .await?;
    Ok(id)
}

pub async fn heartbeat(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<HeartbeatRequest>,
) -> Result<StatusCode, Error> {
    let id = authenticate(&app, &headers).await?;
    sqlx::query("UPDATE agents SET last_seen_at = now(), version = $1 WHERE id = $2")
        .bind(clipped(&body.version))
        .bind(id)
        .execute(&app.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
