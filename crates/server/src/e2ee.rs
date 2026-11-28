//! Optional end-to-end encryption of API bodies on top of TLS.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use axum::{
    Json,
    body::{Body, Bytes, to_bytes},
    extract::{OriginalUri, Request, State},
    http::{HeaderValue, Method, header, response::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use dbm_shared::{
    E2eeInfo,
    e2ee::{self, Keypair, MAX_SKEW_SECS},
};
use futures_util::{StreamExt, future, stream};
use time::OffsetDateTime;

use axum::extract::FromRequestParts;

use crate::{AppState, Error, auth::Ip};

const MAX_BODY: usize = 1024 * 1024;
const MAX_REMEMBERED: usize = 100_000;
/// Far above what a browser or an agent sends in the replay window.
const MAX_PER_ADDRESS: usize = 20_000;

#[derive(Default)]
struct Seen {
    keys: HashMap<Vec<u8>, (Instant, String)>,
    per_address: HashMap<String, usize>,
    pruned: Option<Instant>,
}

impl Seen {
    fn full(&self, address: &str) -> bool {
        self.keys.len() >= MAX_REMEMBERED
            || self
                .per_address
                .get(address)
                .is_some_and(|&n| n >= MAX_PER_ADDRESS)
    }

    /// `false` when the key was already accepted or there is no room for this address.
    fn add(&mut self, enc: &[u8], address: &str, window: Duration) -> bool {
        // Forgetting looks at everything, so a flood must not trigger it every time.
        if self.full(address)
            && self
                .pruned
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(1))
        {
            self.keys.retain(|_, (at, _)| at.elapsed() <= window);
            self.per_address.clear();
            for (_, from) in self.keys.values() {
                *self.per_address.entry(from.clone()).or_default() += 1;
            }
            self.pruned = Some(Instant::now());
        }
        let replayed = self
            .keys
            .get(enc)
            .is_some_and(|(at, _)| at.elapsed() <= window);
        if replayed || self.full(address) {
            return false;
        }
        self.keys
            .insert(enc.to_vec(), (Instant::now(), address.to_owned()));
        *self.per_address.entry(address.to_owned()).or_default() += 1;
        true
    }
}

pub struct Transport {
    key: Keypair,
    /// Refuse requests that are not encrypted. Encrypted ones are accepted either way.
    required: bool,
    /// Encapsulated keys already used, so a captured request cannot be sent twice.
    seen: Mutex<Seen>,
}

impl Transport {
    pub fn new(key: Keypair, required: bool) -> Self {
        Self {
            key,
            required,
            seen: Mutex::default(),
        }
    }

    /// `false` when this request was already accepted (or there is no room for the sender).
    fn remember(&self, enc: &[u8], address: &str) -> bool {
        // A timestamp may be this far in the past or the future.
        let window = Duration::from_secs(2 * MAX_SKEW_SECS as u64);
        self.seen.lock().unwrap().add(enc, address, window)
    }
}

/// Marks a response without an end: it is sealed chunk by chunk instead of as a whole.
#[derive(Clone, Copy)]
pub struct Streaming;

fn seal_stream(exchange: &e2ee::Exchange, mut head: Parts, body: Body) -> Response {
    let mut seed = [0; 32];
    if getrandom::fill(&mut seed).is_err() {
        return Error::Internal("response encryption".into()).into_response();
    }
    let mut cipher = e2ee::StreamCipher::new(exchange, &seed);
    let chunks = body.into_data_stream().map(move |chunk| {
        let chunk = chunk?;
        cipher
            .seal(&chunk)
            .map(Bytes::from)
            .map_err(axum::Error::new)
    });
    let start = stream::once(future::ready(Ok::<_, axum::Error>(Bytes::copy_from_slice(
        &seed,
    ))));
    head.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(e2ee::STREAM_CONTENT_TYPE),
    );
    Response::from_parts(head, Body::from_stream(start.chain(chunks)))
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
    let key = &app.transport.key.private;
    let Ok(opened) = e2ee::open_request(key, &enc, &aad, &ciphertext, MAX_BODY) else {
        return Error::Invalid("e2ee_invalid").into_response();
    };
    let address = match Ip::from_request_parts(&mut parts, &app).await {
        Ok(Ip(address)) => address,
        Err(error) => return error.into_response(),
    };
    if !app.transport.remember(&enc, &address) {
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

    // Only what is read is compressed: no reply to a GET carries a password, token or key,
    // so its length cannot give one away.
    let gzip = parts.method == Method::GET && parts.headers.contains_key(e2ee::GZIP_HEADER);

    let response = next
        .run(Request::from_parts(parts, Body::from(opened.body)))
        .await;
    let (mut head, body) = response.into_parts();
    if head.extensions.remove::<Streaming>().is_some() {
        return seal_stream(&opened.exchange, head, body);
    }
    let Ok(plain) = to_bytes(body, usize::MAX).await else {
        return Error::Internal("response body".into()).into_response();
    };
    let status = head.status.as_u16();
    let Ok(sealed) = e2ee::seal_response(&opened.exchange, status, &plain, gzip) else {
        return Error::Internal("response encryption".into()).into_response();
    };
    // The real status is inside; cookies stay on the outside for the browser to store.
    head.status = axum::http::StatusCode::OK;
    head.headers.remove(header::CONTENT_LENGTH);
    // Tells the caller it may compress its next requests.
    head.headers
        .insert(e2ee::GZIP_HEADER, HeaderValue::from_static("1"));
    head.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(e2ee::CONTENT_TYPE),
    );
    head.headers
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Response::from_parts(head, Body::from(sealed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replays_are_refused_and_one_address_cannot_use_all_the_room() {
        let window = Duration::from_secs(60);
        let mut seen = Seen::default();
        assert!(seen.add(b"a", "1.1.1.1", window));
        assert!(!seen.add(b"a", "2.2.2.2", window));
        for n in 0..MAX_PER_ADDRESS - 1 {
            assert!(seen.add(&n.to_le_bytes(), "1.1.1.1", window));
        }
        assert!(!seen.add(b"one more", "1.1.1.1", window));
        assert!(seen.add(b"one more", "2.2.2.2", window));
    }
}
