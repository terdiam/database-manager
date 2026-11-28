use axum::{
    Router,
    body::Body,
    extract::connect_info::MockConnectInfo,
    http::{Request, StatusCode, header},
};
use dbm_server::{
    AppState, Config, MasterKey, Turnstile, app, hold_running_lock, load_transport_key,
    reset_password, rotate_master_key, rotate_transport_key,
};
use dbm_shared::e2ee::{self, Keypair};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

const PASSWORD: &str = "test-password-123";

fn router(db: PgPool) -> Router {
    router_with(db, None)
}

fn router_with(db: PgPool, turnstile: Option<Turnstile>) -> Router {
    build(db, turnstile, false, Keypair::generate())
}

fn build(db: PgPool, turnstile: Option<Turnstile>, e2ee_required: bool, key: Keypair) -> Router {
    let config = Config {
        cookie_secure: true,
        trust_proxy: false,
        turnstile,
        e2ee_required,
        extra_recipient: None,
        telegram_api: "http://127.0.0.1:9".into(),
    };
    let master = MasterKey::parse("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap();
    app(AppState::new(db, config, key, master)).layer(MockConnectInfo(std::net::SocketAddr::from(
        ([10, 0, 0, 1], 5000),
    )))
}

/// Local stand-in for Cloudflare's siteverify. Tokens read "<action>@<hostname>".
async fn fake_siteverify() -> String {
    let handler = |axum::Form(form): axum::Form<std::collections::HashMap<String, String>>| async move {
        let (action, hostname) = form["response"].split_once('@').unwrap_or_default();
        let ok = !action.is_empty() && form["secret"] == "test-secret";
        axum::Json(json!({ "success": ok, "action": action, "hostname": hostname }))
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = Router::new().route("/", axum::routing::post(handler));
    tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
    url
}

fn turnstile(url: String) -> Turnstile {
    Turnstile::new(
        "site".into(),
        "test-secret".into(),
        " DBM.example.com, ",
        url,
    )
    .unwrap()
}

struct Reply {
    status: StatusCode,
    body: Value,
    cookie: Option<String>,
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> Reply {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("x-dbm-csrf", "1");
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    let body = match body {
        Some(json) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    send(app, request.body(body).unwrap()).await
}

async fn send(app: &Router, request: Request<Body>) -> Reply {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Reply {
        status,
        body,
        cookie,
    }
}

async fn admin(app: &Router) -> String {
    let body = json!({ "name": "Admin", "email": "Admin@Example.com", "password": PASSWORD });
    let reply = call(app, "POST", "/api/auth/setup", None, Some(body)).await;
    assert_eq!(reply.status, StatusCode::OK);
    reply.cookie.unwrap()
}

async fn login(app: &Router, email: &str, password: &str) -> Reply {
    call(
        app,
        "POST",
        "/api/auth/login",
        None,
        Some(json!({ "email": email, "password": password })),
    )
    .await
}

#[sqlx::test]
async fn setup_login_logout(db: PgPool) {
    let app = router(db);
    assert_eq!(
        call(&app, "GET", "/api/auth/state", None, None).await.body["setup_required"],
        true
    );
    admin(&app).await;

    let again = json!({ "name": "Other", "email": "o@example.com", "password": PASSWORD });
    assert_eq!(
        call(&app, "POST", "/api/auth/setup", None, Some(again))
            .await
            .status,
        StatusCode::CONFLICT
    );

    let reply = login(&app, "admin@example.com", PASSWORD).await;
    assert_eq!(reply.status, StatusCode::OK);
    let cookie = reply.cookie.unwrap();
    let state = call(&app, "GET", "/api/auth/state", Some(&cookie), None).await;
    assert_eq!(state.body["user"]["role"], "admin");
    assert!(state.body["user"].get("password_hash").is_none());

    assert_eq!(
        call(&app, "POST", "/api/auth/logout", Some(&cookie), None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, "GET", "/api/agents", Some(&cookie), None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test]
async fn changes_need_the_csrf_header(db: PgPool) {
    let app = router(db);
    let request = Request::post("/api/auth/logout")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, request).await.status, StatusCode::FORBIDDEN);
}

#[sqlx::test]
async fn repeated_failures_lock_the_login_and_are_audited(db: PgPool) {
    let app = router(db);
    let cookie = admin(&app).await;
    for _ in 0..5 {
        assert_eq!(
            login(&app, "admin@example.com", "wrong-password-1")
                .await
                .status,
            StatusCode::UNAUTHORIZED
        );
    }
    // Even the right password is refused while locked.
    assert_eq!(
        login(&app, "admin@example.com", PASSWORD).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );

    let log = call(&app, "GET", "/api/audit?action=login", Some(&cookie), None).await;
    let items = log.body["items"].as_array().unwrap();
    assert_eq!(items.len(), 5);
    assert!(
        items
            .iter()
            .all(|e| e["ok"] == false && e["ip"] == "10.0.0.1")
    );
}

#[sqlx::test]
async fn roles_are_enforced(db: PgPool) {
    let app = router(db);
    let cookie = admin(&app).await;
    let viewer =
        json!({ "name": "V", "email": "v@example.com", "password": PASSWORD, "role": "viewer" });
    let created = call(
        &app,
        "POST",
        "/api/users",
        Some(&cookie),
        Some(viewer.clone()),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    assert_eq!(
        call(&app, "POST", "/api/users", Some(&cookie), Some(viewer))
            .await
            .status,
        StatusCode::CONFLICT
    );

    let viewer_cookie = login(&app, "v@example.com", PASSWORD).await.cookie.unwrap();
    for path in ["/api/users", "/api/audit"] {
        assert_eq!(
            call(&app, "GET", path, Some(&viewer_cookie), None)
                .await
                .status,
            StatusCode::FORBIDDEN
        );
    }
    let token = call(
        &app,
        "POST",
        "/api/agents/tokens",
        Some(&viewer_cookie),
        Some(json!({ "name": "x" })),
    )
    .await;
    assert_eq!(token.status, StatusCode::FORBIDDEN);
    assert_eq!(
        call(&app, "GET", "/api/agents", Some(&viewer_cookie), None)
            .await
            .status,
        StatusCode::OK
    );

    // Deleting the account ends its sessions; admins cannot delete themselves.
    let id = created.body["id"].as_str().unwrap();
    let removed = call(
        &app,
        "DELETE",
        &format!("/api/users/{id}"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    assert_eq!(
        call(&app, "GET", "/api/agents", Some(&viewer_cookie), None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let me = call(&app, "GET", "/api/auth/state", Some(&cookie), None)
        .await
        .body["user"]["id"]
        .clone();
    let own = call(
        &app,
        "DELETE",
        &format!("/api/users/{}", me.as_str().unwrap()),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(own.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test]
async fn password_change_signs_out_other_sessions(db: PgPool) {
    let app = router(db);
    let first = admin(&app).await;
    let other = login(&app, "admin@example.com", PASSWORD)
        .await
        .cookie
        .unwrap();

    let body = json!({ "current": PASSWORD, "new": "another-password-456" });
    let reply = call(&app, "POST", "/api/auth/password", Some(&first), Some(body)).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);

    assert_eq!(
        call(&app, "GET", "/api/agents", Some(&other), None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&app, "GET", "/api/agents", reply.cookie.as_deref(), None)
            .await
            .status,
        StatusCode::OK
    );
}

#[sqlx::test]
async fn agent_enrolls_once_heartbeats_and_can_be_revoked(db: PgPool) {
    let app = router(db);
    let cookie = admin(&app).await;
    let token = call(
        &app,
        "POST",
        "/api/agents/tokens",
        Some(&cookie),
        Some(json!({ "name": "dc-1" })),
    )
    .await;
    let enroll = json!({
        "token": token.body["token"],
        "version": "0.1.0",
        "public_key": e2ee::encode(&Keypair::generate().public),
    });

    let first = call(
        &app,
        "POST",
        "/api/agent/enroll",
        None,
        Some(enroll.clone()),
    )
    .await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(
        call(&app, "POST", "/api/agent/enroll", None, Some(enroll))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );

    let beat = |secret: String| {
        Request::post("/api/agent/heartbeat")
            .header(header::AUTHORIZATION, format!("Bearer {secret}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"version":"0.1.1"}"#))
            .unwrap()
    };
    let secret = first.body["secret"].as_str().unwrap().to_owned();
    assert_eq!(
        send(&app, beat(secret.clone())).await.status,
        StatusCode::NO_CONTENT
    );

    let list = call(&app, "GET", "/api/agents", Some(&cookie), None).await;
    assert_eq!(list.body[0]["online"], true);
    assert_eq!(list.body[0]["version"], "0.1.1");

    let id = first.body["agent_id"].as_str().unwrap();
    call(
        &app,
        "DELETE",
        &format!("/api/agents/{id}"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(
        send(&app, beat(secret)).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test]
async fn audit_lines_name_the_account_and_the_client(db: PgPool) {
    let app = router(db);
    let cookie = admin(&app).await;
    let long = format!("TestBrowser/1.0 {}", "x".repeat(400));
    let request = Request::post("/api/auth/login")
        .header("x-dbm-csrf", "1")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::USER_AGENT, &long)
        .body(Body::from(
            json!({ "email": "nobody@example.com", "password": "wrong-password-1" }).to_string(),
        ))
        .unwrap();
    send(&app, request).await;
    let page = call(&app, "GET", "/api/audit", Some(&cookie), None).await;
    let items = page.body["items"].as_array().unwrap();
    assert_eq!(items[0]["user_agent"], long[..300]);
    assert_eq!(items[0]["user_email"], "");
    assert_eq!(items[0]["ip"], "10.0.0.1");
    let setup = items.last().unwrap();
    assert!(setup["user_email"].as_str().unwrap().contains('@'));
}

#[sqlx::test]
async fn audit_log_cannot_be_changed(db: PgPool) {
    let app = router(db.clone());
    admin(&app).await;
    assert!(
        sqlx::query("DELETE FROM audit_logs")
            .execute(&db)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE audit_logs SET ok = false")
            .execute(&db)
            .await
            .is_err()
    );
}

#[sqlx::test]
async fn turnstile_guards_setup_and_login(db: PgPool) {
    let app = router_with(db.clone(), Some(turnstile(fake_siteverify().await)));
    let state = call(&app, "GET", "/api/auth/state", None, None).await;
    assert_eq!(state.body["turnstile_site_key"], "site");

    let mut setup = json!({ "name": "Admin", "email": "admin@example.com", "password": PASSWORD });
    let refused = call(&app, "POST", "/api/auth/setup", None, Some(setup.clone())).await;
    assert_eq!(refused.body["error"], "captcha");
    setup["turnstile_token"] = json!("setup@dbm.example.com");
    let created = call(&app, "POST", "/api/auth/setup", None, Some(setup)).await;
    assert_eq!(created.status, StatusCode::OK);

    let attempt = |token: &str| json!({ "email": "admin@example.com", "password": PASSWORD, "turnstile_token": token });
    // Rejected by Cloudflare, solved for another form, solved on another site.
    for token in ["bot", "setup@dbm.example.com", "login@evil.example.com"] {
        let reply = call(&app, "POST", "/api/auth/login", None, Some(attempt(token))).await;
        assert_eq!(reply.body["error"], "captcha", "{token}");
        assert!(reply.cookie.is_none());
    }
    let human = call(
        &app,
        "POST",
        "/api/auth/login",
        None,
        Some(attempt("login@dbm.example.com")),
    )
    .await;
    assert_eq!(human.status, StatusCode::OK);

    // An unreachable verifier refuses instead of letting the request through.
    let app = router_with(db, Some(turnstile("http://127.0.0.1:1/".into())));
    let reply = call(
        &app,
        "POST",
        "/api/auth/login",
        None,
        Some(attempt("login@dbm.example.com")),
    )
    .await;
    assert_eq!(reply.body["error"], "captcha_unavailable");
}

#[test]
fn turnstile_needs_a_hostname_allowlist() {
    assert!(Turnstile::new("s".into(), "k".into(), " , ", "http://x/".into()).is_none());
}

#[sqlx::test]
async fn console_reset_replaces_password_and_ends_sessions(db: PgPool) {
    let app = router(db.clone());
    let old_session = admin(&app).await;

    assert!(
        reset_password(&db, "admin@example.com", "short")
            .await
            .is_err()
    );
    let unknown = reset_password(&db, "nobody@example.com", "brand-new-password-9").await;
    assert!(unknown.unwrap_err().contains("admin@example.com"));

    reset_password(&db, " Admin@Example.com ", "brand-new-password-9")
        .await
        .unwrap();
    let stale = call(&app, "GET", "/api/agents", Some(&old_session), None).await;
    assert_eq!(stale.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        login(&app, "admin@example.com", PASSWORD).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        login(&app, "admin@example.com", "brand-new-password-9")
            .await
            .status,
        StatusCode::OK
    );
}

/// What a client does: seal, send, open. Returns the inner status, body and any cookie.
#[allow(clippy::too_many_arguments)] // test client: every part of a request varies
async fn sealed(
    app: &Router,
    server: &Keypair,
    method: &str,
    path: &str,
    timestamp: i64,
    bearer: &str,
    body: &[u8],
    cookie: Option<&str>,
) -> (Reply, Option<Request<Body>>) {
    let aad = e2ee::request_aad(method, path, timestamp);
    let sealed = e2ee::seal_request(&server.public, &aad, bearer, body, false).unwrap();
    let make = || {
        let mut header = format!("1.{timestamp}.{}", e2ee::encode(&sealed.exchange.enc));
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("x-dbm-csrf", "1");
        if method == "GET" {
            header = format!("{header}.{}", e2ee::encode(&sealed.ciphertext));
        } else {
            request = request.header(header::CONTENT_TYPE, e2ee::CONTENT_TYPE);
        }
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie);
        }
        let body = if method == "GET" {
            Body::empty()
        } else {
            Body::from(sealed.ciphertext.clone())
        };
        request.header(e2ee::HEADER, header).body(body).unwrap()
    };
    let response = app.clone().oneshot(make()).await.unwrap();
    let encrypted = response
        .headers()
        .get(header::CONTENT_TYPE)
        .is_some_and(|v| v == e2ee::CONTENT_TYPE);
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned());
    let outer = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let (status, plain) = if encrypted {
        let (status, plain) = e2ee::open_response(&sealed.exchange, &bytes).unwrap();
        // Nothing readable may be left in what travelled.
        assert!(!bytes.windows(6).any(|w| w == b"\"email" || w == b"passwo"));
        (StatusCode::from_u16(status).unwrap(), plain)
    } else {
        (outer, bytes.to_vec())
    };
    let body = serde_json::from_slice(&plain).unwrap_or(Value::Null);
    (
        Reply {
            status,
            body,
            cookie: set_cookie,
        },
        Some(make()),
    )
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[sqlx::test]
async fn payload_encryption_covers_browser_and_agent(db: PgPool) {
    let key = Keypair::generate();
    let app = build(db, None, true, key.clone());

    // Plain requests are refused; the key itself is public.
    let plain = call(&app, "GET", "/api/auth/state", None, None).await;
    assert_eq!(plain.body["error"], "e2ee_required");
    let info = call(&app, "GET", "/api/e2ee", None, None).await;
    assert_eq!(info.body["required"], true);
    assert_eq!(info.body["public_key"], e2ee::encode(&key.public));

    let setup = json!({ "name": "A", "email": "a@example.com", "password": PASSWORD }).to_string();
    let (reply, replay) = sealed(
        &app,
        &key,
        "POST",
        "/api/auth/setup",
        now(),
        "",
        setup.as_bytes(),
        None,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body["email"], "a@example.com");
    let cookie = reply.cookie.unwrap();
    assert_eq!(
        send(&app, replay.unwrap()).await.body["error"],
        "e2ee_replay"
    );

    // A GET carries its own proof, and the query string is part of it.
    let (reply, _) = sealed(
        &app,
        &key,
        "GET",
        "/api/audit?action=create",
        now(),
        "",
        b"",
        Some(&cookie),
    )
    .await;
    assert_eq!(reply.body["items"].as_array().unwrap().len(), 1);
    // Errors come back encrypted with the real status.
    let (reply, _) = sealed(&app, &key, "GET", "/api/users", now(), "", b"", None).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);

    let (stale, _) = sealed(
        &app,
        &key,
        "GET",
        "/api/users",
        now() - 600,
        "",
        b"",
        Some(&cookie),
    )
    .await;
    assert_eq!(stale.body["error"], "e2ee_clock");
    let (wrong, _) = sealed(
        &app,
        &Keypair::generate(),
        "GET",
        "/api/users",
        now(),
        "",
        b"",
        Some(&cookie),
    )
    .await;
    assert_eq!(wrong.body["error"], "e2ee_invalid");

    // The agent's credential travels inside the envelope.
    let token = json!({ "name": "dc-1" }).to_string();
    let (token, _) = sealed(
        &app,
        &key,
        "POST",
        "/api/agents/tokens",
        now(),
        "",
        token.as_bytes(),
        Some(&cookie),
    )
    .await;
    let agent = Keypair::generate();
    let enroll = json!({ "token": token.body["token"], "version": "1", "public_key": e2ee::encode(&agent.public) }).to_string();
    let (enrolled, _) = sealed(
        &app,
        &key,
        "POST",
        "/api/agent/enroll",
        now(),
        "",
        enroll.as_bytes(),
        None,
    )
    .await;
    let bearer = format!("Bearer {}", enrolled.body["secret"].as_str().unwrap());
    let (beat, _) = sealed(
        &app,
        &key,
        "POST",
        "/api/agent/heartbeat",
        now(),
        &bearer,
        br#"{"version":"1"}"#,
        None,
    )
    .await;
    assert_eq!(beat.status, StatusCode::NO_CONTENT);

    let (list, _) = sealed(
        &app,
        &key,
        "GET",
        "/api/agents",
        now(),
        "",
        b"",
        Some(&cookie),
    )
    .await;
    assert_eq!(
        list.body[0]["fingerprint"],
        e2ee::fingerprint(&agent.public)
    );
}

#[sqlx::test]
async fn switched_off_it_still_accepts_pinned_clients(db: PgPool) {
    let key = Keypair::generate();
    let app = build(db, None, false, key.clone());
    assert_eq!(
        call(&app, "GET", "/api/auth/state", None, None)
            .await
            .status,
        StatusCode::OK
    );
    let (reply, _) = sealed(&app, &key, "GET", "/api/auth/state", now(), "", b"", None).await;
    assert_eq!(reply.body["setup_required"], true);
}

#[sqlx::test]
async fn the_server_key_needs_the_right_master_key(db: PgPool) {
    let master = MasterKey::parse("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap();
    let first = load_transport_key(&db, &master).await.unwrap();
    let again = load_transport_key(&db, &master).await.unwrap();
    assert_eq!(first.public, again.public);
    assert_eq!(first.private, again.private);
    let stored: Vec<u8> = sqlx::query_scalar("SELECT secret FROM server_keys")
        .fetch_one(&db)
        .await
        .unwrap();
    assert!(
        !stored
            .windows(first.private.len())
            .any(|w| w == first.private)
    );

    let other = MasterKey::parse("AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=").unwrap();
    assert!(load_transport_key(&db, &other).await.is_err());
    // A value cannot be moved to another row.
    let sealed = master.seal("t", "c", "row-1", b"secret").unwrap();
    assert!(master.open("t", "c", "row-2", &sealed).is_err());
    assert_eq!(master.open("t", "c", "row-1", &sealed).unwrap(), b"secret");
    assert!(MasterKey::parse("c2hvcnQ=").is_err());
}

struct Agent {
    id: String,
    bearer: String,
    keys: Keypair,
}

async fn enroll(app: &Router, cookie: &str, name: &str) -> Agent {
    let token = call(
        app,
        "POST",
        "/api/agents/tokens",
        Some(cookie),
        Some(json!({ "name": name })),
    )
    .await;
    let keys = Keypair::generate();
    let body = json!({ "token": token.body["token"], "version": "1", "public_key": e2ee::encode(&keys.public) });
    let reply = call(app, "POST", "/api/agent/enroll", None, Some(body)).await;
    Agent {
        id: reply.body["agent_id"].as_str().unwrap().into(),
        bearer: format!("Bearer {}", reply.body["secret"].as_str().unwrap()),
        keys,
    }
}

async fn as_agent(app: &Router, agent: &Agent, path: &str, body: Value) -> Reply {
    let request = Request::post(path)
        .header(header::AUTHORIZATION, &agent.bearer)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    send(app, request).await
}

/// What the browser does: seal to the agent, bound to the record's id.
fn sealed_for(agent: &Agent, id: &str, kind: &str, plain: &str) -> String {
    let context = e2ee::SecretContext {
        agent_id: &agent.id,
        secret_id: id,
        kind,
        version: 1,
    };
    e2ee::encode(&e2ee::seal_secret(&agent.keys.public, &context, plain.as_bytes()).unwrap())
}

async fn add_resources(app: &Router, cookie: &str, agent: &Agent) -> (String, String, String) {
    let server = uuid::Uuid::new_v4().to_string();
    let destination = uuid::Uuid::new_v4().to_string();
    let body = json!({
        "id": server, "name": "Prod PG", "agent_id": agent.id, "host": "db", "port": 5432, "username": "u",
        "secret": sealed_for(agent, &server, "database", r#"{"password":"hunter2-very-secret"}"#),
    });
    assert_eq!(
        call(
            app,
            "POST",
            "/api/database-servers",
            Some(cookie),
            Some(body)
        )
        .await
        .status,
        StatusCode::CREATED
    );
    let body = json!({
        "id": destination, "name": "minio", "agent_id": agent.id, "endpoint": "http://s3", "bucket": "b", "prefix": "",
        "secret": sealed_for(agent, &destination, "destination", r#"{"secret_key":"s3-very-secret"}"#),
    });
    assert_eq!(
        call(app, "POST", "/api/destinations", Some(cookie), Some(body))
            .await
            .status,
        StatusCode::CREATED
    );
    let body = json!({
        "name": "nightly", "server_id": server, "database_name": "shop", "destination_id": destination,
        "compression": "zstd", "level": 3,
    });
    assert_eq!(
        call(app, "POST", "/api/backups", Some(cookie), Some(body))
            .await
            .status,
        StatusCode::CREATED
    );
    let backups = call(app, "GET", "/api/backups", Some(cookie), None).await;
    let backup = backups.body[0]["id"].as_str().unwrap().to_owned();
    (server, destination, backup)
}

#[sqlx::test]
async fn backup_restore_and_delete_flow_through_the_agent(db: PgPool) {
    let app = router(db.clone());
    let cookie = admin(&app).await;
    let agent = enroll(&app, &cookie, "a1").await;
    let (server, _, backup) = add_resources(&app, &cookie, &agent).await;

    // Only ciphertext is stored.
    let stored: Vec<u8> = sqlx::query_scalar("SELECT secret FROM database_servers")
        .fetch_one(&db)
        .await
        .unwrap();
    assert!(!stored.windows(7).any(|w| w == b"hunter2"));

    let run = format!("/api/backups/{backup}/run");
    let queued = call(&app, "POST", &run, Some(&cookie), None).await;
    assert_eq!(queued.status, StatusCode::OK);
    let job_id = queued.body["job_id"].as_str().unwrap().to_owned();
    assert_eq!(
        call(&app, "POST", &run, Some(&cookie), None).await.body["error"],
        "already_running"
    );

    let claimed = as_agent(&app, &agent, "/api/agent/jobs/claim", json!({})).await;
    let job = &claimed.body["job"];
    assert_eq!(job["id"], job_id.as_str());
    assert_eq!(job["task"]["kind"], "backup");
    assert!(
        job["task"]["recipients"][0]
            .as_str()
            .unwrap()
            .starts_with("age1")
    );
    assert!(
        job["task"]["object_key"]
            .as_str()
            .unwrap()
            .starts_with("prod-pg/shop/")
    );
    // Nothing else is waiting, and another agent never sees this work.
    assert!(
        as_agent(&app, &agent, "/api/agent/jobs/claim", json!({}))
            .await
            .body["job"]
            .is_null()
    );

    let lease = job["lease"].clone();
    let report = json!({ "id": job_id, "lease": lease, "bytes": 10, "logs": [{ "level": "info", "message": "dumping" }] });
    let reply = as_agent(&app, &agent, "/api/agent/jobs/report", report).await;
    assert_eq!(reply.body["cancel"], false);
    // A stale attempt cannot write.
    let stale = json!({ "id": job_id, "lease": uuid::Uuid::new_v4(), "bytes": 1, "logs": [] });
    assert_eq!(
        as_agent(&app, &agent, "/api/agent/jobs/report", stale)
            .await
            .body["error"],
        "lease_lost"
    );

    let sha = "a".repeat(64);
    let done = json!({ "id": job_id, "lease": lease, "bytes": 1234, "logs": [], "error": null, "sha256": sha });
    assert_eq!(
        as_agent(&app, &agent, "/api/agent/jobs/finish", done)
            .await
            .status,
        StatusCode::NO_CONTENT
    );

    let detail = call(
        &app,
        "GET",
        &format!("/api/jobs/{job_id}?after=0"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(detail.body["status"], "completed");
    assert_eq!(detail.body["logs"][0]["message"], "dumping");
    let snapshots = call(&app, "GET", "/api/snapshots", Some(&cookie), None).await;
    let snapshot = &snapshots.body["items"][0];
    assert_eq!(
        (&snapshot["server_name"], &snapshot["size"]),
        (&json!("Prod PG"), &json!(1234))
    );
    let snapshot_id = snapshot["id"].as_str().unwrap().to_owned();

    // Restore: the private key arrives sealed so only this agent can open it, for this job.
    let restore = format!("/api/snapshots/{snapshot_id}/restore");
    let bad = json!({ "server_id": server, "database_name": "host=evil" });
    assert_eq!(
        call(&app, "POST", &restore, Some(&cookie), Some(bad))
            .await
            .body["error"],
        "database_name"
    );
    let body = json!({ "server_id": server, "database_name": "shop_restore" });
    assert_eq!(
        call(&app, "POST", &restore, Some(&cookie), Some(body))
            .await
            .status,
        StatusCode::OK
    );
    let claimed = as_agent(&app, &agent, "/api/agent/jobs/claim", json!({})).await;
    let identity = &claimed.body["job"]["task"]["identity"];
    let restore_job = claimed.body["job"]["id"].as_str().unwrap();
    let context = e2ee::SecretContext {
        agent_id: &agent.id,
        secret_id: restore_job,
        kind: "backup-key",
        version: 1,
    };
    let opened = e2ee::open_secret(
        &agent.keys.private,
        &context,
        &e2ee::decode(identity["data"].as_str().unwrap()).unwrap(),
    );
    assert!(opened.unwrap().starts_with(b"AGE-SECRET-KEY-"));

    // Records in use cannot be removed.
    let used = call(
        &app,
        "DELETE",
        &format!("/api/database-servers/{server}"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(used.body["error"], "in_use");
    let used = call(
        &app,
        "DELETE",
        &format!("/api/agents/{}", agent.id),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(used.body["error"], "in_use");

    // The record stays until the agent confirms the file is gone.
    call(
        &app,
        "DELETE",
        &format!("/api/snapshots/{snapshot_id}"),
        Some(&cookie),
        None,
    )
    .await;
    let claimed = as_agent(&app, &agent, "/api/agent/jobs/claim", json!({})).await;
    assert_eq!(claimed.body["job"]["task"]["kind"], "delete_snapshot");
    let done = json!({ "id": claimed.body["job"]["id"], "lease": claimed.body["job"]["lease"], "bytes": 0, "logs": [], "error": null, "sha256": null });
    as_agent(&app, &agent, "/api/agent/jobs/finish", done).await;
    let snapshots = call(&app, "GET", "/api/snapshots", Some(&cookie), None).await;
    assert_eq!(snapshots.body["items"].as_array().unwrap().len(), 0);
}

#[sqlx::test]
async fn viewers_read_only_and_key_export_needs_the_password(db: PgPool) {
    let app = router(db.clone());
    let cookie = admin(&app).await;
    let agent = enroll(&app, &cookie, "a1").await;
    let (server, destination, backup) = add_resources(&app, &cookie, &agent).await;
    let other = enroll(&app, &cookie, "a2").await;

    let viewer =
        json!({ "name": "V", "email": "v@example.com", "password": PASSWORD, "role": "viewer" });
    call(&app, "POST", "/api/users", Some(&cookie), Some(viewer)).await;
    let viewer = login(&app, "v@example.com", PASSWORD).await.cookie.unwrap();
    for path in [
        "/api/database-servers",
        "/api/destinations",
        "/api/backups",
        "/api/snapshots",
        "/api/jobs",
    ] {
        assert_eq!(
            call(&app, "GET", path, Some(&viewer), None).await.status,
            StatusCode::OK,
            "{path}"
        );
    }
    for (method, path) in [
        ("POST", format!("/api/backups/{backup}/run")),
        ("POST", format!("/api/database-servers/{server}/test")),
        ("DELETE", format!("/api/destinations/{destination}")),
        ("DELETE", format!("/api/backups/{backup}")),
    ] {
        assert_eq!(
            call(&app, method, &path, Some(&viewer), None).await.status,
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }
    let export = json!({ "password": PASSWORD });
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/backup-keys/export",
            Some(&viewer),
            Some(export.clone())
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );

    let wrong = json!({ "password": "not-the-password" });
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/backup-keys/export",
            Some(&cookie),
            Some(wrong)
        )
        .await
        .body["error"],
        "wrong_password"
    );
    let keys = call(
        &app,
        "POST",
        "/api/backup-keys/export",
        Some(&cookie),
        Some(export),
    )
    .await;
    assert!(
        keys.body[0]["identity"]
            .as_str()
            .unwrap()
            .starts_with("AGE-SECRET-KEY-")
    );
    let stored: Vec<u8> = sqlx::query_scalar("SELECT secret FROM backup_keys")
        .fetch_one(&db)
        .await
        .unwrap();
    assert!(!stored.windows(14).any(|w| w == b"AGE-SECRET-KEY"));

    // Another agent gets none of this one's work.
    call(
        &app,
        "POST",
        &format!("/api/backups/{backup}/run"),
        Some(&cookie),
        None,
    )
    .await;
    assert!(
        as_agent(&app, &other, "/api/agent/jobs/claim", json!({}))
            .await
            .body["job"]
            .is_null()
    );
}

#[sqlx::test]
async fn silent_agents_lose_their_jobs_and_pending_jobs_cancel(db: PgPool) {
    let app = router(db.clone());
    let cookie = admin(&app).await;
    let agent = enroll(&app, &cookie, "a1").await;
    let (server, _, backup) = add_resources(&app, &cookie, &agent).await;

    let test = call(
        &app,
        "POST",
        &format!("/api/database-servers/{server}/test"),
        Some(&cookie),
        None,
    )
    .await;
    let id = test.body["job_id"].as_str().unwrap().to_owned();
    call(
        &app,
        "POST",
        &format!("/api/jobs/{id}/cancel"),
        Some(&cookie),
        None,
    )
    .await;
    assert!(
        as_agent(&app, &agent, "/api/agent/jobs/claim", json!({}))
            .await
            .body["job"]
            .is_null()
    );

    let run = call(
        &app,
        "POST",
        &format!("/api/backups/{backup}/run"),
        Some(&cookie),
        None,
    )
    .await;
    let id = run.body["job_id"].as_str().unwrap().to_owned();
    as_agent(&app, &agent, "/api/agent/jobs/claim", json!({})).await;
    sqlx::query(
        "UPDATE jobs SET lease_expires_at = now() - interval '1 second' WHERE status = 'running'",
    )
    .execute(&db)
    .await
    .unwrap();
    let detail = call(&app, "GET", &format!("/api/jobs/{id}"), Some(&cookie), None).await;
    assert_eq!(detail.body["status"], "failed");
}

fn state_for(db: PgPool, e2ee_required: bool, key: Keypair) -> AppState {
    let config = Config {
        cookie_secure: true,
        trust_proxy: false,
        turnstile: None,
        e2ee_required,
        extra_recipient: None,
        telegram_api: "http://127.0.0.1:9".into(),
    };
    let master = MasterKey::parse("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap();
    AppState::new(db, config, key, master)
}

/// The next chunk of a response that has not ended, or `None` once it has.
async fn chunk(body: &mut Body) -> Option<Vec<u8>> {
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), body.frame())
        .await
        .expect("a chunk within 5 s")?;
    Some(frame.unwrap().into_data().unwrap().to_vec())
}

#[sqlx::test]
async fn events_announce_changes_to_signed_in_pages(db: PgPool) {
    let state = state_for(db.clone(), false, Keypair::generate());
    state.watch_changes().await.unwrap();
    let app = app(state.clone());
    assert_eq!(
        call(&app, "GET", "/api/events", None, None).await.status,
        StatusCode::UNAUTHORIZED
    );

    let cookie = admin(&app).await;
    let viewer =
        json!({ "name": "V", "email": "v@example.com", "password": PASSWORD, "role": "viewer" });
    let created = call(&app, "POST", "/api/users", Some(&cookie), Some(viewer)).await;
    assert!(created.status.is_success());
    let viewer = login(&app, "v@example.com", PASSWORD).await.cookie.unwrap();

    let open = |cookie: String| {
        let app = app.clone();
        async move {
            let request = Request::get("/api/events")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap();
            let response = app.oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let mut body = response.into_body();
            assert_eq!(chunk(&mut body).await.unwrap(), b"*\n");
            body
        }
    };
    // Changes from before the streams may still be in flight.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let mut as_admin = open(cookie.clone()).await;
    let mut as_viewer = open(viewer).await;

    // An update that matches nothing is not a change.
    sqlx::query("UPDATE jobs SET bytes = 1")
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET name = 'Renamed' WHERE email = 'v@example.com'")
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(chunk(&mut as_admin).await.unwrap(), b"users\n");
    sqlx::query("INSERT INTO agents (name, token_hash, version) VALUES ('a', '\\x00', '1')")
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(chunk(&mut as_admin).await.unwrap(), b"agents\n");
    // The viewer never hears about the users table.
    assert_eq!(chunk(&mut as_viewer).await.unwrap(), b"agents\n");

    state.close_streams();
    assert!(chunk(&mut as_admin).await.is_none());
}

#[sqlx::test]
async fn events_are_encrypted_chunk_by_chunk(db: PgPool) {
    let key = Keypair::generate();
    let plain = app(state_for(db.clone(), false, key.clone()));
    let cookie = admin(&plain).await;
    let state = state_for(db, true, key.clone());
    let app = app(state);

    let timestamp = time::OffsetDateTime::now_utc().unix_timestamp();
    let aad = e2ee::request_aad("GET", "/api/events", timestamp);
    let sealed = e2ee::seal_request(&key.public, &aad, "", b"", false).unwrap();
    let header_value = format!(
        "1.{timestamp}.{}.{}",
        e2ee::encode(&sealed.exchange.enc),
        e2ee::encode(&sealed.ciphertext)
    );
    let request = Request::get("/api/events")
        .header(header::COOKIE, cookie)
        .header(e2ee::HEADER, header_value)
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        e2ee::STREAM_CONTENT_TYPE
    );

    let mut body = response.into_body();
    let seed = chunk(&mut body).await.unwrap();
    let mut cipher = e2ee::StreamCipher::new(&sealed.exchange, &seed);
    let first = chunk(&mut body).await.unwrap();
    // Length prefix, two bytes of text and the authentication tag.
    assert_eq!(first.len(), 4 + 2 + 16);
    assert_eq!(cipher.open(&first[4..]).unwrap(), b"*\n");
}

#[sqlx::test]
async fn only_large_replies_to_a_get_that_asks_are_compressed(db: PgPool) {
    let key = Keypair::generate();
    let plain = app(state_for(db.clone(), false, key.clone()));
    let cookie = admin(&plain).await;
    // Enough audit lines to be worth compressing.
    for _ in 0..4 {
        for n in 0..4 {
            login(
                &plain,
                &format!("nobody{n}@example.com"),
                "wrong-password-1",
            )
            .await;
        }
    }
    let app = app(state_for(db, true, key.clone()));
    let read = async |path: &str, gzip: bool| {
        let aad = e2ee::request_aad("GET", path, now());
        let sealed = e2ee::seal_request(&key.public, &aad, "", b"", false).unwrap();
        let value = format!(
            "1.{}.{}.{}",
            now(),
            e2ee::encode(&sealed.exchange.enc),
            e2ee::encode(&sealed.ciphertext)
        );
        let mut request = Request::get(path)
            .header(header::COOKIE, &cookie)
            .header(e2ee::HEADER, value);
        if gzip {
            request = request.header(e2ee::GZIP_HEADER, "1");
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.headers()[e2ee::GZIP_HEADER], "1");
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let (status, body) = e2ee::open_response(&sealed.exchange, &bytes).unwrap();
        assert_eq!(status, 200);
        (bytes.len(), body)
    };
    let (whole, expected) = read("/api/audit", false).await;
    let (small, body) = read("/api/audit", true).await;
    assert!(expected.len() >= e2ee::GZIP_MIN);
    assert!(small < whole / 2, "{small} of {whole}");
    assert_eq!(body, expected);
}

// ---------------------------------------------------------------- schedules, retention, alerts

/// Claims the next job of `agent` and reports it done.
async fn finish_next(app: &Router, agent: &Agent, extra: Value) -> Value {
    let claimed = as_agent(app, agent, "/api/agent/jobs/claim", json!({})).await;
    let job = claimed.body["job"].clone();
    assert!(!job.is_null(), "a job is waiting");
    let mut done = json!({ "id": job["id"], "lease": job["lease"], "bytes": 10, "logs": [], "error": null,
        "sha256": "a".repeat(64) });
    for (key, value) in extra.as_object().unwrap() {
        done[key] = value.clone();
    }
    let reply = as_agent(app, agent, "/api/agent/jobs/finish", done).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    job
}

async fn count(db: &PgPool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(db).await.unwrap()
}

#[sqlx::test]
async fn schedules_run_when_due(db: PgPool) {
    let state = state_for(db.clone(), false, Keypair::generate());
    let app = app(state.clone());
    let cookie = admin(&app).await;
    let agent = enroll(&app, &cookie, "a1").await;
    let (server, destination, backup) = add_resources(&app, &cookie, &agent).await;
    let path = format!("/api/backups/{backup}");
    let entry = |plan: Value| backup_body(&server, &destination, plan);

    for bad in [
        json!({ "schedule": "every day" }),
        json!({ "schedule": "0 1 * * *", "timezone": "Nowhere" }),
        json!({ "retention_unit": "days", "retention_value": 0 }),
        json!({ "retention_unit": "gfs" }),
        json!({ "retention_unit": "forever" }),
    ] {
        let reply = call(&app, "PUT", &path, Some(&cookie), Some(entry(bad.clone()))).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }
    let plan = json!({ "schedule": "0 1 * * *", "timezone": "Asia/Jakarta", "retention_unit": "days", "retention_value": 7 });
    let reply = call(&app, "PUT", &path, Some(&cookie), Some(entry(plan))).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    let list = call(&app, "GET", "/api/backups", Some(&cookie), None).await;
    assert!(list.body[0]["next_run_at"].is_string());

    state.schedule_once().await;
    assert_eq!(
        count(&db, "SELECT count(*) FROM jobs").await,
        0,
        "not due yet"
    );

    sqlx::query("UPDATE backup_configs SET next_run_at = now() - interval '3 days'")
        .execute(&db)
        .await
        .unwrap();
    state.schedule_once().await;
    state.schedule_once().await;
    // Missed runs are made up once, and the next one is in the future.
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM jobs WHERE created_by = 'schedule'"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM backup_configs WHERE next_run_at > now()"
        )
        .await,
        1
    );

    finish_next(&app, &agent, json!({ "manifest": "tag" })).await;
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM snapshots WHERE manifest = 'tag'
                    AND expires_at BETWEEN now() + interval '6 days' AND now() + interval '8 days'"
        )
        .await,
        1
    );
}

/// Snapshots of one backup entry, `age` days old each.
async fn old_snapshots(db: &PgPool, ages: &[i32]) {
    for age in ages {
        sqlx::query(
            "INSERT INTO snapshots (job_id, backup_config_id, server_name, database_name, destination_id,
                 object_key, size, sha256, compression, key_id, created_at, expires_at)
             SELECT gen_random_uuid(), c.id, 'Prod PG', 'shop', c.destination_id, 'k' || $1, 1, 'x', 'none',
                 (SELECT max(id) FROM backup_keys), now() - make_interval(days => $1),
                 CASE c.retention_unit WHEN 'days' THEN now() - make_interval(days => $1 - c.retention_value) END
             FROM backup_configs c",
        )
        .bind(age)
        .execute(db)
        .await
        .unwrap();
    }
}

async fn queued_deletes(db: &PgPool) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT object_key FROM jobs WHERE kind = 'delete_snapshot' ORDER BY object_key",
    )
    .fetch_all(db)
    .await
    .unwrap()
}

#[sqlx::test]
async fn retention_deletes_the_old_but_never_the_last(db: PgPool) {
    let state = state_for(db.clone(), false, Keypair::generate());
    let app = app(state.clone());
    let cookie = admin(&app).await;
    let agent = enroll(&app, &cookie, "a1").await;
    let (server, destination, backup) = add_resources(&app, &cookie, &agent).await;
    sqlx::query("INSERT INTO backup_keys (recipient, secret) VALUES ('age1', '\\x00')")
        .execute(&db)
        .await
        .unwrap();
    let path = format!("/api/backups/{backup}");
    let plan = json!({ "retention_unit": "days", "retention_value": 7 });
    let body = backup_body(&server, &destination, plan);
    call(&app, "PUT", &path, Some(&cookie), Some(body)).await;

    // All expired: the newest one stays anyway.
    old_snapshots(&db, &[30, 20, 10]).await;
    state.schedule_once().await;
    state.schedule_once().await;
    assert_eq!(queued_deletes(&db).await, ["k20", "k30"]);
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM audit_logs WHERE action = 'delete' AND ip = 'system'"
        )
        .await,
        2
    );
}

#[sqlx::test]
async fn gfs_keeps_the_newest_of_each_period(db: PgPool) {
    let state = state_for(db.clone(), false, Keypair::generate());
    let app = app(state.clone());
    let cookie = admin(&app).await;
    let agent = enroll(&app, &cookie, "a1").await;
    add_resources(&app, &cookie, &agent).await;
    sqlx::query("INSERT INTO backup_keys (recipient, secret) VALUES ('age1', '\\x00')")
        .execute(&db)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE backup_configs SET retention_unit = 'gfs', keep_daily = 2, keep_weekly = 0,
                 keep_monthly = 0, retention_since = now() - interval '1 year'",
    )
    .execute(&db)
    .await
    .unwrap();
    old_snapshots(&db, &[0, 1, 2, 3]).await;
    state.schedule_once().await;
    assert_eq!(queued_deletes(&db).await, ["k2", "k3"]);

    // Older than the last change of the retention: left alone.
    sqlx::query("DELETE FROM jobs").execute(&db).await.unwrap();
    sqlx::query("UPDATE backup_configs SET retention_since = now()")
        .execute(&db)
        .await
        .unwrap();
    state.schedule_once().await;
    assert!(queued_deletes(&db).await.is_empty());
}

/// Stands in for Telegram and Mattermost; records what arrives.
async fn fake_chat() -> (
    String,
    std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
) {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = seen.clone();
    let server = Router::new().fallback(
        move |uri: axum::http::Uri, axum::Json(body): axum::Json<Value>| {
            let log = log.clone();
            async move {
                let text = body["text"].as_str().unwrap_or_default().to_owned();
                log.lock().unwrap().push((uri.path().to_owned(), text));
                StatusCode::OK
            }
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
    (url, seen)
}

#[sqlx::test]
async fn failures_and_silent_agents_raise_alerts(db: PgPool) {
    let (chat, seen) = fake_chat().await;
    let config = Config {
        cookie_secure: true,
        trust_proxy: false,
        turnstile: None,
        e2ee_required: false,
        extra_recipient: None,
        telegram_api: chat.clone(),
    };
    let master = MasterKey::parse("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap();
    let state = AppState::new(db.clone(), config, Keypair::generate(), master);
    let app = app(state.clone());
    let cookie = admin(&app).await;
    let agent = enroll(&app, &cookie, "a1").await;
    let (_, _, backup) = add_resources(&app, &cookie, &agent).await;
    let channels = "/api/notification-channels";

    let telegram = json!({ "name": "dba", "events": ["backup_failed"],
        "secret": { "kind": "telegram", "bot_token": "123:very-secret-token", "chat_id": "-100" } });
    let hook = format!("{chat}/hooks/very-secret-hook");
    let mattermost = json!({ "name": "ops", "events": ["agent_offline"],
        "secret": { "kind": "mattermost", "webhook_url": hook } });
    for body in [telegram, mattermost] {
        let reply = call(&app, "POST", channels, Some(&cookie), Some(body)).await;
        assert_eq!(reply.status, StatusCode::CREATED);
    }
    for bad in [
        json!({ "name": "x", "events": [], "secret": { "kind": "mattermost", "webhook_url": hook } }),
        json!({ "name": "x", "events": ["agent_offline"], "secret": { "kind": "mattermost", "webhook_url": "file:///etc/passwd" } }),
        json!({ "name": "x", "events": ["agent_offline"], "secret": { "kind": "telegram", "bot_token": "1:../..", "chat_id": "1" } }),
    ] {
        let reply = call(&app, "POST", channels, Some(&cookie), Some(bad)).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    // Secrets are neither listed nor stored readable.
    let list = call(&app, "GET", channels, Some(&cookie), None).await;
    assert!(!list.body.to_string().contains("very-secret"));
    assert_eq!(
        count(&db, "SELECT count(*) FROM notification_channels WHERE encode(secret, 'escape') LIKE '%very-secret%'").await,
        0
    );
    let viewer =
        json!({ "name": "V", "email": "v@example.com", "password": PASSWORD, "role": "operator" });
    call(&app, "POST", "/api/users", Some(&cookie), Some(viewer)).await;
    let operator = login(&app, "v@example.com", PASSWORD).await.cookie.unwrap();
    let reply = call(&app, "GET", channels, Some(&operator), None).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);

    let run = format!("/api/backups/{backup}/run");
    call(&app, "POST", &run, Some(&cookie), None).await;
    finish_next(
        &app,
        &agent,
        json!({ "error": "disk full", "sha256": null }),
    )
    .await;
    state.schedule_once().await;
    sqlx::query("UPDATE agents SET last_seen_at = now() - interval '1 hour'")
        .execute(&db)
        .await
        .unwrap();
    state.schedule_once().await;
    state.schedule_once().await;

    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "one alert per event: {seen:?}");
    assert_eq!(seen[0].0, "/bot123:very-secret-token/sendMessage");
    assert!(seen[0].1.contains("Backup gagal") && seen[0].1.contains("disk full"));
    assert_eq!(seen[1].0, "/hooks/very-secret-hook");
    assert!(seen[1].1.contains("Agent offline: a1"));

    // An unreachable service is reported without leaking the address.
    sqlx::query("UPDATE notification_channels SET name = name")
        .execute(&db)
        .await
        .unwrap();
    let id = list.body[0]["id"].as_str().unwrap();
    let reply = call(
        &app,
        "POST",
        &format!("{channels}/{id}/test"),
        Some(&cookie),
        None,
    )
    .await;
    assert!(reply.body["error"].is_null());
}

#[sqlx::test]
async fn download_links_are_handed_out_once(db: PgPool) {
    let app = router(db.clone());
    let cookie = admin(&app).await;
    let agent = enroll(&app, &cookie, "a1").await;
    let (_, _, backup) = add_resources(&app, &cookie, &agent).await;
    call(
        &app,
        "POST",
        &format!("/api/backups/{backup}/run"),
        Some(&cookie),
        None,
    )
    .await;
    finish_next(&app, &agent, json!({})).await;
    let snapshots = call(&app, "GET", "/api/snapshots", Some(&cookie), None).await;
    let snapshot = snapshots.body["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let queued = call(
        &app,
        "POST",
        &format!("/api/snapshots/{snapshot}/download"),
        Some(&cookie),
        None,
    )
    .await;
    let job = queued.body["job_id"].as_str().unwrap().to_owned();
    let link = format!("/api/downloads/{job}");
    let early = call(&app, "GET", &link, Some(&cookie), None).await;
    assert_eq!(early.status, StatusCode::NOT_FOUND);

    let url = "https://s3.example/file?X-Amz-Signature=very-secret";
    let claimed = finish_next(&app, &agent, json!({ "result": url, "sha256": null })).await;
    assert_eq!(claimed["task"]["kind"], "presign_snapshot");
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM jobs WHERE encode(result, 'escape') LIKE '%very-secret%'"
        )
        .await,
        0
    );
    let first = call(&app, "GET", &link, Some(&cookie), None).await;
    assert_eq!(first.body["url"], url);
    let second = call(&app, "GET", &link, Some(&cookie), None).await;
    assert_eq!(second.status, StatusCode::NOT_FOUND);

    let dashboard = call(
        &app,
        "GET",
        "/api/dashboard?timezone=Asia/Jakarta",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(dashboard.body["completed_24h"], 1);
    assert_eq!(dashboard.body["activity"].as_array().unwrap().len(), 14);
    assert_eq!(dashboard.body["volumes"][0]["bytes"], 10);
    let bad = call(
        &app,
        "GET",
        "/api/dashboard?timezone=x",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
    let preview = call(
        &app,
        "GET",
        "/api/schedule/preview?schedule=0+1+*+*+*&timezone=UTC",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(preview.body["runs"].as_array().unwrap().len(), 3);
}

// ---------------------------------------------------------------- editing

/// The body of a backup entry, with `plan` fields added.
fn backup_body(server: &str, destination: &str, plan: Value) -> Value {
    let mut body = json!({ "name": "nightly", "server_id": server, "database_name": "shop",
        "destination_id": destination, "compression": "zstd", "level": 3 });
    for (key, value) in plan.as_object().unwrap() {
        body[key] = value.clone();
    }
    body
}

async fn put(app: &Router, cookie: &str, path: &str, body: Value) -> StatusCode {
    call(app, "PUT", path, Some(cookie), Some(body))
        .await
        .status
}

#[sqlx::test]
async fn everything_configured_can_be_edited_by_the_right_role(db: PgPool) {
    let app = router(db.clone());
    let cookie = admin(&app).await;
    let agent = enroll(&app, &cookie, "a1").await;
    enroll(&app, &cookie, "taken").await;
    let (server, destination, backup) = add_resources(&app, &cookie, &agent).await;
    let viewer =
        json!({ "name": "V", "email": "v@example.com", "password": PASSWORD, "role": "viewer" });
    let created = call(&app, "POST", "/api/users", Some(&cookie), Some(viewer)).await;
    let viewer_id = created.body["id"].as_str().unwrap().to_owned();
    let viewer = login(&app, "v@example.com", PASSWORD).await.cookie.unwrap();
    let channel = json!({ "name": "ops", "events": ["agent_offline"],
        "secret": { "kind": "mattermost", "webhook_url": "https://chat.example.com/hooks/first" } });
    call(
        &app,
        "POST",
        "/api/notification-channels",
        Some(&cookie),
        Some(channel),
    )
    .await;
    let channels = call(
        &app,
        "GET",
        "/api/notification-channels",
        Some(&cookie),
        None,
    )
    .await;
    let channel = channels.body[0]["id"].as_str().unwrap().to_owned();

    let server_path = format!("/api/database-servers/{server}");
    let destination_path = format!("/api/destinations/{destination}");
    let backup_path = format!("/api/backups/{backup}");
    let channel_path = format!("/api/notification-channels/{channel}");
    let agent_path = format!("/api/agents/{}", agent.id);
    let user_path = format!("/api/users/{viewer_id}");
    let user = json!({ "name": "Op", "email": "OP@example.com", "role": "operator" });
    for (path, body) in [
        (&server_path, json!({ "name": "x" })),
        (&destination_path, json!({ "name": "x" })),
        (&backup_path, backup_body(&server, &destination, json!({}))),
        (
            &channel_path,
            json!({ "name": "x", "events": ["agent_offline"] }),
        ),
        (&agent_path, json!({ "name": "x" })),
        (&user_path, user.clone()),
    ] {
        assert_eq!(
            put(&app, &viewer, path, body).await,
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }

    // A rename keeps the sealed secret; a new connection replaces it as a whole.
    let sealed_before: Vec<u8> = sqlx::query_scalar("SELECT secret FROM database_servers")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(
        put(&app, &cookie, &server_path, json!({ "name": "Renamed" })).await,
        StatusCode::NO_CONTENT
    );
    let sealed: Vec<u8> = sqlx::query_scalar("SELECT secret FROM database_servers")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(sealed, sealed_before);
    let secret = sealed_for(&agent, &server, "database", r#"{"password":"new"}"#);
    let connection = json!({ "host": "db2", "port": 6000, "username": "u2", "ssl_mode": "require", "secret": secret });
    assert_eq!(
        put(
            &app,
            &cookie,
            &server_path,
            json!({ "name": "Renamed", "connection": connection })
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let list = call(&app, "GET", "/api/database-servers", Some(&cookie), None).await;
    assert_eq!(
        (&list.body[0]["host"], &list.body[0]["ssl_mode"]),
        (&json!("db2"), &json!("require"))
    );
    // Host without a new secret is not accepted: the two would disagree.
    let partial = json!({ "name": "Renamed", "connection": { "host": "evil" } });
    assert_eq!(
        put(&app, &cookie, &server_path, partial).await,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let storage = |bucket: &str| {
        json!({ "name": "minio", "connection": { "endpoint": "http://s3-new", "bucket": bucket, "prefix": "",
        "public_endpoint": "http://localhost:9000", "virtual_host": false,
        "secret": sealed_for(&agent, &destination, "destination", "{}") } })
    };
    assert_eq!(
        put(&app, &cookie, &destination_path, storage("other")).await,
        StatusCode::NO_CONTENT
    );
    // With backups stored there, the place stays; the address may still change.
    call(
        &app,
        "POST",
        &format!("/api/backups/{backup}/run"),
        Some(&cookie),
        None,
    )
    .await;
    finish_next(&app, &agent, json!({})).await;
    assert_eq!(
        put(&app, &cookie, &destination_path, storage("moved")).await,
        StatusCode::CONFLICT
    );
    assert_eq!(
        put(&app, &cookie, &destination_path, storage("other")).await,
        StatusCode::NO_CONTENT
    );
    let list = call(&app, "GET", "/api/destinations", Some(&cookie), None).await;
    assert_eq!(list.body[0]["public_endpoint"], "http://localhost:9000");

    let mut changed = backup_body(&server, &destination, json!({ "schedule": "0 2 * * *" }));
    changed["name"] = json!("weekly");
    changed["database_name"] = json!("orders");
    assert_eq!(
        put(&app, &cookie, &backup_path, changed.clone()).await,
        StatusCode::NO_CONTENT
    );
    let list = call(&app, "GET", "/api/backups", Some(&cookie), None).await;
    assert_eq!(
        (&list.body[0]["name"], &list.body[0]["database_name"]),
        (&json!("weekly"), &json!("orders"))
    );
    changed["database_name"] = json!("bad name");
    assert_eq!(
        put(&app, &cookie, &backup_path, changed).await,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // The webhook stays unless a new one is given, and is never echoed.
    assert_eq!(
        put(
            &app,
            &cookie,
            &channel_path,
            json!({ "name": "ops2", "events": ["backup_failed"] })
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let list = call(
        &app,
        "GET",
        "/api/notification-channels",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(list.body[0]["target"], "chat.example.com");
    let replaced = json!({ "name": "ops2", "events": ["backup_failed"],
        "secret": { "kind": "telegram", "bot_token": "1:abc", "chat_id": "-5" } });
    assert_eq!(
        put(&app, &cookie, &channel_path, replaced).await,
        StatusCode::NO_CONTENT
    );
    let list = call(
        &app,
        "GET",
        "/api/notification-channels",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(
        (&list.body[0]["kind"], &list.body[0]["target"]),
        (&json!("telegram"), &json!("chat -5"))
    );

    let taken = call(
        &app,
        "PUT",
        &agent_path,
        Some(&cookie),
        Some(json!({ "name": "taken" })),
    )
    .await;
    assert_eq!(taken.body["error"], "agent_name_taken");
    assert_eq!(
        put(&app, &cookie, &agent_path, json!({ "name": "dc-1" })).await,
        StatusCode::NO_CONTENT
    );

    // A new role applies at once; nobody changes their own.
    assert_eq!(
        put(&app, &cookie, &user_path, user).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        put(
            &app,
            &viewer,
            &server_path,
            json!({ "name": "By operator" })
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let me = call(&app, "GET", "/api/auth/state", Some(&cookie), None).await;
    let my_path = format!("/api/users/{}", me.body["user"]["id"].as_str().unwrap());
    let demote = json!({ "name": "Admin", "email": "admin@example.com", "role": "viewer" });
    assert_eq!(
        put(&app, &cookie, &my_path, demote).await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let clash = json!({ "name": "Op", "email": "admin@example.com", "role": "operator" });
    let reply = call(&app, "PUT", &user_path, Some(&cookie), Some(clash)).await;
    assert_eq!(reply.body["error"], "email_taken");

    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM audit_logs WHERE action = 'update'"
        )
        .await,
        10
    );
}

// ---------------------------------------------------------------- engines

#[sqlx::test]
async fn a_backup_only_restores_into_its_own_engine(db: PgPool) {
    let app = router(db.clone());
    let cookie = admin(&app).await;
    let agent = enroll(&app, &cookie, "a1").await;
    let (pg, destination, _) = add_resources(&app, &cookie, &agent).await;
    let add = |name: &str, engine: &str| {
        let id = uuid::Uuid::new_v4().to_string();
        let body = json!({ "id": id, "name": name, "engine": engine, "agent_id": agent.id, "host": "", "port": 0,
            "username": "", "secret": sealed_for(&agent, &id, "database", "{}") });
        (id, body)
    };
    let (_, bad) = add("x", "oracle");
    let reply = call(
        &app,
        "POST",
        "/api/database-servers",
        Some(&cookie),
        Some(bad),
    )
    .await;
    assert_eq!(reply.body["error"], "engine");
    let (lite, body) = add("files", "sqlite");
    let reply = call(
        &app,
        "POST",
        "/api/database-servers",
        Some(&cookie),
        Some(body),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED);

    // A file name may carry an extension; a database name may not.
    let entry = |name: &str, server: &str, database: &str| {
        json!({ "name": name, "server_id": server, "database_name": database, "destination_id": destination,
            "compression": "gzip", "level": 3 })
    };
    let reply = call(
        &app,
        "POST",
        "/api/backups",
        Some(&cookie),
        Some(entry("p", &pg, "app.db")),
    )
    .await;
    assert_eq!(reply.body["error"], "database_name");
    for escaping in ["../app.db", ".env"] {
        let reply = call(
            &app,
            "POST",
            "/api/backups",
            Some(&cookie),
            Some(entry("e", &lite, escaping)),
        )
        .await;
        assert_eq!(reply.body["error"], "database_name", "{escaping}");
    }
    let reply = call(
        &app,
        "POST",
        "/api/backups",
        Some(&cookie),
        Some(entry("l", &lite, "app.db")),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED);

    let backups = call(&app, "GET", "/api/backups", Some(&cookie), None).await;
    let id = backups
        .body
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "l")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    call(
        &app,
        "POST",
        &format!("/api/backups/{id}/run"),
        Some(&cookie),
        None,
    )
    .await;
    let job = finish_next(&app, &agent, json!({})).await;
    assert!(
        job["task"]["object_key"]
            .as_str()
            .unwrap()
            .ends_with(".sql.gz.age")
    );
    let snapshots = call(&app, "GET", "/api/snapshots", Some(&cookie), None).await;
    let snapshot = &snapshots.body["items"][0];
    assert_eq!(snapshot["engine"], "sqlite");

    let restore = format!(
        "/api/snapshots/{}/restore",
        snapshot["id"].as_str().unwrap()
    );
    let other = json!({ "server_id": pg, "database_name": "shop" });
    let reply = call(&app, "POST", &restore, Some(&cookie), Some(other)).await;
    assert_eq!(reply.body["error"], "different_engine");
    let same = json!({ "server_id": lite, "database_name": "copy.db" });
    let reply = call(&app, "POST", &restore, Some(&cookie), Some(same)).await;
    assert_eq!(reply.status, StatusCode::OK);
    let claimed = as_agent(&app, &agent, "/api/agent/jobs/claim", json!({})).await;
    assert_eq!(claimed.body["job"]["task"]["source"], "app.db");
}

// ---------------------------------------------------------------- SSH tunnels and storage kinds

/// A MySQL server with a full entry keeping 7 days; returns (server, destination, full).
async fn chain_setup(app: &Router, cookie: &str, agent: &Agent) -> (String, String, String) {
    let (_, destination, _) = add_resources(app, cookie, agent).await;
    let server = uuid::Uuid::new_v4().to_string();
    let body = json!({ "id": server, "name": "shop-db", "engine": "mysql", "agent_id": agent.id, "host": "db",
        "port": 3306, "username": "u", "secret": sealed_for(agent, &server, "database", "{}") });
    let reply = call(
        app,
        "POST",
        "/api/database-servers",
        Some(cookie),
        Some(body),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    let body = json!({ "name": "full", "server_id": server, "database_name": "shop", "destination_id": destination,
        "compression": "zstd", "level": 3, "retention_unit": "days", "retention_value": 7 });
    let reply = call(app, "POST", "/api/backups", Some(cookie), Some(body)).await;
    assert_eq!(reply.status, StatusCode::CREATED);
    (server, destination, entry_id(app, cookie, "full").await)
}

async fn entry_id(app: &Router, cookie: &str, name: &str) -> String {
    let list = call(app, "GET", "/api/backups", Some(cookie), None).await;
    let entries = list.body.as_array().unwrap();
    let entry = entries.iter().find(|b| b["name"] == name).unwrap();
    entry["id"].as_str().unwrap().into()
}

/// Runs an entry and lets the agent finish it; returns the task it was given.
async fn run_and_finish(
    app: &Router,
    cookie: &str,
    agent: &Agent,
    entry: &str,
    extra: Value,
) -> Value {
    let reply = call(
        app,
        "POST",
        &format!("/api/backups/{entry}/run"),
        Some(cookie),
        None,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    finish_next(app, agent, extra).await["task"].clone()
}

#[sqlx::test]
async fn increments_continue_a_chain_that_restores_and_expires_as_a_whole(db: PgPool) {
    let state = state_for(db.clone(), false, Keypair::generate());
    let app = app(state.clone());
    let cookie = admin(&app).await;
    let agent = enroll(&app, &cookie, "a1").await;
    let (server, destination, full) = chain_setup(&app, &cookie, &agent).await;

    let follow =
        |name: &str, base: &str| json!({ "name": name, "base_id": base, "schedule": "0 * * * *" });
    let create = |body: Value| call(&app, "POST", "/api/backups", Some(&cookie), Some(body));
    // Only MySQL and MariaDB entries that are full can be followed.
    let nightly = entry_id(&app, &cookie, "nightly").await;
    assert_eq!(create(follow("x", &nightly)).await.body["error"], "base");
    assert_eq!(
        create(follow("inc", &full)).await.status,
        StatusCode::CREATED
    );
    let inc = entry_id(&app, &cookie, "inc").await;
    assert_eq!(create(follow("y", &inc)).await.body["error"], "base");
    let list = call(&app, "GET", "/api/backups", Some(&cookie), None).await;
    let entry = list
        .body
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "inc")
        .unwrap()
        .clone();
    assert_eq!(
        (
            entry["base"].as_str(),
            entry["database_name"].as_str(),
            entry["retention_value"].as_i64()
        ),
        (Some("full"), Some("shop"), Some(7))
    );
    // Rotation by generations cannot expire chains, and a followed entry cannot go.
    let gfs = json!({ "name": "full", "server_id": server, "database_name": "shop", "destination_id": destination,
        "compression": "zstd", "level": 3, "retention_unit": "gfs", "keep_daily": 3 });
    let path = format!("/api/backups/{full}");
    let reply = call(&app, "PUT", &path, Some(&cookie), Some(gfs)).await;
    assert_eq!(reply.body["error"], "retention_chain");
    let reply = call(&app, "DELETE", &path, Some(&cookie), None).await;
    assert_eq!(reply.body["error"], "in_use");

    // Without a chain the increment takes the full backup, recording where it ends.
    let task = run_and_finish(
        &app,
        &cookie,
        &agent,
        &inc,
        json!({ "manifest": "m0", "position_to": "s|bin.1|100" }),
    )
    .await;
    assert_eq!(
        (task["track"].as_bool(), task["since"].as_str()),
        (Some(true), None)
    );
    let task = run_and_finish(
        &app,
        &cookie,
        &agent,
        &inc,
        json!({ "manifest": "m1", "position_from": "s|bin.1|100", "position_to": "s|bin.1|200" }),
    )
    .await;
    assert_eq!(
        (task["track"].as_bool(), task["since"].as_str()),
        (Some(false), Some("s|bin.1|100"))
    );
    assert!(
        task["object_key"]
            .as_str()
            .unwrap()
            .ends_with(".binlog.zst.age")
    );

    let snapshots = call(&app, "GET", "/api/snapshots", Some(&cookie), None).await;
    let rows = snapshots.body["items"].as_array().unwrap().clone();
    let (increment, first) = (&rows[0], &rows[1]);
    assert_eq!(increment["parent_id"], first["id"]);
    assert!(increment["chain_started_at"].is_string() && first["parent_id"].is_null());
    let id = increment["id"].as_str().unwrap();

    // The parent stays while something continues it.
    let reply = call(
        &app,
        "DELETE",
        &format!("/api/snapshots/{}", first["id"].as_str().unwrap()),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(reply.body["error"], "has_increments");

    let restore = format!("/api/snapshots/{id}/restore");
    let late =
        json!({ "server_id": server, "database_name": "copy", "until": "2999-01-01T00:00:00Z" });
    assert_eq!(
        call(&app, "POST", &restore, Some(&cookie), Some(late))
            .await
            .body["error"],
        "until"
    );
    let body =
        json!({ "server_id": server, "database_name": "copy", "until": increment["created_at"] });
    assert_eq!(
        call(&app, "POST", &restore, Some(&cookie), Some(body))
            .await
            .status,
        StatusCode::OK
    );
    let task = finish_next(&app, &agent, json!({})).await["task"].clone();
    assert_eq!(task["position"], "s|bin.1|100");
    assert_eq!(task["object_key"], first["object_key"]);
    assert_eq!(task["increments"].as_array().unwrap().len(), 1);
    assert_eq!(task["increments"][0]["from"], "s|bin.1|100");
    assert_eq!(task["until"].as_str().unwrap().len(), 19);

    // A log the agent can no longer read starts a new chain by itself.
    run_and_finish(
        &app,
        &cookie,
        &agent,
        &inc,
        json!({ "error": "log gone", "chain_broken": true }),
    )
    .await;
    let task = finish_next(
        &app,
        &agent,
        json!({ "manifest": "m2", "position_to": "s|bin.9|4" }),
    )
    .await["task"]
        .clone();
    assert_eq!(task["track"], true);

    // The old chain is expired but one link: nothing goes. Once all are, newest first.
    sqlx::query(
        "UPDATE snapshots SET expires_at = now() - interval '1 day' WHERE manifest IN ('m0', 'm2')",
    )
    .execute(&db)
    .await
    .unwrap();
    state.schedule_once().await;
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM jobs WHERE kind = 'delete_snapshot'"
        )
        .await,
        0
    );
    sqlx::query("UPDATE snapshots SET expires_at = now() - interval '1 day'")
        .execute(&db)
        .await
        .unwrap();
    state.schedule_once().await;
    finish_next(&app, &agent, json!({})).await;
    assert_eq!(
        count(&db, "SELECT count(*) FROM snapshots WHERE manifest = 'm1'").await,
        0
    );
    state.schedule_once().await;
    finish_next(&app, &agent, json!({})).await;
    // The newest chain stays, however old.
    assert_eq!(count(&db, "SELECT count(*) FROM snapshots").await, 1);
}

#[sqlx::test]
async fn only_a_whole_mongodb_instance_has_incrementals(db: PgPool) {
    let app = router(db.clone());
    let cookie = admin(&app).await;
    let agent = enroll(&app, &cookie, "a1").await;
    let (_, destination, _) = add_resources(&app, &cookie, &agent).await;
    let server = uuid::Uuid::new_v4().to_string();
    let body = json!({ "id": server, "name": "mongo", "engine": "mongodb", "agent_id": agent.id, "host": "db",
        "port": 27017, "username": "u", "secret": sealed_for(&agent, &server, "database", "{}") });
    let reply = call(
        &app,
        "POST",
        "/api/database-servers",
        Some(&cookie),
        Some(body),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    for (name, database) in [("one", "shop"), ("whole", "_all")] {
        let body = json!({ "name": name, "server_id": server, "database_name": database,
            "destination_id": destination, "compression": "zstd", "level": 3 });
        let reply = call(&app, "POST", "/api/backups", Some(&cookie), Some(body)).await;
        assert_eq!(reply.status, StatusCode::CREATED);
    }
    let follow = |name: &str, base: &str| json!({ "name": name, "base_id": base });
    let one = entry_id(&app, &cookie, "one").await;
    let reply = call(
        &app,
        "POST",
        "/api/backups",
        Some(&cookie),
        Some(follow("a", &one)),
    )
    .await;
    assert_eq!(reply.body["error"], "base");
    let whole = entry_id(&app, &cookie, "whole").await;
    let reply = call(
        &app,
        "POST",
        "/api/backups",
        Some(&cookie),
        Some(follow("inc", &whole)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED);

    let inc = entry_id(&app, &cookie, "inc").await;
    let task = run_and_finish(
        &app,
        &cookie,
        &agent,
        &inc,
        json!({ "manifest": "m", "position_to": "100:1" }),
    )
    .await;
    assert_eq!(task["track"], true);
    let task = run_and_finish(
        &app,
        &cookie,
        &agent,
        &inc,
        json!({ "manifest": "n", "position_from": "100:1", "position_to": "200:1" }),
    )
    .await;
    assert!(
        task["object_key"]
            .as_str()
            .unwrap()
            .ends_with(".oplog.zst.age")
    );

    let snapshots = call(&app, "GET", "/api/snapshots", Some(&cookie), None).await;
    let id = snapshots.body["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let restore = format!("/api/snapshots/{id}/restore");
    let renamed = json!({ "server_id": server, "database_name": "copy" });
    let reply = call(&app, "POST", &restore, Some(&cookie), Some(renamed)).await;
    assert_eq!(reply.body["error"], "instance");
    let whole = json!({ "server_id": server, "database_name": "_all" });
    let reply = call(&app, "POST", &restore, Some(&cookie), Some(whole)).await;
    assert_eq!(reply.status, StatusCode::OK);
}

#[sqlx::test]
async fn a_whole_postgresql_cluster_is_followed_and_restored_to_a_folder(db: PgPool) {
    let app = router(db.clone());
    let cookie = admin(&app).await;
    let agent = enroll(&app, &cookie, "a1").await;
    // `nightly` is a single database of this server: nothing can follow it.
    let (server, destination, one) = add_resources(&app, &cookie, &agent).await;
    let follow = |name: &str, base: &str| json!({ "name": name, "base_id": base });
    let reply = call(
        &app,
        "POST",
        "/api/backups",
        Some(&cookie),
        Some(follow("a", &one)),
    )
    .await;
    assert_eq!(reply.body["error"], "base");
    let body = json!({ "name": "cluster", "server_id": server, "database_name": "_all",
        "destination_id": destination, "compression": "zstd", "level": 3 });
    let reply = call(&app, "POST", "/api/backups", Some(&cookie), Some(body)).await;
    assert_eq!(reply.status, StatusCode::CREATED);
    let whole = entry_id(&app, &cookie, "cluster").await;
    let reply = call(
        &app,
        "POST",
        "/api/backups",
        Some(&cookie),
        Some(follow("inc", &whole)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED);

    let inc = entry_id(&app, &cookie, "inc").await;
    let task = run_and_finish(
        &app,
        &cookie,
        &agent,
        &inc,
        json!({ "manifest": "m", "position_to": "k1" }),
    )
    .await;
    assert!(
        task["object_key"]
            .as_str()
            .unwrap()
            .ends_with(".tar.zst.age")
    );
    let task = run_and_finish(
        &app,
        &cookie,
        &agent,
        &inc,
        json!({ "manifest": "n", "position_from": "k1", "position_to": "k2" }),
    )
    .await;
    assert_eq!(task["since"], "k1");

    let snapshots = call(&app, "GET", "/api/snapshots", Some(&cookie), None).await;
    let increment = snapshots.body["items"][0].clone();
    let restore = format!(
        "/api/snapshots/{}/restore",
        increment["id"].as_str().unwrap()
    );
    // A cluster has no log to stop in the middle of.
    let timed = json!({ "server_id": server, "database_name": "restore-1", "until": increment["created_at"] });
    assert_eq!(
        call(&app, "POST", &restore, Some(&cookie), Some(timed))
            .await
            .body["error"],
        "until"
    );
    let escaping = json!({ "server_id": server, "database_name": "../x" });
    assert_eq!(
        call(&app, "POST", &restore, Some(&cookie), Some(escaping))
            .await
            .body["error"],
        "database_name"
    );
    let folder = json!({ "server_id": server, "database_name": "restore-1" });
    assert_eq!(
        call(&app, "POST", &restore, Some(&cookie), Some(folder))
            .await
            .status,
        StatusCode::OK
    );
}

#[sqlx::test]
async fn keys_rotate_only_with_the_right_key_and_the_server_stopped(db: PgPool) {
    let old = MasterKey::parse("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap();
    let new = MasterKey::parse("AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=").unwrap();
    let key = load_transport_key(&db, &old).await.unwrap();
    let channel = uuid::Uuid::new_v4().to_string();
    let sealed = old
        .seal("notification_channels", "secret", &channel, b"token")
        .unwrap();
    sqlx::query("INSERT INTO notification_channels (id, name, kind, target, secret, events) VALUES ($1::uuid, 'c', 'telegram', 't', $2, '{}')")
        .bind(&channel)
        .bind(sealed)
        .execute(&db)
        .await
        .unwrap();

    {
        let _running = hold_running_lock(&db).await.unwrap();
        let refused = rotate_master_key(&db, &old, &new).await.unwrap_err();
        assert!(refused.contains("stop the server"), "{refused}");
        sqlx::query("SELECT pg_advisory_unlock_all()")
            .execute(&mut *{ _running })
            .await
            .unwrap();
    }
    assert!(
        rotate_master_key(&db, &new, &old).await.is_err(),
        "wrong old key"
    );
    assert!(
        load_transport_key(&db, &old).await.is_ok(),
        "nothing changed"
    );

    assert_eq!(rotate_master_key(&db, &old, &new).await.unwrap(), 2);
    assert!(load_transport_key(&db, &old).await.is_err());
    assert_eq!(
        load_transport_key(&db, &new).await.unwrap().public,
        key.public
    );
    let stored: Vec<u8> = sqlx::query_scalar("SELECT secret FROM notification_channels")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(
        new.open("notification_channels", "secret", &channel, &stored)
            .unwrap(),
        b"token"
    );

    assert!(rotate_transport_key(&db, &old).await.is_err());
    let fingerprint = rotate_transport_key(&db, &new).await.unwrap();
    let replaced = load_transport_key(&db, &new).await.unwrap();
    assert_ne!(replaced.public, key.public);
    assert_eq!(e2ee::fingerprint(&replaced.public), fingerprint);
    assert_eq!(
        count(&db, "SELECT count(*) FROM audit_logs WHERE ip = 'console'").await,
        2
    );
}

#[sqlx::test]
async fn a_revoked_agent_is_shut_out_until_linked_again_with_its_own_key(db: PgPool) {
    let app = router(db.clone());
    let cookie = admin(&app).await;
    let agent = enroll(&app, &cookie, "a1").await;
    let (_, _, backup) = add_resources(&app, &cookie, &agent).await;
    call(
        &app,
        "POST",
        &format!("/api/backups/{backup}/run"),
        Some(&cookie),
        None,
    )
    .await;

    let path = format!("/api/agents/{}/revoke", agent.id);
    let reply = call(&app, "POST", &path, Some(&cookie), None).await;
    assert_eq!(reply.status, StatusCode::OK);
    let token = reply.body["token"].as_str().unwrap().to_owned();
    let refused = as_agent(&app, &agent, "/api/agent/jobs/claim", json!({})).await;
    assert_eq!(refused.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        count(&db, "SELECT count(*) FROM jobs WHERE status = 'failed'").await,
        1
    );

    let link = |key: &[u8]| {
        let body = json!({ "token": token, "version": "2", "public_key": e2ee::encode(key) });
        call(&app, "POST", "/api/agent/enroll", None, Some(body))
    };
    // Someone else's key is refused and does not spend the token.
    assert_eq!(
        link(&Keypair::generate().public).await.body["error"],
        "public_key"
    );
    let linked = link(&agent.keys.public).await;
    assert_eq!(linked.status, StatusCode::OK);
    assert_eq!(linked.body["agent_id"], agent.id.as_str());
    let again = Agent {
        id: agent.id.clone(),
        bearer: format!("Bearer {}", linked.body["secret"].as_str().unwrap()),
        keys: Keypair::generate(),
    };
    let claimed = as_agent(&app, &again, "/api/agent/jobs/claim", json!({})).await;
    assert_eq!(claimed.status, StatusCode::OK);
    assert_eq!(
        link(&agent.keys.public).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test]
async fn each_person_has_their_own_place_in_the_notifications(db: PgPool) {
    let app = router(db.clone());
    let admin = admin(&app).await;
    let body =
        json!({ "name": "V", "email": "v@example.com", "role": "viewer", "password": PASSWORD });
    let reply = call(&app, "POST", "/api/users", Some(&admin), Some(body)).await;
    assert_eq!(reply.status, StatusCode::CREATED);
    let viewer = login(&app, "v@example.com", PASSWORD).await.cookie.unwrap();
    sqlx::query("UPDATE users SET notices_read_at = '2026-01-01T00:00:00Z'")
        .execute(&db)
        .await
        .unwrap();

    let path = "/api/notifications/read";
    let read = |cookie: &str| {
        let (app, cookie) = (&app, cookie.to_owned());
        async move { call(app, "GET", path, Some(&cookie), None).await.body["read_at"].clone() }
    };
    let mark = |cookie: &str, at: &str| {
        let (app, cookie, body) = (&app, cookie.to_owned(), json!({ "read_at": at }));
        async move { call(app, "POST", path, Some(&cookie), Some(body)).await }
    };

    assert_eq!(
        mark(&viewer, "2026-03-01T10:00:00Z").await.status,
        StatusCode::OK
    );
    assert_eq!(read(&viewer).await, "2026-03-01T10:00:00Z");
    assert_eq!(
        read(&admin).await,
        "2026-01-01T00:00:00Z",
        "the other account is untouched"
    );
    // It does not move back, and a far future value stops at the present.
    mark(&viewer, "2026-02-01T00:00:00Z").await;
    assert_eq!(read(&viewer).await, "2026-03-01T10:00:00Z");
    mark(&viewer, "2999-01-01T00:00:00Z").await;
    assert!(!read(&viewer).await.as_str().unwrap().starts_with("2999"));

    let reply = call(&app, "GET", path, None, None).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn redis_and_search_engines_name_what_they_back_up(db: PgPool) {
    let app = router(db.clone());
    let cookie = admin(&app).await;
    let agent = enroll(&app, &cookie, "a1").await;
    let (_, destination, _) = add_resources(&app, &cookie, &agent).await;
    for (engine, good, bad) in [
        ("redis", "all", "0"),
        ("elasticsearch", "_all", ".security"),
        ("opensearch", "logs-2026.11", "a/_search"),
    ] {
        let id = uuid::Uuid::new_v4().to_string();
        let body = json!({ "id": id, "name": engine, "engine": engine, "agent_id": agent.id, "host": "db", "port": 1,
            "username": "", "ssl_mode": "require", "secret": sealed_for(&agent, &id, "database", "{}") });
        let reply = call(
            &app,
            "POST",
            "/api/database-servers",
            Some(&cookie),
            Some(body),
        )
        .await;
        assert_eq!(reply.status, StatusCode::CREATED, "{engine}");
        let entry = |database: &str| {
            json!({ "name": format!("{engine}-b"), "server_id": id, "database_name": database,
                "destination_id": destination, "compression": "gzip", "level": 3 })
        };
        let reply = call(
            &app,
            "POST",
            "/api/backups",
            Some(&cookie),
            Some(entry(bad)),
        )
        .await;
        assert_eq!(reply.body["error"], "database_name", "{engine} {bad}");
        let reply = call(
            &app,
            "POST",
            "/api/backups",
            Some(&cookie),
            Some(entry(good)),
        )
        .await;
        assert_eq!(reply.status, StatusCode::CREATED, "{engine} {good}");
    }
}

#[sqlx::test]
async fn tunnels_belong_to_their_agent_and_go_with_the_job(db: PgPool) {
    let app = router(db.clone());
    let cookie = admin(&app).await;
    let agent = enroll(&app, &cookie, "a1").await;
    let other = enroll(&app, &cookie, "a2").await;
    let tunnel = uuid::Uuid::new_v4().to_string();
    let body = json!({ "id": tunnel, "name": "bastion", "agent_id": agent.id, "host": "bastion", "port": 22,
        "username": "backup", "auth": "password",
        "secret": sealed_for(&agent, &tunnel, "ssh", r#"{"password":"very-secret"}"#) });
    let mut bad = body.clone();
    bad["host"] = json!("-oProxyCommand=x");
    let reply = call(&app, "POST", "/api/ssh-configs", Some(&cookie), Some(bad)).await;
    assert_eq!(reply.body["error"], "host");
    let reply = call(&app, "POST", "/api/ssh-configs", Some(&cookie), Some(body)).await;
    assert_eq!(reply.status, StatusCode::CREATED);

    let add = |name: &str, engine: &str, owner: &Agent| {
        let id = uuid::Uuid::new_v4().to_string();
        let body = json!({ "id": id, "name": name, "engine": engine, "agent_id": owner.id, "host": "db", "port": 5432,
            "username": "u", "ssh_config_id": tunnel, "secret": sealed_for(owner, &id, "database", "{}") });
        (id, body)
    };
    for (case, code) in [
        (add("x", "sqlite", &agent), "ssh_sqlite"),
        (add("y", "mysql", &other), "different_agent"),
    ] {
        let reply = call(
            &app,
            "POST",
            "/api/database-servers",
            Some(&cookie),
            Some(case.1),
        )
        .await;
        assert_eq!(reply.body["error"], code);
    }
    let (server, body) = add("through", "postgresql", &agent);
    let reply = call(
        &app,
        "POST",
        "/api/database-servers",
        Some(&cookie),
        Some(body),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    let reply = call(
        &app,
        "DELETE",
        &format!("/api/ssh-configs/{tunnel}"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(reply.body["error"], "in_use");

    // The job carries both secrets; neither is readable by the server's listing.
    call(
        &app,
        "POST",
        &format!("/api/database-servers/{server}/test"),
        Some(&cookie),
        None,
    )
    .await;
    let task = finish_next(&app, &agent, json!({ "sha256": null })).await;
    assert_eq!(task["task"]["ssh"]["id"], tunnel);
    call(
        &app,
        "POST",
        &format!("/api/ssh-configs/{tunnel}/test"),
        Some(&cookie),
        None,
    )
    .await;
    let task = finish_next(&app, &agent, json!({ "sha256": null })).await;
    assert_eq!(task["task"]["kind"], "test_ssh");
    let list = call(&app, "GET", "/api/ssh-configs", Some(&cookie), None).await;
    assert_eq!(list.body[0]["used"], 1);
    assert!(!list.body.to_string().contains("secret"));

    // The tunnel is sealed with the password, so taking it away needs the password again.
    let path = format!("/api/database-servers/{server}");
    let bare = json!({ "name": "through" });
    let reply = call(&app, "PUT", &path, Some(&cookie), Some(bare)).await;
    assert_eq!(reply.body["error"], "tunnel_password");
    let connection = json!({ "host": "db", "port": 5432, "username": "u", "ssl_mode": "disable",
        "secret": sealed_for(&agent, &server, "database", "{}") });
    let body = json!({ "name": "through", "connection": connection });
    let reply = call(&app, "PUT", &path, Some(&cookie), Some(body)).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    let list = call(&app, "GET", "/api/database-servers", Some(&cookie), None).await;
    assert!(list.body[0]["ssh_config_id"].is_null());
}

#[sqlx::test]
async fn only_s3_snapshots_can_be_downloaded(db: PgPool) {
    let app = router(db.clone());
    let cookie = admin(&app).await;
    let agent = enroll(&app, &cookie, "a1").await;
    let (server, _, _) = add_resources(&app, &cookie, &agent).await;
    let nas = uuid::Uuid::new_v4().to_string();
    let mut body = json!({ "id": nas, "name": "nas", "kind": "nfs", "agent_id": agent.id, "endpoint": "smb://nas:445",
        "bucket": "share", "prefix": "", "username": "u", "secret": sealed_for(&agent, &nas, "destination", "{}") });
    let reply = call(
        &app,
        "POST",
        "/api/destinations",
        Some(&cookie),
        Some(body.clone()),
    )
    .await;
    assert_eq!(reply.body["error"], "kind");
    body["kind"] = json!("smb");
    let reply = call(&app, "POST", "/api/destinations", Some(&cookie), Some(body)).await;
    assert_eq!(reply.status, StatusCode::CREATED);
    let entry = json!({ "name": "to-nas", "server_id": server, "database_name": "shop", "destination_id": nas,
        "compression": "zstd", "level": 3 });
    call(&app, "POST", "/api/backups", Some(&cookie), Some(entry)).await;
    let list = call(&app, "GET", "/api/backups", Some(&cookie), None).await;
    let id = list
        .body
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "to-nas")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    call(
        &app,
        "POST",
        &format!("/api/backups/{id}/run"),
        Some(&cookie),
        None,
    )
    .await;
    finish_next(&app, &agent, json!({})).await;
    let snapshots = call(&app, "GET", "/api/snapshots", Some(&cookie), None).await;
    let snapshot = &snapshots.body["items"][0];
    assert_eq!(snapshot["storage"], "smb");
    let path = format!(
        "/api/snapshots/{}/download",
        snapshot["id"].as_str().unwrap()
    );
    let reply = call(&app, "POST", &path, Some(&cookie), None).await;
    assert_eq!(reply.body["error"], "download_unsupported");
}
