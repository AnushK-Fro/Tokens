mod assets;
mod docs;
mod events;
mod health;
mod ingest;
mod summary;

use axum::extract::DefaultBodyLimit;
use axum::http::{header, HeaderValue};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{middleware, Router};
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

/// ccusage reports are a few hundred KB at most; this leaves plenty of room.
const MAX_BODY_BYTES: usize = 10_000_000;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api", get(docs::show))
        .route("/api/", get(docs::show))
        .route("/api/summary", get(summary::show))
        .route("/api/ingest", post(ingest::accept))
        .route("/api/events", post(events::accept))
        .route("/api/health", get(health::check))
        .fallback(get(assets::serve))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(middleware::map_response(security_headers))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn security_headers(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'self'; img-src 'self' data:; style-src 'self'; script-src 'self'; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'"),
    );
    response
}
