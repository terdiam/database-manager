mod api;
mod state;

use std::{env, error::Error, path::Path, time::Duration};

use api::Api;
use dbm_shared::{
    ENROLL_PATH, EnrollRequest, EnrollResponse, HEARTBEAT_PATH, HEARTBEAT_SECS, HeartbeatRequest,
    e2ee::{self, Keypair},
};
use reqwest::Url;
use state::State;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Plain HTTP would expose the credential, so it is allowed only on loopback unless opted into.
fn server_url(raw: &str, allow_insecure: bool) -> Result<Url> {
    let url = Url::parse(raw)?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    match url.scheme() {
        "https" => Ok(url),
        "http" if loopback || allow_insecure => Ok(url),
        _ => Err("DBM_SERVER_URL must use https (set DBM_ALLOW_INSECURE=true for a trusted private network)".into()),
    }
}

/// The server key to encrypt to, taken from the server the first time and pinned after.
/// `expected` is the fingerprint from the install command, when the operator passed it.
async fn server_key(api: &Api, expected: Option<&str>) -> Result<Option<Vec<u8>>> {
    let info = api.e2ee_info().await?;
    let key = e2ee::decode(&info.public_key)?;
    if let Some(expected) = expected {
        if e2ee::fingerprint(&key) != expected {
            return Err("server key does not match DBM_SERVER_FINGERPRINT".into());
        }
    } else if info.required {
        tracing::warn!(
            "trusting server key {} on first use",
            e2ee::fingerprint(&key)
        );
    }
    Ok(info.required.then_some(key))
}

async fn enroll(api: &Api, expected: Option<&str>, path: &Path) -> Result<State> {
    let token = env::var("DBM_ENROLL_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
        .ok_or("not enrolled yet: set DBM_ENROLL_TOKEN")?;
    state::check_writable(path)?;

    let pinned = server_key(api, expected).await?;
    let keys = Keypair::generate();
    let public_key = e2ee::encode(&keys.public);
    let request = EnrollRequest {
        token,
        version: VERSION.into(),
        public_key: public_key.clone(),
    };
    let reply = api
        .post(ENROLL_PATH, pinned.as_deref(), None, &request)
        .await?;
    if reply.status != 200 {
        let reason = reply.error_code().unwrap_or_default();
        return Err(format!("enrollment refused: HTTP {} {reason}", reply.status).into());
    }
    let response: EnrollResponse = serde_json::from_slice(&reply.body)?;
    let state = State {
        version: state::VERSION,
        secret: response.secret,
        private_key: e2ee::encode(&keys.private),
        public_key,
        server_key: pinned.map(|key| e2ee::encode(&key)),
    };
    state::save(path, &state)?;
    tracing::info!("enrolled as agent {}", response.agent_id);
    Ok(state)
}

async fn run() -> Result<()> {
    let server = env::var("DBM_SERVER_URL").map_err(|_| "DBM_SERVER_URL is required")?;
    let insecure = env::var("DBM_ALLOW_INSECURE").is_ok_and(|v| v == "true" || v == "1");
    let api = Api::new(server_url(&server, insecure)?)?;
    let expected = env::var("DBM_SERVER_FINGERPRINT")
        .ok()
        .filter(|v| !v.is_empty());
    let expected = expected.as_deref();
    let path = env::var("DBM_STATE_FILE").unwrap_or_else(|_| "agent-state".into());
    let path = Path::new(&path);

    let mut state = match state::load(path) {
        Some(state) => state,
        None => enroll(&api, expected, path).await?,
    };
    tracing::info!(
        "agent key fingerprint {}",
        e2ee::fingerprint(&e2ee::decode(&state.public_key)?)
    );

    let mut tick = tokio::time::interval(Duration::from_secs(HEARTBEAT_SECS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let pinned = state.server_key.as_deref().map(e2ee::decode).transpose()?;
        let beat = HeartbeatRequest {
            version: VERSION.into(),
        };
        let sent = api
            .post(
                HEARTBEAT_PATH,
                pinned.as_deref(),
                Some(&state.secret),
                &beat,
            )
            .await;
        match sent {
            Ok(r) if r.status == 401 => {
                return Err("credential rejected: this agent was removed from the server".into());
            }
            Ok(r) if (200..300).contains(&r.status) => tracing::debug!("heartbeat ok"),
            // Encryption was switched on after this agent enrolled: pin the key now.
            Ok(r) if pinned.is_none() && r.error_code().as_deref() == Some("e2ee_required") => {
                state.server_key = server_key(&api, expected).await?.map(|k| e2ee::encode(&k));
                state::save(path, &state)?;
                tracing::info!("server now requires encryption; key pinned");
                tick.reset_immediately();
            }
            Ok(r) => tracing::warn!(
                "heartbeat failed: HTTP {} {}",
                r.status,
                r.error_code().unwrap_or_default()
            ),
            // A network blip must not kill the agent; the next tick retries.
            Err(e) => tracing::warn!("heartbeat failed: {e}"),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::process::ExitCode {
    use tokio::signal::unix::{SignalKind, signal};

    tracing_subscriber::fmt().init();
    let mut term = signal(SignalKind::terminate()).expect("signal handler");
    let result = tokio::select! {
        result = run() => result,
        _ = term.recv() => Ok(()),
        _ = tokio::signal::ctrl_c() => Ok(()),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::server_url;

    #[test]
    fn plain_http_is_limited() {
        assert!(server_url("https://dbm.example.com", false).is_ok());
        assert!(server_url("http://localhost:8080", false).is_ok());
        assert!(server_url("http://[::1]:8080", false).is_ok());
        assert!(server_url("http://dbm.example.com", false).is_err());
        assert!(server_url("http://localhost.evil.example", false).is_err());
        assert!(server_url("http://server:8080", true).is_ok());
        assert!(server_url("ftp://server", true).is_err());
    }
}
