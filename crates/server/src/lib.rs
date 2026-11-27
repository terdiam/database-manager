mod agents;
mod audit;
mod auth;
mod crypto;
mod e2ee;
mod error;
mod recover;
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
    routing::{delete, get, post},
};
use dbm_shared::e2ee::Keypair;
use sqlx::PgPool;

pub use crypto::{MasterKey, load_transport_key};
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
}

#[derive(Clone)]
pub struct AppState {
    db: PgPool,
    config: Arc<Config>,
    guard: Arc<auth::LoginGuard>,
    transport: Arc<e2ee::Transport>,
}

impl AppState {
    /// `transport_key` comes from `load_transport_key`.
    pub fn new(db: PgPool, config: Config, transport_key: Keypair) -> Self {
        let transport = e2ee::Transport::new(transport_key, config.e2ee_required);
        Self {
            db,
            config: Arc::new(config),
            guard: Arc::default(),
            transport: Arc::new(transport),
        }
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
        .layer(middleware::from_fn_with_state(state.clone(), e2ee::layer))
        .layer(middleware::from_fn(csrf));

    // Agent API: bearer credential, no cookies.
    let agent = Router::new()
        .route("/agent/enroll", post(agents::enroll))
        .route("/agent/heartbeat", post(agents::heartbeat))
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
