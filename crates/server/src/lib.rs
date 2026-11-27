mod agents;
mod audit;
mod auth;
mod crypto;
mod dashboard;
mod e2ee;
mod error;
mod events;
mod jobs;
mod keys;
mod notify;
mod recover;
mod resources;
mod schedule;
mod scheduler;
mod snapshots;
mod turnstile;
mod users;
mod web;

use std::sync::Arc;

use axum::{
    Router,
    extract::Request,
    http::{HeaderValue, Method, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use dbm_shared::e2ee::Keypair;
use sqlx::PgPool;

pub use crypto::{MasterKey, load_transport_key};

/// `None` unless `text` is an age X25519 recipient.
pub fn parse_recipient(text: &str) -> Option<String> {
    text.trim()
        .parse::<age::x25519::Recipient>()
        .ok()
        .map(|r| r.to_string())
}
pub use error::Error;
pub use recover::reset_password;
pub use turnstile::{Turnstile, VERIFY_URL};

pub struct Config {
    pub cookie_secure: bool,
    /// Read the client address from X-Forwarded-For (only behind a trusted proxy).
    pub trust_proxy: bool,
    /// Bot check on sign-in and first-time setup.
    pub turnstile: Option<Turnstile>,
    /// Refuse API requests that are not end-to-end encrypted.
    pub e2ee_required: bool,
    /// A second age recipient whose private key is kept offline, for disaster recovery.
    pub extra_recipient: Option<String>,
    /// Base address of the Telegram Bot API; tests point it elsewhere.
    pub telegram_api: String,
}

pub const TELEGRAM_API: &str = "https://api.telegram.org";

#[derive(Clone)]
pub struct AppState {
    db: PgPool,
    config: Arc<Config>,
    guard: Arc<auth::LoginGuard>,
    transport: Arc<e2ee::Transport>,
    master: Arc<MasterKey>,
    events: events::Sender,
}

impl AppState {
    /// `transport_key` comes from `load_transport_key`.
    pub fn new(db: PgPool, config: Config, transport_key: Keypair, master: MasterKey) -> Self {
        let transport = e2ee::Transport::new(transport_key, config.e2ee_required);
        Self {
            db,
            config: Arc::new(config),
            guard: Arc::default(),
            transport: Arc::new(transport),
            master: Arc::new(master),
            events: events::channel(),
        }
    }

    /// Starts forwarding database changes to open pages. Holds one database connection.
    pub async fn watch_changes(&self) -> Result<(), sqlx::Error> {
        events::watch(&self.db, self.events.clone()).await
    }

    /// Runs due backups, retention and alerts every few seconds, forever.
    pub async fn run_scheduler(self) {
        scheduler::run(self).await;
    }

    /// One pass of the scheduler.
    pub async fn schedule_once(&self) {
        scheduler::round(self).await;
    }

    /// Ends the open event streams, so a graceful shutdown does not wait for them.
    pub fn close_streams(&self) {
        let _ = self.events.send(events::Signal::Close);
    }
}

pub fn app(state: AppState) -> Router {
    // Browser API: cookie session plus the CSRF header on every change.
    let browser = Router::new()
        .route("/auth/state", get(auth::state))
        .route("/auth/setup", post(auth::setup))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/password", post(auth::change_password))
        .route("/users", get(users::list).post(users::create))
        .route("/users/{id}", delete(users::remove))
        .route("/agents", get(agents::list))
        .route("/agents/tokens", post(agents::create_token))
        .route("/agents/{id}", delete(agents::remove))
        .route("/audit", get(audit::list))
        .route(
            "/database-servers",
            get(resources::list_servers).post(resources::create_server),
        )
        .route("/database-servers/{id}", delete(resources::delete_server))
        .route("/database-servers/{id}/test", post(resources::test_server))
        .route(
            "/destinations",
            get(resources::list_destinations).post(resources::create_destination),
        )
        .route("/destinations/{id}", delete(resources::delete_destination))
        .route("/destinations/{id}/test", post(resources::test_destination))
        .route(
            "/backups",
            get(resources::list_backups).post(resources::create_backup),
        )
        .route(
            "/backups/{id}",
            put(resources::update_backup).delete(resources::delete_backup),
        )
        .route("/backups/{id}/run", post(snapshots::run_backup))
        .route("/snapshots", get(snapshots::list))
        .route(
            "/snapshots/{id}",
            get(snapshots::get).delete(snapshots::delete),
        )
        .route("/snapshots/{id}/restore", post(snapshots::restore))
        .route("/snapshots/{id}/download", post(snapshots::download))
        .route("/downloads/{job}", get(snapshots::download_link))
        .route("/schedule/preview", get(schedule::preview))
        .route("/dashboard", get(dashboard::get))
        .route(
            "/notification-channels",
            get(notify::list).post(notify::create),
        )
        .route("/notification-channels/{id}", delete(notify::delete))
        .route("/notification-channels/{id}/test", post(notify::test))
        .route("/jobs", get(jobs::list))
        .route("/jobs/{id}", get(jobs::get))
        .route("/jobs/{id}/cancel", post(jobs::cancel))
        .route("/backup-keys/export", post(keys::export))
        .route("/events", get(events::stream))
        .layer(middleware::from_fn_with_state(state.clone(), e2ee::layer))
        .layer(middleware::from_fn(csrf));

    // Agent API: bearer credential, no cookies.
    let agent = Router::new()
        .route("/agent/enroll", post(agents::enroll))
        .route("/agent/heartbeat", post(agents::heartbeat))
        .route("/agent/jobs/claim", post(jobs::claim))
        .route("/agent/jobs/report", post(jobs::report))
        .route("/agent/jobs/finish", post(jobs::finish))
        .layer(middleware::from_fn_with_state(state.clone(), e2ee::layer));

    Router::new()
        .nest(
            "/api",
            browser
                .merge(agent)
                .route("/e2ee", get(e2ee::info))
                .fallback(|| async { Error::NotFound }),
        )
        .route("/healthz", get(|| async { "ok" }))
        .fallback(web::spa)
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}

/// A custom header cannot be sent cross-origin without a CORS preflight, which is never allowed here.
async fn csrf(request: Request, next: Next) -> Response {
    if request.method() != Method::GET && !request.headers().contains_key("x-dbm-csrf") {
        return Error::Forbidden.into_response();
    }
    next.run(request).await
}

async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    // The rest of the policy is in the page itself (see web/vite.config.ts).
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("frame-ancestors 'none'"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    response
}
