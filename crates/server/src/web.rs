use axum::{
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

/// The built SPA (`pnpm --dir web build`). Empty when it has not been built.
#[derive(RustEmbed)]
#[folder = "../../web/build"]
#[allow_missing = true]
struct Assets;

/// The build also writes `.br` and `.gz` copies. These files are public, so compressing
/// them gives nothing away.
const ENCODINGS: [(&str, &str); 2] = [("br", "br"), ("gzip", "gz")];

fn accepts(headers: &HeaderMap, encoding: &str) -> bool {
    headers
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|item| {
            let mut parts = item.split(';').map(str::trim);
            // `q=0` means "not this one".
            parts
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case(encoding))
                && !parts.any(|p| {
                    p.strip_prefix("q=")
                        .is_some_and(|q| q.parse() == Ok(0.0_f32))
                })
        })
}

/// Route segments are words and ids, never `name.ext`.
fn is_file(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|name| name.contains('.'))
}

pub async fn spa(uri: Uri, headers: HeaderMap) -> Response {
    let path = uri.path().trim_start_matches('/');
    // Unknown paths are client-side routes. A missing file (`/llms.txt`) is not one.
    let (name, file) = match Assets::get(path) {
        Some(file) => (path, file),
        None if is_file(path) => return StatusCode::NOT_FOUND.into_response(),
        None => match Assets::get("200.html") {
            Some(file) => ("200.html", file),
            None => return (StatusCode::NOT_FOUND, "web UI not built").into_response(),
        },
    };
    let cache = if name.starts_with("_app/immutable/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let mime = mime_guess::from_path(name).first_or_octet_stream();
    let compressed = ENCODINGS.iter().find_map(|(encoding, extension)| {
        accepts(&headers, encoding)
            .then(|| Assets::get(&format!("{name}.{extension}")))
            .flatten()
            .map(|file| (*encoding, file))
    });
    let (encoding, file) = match compressed {
        Some((encoding, file)) => (Some(encoding), file),
        None => (None, file),
    };
    let mut response = (
        [
            (header::CONTENT_TYPE, mime.as_ref()),
            (header::CACHE_CONTROL, cache),
            (header::VARY, "accept-encoding"),
        ],
        file.data,
    )
        .into_response();
    if let Some(encoding) = encoding {
        response
            .headers_mut()
            .insert(header::CONTENT_ENCODING, HeaderValue::from_static(encoding));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_are_not_files() {
        assert!(is_file("llms.txt"));
        assert!(is_file(".well-known/agent.json"));
        assert!(!is_file("snapshots/01a221e1-1e2e-7a4c-a50d-4bc5c374e08f"));
        assert!(!is_file(""));
    }

    #[test]
    fn only_encodings_the_client_takes_are_used() {
        let with = |value: &'static str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::ACCEPT_ENCODING, HeaderValue::from_static(value));
            headers
        };
        assert!(accepts(&with("gzip, deflate, br"), "br"));
        assert!(accepts(&with("br;q=0.5"), "br"));
        assert!(!accepts(&with("br;q=0, gzip"), "br"));
        assert!(!accepts(&with("brotli"), "br"));
        assert!(!accepts(&HeaderMap::new(), "gzip"));
    }
}
