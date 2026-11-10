mod assets;
mod docs;
mod health;
mod ingest;
mod summary;

use axum::extract::{DefaultBodyLimit, Query, Request, State};
use axum::http::{header, HeaderValue, Method, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{middleware, Router};
use serde::Deserialize;
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
        .route("/api/health", get(health::check))
        // The PHP version answered on `?api=…` (and took a bare POST as an ingest);
        // old sync scripts keep working unchanged.
        .route("/", any(legacy_or_index))
        .route("/index.php", any(legacy_or_index))
        .route("/tokens.php", any(legacy_or_index))
        .fallback(get(assets::serve))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(middleware::map_response(security_headers))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[derive(Deserialize)]
struct LegacyQuery {
    api: Option<String>,
}

async fn legacy_or_index(State(state): State<AppState>, Query(query): Query<LegacyQuery>, request: Request) -> Response {
    let method = request.method().clone();

    match (query.api.as_deref(), method) {
        (Some("ingest") | None, Method::POST) => {
            let (parts, body) = request.into_parts();
            match axum::body::to_bytes(body, MAX_BODY_BYTES).await {
                Ok(bytes) => ingest::accept(State(state), parts.headers, bytes).await.into_response(),
                Err(_) => (axum::http::StatusCode::PAYLOAD_TOO_LARGE, "Payload is too large").into_response(),
            }
        }
        (Some("summary"), Method::GET) => summary::show(State(state)).await.into_response(),
        (Some("health"), Method::GET) => health::check(State(state)).await.into_response(),
        (Some(_), _) => axum::http::StatusCode::METHOD_NOT_ALLOWED.into_response(),
        (None, Method::GET | Method::HEAD) => assets::serve(Uri::from_static("/"), request.headers().clone()).await.into_response(),
        (None, _) => axum::http::StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
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
