//! The dashboard's static files, compiled into the binary from `web/`.

use axum::http::{header, HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/"]
struct Assets;

pub async fn serve(uri: Uri, headers: HeaderMap) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    let Some(file) = Assets::get(path) else {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    };

    let etag = format!("\"{}\"", file.metadata.sha256_hash().iter().take(12).map(|byte| format!("{byte:02x}")).collect::<String>());
    let cache = if path.starts_with("fonts/") { "public, max-age=604800" } else { "no-cache" };

    if headers.get(header::IF_NONE_MATCH).and_then(|value| value.to_str().ok()) == Some(etag.as_str()) {
        return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag), (header::CACHE_CONTROL, cache.to_owned())]).into_response();
    }

    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let mut response = file.data.into_owned().into_response();
    let response_headers = response.headers_mut();
    response_headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(mime.as_ref()).unwrap_or(HeaderValue::from_static("application/octet-stream")));
    response_headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    if let Ok(value) = HeaderValue::from_str(&etag) {
        response_headers.insert(header::ETAG, value);
    }
    response
}
