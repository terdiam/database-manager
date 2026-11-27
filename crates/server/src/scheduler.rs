//! The server's own work: due backups, retention, silent agents and waiting alerts.

use std::time::Duration;

use time::OffsetDateTime;
use uuid::Uuid;

use crate::{AppState, Error, audit, jobs, notify, schedule, snapshots};

const EVERY: Duration = Duration::from_secs(10);
/// Longer than the page uses for "online", so a short hiccup raises no alert.
const OFFLINE_SECS: f64 = 120.0;

pub async fn run(app: AppState) {
    let mut tick = tokio::time::interval(EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        round(&app).await;
    }
}

/// One pass. A failing step is logged and does not stop the others.
pub async fn round(app: &AppState) {
    report("expiring jobs", jobs::expire_stale(&app.db).await);
    report("starting scheduled backups", due(app).await);
    report("applying retention", retention(app).await);
    report("checking agents", agents(app).await);
    report("sending alerts", notify::send_pending(app).await);
}

fn report(step: &str, result: Result<(), Error>) {
    if let Err(error) = result {
        tracing::warn!("{step} failed: {}", error.detail());
    }
}

async fn due(app: &AppState) -> Result<(), Error> {
    let mut tx = app.db.begin().await?;
    let due: Vec<(Uuid, String, String, String)> = sqlx::query_as(
        "SELECT id, name, schedule, timezone FROM backup_configs
         WHERE next_run_at <= now() FOR UPDATE SKIP LOCKED LIMIT 50",
    )
    .fetch_all(&mut *tx)
    .await?;
    // The next run is set first: runs missed while the server was off happen once, not once each.
    for (id, _, pattern, zone) in &due {
        let next = schedule::next_after(pattern, zone, OffsetDateTime::now_utc()).ok();
        sqlx::query("UPDATE backup_configs SET next_run_at = $2 WHERE id = $1")
            .bind(id)
            .bind(next)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    for (id, name, ..) in due {
        match snapshots::queue_backup(app, id, "schedule").await {
            Ok(_) => {}
            Err(error) => tracing::warn!("scheduled backup {name} not started: {}", error.detail()),
        }
    }
    Ok(())
}

/// Snapshots nothing is working on, and that were not just given up on.
const IDLE: &str = "NOT EXISTS (SELECT 1 FROM jobs j WHERE j.snapshot_id = s.id
    AND (j.status IN ('pending', 'running')
         OR (j.kind = 'delete_snapshot' AND j.created_at > now() - interval '6 hours')))";

async fn retention(app: &AppState) -> Result<(), Error> {
    // Days and months. The newest backup of a database in a destination always stays.
    let expired: Vec<Uuid> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT s.id FROM snapshots s WHERE s.expires_at <= now()
         AND EXISTS (SELECT 1 FROM snapshots n WHERE n.server_name = s.server_name
                     AND n.database_name = s.database_name AND n.destination_id = s.destination_id
                     AND (n.created_at, n.id) > (s.created_at, s.id))
         AND {IDLE} LIMIT 100"
    )))
    .fetch_all(&app.db)
    .await?;

    // Grandfather-father-son: the newest backup of each of the last N days, weeks and months.
    let rotated: Vec<Uuid> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "WITH s AS (
             SELECT s.id, s.created_at, s.created_at AT TIME ZONE c.timezone AS local, c.id AS config,
                    c.keep_daily, c.keep_weekly, c.keep_monthly
             FROM snapshots s JOIN backup_configs c ON c.id = s.backup_config_id
             WHERE c.retention_unit = 'gfs' AND s.created_at >= c.retention_since
         ), r AS (
             SELECT *,
                 row_number() OVER (PARTITION BY config ORDER BY created_at DESC) AS newest,
                 row_number() OVER (PARTITION BY config, date_trunc('day', local) ORDER BY created_at DESC) AS in_day,
                 dense_rank() OVER (PARTITION BY config ORDER BY date_trunc('day', local) DESC) AS day,
                 row_number() OVER (PARTITION BY config, date_trunc('week', local) ORDER BY created_at DESC) AS in_week,
                 dense_rank() OVER (PARTITION BY config ORDER BY date_trunc('week', local) DESC) AS week,
                 row_number() OVER (PARTITION BY config, date_trunc('month', local) ORDER BY created_at DESC) AS in_month,
                 dense_rank() OVER (PARTITION BY config ORDER BY date_trunc('month', local) DESC) AS month
             FROM s
         )
         SELECT s.id FROM r s WHERE s.newest > 1
             AND NOT (s.in_day = 1 AND s.day <= s.keep_daily)
             AND NOT (s.in_week = 1 AND s.week <= s.keep_weekly)
             AND NOT (s.in_month = 1 AND s.month <= s.keep_monthly)
             AND {IDLE} LIMIT 100"
    )))
    .fetch_all(&app.db)
    .await?;

    for id in expired.into_iter().chain(rotated) {
        let (_, object_key) = snapshots::queue_delete(app, id, "retention").await?;
        let target = format!("snapshot {object_key} (retention)");
        audit::record(&app.db, None, "delete", &target, "system", true).await?;
    }
    // Download links that were never fetched.
    sqlx::query("UPDATE jobs SET result = NULL WHERE result IS NOT NULL AND finished_at < now() - interval '10 minutes'")
        .execute(&app.db)
        .await?;
    Ok(())
}

/// One alert per outage.
async fn agents(app: &AppState) -> Result<(), Error> {
    sqlx::query(
        "WITH gone AS (
             UPDATE agents SET offline_alerted = true
             WHERE NOT offline_alerted AND last_seen_at < now() - make_interval(secs => $1)
             RETURNING name
         )
         SELECT queue_alert('agent_offline', format('Agent offline: %s', name)) FROM gone",
    )
    .bind(OFFLINE_SECS)
    .execute(&app.db)
    .await?;
    sqlx::query(
        "UPDATE agents SET offline_alerted = false
         WHERE offline_alerted AND last_seen_at >= now() - make_interval(secs => $1)",
    )
    .bind(OFFLINE_SECS)
    .execute(&app.db)
    .await?;
    Ok(())
}
