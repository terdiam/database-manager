use axum::{
    Router,
    body::Body,
    extract::connect_info::MockConnectInfo,
    http::{Request, StatusCode, header},
};
use dbm_server::{AppState, Config, MasterKey, Turnstile, app, load_transport_key, reset_password};
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
    let sealed = e2ee::seal_request(&server.public, &aad, bearer, body).unwrap();
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
    let sealed = e2ee::seal_request(&key.public, &aad, "", b"").unwrap();
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
