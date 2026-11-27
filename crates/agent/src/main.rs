mod api;
mod engines;
mod manifest;
mod pipeline;
mod progress;
mod runner;
mod state;
mod storage;

use std::{
    env,
    error::Error,
    path::Path,
    time::{Duration, Instant},
};

use api::Api;
use dbm_shared::{
    ENROLL_PATH, EnrollRequest, EnrollResponse, HEARTBEAT_PATH, HEARTBEAT_SECS, HeartbeatRequest,
    e2ee::{self, Keypair},
    jobs::{CLAIM_PATH, Claim, FINISH_PATH, Finish, Job, REPORT_PATH, Report, ReportReply},
};
use progress::Progress;
use reqwest::Url;
use state::State;

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// How often an idle agent asks for work and a busy one reports.
const POLL: Duration = Duration::from_secs(3);

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
        agent_id: response.agent_id.to_string(),
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

    serve(&api, expected, path, &mut state).await
}

/// Heartbeat, then ask for work; one job at a time.
async fn serve(api: &Api, expected: Option<&str>, path: &Path, state: &mut State) -> Result<()> {
    let mut tick = tokio::time::interval(POLL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut beaten: Option<Instant> = None;
    loop {
        tick.tick().await;
        let due = beaten.is_none_or(|at| at.elapsed() >= Duration::from_secs(HEARTBEAT_SECS));
        let outcome = async {
            if due {
                let beat = HeartbeatRequest {
                    version: VERSION.into(),
                };
                call(api, state, HEARTBEAT_PATH, &beat).await?;
            }
            let reply = call(api, state, CLAIM_PATH, &serde_json::json!({})).await?;
            Ok::<Claim, Failure>(serde_json::from_slice(&reply.body)?)
        }
        .await;
        match outcome {
            Ok(claim) => {
                if due {
                    beaten = Some(Instant::now());
                }
                if let Some(job) = claim.job {
                    execute(api, state, job).await;
                    tick.reset_immediately();
                }
            }
            Err(Failure::Revoked) => {
                return Err("credential rejected: this agent was removed from the server".into());
            }
            // Encryption was switched on after this agent enrolled: pin the key now.
            Err(Failure::Encryption) if state.server_key.is_none() => {
                state.server_key = server_key(api, expected).await?.map(|k| e2ee::encode(&k));
                state::save(path, state)?;
                tracing::info!("server now requires encryption; key pinned");
            }
            // A network blip must not kill the agent; the next tick retries.
            Err(error) => tracing::warn!("server call failed: {error}"),
        }
    }
}

enum Failure {
    Revoked,
    Encryption,
    Other(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Revoked => f.write_str("credential rejected"),
            Failure::Encryption => f.write_str("the server requires encryption"),
            Failure::Other(message) => f.write_str(message),
        }
    }
}

impl From<Box<dyn Error>> for Failure {
    fn from(error: Box<dyn Error>) -> Self {
        Failure::Other(error.to_string())
    }
}

impl From<serde_json::Error> for Failure {
    fn from(error: serde_json::Error) -> Self {
        Failure::Other(error.to_string())
    }
}

impl From<e2ee::CryptoError> for Failure {
    fn from(error: e2ee::CryptoError) -> Self {
        Failure::Other(error.to_string())
    }
}

async fn call(
    api: &Api,
    state: &State,
    path: &str,
    body: &impl serde::Serialize,
) -> std::result::Result<api::Reply, Failure> {
    let pinned = state.server_key.as_deref().map(e2ee::decode).transpose()?;
    let reply = api
        .post(path, pinned.as_deref(), Some(&state.secret), body)
        .await?;
    match reply.status {
        200..300 => Ok(reply),
        401 => Err(Failure::Revoked),
        status => match reply.error_code() {
            Some(code) if code == "e2ee_required" => Err(Failure::Encryption),
            code => Err(Failure::Other(format!(
                "HTTP {status} {}",
                code.unwrap_or_default()
            ))),
        },
    }
}

/// Runs a job while reporting progress; the report's answer can cancel it.
async fn execute(api: &Api, state: &State, job: Job) {
    let (id, lease) = (job.id, job.lease);
    tracing::info!("job {id} started");
    let progress = Progress::default();
    let work = runner::run(state, job, progress.clone());
    tokio::pin!(work);
    let mut tick = tokio::time::interval(POLL);
    let result = loop {
        tokio::select! {
            result = &mut work => break result,
            _ = tick.tick() => {
                let report = Report { id, lease, bytes: progress.bytes(), logs: progress.take_logs() };
                match call(api, state, REPORT_PATH, &report).await {
                    Ok(reply) => {
                        if serde_json::from_slice::<ReportReply>(&reply.body).is_ok_and(|r| r.cancel) {
                            progress.cancel();
                        }
                    }
                    // The server gave this job up: stop working on it.
                    Err(Failure::Other(message)) if message.contains("lease_lost") => progress.cancel(),
                    Err(error) => tracing::warn!("report failed: {error}"),
                }
            }
        }
    };
    let (error, output) = match result {
        Ok(output) => (None, output),
        Err(error) => {
            progress.log("error", error.to_string());
            (Some(error.to_string()), runner::Output::default())
        }
    };
    tracing::info!(
        "job {id} {}",
        if error.is_none() {
            "completed"
        } else {
            "failed"
        }
    );
    let finish = Finish {
        id,
        lease,
        bytes: progress.bytes(),
        logs: progress.take_logs(),
        error,
        sha256: output.sha256,
        manifest: output.manifest,
        result: output.result,
    };
    // Losing this message would leave the job to expire, so try a few times.
    for attempt in 1..=5 {
        match call(api, state, FINISH_PATH, &finish).await {
            Ok(_) => return,
            Err(error) => tracing::warn!("could not report the result (try {attempt}): {error}"),
        }
        tokio::time::sleep(Duration::from_secs(attempt * 2)).await;
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::process::ExitCode {
    use tokio::signal::unix::{SignalKind, signal};

    tracing_subscriber::fmt().init();
    // OpenDAL does not pick an HTTP client by itself when built without its defaults.
    opendal::install_default();
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
