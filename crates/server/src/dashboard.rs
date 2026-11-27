//! One request with everything the front page shows.

use std::str::FromStr;

use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{AppState, Error, auth::Auth};
use dbm_shared::OFFLINE_AFTER_SECS;

#[derive(Deserialize)]
pub struct Zone {
    /// The viewer's time zone, so "per day" means their days.
    timezone: String,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Day {
    day: String,
    completed: i64,
    failed: i64,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Volume {
    name: String,
    bytes: i64,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Agent {
    name: String,
    online: bool,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Recent {
    id: Uuid,
    kind: String,
    status: String,
    target: String,
    bytes: i64,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Failing {
    name: String,
    error: Option<String>,
}

#[derive(Serialize)]
pub struct Dashboard {
    completed_24h: i64,
    failed_24h: i64,
    /// The last 14 days, oldest first.
    activity: Vec<Day>,
    volumes: Vec<Volume>,
    agents: Vec<Agent>,
    /// The last 60 jobs, newest first.
    jobs: Vec<Recent>,
    /// Backup entries whose latest run failed.
    failing: Vec<Failing>,
}

pub async fn get(
    State(app): State<AppState>,
    Auth(_): Auth,
    Query(zone): Query<Zone>,
) -> Result<Json<Dashboard>, Error> {
    let zone = chrono_tz::Tz::from_str(&zone.timezone)
        .map_err(|_| Error::Invalid("timezone"))?
        .name();
    let (completed_24h, failed_24h): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE status = 'completed'), count(*) FILTER (WHERE status = 'failed')
         FROM jobs WHERE kind = 'backup' AND created_at > now() - interval '24 hours'",
    )
    .fetch_one(&app.db)
    .await?;
    let activity = sqlx::query_as(
        "WITH days AS (
             SELECT generate_series(date_trunc('day', now() AT TIME ZONE $1) - interval '13 days',
                                    date_trunc('day', now() AT TIME ZONE $1), interval '1 day') AS day
         )
         SELECT to_char(d.day, 'YYYY-MM-DD') AS day,
                count(j.id) FILTER (WHERE j.status = 'completed') AS completed,
                count(j.id) FILTER (WHERE j.status = 'failed') AS failed
         FROM days d
         LEFT JOIN jobs j ON j.kind = 'backup'
             AND j.created_at >= d.day AT TIME ZONE $1
             AND j.created_at < (d.day + interval '1 day') AT TIME ZONE $1
         GROUP BY d.day ORDER BY d.day",
    )
    .bind(zone)
    .fetch_all(&app.db)
    .await?;
    let volumes = sqlx::query_as(
        "SELECT d.name, coalesce(sum(s.size), 0)::bigint AS bytes
         FROM destinations d LEFT JOIN snapshots s ON s.destination_id = d.id
         GROUP BY d.id ORDER BY bytes DESC, d.name LIMIT 100",
    )
    .fetch_all(&app.db)
    .await?;
    let agents = sqlx::query_as(
        "SELECT name, last_seen_at > now() - make_interval(secs => $1) AS online
         FROM agents ORDER BY name LIMIT 200",
    )
    .bind(OFFLINE_AFTER_SECS as f64)
    .fetch_all(&app.db)
    .await?;
    let jobs = sqlx::query_as(
        "SELECT id, kind, status, target, bytes, created_at FROM jobs
         ORDER BY created_at DESC, id DESC LIMIT 60",
    )
    .fetch_all(&app.db)
    .await?;
    let failing = sqlx::query_as(
        "SELECT b.name, j.error FROM backup_configs b
         JOIN LATERAL (
             SELECT status, error FROM jobs
             WHERE backup_config_id = b.id AND kind = 'backup' AND status IN ('completed', 'failed')
             ORDER BY created_at DESC LIMIT 1
         ) j ON j.status = 'failed'
         ORDER BY b.name LIMIT 20",
    )
    .fetch_all(&app.db)
    .await?;
    Ok(Json(Dashboard {
        completed_24h,
        failed_24h,
        activity,
        volumes,
        agents,
        jobs,
        failing,
    }))
}
