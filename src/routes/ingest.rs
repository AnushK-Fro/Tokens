use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use chrono::{SecondsFormat, Utc};
use serde_json::{json, Value};
use subtle::ConstantTimeEq;

use crate::error::AppError;
use crate::ingest::{self, clean_label, valid_identifier, Source};
use crate::state::AppState;
use crate::store;

pub async fn accept(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Result<Json<Value>, AppError> {
    authorize(&state, &headers)?;

    let payload: Value = serde_json::from_slice(&body).map_err(|_| AppError::Unprocessable("Invalid JSON".to_owned()))?;
    let rows = ingest::parse(&payload).map_err(|error| AppError::Unprocessable(error.to_string()))?;

    let source = Source {
        id: header(&headers, "x-usage-source-id"),
        name: clean_label(&header(&headers, "x-usage-source-name"), 160),
        agent: header(&headers, "x-usage-agent").to_ascii_lowercase(),
        project: clean_label(&header(&headers, "x-usage-project"), 160),
    };

    if !valid_identifier(&source.id, 128) || !valid_identifier(&source.agent, 32) {
        return Err(AppError::Unprocessable("Invalid X-Usage-Source-Id or X-Usage-Agent header".to_owned()));
    }

    let source = Source { name: if source.name.is_empty() { "devcontainer".to_owned() } else { source.name }, ..source };

    if header(&headers, "x-usage-scope") == "full" {
        tracing::debug!(source = %source.id, "X-Usage-Scope: full is ignored; stored usage is never cleared");
    }

    let outcome = store::record_report(&state.pool, &source, &rows, Utc::now()).await?;

    if outcome.protected > 0 {
        tracing::info!(source = %source.id, agent = %source.agent, protected = outcome.protected, "kept stored values higher than the report");
    }

    Ok(Json(json!({
        "ok": true,
        "rows": outcome.rows,
        "inserted": outcome.inserted,
        "increased": outcome.increased,
        "unchanged": outcome.unchanged,
        "protected": outcome.protected,
        "observed_at": Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
    })))
}

fn authorize(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    let Some(expected) = state.config.ingest_token.as_deref() else {
        return Err(AppError::Unauthorized);
    };

    let presented = header(headers, "authorization");
    let Some(token) = presented.strip_prefix("Bearer ") else {
        return Err(AppError::Unauthorized);
    };

    if bool::from(token.as_bytes().ct_eq(expected.as_bytes())) {
        Ok(())
    } else {
        Err(AppError::Unauthorized)
    }
}

fn header(headers: &HeaderMap, name: &str) -> String {
    headers.get(name).and_then(|value| value.to_str().ok()).unwrap_or("").trim().to_owned()
}
