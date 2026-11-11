//! Reading usage for the dashboard. Source names and projects never appear in these results.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use serde::Serialize;
use sqlx::mysql::MySqlRow;
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

/// Every day × agent × model, with days counted in `view`.
///
/// `daily_usage` rows are already days: sync clients bucket them in `recorded` (the server's
/// `TOKEN_USAGE_TIMEZONE`) and they can't be split again. Where per-request events exist, those
/// requests are moved to their day in `view`; whatever a stored row has beyond its events (history
/// from before events were sent, or a higher ccusage figure) stays on the day it was recorded.
pub async fn daily_usage(pool: &MySqlPool, recorded: Tz, view: Tz) -> sqlx::Result<Vec<DailyModelUsage>> {
    let days = stored_days(pool).await?;
    let events = if view == recorded { Vec::new() } else { event_buckets(pool).await? };
    Ok(merge_same_names(rebucket(days, events, recorded, view)))
}

/// Seconds per event bucket. Every zone in use is offset from UTC by a multiple of 15 minutes, so
/// a bucket never straddles midnight in any of them.
const BUCKET_SECONDS: i64 = 900;

/// One stored row, before sources are summed.
struct StoredDay {
    source_id: String,
    day: NaiveDate,
    agent: String,
    model: String,
    counts: Counts,
}

/// Requests of one source, agent and model within one 15-minute bucket.
struct EventBucket {
    source_id: String,
    agent: String,
    model: String,
    start: DateTime<Utc>,
    counts: Counts,
}

fn counts_from(record: &MySqlRow) -> sqlx::Result<Counts> {
    Ok(Counts {
        input_tokens: record.try_get("input_tokens")?,
        output_tokens: record.try_get("output_tokens")?,
        cache_read_tokens: record.try_get("cache_read_tokens")?,
        cache_creation_tokens: record.try_get("cache_creation_tokens")?,
        reasoning_tokens: record.try_get("reasoning_tokens")?,
        total_tokens: record.try_get("total_tokens")?,
        cost_usd: record.try_get("cost_usd")?,
    })
}

async fn stored_days(pool: &MySqlPool) -> sqlx::Result<Vec<StoredDay>> {
    sqlx::query(
        "SELECT CAST(source_id AS CHAR) AS source_id, day, CAST(agent AS CHAR) AS agent, CAST(model AS CHAR) AS model,
            input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens, reasoning_tokens, total_tokens, cost_usd
         FROM daily_usage",
    )
    .fetch_all(pool)
    .await?
    .iter()
    .map(|record| {
        Ok(StoredDay {
            source_id: record.try_get("source_id")?,
            day: record.try_get("day")?,
            agent: record.try_get("agent")?,
            model: record.try_get("model")?,
            counts: counts_from(record)?,
        })
    })
    .collect()
}

async fn event_buckets(pool: &MySqlPool) -> sqlx::Result<Vec<EventBucket>> {
    // The pool's session zone is UTC, so UNIX_TIMESTAMP reads occurred_at as stored.
    sqlx::query(
        "SELECT CAST(source_id AS CHAR) AS source_id, CAST(agent AS CHAR) AS agent, CAST(model AS CHAR) AS model,
            CAST(FLOOR(UNIX_TIMESTAMP(occurred_at) / ?) AS SIGNED) AS bucket,
            CAST(SUM(input_tokens) AS UNSIGNED) AS input_tokens,
            CAST(SUM(output_tokens) AS UNSIGNED) AS output_tokens,
            CAST(SUM(cache_read_tokens) AS UNSIGNED) AS cache_read_tokens,
            CAST(SUM(cache_creation_tokens) AS UNSIGNED) AS cache_creation_tokens,
            CAST(SUM(reasoning_tokens) AS UNSIGNED) AS reasoning_tokens,
            CAST(SUM(total_tokens) AS UNSIGNED) AS total_tokens,
            SUM(cost_usd) AS cost_usd
         FROM usage_events
         GROUP BY source_id, agent, model, bucket",
    )
    .bind(BUCKET_SECONDS)
    .fetch_all(pool)
    .await?
    .iter()
    .map(|record| {
        let bucket: i64 = record.try_get("bucket")?;
        Ok(EventBucket {
            source_id: record.try_get("source_id")?,
            agent: record.try_get("agent")?,
            model: record.try_get("model")?,
            start: DateTime::from_timestamp(bucket * BUCKET_SECONDS, 0).unwrap_or_default(),
            counts: counts_from(record)?,
        })
    })
    .collect()
}

/// Sums stored days and events into days of `view`; see [`daily_usage`].
fn rebucket(days: Vec<StoredDay>, events: Vec<EventBucket>, recorded: Tz, view: Tz) -> Vec<DailyModelUsage> {
    let mut covered: HashMap<(String, String, NaiveDate, String), Counts> = HashMap::new();
    let mut totals: BTreeMap<(NaiveDate, String, String), Counts> = BTreeMap::new();

    for event in events {
        let recorded_day = event.start.with_timezone(&recorded).date_naive();
        let view_day = event.start.with_timezone(&view).date_naive();
        covered.entry((event.source_id, event.agent.clone(), recorded_day, event.model.clone())).or_default().add(&event.counts);
        totals.entry((view_day, event.agent, event.model)).or_default().add(&event.counts);
    }

    for stored in days {
        let mut counts = stored.counts;
        counts.input_tokens = uncached_input(&counts);
        let rest = match covered.get(&(stored.source_id, stored.agent.clone(), stored.day, stored.model.clone())) {
            Some(seen) => counts.saturating_sub(seen),
            None => counts,
        };
        if !rest.is_empty() {
            totals.entry((stored.day, stored.agent, stored.model)).or_default().add(&rest);
        }
    }

    totals
        .into_iter()
        .filter(|(_, counts)| !counts.is_empty())
        .map(|((day, agent, model), counts)| DailyModelUsage {
            day,
            agent,
            model: display_model(&model).to_owned(),
            input_tokens: counts.input_tokens,
            output_tokens: counts.output_tokens,
            cache_read_tokens: counts.cache_read_tokens,
            cache_creation_tokens: counts.cache_creation_tokens,
            reasoning_tokens: counts.reasoning_tokens,
            total_tokens: counts.total_tokens,
            cost_usd: counts.cost_usd,
        })
        .collect()
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

    fn tokens(total: u64) -> Counts {
        Counts { input_tokens: total, total_tokens: total, cost_usd: total as f64 / 100.0, ..Counts::default() }
    }

    fn stored(day: &str, total: u64) -> StoredDay {
        StoredDay { source_id: "a".into(), day: day.parse().unwrap(), agent: "claude".into(), model: "opus".into(), counts: tokens(total) }
    }

    fn event(at: &str, total: u64) -> EventBucket {
        EventBucket { source_id: "a".into(), agent: "claude".into(), model: "opus".into(), start: at.parse().unwrap(), counts: tokens(total) }
    }

    fn by_day(rows: &[DailyModelUsage]) -> Vec<(String, u64)> {
        rows.iter().map(|row| (row.day.to_string(), row.total_tokens)).collect()
    }

    #[test]
    fn events_move_to_the_viewers_day() {
        // Nov 10 UTC: 30 tokens at 02:00 UTC (Nov 9 evening in New York) and 70 at 15:00.
        let days = vec![stored("2026-11-10", 100)];
        let events = vec![event("2026-11-10T02:00:00Z", 30), event("2026-11-10T15:00:00Z", 70)];
        let rows = rebucket(days, events, Tz::UTC, Tz::America__New_York);
        assert_eq!(by_day(&rows), vec![("2026-11-09".into(), 30), ("2026-11-10".into(), 70)]);
    }

    #[test]
    fn usage_without_events_stays_on_its_recorded_day() {
        // 40 of Nov 10's 100 tokens have events; the other 60 (purged before events were sent)
        // stay on Nov 10, as does Nov 9, which has no events at all.
        let days = vec![stored("2026-11-09", 50), stored("2026-11-10", 100)];
        let events = vec![event("2026-11-10T01:00:00Z", 40)];
        let rows = rebucket(days, events, Tz::UTC, Tz::America__New_York);
        assert_eq!(by_day(&rows), vec![("2026-11-09".into(), 90), ("2026-11-10".into(), 60)]);
        let total: u64 = rows.iter().map(|row| row.total_tokens).sum();
        assert_eq!(total, 150);
    }

    #[test]
    fn events_ahead_of_the_daily_sync_still_count() {
        let rows = rebucket(Vec::new(), vec![event("2026-11-10T23:30:00Z", 10)], Tz::UTC, Tz::Asia__Kolkata);
        assert_eq!(by_day(&rows), vec![("2026-11-11".into(), 10)]);
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
