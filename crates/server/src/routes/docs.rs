//! `GET /api`: plain Markdown docs, written to be read by agents as well as people.

use axum::http::{header, HeaderMap};
use axum::response::IntoResponse;

const API_DOCS: &str = include_str!("../../docs/API.md");

pub async fn show(headers: HeaderMap) -> impl IntoResponse {
    let host = headers.get(header::HOST).and_then(|value| value.to_str().ok()).filter(|host| host.chars().all(|c| c.is_ascii_alphanumeric() || ".:-[]".contains(c)));
    let scheme = headers.get("x-forwarded-proto").and_then(|value| value.to_str().ok()).filter(|scheme| matches!(*scheme, "http" | "https")).unwrap_or("http");
    let base = host.map_or_else(|| "http://localhost:8787".to_owned(), |host| format!("{scheme}://{host}"));

    ([(header::CONTENT_TYPE, "text/markdown; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], API_DOCS.replace("{{BASE_URL}}", &base))
}
