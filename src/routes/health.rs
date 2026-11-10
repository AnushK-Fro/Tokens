use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};

use crate::state::AppState;

pub async fn check(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => (StatusCode::OK, Json(json!({ "ok": true }))),
        Err(error) => {
            tracing::error!("health check failed: {error}");
            (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "ok": false })))
        }
    }
}
