//! Reading usage for the dashboard. Source names and projects never appear in these results.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::{MySqlPool, Row};

use crate::usage::{uncached_input, Counts};

/// One day of one model, summed across every source that reported it.
#[derive(Debug, Serialize)]
pub struct DailyModelUsage {
    pub day: NaiveDate,
    pub agent: String,
    pub model: String,
    /// Uncached input, whichever way the reporting ccusage version counted it; see
    /// [`uncached_input`].
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    /// Part of output for agents that report it; not added to the total separately.
    pub reasoning_tokens: u64,
    pub total_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Serialize)]
pub struct Retention {
    /// Stored rows for days older than anything in the latest report from their source.
    pub preserved_rows: i64,
    pub preserved_cost_usd: f64,
    /// Rows in the latest reports where ccusage gave a lower figure than the stored one.
    pub protected_rows: i64,
}

#[derive(Debug, Serialize)]
pub struct Meta {
    pub source_count: i64,
    pub last_sync_at: Option<DateTime<Utc>>,
    pub retention: Retention,
}

pub async fn daily_usage(pool: &MySqlPool) -> sqlx::Result<Vec<DailyModelUsage>> {
    let records = sqlx::query(
        "SELECT day, CAST(agent AS CHAR) AS agent, CAST(model AS CHAR) AS model,
            CAST(SUM(input_tokens) AS UNSIGNED) AS input_tokens,
            CAST(SUM(output_tokens) AS UNSIGNED) AS output_tokens,
            CAST(SUM(cache_read_tokens) AS UNSIGNED) AS cache_read_tokens,
            CAST(SUM(cache_creation_tokens) AS UNSIGNED) AS cache_creation_tokens,
            CAST(SUM(reasoning_tokens) AS UNSIGNED) AS reasoning_tokens,
            CAST(SUM(total_tokens) AS UNSIGNED) AS total_tokens,
            SUM(cost_usd) AS cost_usd
         FROM daily_usage
         GROUP BY day, agent, model
         ORDER BY day, total_tokens DESC",
    )
    .fetch_all(pool)
    .await?;

    let rows = records
        .into_iter()
        .map(|record| {
            let counts = Counts {
                input_tokens: record.try_get("input_tokens")?,
                output_tokens: record.try_get("output_tokens")?,
                cache_read_tokens: record.try_get("cache_read_tokens")?,
                cache_creation_tokens: record.try_get("cache_creation_tokens")?,
                reasoning_tokens: record.try_get("reasoning_tokens")?,
                total_tokens: record.try_get("total_tokens")?,
                cost_usd: record.try_get("cost_usd")?,
            };

            let model: String = record.try_get("model")?;
            Ok(DailyModelUsage {
                day: record.try_get("day")?,
                agent: record.try_get("agent")?,
                model: display_model(&model).to_owned(),
                input_tokens: uncached_input(&counts),
                output_tokens: counts.output_tokens,
                cache_read_tokens: counts.cache_read_tokens,
                cache_creation_tokens: counts.cache_creation_tokens,
                reasoning_tokens: counts.reasoning_tokens,
                total_tokens: counts.total_tokens,
                cost_usd: counts.cost_usd,
            })
        })
        .collect::<sqlx::Result<Vec<_>>>()?;

    Ok(merge_same_names(rows))
}

/// The name a model is shown under: provider and org prefixes dropped, so
/// `deepseek/deepseek-v4.1-flash` and `deepseek-v4.1-flash` read as one model and
/// `mlx-community/Qwen3.6-35B-A3B-4bit` as `Qwen3.6-35B-A3B-4bit`. Display only: stored rows
/// keep the name ccusage reported, so the merge rule still sees each one separately.
pub fn display_model(model: &str) -> &str {
    match model.rsplit_once('/') {
        Some((_, name)) if !name.trim().is_empty() => name,
        _ => model,
    }
}

/// Sums rows that share a day, agent and display name.
fn merge_same_names(rows: Vec<DailyModelUsage>) -> Vec<DailyModelUsage> {
    let mut merged: BTreeMap<(NaiveDate, String, String), DailyModelUsage> = BTreeMap::new();

    for row in rows {
        match merged.entry((row.day, row.agent.clone(), row.model.clone())) {
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(row);
            }
            std::collections::btree_map::Entry::Occupied(mut slot) => {
                let total = slot.get_mut();
                total.input_tokens += row.input_tokens;
                total.output_tokens += row.output_tokens;
                total.cache_read_tokens += row.cache_read_tokens;
                total.cache_creation_tokens += row.cache_creation_tokens;
                total.reasoning_tokens += row.reasoning_tokens;
                total.total_tokens += row.total_tokens;
                total.cost_usd += row.cost_usd;
            }
        }
    }

    let mut rows: Vec<_> = merged.into_values().collect();
    rows.sort_by(|a, b| a.day.cmp(&b.day).then(b.total_tokens.cmp(&a.total_tokens)));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(model: &str, total: u64, cost: f64) -> DailyModelUsage {
        DailyModelUsage {
            day: NaiveDate::from_ymd_opt(2026, 11, 7).unwrap(),
            agent: "opencode".to_owned(),
            model: display_model(model).to_owned(),
            input_tokens: total,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            total_tokens: total,
            cost_usd: cost,
        }
    }

    #[test]
    fn display_names_drop_prefixes() {
        assert_eq!(display_model("deepseek/deepseek-v4.1-flash"), "deepseek-v4.1-flash");
        assert_eq!(display_model("mlx-community/Qwen3.6-35B-A3B-4bit"), "Qwen3.6-35B-A3B-4bit");
        assert_eq!(display_model("claude-opus-5-5"), "claude-opus-5-5");
        assert_eq!(display_model("weird/"), "weird/");
    }

    #[test]
    fn prefixed_and_bare_names_are_summed() {
        let rows = merge_same_names(vec![
            row("deepseek-v4.1-flash", 10, 1.0),
            row("deepseek/deepseek-v4.1-flash", 5, 0.5),
            row("glm-5.3", 7, 0.25),
        ]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].model, "deepseek-v4.1-flash");
        assert_eq!(rows[0].total_tokens, 15);
        assert!((rows[0].cost_usd - 1.5).abs() < 1e-12);
    }
}

pub async fn meta(pool: &MySqlPool) -> sqlx::Result<Meta> {
    let source_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sources").fetch_one(pool).await?;
    let last_sync_at: Option<DateTime<Utc>> = sqlx::query_scalar("SELECT MAX(received_at) FROM ingests").fetch_one(pool).await?;

    let latest = "SELECT i.source_id, i.agent, i.first_day, i.rows_protected
                  FROM ingests i
                  JOIN (SELECT MAX(id) AS id FROM ingests GROUP BY source_id, agent) newest ON newest.id = i.id";

    let preserved = sqlx::query(&format!(
        "SELECT COUNT(*) AS preserved_rows, COALESCE(SUM(u.cost_usd), 0) AS preserved_cost
         FROM daily_usage u
         JOIN ({latest}) latest ON latest.source_id = u.source_id AND latest.agent = u.agent
         WHERE latest.first_day IS NOT NULL AND u.day < latest.first_day"
    ))
    .fetch_one(pool)
    .await?;

    let protected_rows: i64 = sqlx::query_scalar(&format!("SELECT CAST(COALESCE(SUM(rows_protected), 0) AS SIGNED) FROM ({latest}) latest"))
        .fetch_one(pool)
        .await?;

    Ok(Meta {
        source_count,
        last_sync_at,
        retention: Retention {
            preserved_rows: preserved.try_get("preserved_rows")?,
            preserved_cost_usd: preserved.try_get("preserved_cost")?,
            protected_rows,
        },
    })
}
