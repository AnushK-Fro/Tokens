use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::ingest::{authorize, source};
use crate::error::AppError;
use crate::ingest::clean_label;
use crate::state::AppState;
use crate::store::{self, UsageEvent};

pub const MAX_EVENTS: usize = 5000;

#[derive(Deserialize)]
struct Batch {
    events: Vec<UsageEvent>,
}

/// `POST /api/events`: per-request usage from tokens-cli, same headers as `/api/ingest`.
pub async fn accept(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Result<Json<Value>, AppError> {
    authorize(&state, &headers)?;
    let source = source(&headers)?;

    let batch: Batch = serde_json::from_slice(&body).map_err(|error| AppError::Unprocessable(format!("Invalid events: {error}")))?;
    if batch.events.len() > MAX_EVENTS {
        return Err(AppError::Unprocessable(format!("Too many events (limit {MAX_EVENTS} per request)")));
    }

    let events: Vec<UsageEvent> = batch.events.into_iter().filter_map(clean).collect();
    let written = store::record_events(&state.pool, &source, &events).await?;

    Ok(Json(json!({ "ok": true, "events": events.len(), "written": written })))
}

/// Caps labels to their column sizes; drops events without an id or model.
fn clean(event: UsageEvent) -> Option<UsageEvent> {
    let id = clean_label(&event.id, 255);
    let model = clean_label(&event.model, 160);
    if id.is_empty() || model.is_empty() {
        return None;
    }
    let cost_usd = if event.cost_usd.is_finite() { event.cost_usd.max(0.0) } else { 0.0 };

    Some(UsageEvent {
        id,
        model,
        provider: clean_label(&event.provider, 64),
        session_id: clean_label(&event.session_id, 255),
        project: clean_label(&event.project, 512),
        cost_source: clean_label(&event.cost_source, 16),
        cost_usd,
        ..event
    })
}
