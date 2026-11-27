use axum::{
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

/// The built SPA (`pnpm --dir web build`). Empty when it has not been built.
#[derive(RustEmbed)]
#[folder = "../../web/build"]
#[allow_missing = true]
struct Assets;

pub async fn spa(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    // Unknown paths are client-side routes.
    let (name, file) = match Assets::get(path) {
        Some(file) => (path, file),
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
    (
        [
            (header::CONTENT_TYPE, mime.as_ref()),
            (header::CACHE_CONTROL, cache),
        ],
        file.data,
    )
        .into_response()
}
