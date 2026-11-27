//! Optional end-to-end encryption of API bodies on top of TLS.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use axum::{
    Json,
    body::{Body, to_bytes},
    extract::{OriginalUri, Request, State},
    http::{HeaderValue, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use dbm_shared::{
    E2eeInfo,
    e2ee::{self, Keypair, MAX_SKEW_SECS},
};
use time::OffsetDateTime;

use crate::{AppState, Error};

const MAX_BODY: usize = 1024 * 1024;
const MAX_REMEMBERED: usize = 100_000;

pub struct Transport {
    key: Keypair,
    /// Refuse requests that are not encrypted. Encrypted ones are accepted either way.
    required: bool,
    /// Encapsulated keys already used, so a captured request cannot be sent twice.
    seen: Mutex<HashMap<Vec<u8>, Instant>>,
}

impl Transport {
    pub fn new(key: Keypair, required: bool) -> Self {
        Self {
            key,
            required,
            seen: Mutex::default(),
        }
    }

    /// `false` when this request was already accepted (or the memory is full).
    fn remember(&self, enc: &[u8]) -> bool {
        // A timestamp may be this far in the past or the future.
        let window = Duration::from_secs(2 * MAX_SKEW_SECS as u64);
        let mut seen = self.seen.lock().unwrap();
        if seen.len() >= MAX_REMEMBERED {
            seen.retain(|_, at| at.elapsed() <= window);
        }
        if seen.len() >= MAX_REMEMBERED || seen.get(enc).is_some_and(|at| at.elapsed() <= window) {
            return false;
        }
        seen.insert(enc.to_vec(), Instant::now());
        true
    }
}

pub async fn info(State(app): State<AppState>) -> Json<E2eeInfo> {
    let public = &app.transport.key.public;
    Json(E2eeInfo {
        required: app.transport.required,
        public_key: e2ee::encode(public),
        fingerprint: e2ee::fingerprint(public),
    })
}

/// Header: `1.<unix time>.<enc>`, plus `.<ciphertext>` when the request has no body.
fn parse(header: &str) -> Option<(i64, Vec<u8>, Option<Vec<u8>>)> {
    let mut parts = header.split('.');
    if parts.next()? != "1" {
        return None;
    }
    let timestamp = parts.next()?.parse().ok()?;
    let enc = e2ee::decode(parts.next()?).ok()?;
    let inline = match parts.next() {
        Some(text) => Some(e2ee::decode(text).ok()?),
        None => None,
    };
    parts.next().is_none().then_some((timestamp, enc, inline))
}

pub async fn layer(State(app): State<AppState>, request: Request, next: Next) -> Response {
    let Some(value) = request.headers().get(e2ee::HEADER) else {
        if app.transport.required {
            return Error::Invalid("e2ee_required").into_response();
        }
        return next.run(request).await;
    };
    let Some((timestamp, enc, inline)) = value.to_str().ok().and_then(parse) else {
        return Error::Invalid("e2ee_invalid").into_response();
    };
    if OffsetDateTime::now_utc()
        .unix_timestamp()
        .abs_diff(timestamp)
        > MAX_SKEW_SECS as u64
    {
        return Error::Invalid("e2ee_clock").into_response();
    }

    // Routers are nested, so the path the client signed is the original one.
    let path = request
        .extensions()
        .get::<OriginalUri>()
        .map_or(request.uri(), |original| &original.0)
        .path_and_query()
        .map_or("/", |p| p.as_str())
        .to_owned();
    let aad = e2ee::request_aad(request.method().as_str(), &path, timestamp);

    let (mut parts, body) = request.into_parts();
    let Ok(body) = to_bytes(body, MAX_BODY).await else {
        return Error::Invalid("e2ee_invalid").into_response();
    };
    let ciphertext = match inline {
        Some(inline) if body.is_empty() => inline,
        None => body.to_vec(),
        Some(_) => return Error::Invalid("e2ee_invalid").into_response(),
    };
    let Ok(opened) = e2ee::open_request(&app.transport.key.private, &enc, &aad, &ciphertext) else {
        return Error::Invalid("e2ee_invalid").into_response();
    };
    if !app.transport.remember(&enc) {
        return Error::Invalid("e2ee_replay").into_response();
    }

    parts.headers.remove(e2ee::HEADER);
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.remove(header::CONTENT_TYPE);
    parts.headers.remove(header::AUTHORIZATION);
    if !opened.body.is_empty() {
        parts.headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
    }
    if !opened.authorization.is_empty() {
        let Ok(value) = HeaderValue::from_str(&opened.authorization) else {
            return Error::Invalid("e2ee_invalid").into_response();
        };
        parts.headers.insert(header::AUTHORIZATION, value);
    }

    let response = next
        .run(Request::from_parts(parts, Body::from(opened.body)))
        .await;
    let (mut head, body) = response.into_parts();
    let Ok(plain) = to_bytes(body, usize::MAX).await else {
        return Error::Internal("response body".into()).into_response();
    };
    let Ok(sealed) = e2ee::seal_response(&opened.exchange, head.status.as_u16(), &plain) else {
        return Error::Internal("response encryption".into()).into_response();
    };
    // The real status is inside; cookies stay on the outside for the browser to store.
    head.status = axum::http::StatusCode::OK;
    head.headers.remove(header::CONTENT_LENGTH);
    head.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(e2ee::CONTENT_TYPE),
    );
    head.headers
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Response::from_parts(head, Body::from(sealed))
}
