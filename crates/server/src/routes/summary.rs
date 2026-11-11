use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use chrono_tz::Tz;
use serde::Deserialize;
use serde_json::json;

use crate::error::AppError;
use crate::state::AppState;
use crate::store::{payments, summary};

#[derive(Deserialize)]
pub struct Params {
    /// IANA zone to count days in, normally the browser's. Unknown or missing: the server's zone.
    tz: Option<String>,
}

pub async fn show(State(state): State<AppState>, Query(params): Query<Params>) -> Result<Response, AppError> {
    let recorded = state.config.timezone;
    let view = params.tz.and_then(|name| name.parse::<Tz>().ok()).unwrap_or(recorded);

    let (rows, meta, payments) = tokio::try_join!(
        summary::daily_usage(&state.pool, recorded, view),
        summary::meta(&state.pool),
        payments::payments(&state.pool)
    )?;
    let now = Utc::now();

    let body = json!({
        "generated_at": now,
        "timezone": view.name(),
        "today": now.with_timezone(&view).date_naive(),
        "source_count": meta.source_count,
        "last_sync_at": meta.last_sync_at,
        "retention": meta.retention,
        "daily": rows,
        "payments": payments,
    });

    Ok(([("cache-control", "no-store")], Json(body)).into_response())
}
