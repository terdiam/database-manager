//! Alerts to Telegram and Mattermost. The bot token or webhook URL is encrypted at rest,
//! and never appears in a response, a log or an error message.

use std::time::Duration;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use reqwest::{Url, redirect::Policy};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState, Error, audit,
    auth::{Auth, Ip},
};

const TABLE: &str = "notification_channels";
const EVENTS: [&str; 3] = ["backup_failed", "restore_failed", "agent_offline"];
const ATTEMPTS: i32 = 5;

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Secret {
    Telegram { bot_token: String, chat_id: String },
    Mattermost { webhook_url: String },
}

impl Secret {
    /// Checked on the server: the values end up in an outgoing request.
    fn target(&self) -> Result<(&'static str, String), Error> {
        let word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
        match self {
            Secret::Telegram { bot_token, chat_id } => {
                let token_ok = bot_token.len() <= 100
                    && bot_token.split_once(':').is_some_and(|(id, key)| {
                        !id.is_empty()
                            && id.bytes().all(|b| b.is_ascii_digit())
                            && !key.is_empty()
                            && key.chars().all(word)
                    });
                if !token_ok {
                    return Err(Error::Invalid("bot_token"));
                }
                let id = chat_id.strip_prefix('@').unwrap_or(chat_id);
                if id.is_empty() || chat_id.len() > 64 || !id.chars().all(word) {
                    return Err(Error::Invalid("chat_id"));
                }
                Ok(("telegram", format!("chat {chat_id}")))
            }
            Secret::Mattermost { webhook_url } => {
                let url = Url::parse(webhook_url)
                    .ok()
                    .filter(|u| matches!(u.scheme(), "http" | "https") && webhook_url.len() <= 500)
                    .ok_or(Error::Invalid("webhook_url"))?;
                let host = url.host_str().ok_or(Error::Invalid("webhook_url"))?;
                Ok(("mattermost", host.to_owned()))
            }
        }
    }
}

/// The error text is safe to store and show: it never contains the URL.
async fn send(app: &AppState, secret: &Secret, message: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(Policy::none())
        .build()
        .map_err(|_| "cannot create the HTTP client".to_owned())?;
    let text = format!("DB Manager\n{message}");
    let request = match secret {
        Secret::Telegram { bot_token, chat_id } => client
            .post(format!(
                "{}/bot{bot_token}/sendMessage",
                app.config.telegram_api
            ))
            .json(&serde_json::json!({ "chat_id": chat_id, "text": text })),
        Secret::Mattermost { webhook_url } => client
            .post(webhook_url)
            .json(&serde_json::json!({ "text": text })),
    };
    match request.send().await {
        Ok(reply) if reply.status().is_success() => Ok(()),
        Ok(reply) => Err(format!("HTTP {}", reply.status().as_u16())),
        Err(error) if error.is_timeout() => Err("timed out".into()),
        Err(_) => Err("could not connect".into()),
    }
}

fn open(app: &AppState, id: Uuid, sealed: &[u8]) -> Result<Secret, Error> {
    let plain = app
        .master
        .open(TABLE, "secret", &id.to_string(), sealed)
        .map_err(Error::Internal)?;
    serde_json::from_slice(&plain).map_err(|e| Error::Internal(e.to_string()))
}

/// Sends what is due. Claiming first means a slow service never causes a double send.
pub async fn send_pending(app: &AppState) -> Result<(), Error> {
    let due: Vec<(i64, String, Uuid, Vec<u8>)> = sqlx::query_as(
        "UPDATE notifications n SET attempts = attempts + 1,
                next_attempt_at = now() + make_interval(mins => attempts + 1)
         FROM notification_channels c
         WHERE c.id = n.channel_id AND n.id IN (
             SELECT id FROM notifications
             WHERE sent_at IS NULL AND attempts < $1 AND next_attempt_at <= now()
             ORDER BY id LIMIT 20 FOR UPDATE SKIP LOCKED)
         RETURNING n.id, n.message, c.id, c.secret",
    )
    .bind(ATTEMPTS)
    .fetch_all(&app.db)
    .await?;
    for (id, message, channel, sealed) in due {
        let result = send(app, &open(app, channel, &sealed)?, &message).await;
        if result.is_ok() {
            sqlx::query("UPDATE notifications SET sent_at = now() WHERE id = $1")
                .bind(id)
                .execute(&app.db)
                .await?;
        }
        record(app, channel, result.err()).await?;
    }
    sqlx::query("DELETE FROM notifications WHERE created_at < now() - interval '30 days'")
        .execute(&app.db)
        .await?;
    Ok(())
}

async fn record(app: &AppState, channel: Uuid, error: Option<String>) -> Result<(), Error> {
    sqlx::query(
        "UPDATE notification_channels SET last_error = $2,
                last_sent_at = CASE WHEN $2::text IS NULL THEN now() ELSE last_sent_at END
         WHERE id = $1",
    )
    .bind(channel)
    .bind(error)
    .execute(&app.db)
    .await?;
    Ok(())
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Channel {
    id: Uuid,
    name: String,
    kind: String,
    target: String,
    events: Vec<String>,
    last_error: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    last_sent_at: Option<OffsetDateTime>,
}

pub async fn list(
    State(app): State<AppState>,
    Auth(me): Auth,
) -> Result<Json<Vec<Channel>>, Error> {
    me.require_admin()?;
    let rows = sqlx::query_as(
        "SELECT id, name, kind, target, events, last_error, last_sent_at
         FROM notification_channels ORDER BY name LIMIT 100",
    )
    .fetch_all(&app.db)
    .await?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
pub struct NewChannel {
    name: String,
    events: Vec<String>,
    secret: serde_json::Value,
}

pub async fn create(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Json(body): Json<NewChannel>,
) -> Result<StatusCode, Error> {
    me.require_admin()?;
    let name = body.name.trim();
    if name.is_empty() || name.len() > 64 {
        return Err(Error::Invalid("name"));
    }
    if body.events.is_empty() || !body.events.iter().all(|e| EVENTS.contains(&e.as_str())) {
        return Err(Error::Invalid("events"));
    }
    // Parsed by hand: a serde error could echo the secret back.
    let secret: Secret =
        serde_json::from_value(body.secret).map_err(|_| Error::Invalid("secret"))?;
    let (kind, target) = secret.target()?;
    let id = Uuid::new_v4();
    let plain = serde_json::to_vec(&secret).map_err(|e| Error::Internal(e.to_string()))?;
    let sealed = app
        .master
        .seal(TABLE, "secret", &id.to_string(), &plain)
        .map_err(Error::Internal)?;
    sqlx::query(
        "INSERT INTO notification_channels (id, name, kind, target, secret, events)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(name)
    .bind(kind)
    .bind(target)
    .bind(sealed)
    .bind(&body.events)
    .execute(&app.db)
    .await?;
    let target = format!("notification {name}");
    audit::record(&app.db, Some(&me), "create", &target, &ip, true).await?;
    Ok(StatusCode::CREATED)
}

pub async fn delete(
    State(app): State<AppState>,
    Auth(me): Auth,
    Ip(ip): Ip,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    me.require_admin()?;
    let name: String =
        sqlx::query_scalar("DELETE FROM notification_channels WHERE id = $1 RETURNING name")
            .bind(id)
            .fetch_optional(&app.db)
            .await?
            .ok_or(Error::NotFound)?;
    let target = format!("notification {name}");
    audit::record(&app.db, Some(&me), "delete", &target, &ip, true).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct Tested {
    /// `None` when the message was accepted.
    error: Option<String>,
}

pub async fn test(
    State(app): State<AppState>,
    Auth(me): Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<Tested>, Error> {
    me.require_admin()?;
    let sealed: Vec<u8> =
        sqlx::query_scalar("SELECT secret FROM notification_channels WHERE id = $1")
            .bind(id)
            .fetch_optional(&app.db)
            .await?
            .ok_or(Error::NotFound)?;
    let result = send(
        &app,
        &open(&app, id, &sealed)?,
        "Pesan tes: notifikasi aktif.",
    )
    .await;
    let error = result.err();
    record(&app, id, error.clone()).await?;
    Ok(Json(Tested { error }))
}
