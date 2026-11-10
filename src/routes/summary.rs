use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde_json::json;

use crate::error::AppError;
use crate::state::AppState;
use crate::store::summary;

pub async fn show(State(state): State<AppState>) -> Result<Response, AppError> {
    let (rows, meta) = tokio::try_join!(summary::daily_usage(&state.pool), summary::meta(&state.pool))?;
    let now = Utc::now();

    let body = json!({
        "generated_at": now,
        "timezone": state.config.timezone.name(),
        "today": now.with_timezone(&state.config.timezone).date_naive(),
        "source_count": meta.source_count,
        "last_sync_at": meta.last_sync_at,
        "retention": meta.retention,
        "daily": rows,
    });

    Ok(([("cache-control", "no-store")], Json(body)).into_response())
}
