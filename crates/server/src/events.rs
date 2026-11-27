//! Tells open pages that something changed. Only table names travel; the page then asks the
//! normal API, which applies the permissions.

use std::time::Duration;

use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
};
use futures_util::stream;
use sqlx::postgres::PgListener;
use tokio::{
    sync::broadcast::{self, error::RecvError},
    time::{Instant, sleep, timeout_at},
};

use crate::{AppState, Error, auth, e2ee::Streaming};

/// Also how often the session is checked again, so a signed-out stream ends.
const KEEPALIVE: Duration = Duration::from_secs(15);
const CHANNEL: &str = "dbm_events";

#[derive(Clone)]
pub enum Signal {
    Changed(String),
    /// The server is stopping.
    Close,
}

pub type Sender = broadcast::Sender<Signal>;

pub fn channel() -> Sender {
    broadcast::channel(256).0
}

/// Forwards database notifications to every open stream until `Signal::Close`.
pub async fn watch(db: &sqlx::PgPool, events: Sender) -> Result<(), sqlx::Error> {
    let mut listener = PgListener::connect_with(db).await?;
    listener.listen(CHANNEL).await?;
    let mut close = events.subscribe();
    tokio::spawn(async move {
        loop {
            let topic = tokio::select! {
                signal = close.recv() => match signal {
                    Ok(Signal::Close) | Err(RecvError::Closed) => break,
                    _ => continue,
                },
                received = listener.try_recv() => match received {
                    Ok(Some(notification)) => notification.payload().to_owned(),
                    // The connection dropped and notifications may be lost: reload everything.
                    Ok(None) => "*".into(),
                    Err(error) => {
                        tracing::warn!("listening for changes failed: {error}");
                        sleep(Duration::from_secs(5)).await;
                        "*".into()
                    }
                },
            };
            // Fails only when no page is open.
            let _ = events.send(Signal::Changed(topic));
        }
    });
    Ok(())
}

/// One line per message: a table name, `*` for everything, or empty to show the stream is alive.
pub async fn stream(State(app): State<AppState>, headers: HeaderMap) -> Result<Response, Error> {
    let admin = auth::session_user(&app, &headers)
        .await?
        .require_admin()
        .is_ok();
    let receiver = app.events.subscribe();
    // The first line makes the page catch up on what it missed while disconnected.
    let state = (receiver, app, headers, Instant::now() + KEEPALIVE, true);

    let lines = stream::unfold(state, move |state| async move {
        let (mut receiver, app, headers, mut check, first) = state;
        let line = if first {
            "*".to_owned()
        } else {
            loop {
                if Instant::now() >= check {
                    auth::session_user(&app, &headers).await.ok()?;
                    check = Instant::now() + KEEPALIVE;
                    break String::new();
                }
                match timeout_at(check, receiver.recv()).await {
                    Err(_) => continue,
                    Ok(Ok(Signal::Changed(topic))) => {
                        // These pages are for admins.
                        if admin || !matches!(topic.as_str(), "users" | "audit_logs") {
                            break topic;
                        }
                    }
                    Ok(Err(RecvError::Lagged(_))) => break "*".into(),
                    Ok(Ok(Signal::Close)) | Ok(Err(RecvError::Closed)) => return None,
                }
            }
        };
        let chunk = Ok::<_, std::convert::Infallible>(Bytes::from(line + "\n"));
        Some((chunk, (receiver, app, headers, check, false)))
    });

    let mut response = (
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
            // Tells nginx-like proxies to pass each line on at once.
            (header::HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        Body::from_stream(lines),
    )
        .into_response();
    response.extensions_mut().insert(Streaming);
    Ok(response)
}
