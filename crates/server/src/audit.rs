use axum::{
    Json,
    extract::{Query, Request, State},
    http::header,
    middleware::Next,
    response::Response,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgExecutor, Postgres, QueryBuilder};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState, Error,
    auth::{Auth, User},
};

const PAGE: i64 = 100;
/// Longest user agent kept; the caller chooses it.
const MAX_AGENT: usize = 300;

tokio::task_local! {
    static USER_AGENT: String;
}

/// Keeps what the caller says it is at hand for `record`, whichever handler calls it.
pub async fn remember_client(request: Request, next: Next) -> Response {
    let agent = request
        .headers()
        .get(header::USER_AGENT)
        .map(|v| String::from_utf8_lossy(v.as_bytes()))
        .map(|v| v.chars().take(MAX_AGENT).collect())
        .unwrap_or_default();
    USER_AGENT.scope(agent, next.run(request)).await
}

/// Never pass secrets in `target`.
pub async fn record(
    db: impl PgExecutor<'_>,
    user: Option<&User>,
    action: &str,
    target: &str,
    ip: &str,
    ok: bool,
) -> Result<(), Error> {
    // Empty for the console and the scheduler, which no request starts.
    let agent = USER_AGENT.try_with(Clone::clone).unwrap_or_default();
    sqlx::query("INSERT INTO audit_logs (user_id, user_name, action, target, ip, ok, user_email, user_agent) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)")
        .bind(user.map(|u| u.id))
        .bind(user.map_or("-", |u| u.name.as_str()))
        .bind(action)
        .bind(target)
        .bind(ip)
        .bind(ok)
        .bind(user.map_or("", |u| u.email.as_str()))
        .bind(agent)
        .execute(db)
        .await?;
    Ok(())
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Entry {
    id: i64,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    user_name: String,
    user_email: String,
    user_agent: String,
    action: String,
    target: String,
    ip: String,
    ok: bool,
}

#[derive(Deserialize)]
pub struct Filter {
    /// Instants chosen by the browser, so day bounds follow the user's timezone. `to` is exclusive.
    #[serde(default, with = "time::serde::rfc3339::option")]
    from: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    to: Option<OffsetDateTime>,
    user: Option<Uuid>,
    action: Option<String>,
    q: Option<String>,
    /// Keyset cursor: id of the last row already shown.
    before: Option<i64>,
}

#[derive(Serialize)]
pub struct Page {
    items: Vec<Entry>,
    next: Option<i64>,
}

pub async fn list(
    State(app): State<AppState>,
    Auth(me): Auth,
    Query(filter): Query<Filter>,
) -> Result<Json<Page>, Error> {
    me.require_admin()?;
    let mut sql = QueryBuilder::<Postgres>::new(
        "SELECT id, created_at, user_name, user_email, user_agent, action, target, ip, ok FROM audit_logs WHERE true",
    );
    if let Some(from) = filter.from {
        sql.push(" AND created_at >= ").push_bind(from);
    }
    if let Some(to) = filter.to {
        sql.push(" AND created_at < ").push_bind(to);
    }
    if let Some(user) = filter.user {
        sql.push(" AND user_id = ").push_bind(user);
    }
    if let Some(action) = filter.action.filter(|a| !a.is_empty()) {
        sql.push(" AND action = ").push_bind(action);
    }
    if let Some(q) = filter.q.filter(|q| !q.is_empty()) {
        // strpos avoids LIKE wildcard escaping.
        let q = q.to_lowercase();
        sql.push(" AND (strpos(lower(target), ")
            .push_bind(q.clone())
            .push(") > 0 OR strpos(lower(user_email), ")
            .push_bind(q.clone())
            .push(") > 0 OR strpos(lower(user_agent), ")
            .push_bind(q.clone())
            .push(") > 0 OR strpos(ip, ")
            .push_bind(q)
            .push(") > 0)");
    }
    if let Some(before) = filter.before {
        sql.push(" AND (created_at, id) < (SELECT created_at, id FROM audit_logs WHERE id = ")
            .push_bind(before)
            .push(")");
    }
    sql.push(" ORDER BY created_at DESC, id DESC LIMIT ")
        .push_bind(PAGE + 1);

    let mut items: Vec<Entry> = sql.build_query_as().fetch_all(&app.db).await?;
    let more = items.len() as i64 > PAGE;
    items.truncate(PAGE as usize);
    let next = items.last().filter(|_| more).map(|e| e.id);
    Ok(Json(Page { items, next }))
}
