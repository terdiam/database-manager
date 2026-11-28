//! The bell: how far each person has read the finished jobs, the same on every device.

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{AppState, Error, auth::Auth};

#[derive(Serialize, Deserialize)]
pub struct Read {
    #[serde(with = "time::serde::rfc3339")]
    read_at: OffsetDateTime,
}

pub async fn get(State(app): State<AppState>, Auth(me): Auth) -> Result<Json<Read>, Error> {
    let read_at = sqlx::query_scalar("SELECT notices_read_at FROM users WHERE id = $1")
        .bind(me.id)
        .fetch_one(&app.db)
        .await?;
    Ok(Json(Read { read_at }))
}

/// `read_at` is the newest job that was on screen, so one finishing meanwhile stays unread.
/// It never moves back, nor into the future.
pub async fn mark(
    State(app): State<AppState>,
    Auth(me): Auth,
    Json(body): Json<Read>,
) -> Result<Json<Read>, Error> {
    let read_at = sqlx::query_scalar(
        "UPDATE users SET notices_read_at = greatest(notices_read_at, least($2, now()))
         WHERE id = $1 RETURNING notices_read_at",
    )
    .bind(me.id)
    .bind(body.read_at)
    .fetch_one(&app.db)
    .await?;
    Ok(Json(Read { read_at }))
}
