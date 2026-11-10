//! Per-request usage events from tokens-cli. Rows are only inserted or raised, like daily_usage.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::{MySqlPool, QueryBuilder};

use crate::ingest::Source;
use crate::store::ingest::upsert_source;

/// 21 columns × 200 rows stays well inside MariaDB's placeholder limit.
const INSERT_CHUNK: usize = 200;

#[derive(Debug, Clone, Deserialize)]
pub struct UsageEvent {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub model: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_creation_tokens: u64,
    #[serde(default)]
    pub cache_creation_5m_tokens: u64,
    #[serde(default)]
    pub cache_creation_1h_tokens: u64,
    #[serde(default)]
    pub reasoning_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(default)]
    pub cost_source: String,
    #[serde(default)]
    pub details: Option<serde_json::Value>,
}

/// Stores a batch of events. Returns how many rows MariaDB reports as written (inserts count 1,
/// raised rows 2, unchanged rows 0).
pub async fn record_events(pool: &MySqlPool, source: &Source, events: &[UsageEvent]) -> sqlx::Result<u64> {
    let now = Utc::now();
    let mut tx = pool.begin().await?;
    upsert_source(&mut tx, source, now).await?;

    let mut affected = 0;
    for chunk in events.chunks(INSERT_CHUNK) {
        let mut query = QueryBuilder::new(
            "INSERT INTO usage_events (agent, event_id, source_id, occurred_at, model, provider, session_id, project,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens, cache_creation_5m_tokens,
                cache_creation_1h_tokens, reasoning_tokens, total_tokens, cost_usd, cost_source, details,
                first_received_at, last_received_at) ",
        );
        query.push_values(chunk, |mut row, event| {
            row.push_bind(&source.agent)
                .push_bind(&event.id)
                .push_bind(&source.id)
                .push_bind(event.timestamp)
                .push_bind(&event.model)
                .push_bind(&event.provider)
                .push_bind(&event.session_id)
                .push_bind(&event.project)
                .push_bind(event.input_tokens)
                .push_bind(event.output_tokens)
                .push_bind(event.cache_read_tokens)
                .push_bind(event.cache_creation_tokens)
                .push_bind(event.cache_creation_5m_tokens)
                .push_bind(event.cache_creation_1h_tokens)
                .push_bind(event.reasoning_tokens)
                .push_bind(event.total_tokens)
                .push_bind(event.cost_usd)
                .push_bind(&event.cost_source)
                .push_bind(event.details.as_ref().map(serde_json::Value::to_string))
                .push_bind(now)
                .push_bind(now);
        });
        // Assignments run left to right, so details and labels are compared against the old total
        // before total_tokens is raised. The trigger keeps every count at its maximum.
        query.push(
            " ON DUPLICATE KEY UPDATE
                details = IF(VALUES(total_tokens) >= total_tokens, VALUES(details), details),
                cost_source = IF(VALUES(cost_usd) >= cost_usd, VALUES(cost_source), cost_source),
                input_tokens = GREATEST(input_tokens, VALUES(input_tokens)),
                output_tokens = GREATEST(output_tokens, VALUES(output_tokens)),
                cache_read_tokens = GREATEST(cache_read_tokens, VALUES(cache_read_tokens)),
                cache_creation_tokens = GREATEST(cache_creation_tokens, VALUES(cache_creation_tokens)),
                cache_creation_5m_tokens = GREATEST(cache_creation_5m_tokens, VALUES(cache_creation_5m_tokens)),
                cache_creation_1h_tokens = GREATEST(cache_creation_1h_tokens, VALUES(cache_creation_1h_tokens)),
                reasoning_tokens = GREATEST(reasoning_tokens, VALUES(reasoning_tokens)),
                total_tokens = GREATEST(total_tokens, VALUES(total_tokens)),
                cost_usd = GREATEST(cost_usd, VALUES(cost_usd)),
                last_received_at = VALUES(last_received_at)",
        );
        affected += query.build().execute(&mut *tx).await?.rows_affected();
    }

    tx.commit().await?;
    Ok(affected)
}
