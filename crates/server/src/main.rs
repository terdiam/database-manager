use std::{env, net::SocketAddr};

use dbm_server::{
    AppState, Config, MasterKey, TELEGRAM_API, Turnstile, VERIFY_URL, app, hold_running_lock,
    load_transport_key, parse_recipient, reset_password, rotate_master_key, rotate_transport_key,
};
use sqlx::postgres::PgPoolOptions;

fn turnstile() -> Result<Option<Turnstile>, &'static str> {
    let read = |name| env::var(name).ok().filter(|v| !v.is_empty());
    match (
        read("DBM_TURNSTILE_SITE_KEY"),
        read("DBM_TURNSTILE_SECRET_KEY"),
    ) {
        (None, None) => Ok(None),
        (Some(site), Some(secret)) => {
            let hostnames = read("DBM_TURNSTILE_HOSTNAMES").unwrap_or_default();
            Turnstile::new(site, secret, &hostnames, VERIFY_URL.into())
                .map(Some)
                .ok_or("DBM_TURNSTILE_HOSTNAMES must list the hostnames you open DB Manager on")
        }
        // Half a configuration would silently leave sign-in unprotected.
        _ => Err("set both DBM_TURNSTILE_SITE_KEY and DBM_TURNSTILE_SECRET_KEY, or neither"),
    }
}

fn flag(name: &str, default: bool) -> bool {
    env::var(name).map_or(default, |v| v == "true" || v == "1")
}

#[tokio::main(worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().init();

    let url = env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is required")?;
    let pool = PgPoolOptions::new()
        // One of these is held by the change listener, one marks the database as in use.
        .max_connections(9)
        .connect(&url)
        .await?;
    sqlx::migrate!().run(&pool).await?;

    let args: Vec<String> = env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => {}
        ["reset-password", email] => return reset(&pool, email).await,
        ["rotate-master-key"] => {
            let new = MasterKey::from_variable("DBM_NEW_MASTER_KEY")?;
            let count = rotate_master_key(&pool, &MasterKey::from_env()?, &new).await?;
            println!(
                "{count} values re-encrypted. Put the new key in DBM_MASTER_KEY, then start the server."
            );
            return Ok(());
        }
        ["rotate-transport-key"] => {
            let fingerprint = rotate_transport_key(&pool, &MasterKey::from_env()?).await?;
            println!(
                "New server key {fingerprint}. Restart every agent once with DBM_SERVER_FINGERPRINT={fingerprint}."
            );
            return Ok(());
        }
        _ => {
            return Err(
                "usage: dbm-server [reset-password <email> | rotate-master-key | rotate-transport-key]"
                    .into(),
            );
        }
    }
    let _running = hold_running_lock(&pool).await?;

    let master = MasterKey::from_env()?;
    let transport_key = load_transport_key(&pool, &master).await?;
    let e2ee_required = match env::var("DBM_E2EE").unwrap_or_default().as_str() {
        "" | "on" => true,
        "off" => false,
        _ => return Err("DBM_E2EE must be on or off".into()),
    };

    let config = Config {
        cookie_secure: flag("DBM_COOKIE_SECURE", true),
        trust_proxy: flag("DBM_TRUST_PROXY", false),
        turnstile: turnstile()?,
        e2ee_required,
        telegram_api: TELEGRAM_API.into(),
        extra_recipient: match env::var("DBM_BACKUP_EXTRA_RECIPIENT")
            .ok()
            .filter(|v| !v.is_empty())
        {
            None => None,
            Some(text) => Some(
                parse_recipient(&text)
                    .ok_or("DBM_BACKUP_EXTRA_RECIPIENT must be an age recipient (age1...)")?,
            ),
        },
    };
    let listen = env::var("DBM_LISTEN").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    tracing::info!(
        "listening on {listen}, payload encryption {}",
        if e2ee_required { "on" } else { "off" }
    );

    let state = AppState::new(pool, config, transport_key, master);
    state.watch_changes().await?;
    tokio::spawn(state.clone().run_scheduler());
    let streams = state.clone();
    axum::serve(
        listener,
        app(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown().await;
        streams.close_streams();
    })
    .await?;
    Ok(())
}

/// Asks for the password without echoing it, so it never lands in shell history or logs.
async fn reset(pool: &sqlx::PgPool, email: &str) -> Result<(), Box<dyn std::error::Error>> {
    let password = rpassword::prompt_password("New password (12 characters or more): ")?;
    if rpassword::prompt_password("Repeat it: ")? != password {
        return Err("the two passwords differ; nothing was changed".into());
    }
    reset_password(pool, email, &password).await?;
    println!("Password changed. All sessions of {email} were signed out.");
    Ok(())
}

async fn shutdown() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("signal handler");
    tokio::select! {
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}
