use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

use argon2::{Argon2, PasswordHasher, PasswordVerifier, password_hash::phc::PasswordHash};
use axum::{
    Json,
    extract::{ConnectInfo, FromRequestParts, State},
    http::{StatusCode, request::Parts},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::{AppState, Error, audit};

const COOKIE: &str = "dbm_session";
const SESSION_DAYS: i64 = 7;
const MAX_FAILURES: u32 = 5;
const LOCK_WINDOW: Duration = Duration::from_secs(300);

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Viewer,
    Operator,
    Admin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Operator => "operator",
            Role::Admin => "admin",
        }
    }
}

#[derive(Serialize, sqlx::FromRow, Clone)]
pub struct User {
    pub id: Uuid,
    pub name: String,
    pub email: String,
    pub role: String,
}

impl User {
    /// Admins and operators: anyone who may change things or run backups and restores.
    pub fn require_operator(&self) -> Result<(), Error> {
        if self.role == Role::Viewer.as_str() {
            Err(Error::Forbidden)
        } else {
            Ok(())
        }
    }

    pub fn require_admin(&self) -> Result<(), Error> {
        if self.role == Role::Admin.as_str() {
            Ok(())
        } else {
            Err(Error::Forbidden)
        }
    }
}

/// Login throttling and a cap on parallel argon2 work (each run uses ~19 MiB).
pub struct LoginGuard {
    failures: Mutex<HashMap<String, (u32, Instant)>>,
    hashing: Semaphore,
}

impl Default for LoginGuard {
    fn default() -> Self {
        Self {
            failures: Mutex::default(),
            hashing: Semaphore::new(2),
        }
    }
}

impl LoginGuard {
    fn locked(&self, key: &str) -> bool {
        let mut map = self.failures.lock().unwrap();
        map.retain(|_, (_, since)| since.elapsed() < LOCK_WINDOW);
        map.get(key)
            .is_some_and(|(count, _)| *count >= MAX_FAILURES)
    }

    fn fail(&self, key: String) {
        let mut map = self.failures.lock().unwrap();
        map.entry(key).or_insert((0, Instant::now())).0 += 1;
    }

    fn clear(&self, key: &str) {
        self.failures.lock().unwrap().remove(key);
    }
}

/// 256 random bits, hex encoded.
pub fn new_token() -> String {
    rand::random::<[u8; 32]>()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

pub async fn hash_password(state: &AppState, password: String) -> Result<String, Error> {
    let _permit = state
        .guard
        .hashing
        .acquire()
        .await
        .map_err(|e| Error::Internal(e.to_string()))?;
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|h: PasswordHash| h.to_string())
    })
    .await
    .map_err(|e| Error::Internal(e.to_string()))?
    .map_err(|e| Error::Internal(e.to_string()))
}

async fn verify_password(state: &AppState, password: String, hash: String) -> Result<bool, Error> {
    let _permit = state
        .guard
        .hashing
        .acquire()
        .await
        .map_err(|e| Error::Internal(e.to_string()))?;
    tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash).is_ok_and(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
    })
    .await
    .map_err(|e| Error::Internal(e.to_string()))
}

pub fn check_password_rules(password: &str) -> Result<(), Error> {
    if (12..=256).contains(&password.chars().count()) {
        Ok(())
    } else {
        Err(Error::Invalid("password_length"))
    }
}

pub fn normalize_account(name: &str, email: &str) -> Result<(String, String), Error> {
    let name = name.trim();
    let email = email.trim().to_lowercase();
    if name.is_empty() || name.len() > 100 {
        return Err(Error::Invalid("name"));
    }
    if email.len() > 254
        || !email
            .split_once('@')
            .is_some_and(|(a, b)| !a.is_empty() && !b.is_empty())
    {
        return Err(Error::Invalid("email"));
    }
    Ok((name.to_owned(), email))
}

/// `action` must match what the web form renders the widget with.
async fn check_human(
    app: &AppState,
    token: Option<&str>,
    action: &str,
    ip: &str,
) -> Result<(), Error> {
    match &app.config.turnstile {
        Some(turnstile) => turnstile.verify(token, action, ip).await,
        None => Ok(()),
    }
}

/// Client address, for the audit log and throttling.
pub struct Ip(pub String);

impl FromRequestParts<AppState> for Ip {
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Error> {
        if state.config.trust_proxy {
            // The last entry is the one our own proxy appended.
            if let Some(ip) = parts
                .headers
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit(',').next())
            {
                return Ok(Ip(ip.trim().to_owned()));
            }
        }
        let peer = ConnectInfo::<SocketAddr>::from_request_parts(parts, state).await;
        Ok(Ip(peer.map_or_else(
            |_| "unknown".into(),
            |c| c.0.ip().to_string(),
        )))
    }
}

/// The signed-in user. Extracting it is what protects a route.
pub struct Auth(pub User);

impl FromRequestParts<AppState> for Auth {
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Error> {
        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar
            .get(COOKIE)
            .ok_or(Error::Unauthorized)?
            .value()
            .to_owned();
        sqlx::query_as::<_, User>(
            "SELECT u.id, u.name, u.email, u.role FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = $1 AND s.expires_at > now()",
        )
        .bind(token_hash(&token))
        .fetch_optional(&state.db)
        .await?
        .map(Auth)
        .ok_or(Error::Unauthorized)
    }
}

fn session_cookie(state: &AppState, value: String, max_age: time::Duration) -> Cookie<'static> {
    Cookie::build((COOKIE, value))
        .path("/")
        .http_only(true)
        .secure(state.config.cookie_secure)
        .same_site(SameSite::Strict)
        .max_age(max_age)
        .build()
}

async fn start_session(
    state: &AppState,
    jar: CookieJar,
    user_id: Uuid,
) -> Result<CookieJar, Error> {
    let token = new_token();
    sqlx::query("DELETE FROM sessions WHERE expires_at <= now()")
        .execute(&state.db)
        .await?;
    sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, now() + make_interval(days => $3))")
        .bind(token_hash(&token))
        .bind(user_id)
        .bind(SESSION_DAYS as i32)
        .execute(&state.db)
        .await?;
    Ok(jar.add(session_cookie(
        state,
        token,
        time::Duration::days(SESSION_DAYS),
    )))
}

#[derive(Serialize)]
pub struct AuthState {
    setup_required: bool,
    user: Option<User>,
    /// Public site key; present when the sign-in form must show the widget.
    turnstile_site_key: Option<String>,
}

pub async fn state(
    State(app): State<AppState>,
    user: Result<Auth, Error>,
) -> Result<Json<AuthState>, Error> {
    let user = match user {
        Ok(Auth(user)) => Some(user),
        Err(Error::Unauthorized) => None,
        Err(other) => return Err(other),
    };
    let setup_required = user.is_none()
        && !sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM users)")
            .fetch_one(&app.db)
            .await?;
    let turnstile_site_key = app.config.turnstile.as_ref().map(|t| t.site_key.clone());
    Ok(Json(AuthState {
        setup_required,
        user,
        turnstile_site_key,
    }))
}

#[derive(Deserialize)]
pub struct Setup {
    name: String,
    email: String,
    password: String,
    turnstile_token: Option<String>,
}

/// Creates the first administrator. Refused once any user exists.
pub async fn setup(
    State(app): State<AppState>,
    Ip(ip): Ip,
    jar: CookieJar,
    Json(body): Json<Setup>,
) -> Result<(CookieJar, Json<User>), Error> {
    let (name, email) = normalize_account(&body.name, &body.email)?;
    check_password_rules(&body.password)?;
    check_human(&app, body.turnstile_token.as_deref(), "setup", &ip).await?;
    let hash = hash_password(&app, body.password).await?;

    let mut tx = app.db.begin().await?;
    // Two first-time requests must not both succeed.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('dbm-users'))")
        .execute(&mut *tx)
        .await?;
    let user = sqlx::query_as::<_, User>(
        "INSERT INTO users (name, email, password_hash, role)
         SELECT $1, $2, $3, 'admin' WHERE NOT EXISTS (SELECT 1 FROM users)
         RETURNING id, name, email, role",
    )
    .bind(name)
    .bind(email)
    .bind(hash)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(Error::Conflict("already_set_up"))?;
    audit::record(
        &mut *tx,
        Some(&user),
        "create",
        "first administrator",
        &ip,
        true,
    )
    .await?;
    tx.commit().await?;

    let jar = start_session(&app, jar, user.id).await?;
    Ok((jar, Json(user)))
}

#[derive(Deserialize)]
pub struct Login {
    email: String,
    password: String,
    turnstile_token: Option<String>,
}

pub async fn login(
    State(app): State<AppState>,
    Ip(ip): Ip,
    jar: CookieJar,
    Json(body): Json<Login>,
) -> Result<(CookieJar, Json<User>), Error> {
    let email: String = body.email.trim().to_lowercase().chars().take(254).collect();
    // Keyed on both, so one address cannot lock every account and one account cannot be sprayed.
    let key = format!("{ip}|{email}");
    if app.guard.locked(&key) {
        return Err(Error::TooManyAttempts);
    }
    if body.password.len() > 1024 {
        return Err(Error::BadCredentials);
    }
    // Before any password work, so bots cost neither argon2 time nor lockouts of real users.
    check_human(&app, body.turnstile_token.as_deref(), "login", &ip).await?;

    #[derive(sqlx::FromRow)]
    struct Row {
        #[sqlx(flatten)]
        user: User,
        password_hash: String,
    }
    let row = sqlx::query_as::<_, Row>(
        "SELECT id, name, email, role, password_hash FROM users WHERE email = $1",
    )
    .bind(&email)
    .fetch_optional(&app.db)
    .await?;

    let ok = match &row {
        Some(row) => verify_password(&app, body.password, row.password_hash.clone()).await?,
        // Spend the same time for unknown accounts.
        None => hash_password(&app, body.password).await.map(|_| false)?,
    };
    let Some(row) = row.filter(|_| ok) else {
        app.guard.fail(key);
        audit::record(&app.db, None, "login", &email, &ip, false).await?;
        return Err(Error::BadCredentials);
    };

    app.guard.clear(&key);
    audit::record(&app.db, Some(&row.user), "login", "web", &ip, true).await?;
    let jar = start_session(&app, jar, row.user.id).await?;
    Ok((jar, Json(row.user)))
}

pub async fn logout(
    State(app): State<AppState>,
    jar: CookieJar,
) -> Result<(CookieJar, StatusCode), Error> {
    if let Some(cookie) = jar.get(COOKIE) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(token_hash(cookie.value()))
            .execute(&app.db)
            .await?;
    }
    let jar = jar.add(session_cookie(&app, String::new(), time::Duration::ZERO));
    Ok((jar, StatusCode::NO_CONTENT))
}

#[derive(Deserialize)]
pub struct ChangePassword {
    current: String,
    new: String,
}

/// Also signs out every other session of this user.
pub async fn change_password(
    State(app): State<AppState>,
    Auth(user): Auth,
    Ip(ip): Ip,
    jar: CookieJar,
    Json(body): Json<ChangePassword>,
) -> Result<(CookieJar, StatusCode), Error> {
    check_password_rules(&body.new)?;
    let key = format!("{ip}|{}", user.email);
    if app.guard.locked(&key) {
        return Err(Error::TooManyAttempts);
    }
    let current: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(user.id)
        .fetch_one(&app.db)
        .await?;
    if body.current.len() > 1024 || !verify_password(&app, body.current, current).await? {
        app.guard.fail(key);
        return Err(Error::Invalid("wrong_password"));
    }
    let hash = hash_password(&app, body.new).await?;
    sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
        .bind(hash)
        .bind(user.id)
        .execute(&app.db)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(user.id)
        .execute(&app.db)
        .await?;
    audit::record(&app.db, Some(&user), "update", "own password", &ip, true).await?;
    let jar = start_session(&app, jar, user.id).await?;
    Ok((jar, StatusCode::NO_CONTENT))
}

/// Re-checks the password before a sensitive action, with the same lockout as signing in.
pub async fn confirm_password(
    app: &AppState,
    user: &User,
    ip: &str,
    password: String,
) -> Result<(), Error> {
    let key = format!("{ip}|{}", user.email);
    if app.guard.locked(&key) {
        return Err(Error::TooManyAttempts);
    }
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(user.id)
        .fetch_one(&app.db)
        .await?;
    if password.len() > 1024 || !verify_password(app, password, hash).await? {
        app.guard.fail(key);
        return Err(Error::Invalid("wrong_password"));
    }
    Ok(())
}
