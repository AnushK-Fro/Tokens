//! What was actually paid, for the dashboard's Spending panel. Rows come from
//! `tokens import-payments`; nothing writes them at runtime. Invoice and receipt numbers stay in the database and are not served.

use chrono::NaiveDate;
use serde::Serialize;
use sqlx::{MySqlPool, Row};

#[derive(Debug, Serialize)]
pub struct Payment {
    pub paid_on: NaiveDate,
    /// Billing account: `chatgpt`, `claude`, `cursor`, `kimi`, `zai`, `opencode`, `openrouter`,
    /// `openai-api`.
    pub provider: String,
    pub product: String,
    /// `subscription` or `usage`.
    pub kind: String,
    /// The dashboard agent the payment funds, if any.
    pub agent: Option<String>,
    pub amount_usd: f64,
    pub note: String,
}

/// Every payment, newest first.
pub async fn payments(pool: &MySqlPool) -> sqlx::Result<Vec<Payment>> {
    let records = sqlx::query(
        "SELECT paid_on, CAST(provider AS CHAR) AS provider, CAST(product AS CHAR) AS product,
            CAST(kind AS CHAR) AS kind, CAST(agent AS CHAR) AS agent,
            CAST(amount_usd AS DOUBLE) AS amount_usd, CAST(note AS CHAR) AS note
         FROM payments
         ORDER BY paid_on DESC, id DESC",
    )
    .fetch_all(pool)
    .await?;

    records
        .into_iter()
        .map(|record| {
            Ok(Payment {
                paid_on: record.try_get("paid_on")?,
                provider: record.try_get("provider")?,
                product: record.try_get("product")?,
                kind: record.try_get("kind")?,
                agent: record.try_get("agent")?,
                amount_usd: record.try_get("amount_usd")?,
                note: record.try_get("note")?,
            })
        })
        .collect()
}
