//! Writing reports. Everything goes through [`record_report`], which only ever inserts rows or
//! raises columns; it has no code path that lowers or deletes usage.

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::{MySql, MySqlPool, QueryBuilder, Row, Transaction};

use crate::ingest::{ReportRow, Source};
use crate::usage::Counts;

/// MariaDB caps placeholders per statement; 13 columns × 500 rows stays well inside it.
const INSERT_CHUNK: usize = 500;

#[derive(Debug, Default, Serialize)]
pub struct IngestOutcome {
    pub ingest_id: u64,
    pub rows: usize,
    pub inserted: usize,
    pub increased: usize,
    pub unchanged: usize,
    /// Rows where the report was lower than what was stored; the stored values were kept.
    pub protected: usize,
}

struct Change {
    day: NaiveDate,
    model: String,
    counts: Counts,
}

/// Merges one report from one source into storage, monotonically.
///
/// `seen_at` is when the source was last active (now, for live syncs; the original timestamp for
/// imports) so importing an old database never makes a source look older than it is.
pub async fn record_report(pool: &MySqlPool, source: &Source, rows: &[ReportRow], seen_at: DateTime<Utc>) -> sqlx::Result<IngestOutcome> {
    let now = Utc::now();
    let mut tx = pool.begin().await?;

    upsert_source(&mut tx, source, seen_at).await?;

    // Serialises reports from the same source so two overlapping syncs cannot both read the old
    // values and race each other; the update trigger would still keep the maximum regardless.
    sqlx::query("SELECT id FROM sources WHERE id = ? FOR UPDATE").bind(&source.id).fetch_one(&mut *tx).await?;

    let mut stored = load_existing(&mut tx, source, rows).await?;
    let mut outcome = IngestOutcome { rows: rows.len(), ..IngestOutcome::default() };
    let mut inserts = Vec::new();
    let mut updates = Vec::new();

    for row in rows {
        match stored.get_mut(&(row.day, row.model.clone())) {
            None => {
                outcome.inserted += 1;
                inserts.push(Change { day: row.day, model: row.model.clone(), counts: row.counts });
            }
            Some(current) => {
                let merge = current.merge_max(&row.counts);
                if merge.protected {
                    outcome.protected += 1;
                }
                if merge.increased {
                    outcome.increased += 1;
                    updates.push(Change { day: row.day, model: row.model.clone(), counts: *current });
                } else {
                    outcome.unchanged += 1;
                }
            }
        }
    }

    // Keeps a later duplicate in the same payload from being inserted twice.
    for change in &inserts {
        stored.insert((change.day, change.model.clone()), change.counts);
    }

    let ingest_id = sqlx::query(
        "INSERT INTO ingests (source_id, agent, received_at, rows_received, rows_inserted, rows_increased, rows_unchanged, rows_protected, first_day, last_day)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&source.id)
    .bind(&source.agent)
    .bind(now)
    .bind(outcome.rows as u32)
    .bind(outcome.inserted as u32)
    .bind(outcome.increased as u32)
    .bind(outcome.unchanged as u32)
    .bind(outcome.protected as u32)
    .bind(rows.iter().map(|row| row.day).min())
    .bind(rows.iter().map(|row| row.day).max())
    .execute(&mut *tx)
    .await?
    .last_insert_id();
    outcome.ingest_id = ingest_id;

    insert_rows(&mut tx, source, &inserts, now).await?;

    for change in &updates {
        // GREATEST() here as well as in Rust: belt and braces, plus the table trigger.
        sqlx::query(
            "UPDATE daily_usage SET
                input_tokens = GREATEST(input_tokens, ?),
                output_tokens = GREATEST(output_tokens, ?),
                cache_read_tokens = GREATEST(cache_read_tokens, ?),
                cache_creation_tokens = GREATEST(cache_creation_tokens, ?),
                reasoning_tokens = GREATEST(reasoning_tokens, ?),
                total_tokens = GREATEST(total_tokens, ?),
                cost_usd = GREATEST(cost_usd, ?),
                last_changed_at = ?, last_observed_at = ?
             WHERE source_id = ? AND day = ? AND agent = ? AND model = ?",
        )
        .bind(change.counts.input_tokens)
        .bind(change.counts.output_tokens)
        .bind(change.counts.cache_read_tokens)
        .bind(change.counts.cache_creation_tokens)
        .bind(change.counts.reasoning_tokens)
        .bind(change.counts.total_tokens)
        .bind(change.counts.cost_usd)
        .bind(now)
        .bind(now)
        .bind(&source.id)
        .bind(change.day)
        .bind(&source.agent)
        .bind(&change.model)
        .execute(&mut *tx)
        .await?;
    }

    let revisions: Vec<&Change> = inserts.iter().chain(updates.iter()).collect();
    insert_revisions(&mut tx, ingest_id, source, &revisions, now).await?;

    tx.commit().await?;
    Ok(outcome)
}

pub(crate) async fn upsert_source(tx: &mut Transaction<'_, MySql>, source: &Source, seen_at: DateTime<Utc>) -> sqlx::Result<()> {
    // Labels follow the most recent sighting; assignments run left to right, so last_seen_at
    // is compared before it is moved forward.
    sqlx::query(
        "INSERT INTO sources (id, name, agent, project, first_seen_at, last_seen_at) VALUES (?, ?, ?, ?, ?, ?)
         ON DUPLICATE KEY UPDATE
            name = IF(VALUES(last_seen_at) >= last_seen_at, VALUES(name), name),
            agent = IF(VALUES(last_seen_at) >= last_seen_at, VALUES(agent), agent),
            project = IF(VALUES(last_seen_at) >= last_seen_at, VALUES(project), project),
            first_seen_at = LEAST(first_seen_at, VALUES(first_seen_at)),
            last_seen_at = GREATEST(last_seen_at, VALUES(last_seen_at))",
    )
    .bind(&source.id)
    .bind(&source.name)
    .bind(&source.agent)
    .bind(&source.project)
    .bind(seen_at)
    .bind(seen_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn load_existing(tx: &mut Transaction<'_, MySql>, source: &Source, rows: &[ReportRow]) -> sqlx::Result<HashMap<(NaiveDate, String), Counts>> {
    let (Some(first), Some(last)) = (rows.iter().map(|row| row.day).min(), rows.iter().map(|row| row.day).max()) else {
        return Ok(HashMap::new());
    };

    let records = sqlx::query(
        // Text columns use a binary collation (exact model-name matching); CAST so the driver
        // decodes them as strings.
        "SELECT day, CAST(model AS CHAR) AS model, input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                reasoning_tokens, total_tokens, cost_usd
         FROM daily_usage
         WHERE source_id = ? AND agent = ? AND day BETWEEN ? AND ?
         FOR UPDATE",
    )
    .bind(&source.id)
    .bind(&source.agent)
    .bind(first)
    .bind(last)
    .fetch_all(&mut **tx)
    .await?;

    records
        .into_iter()
        .map(|record| {
            Ok((
                (record.try_get("day")?, record.try_get("model")?),
                Counts {
                    input_tokens: record.try_get("input_tokens")?,
                    output_tokens: record.try_get("output_tokens")?,
                    cache_read_tokens: record.try_get("cache_read_tokens")?,
                    cache_creation_tokens: record.try_get("cache_creation_tokens")?,
                    reasoning_tokens: record.try_get("reasoning_tokens")?,
                    total_tokens: record.try_get("total_tokens")?,
                    cost_usd: record.try_get("cost_usd")?,
                },
            ))
        })
        .collect()
}

async fn insert_rows(tx: &mut Transaction<'_, MySql>, source: &Source, rows: &[Change], now: DateTime<Utc>) -> sqlx::Result<()> {
    for chunk in rows.chunks(INSERT_CHUNK) {
        let mut query = QueryBuilder::<MySql>::new(
            "INSERT INTO daily_usage (source_id, day, agent, model, input_tokens, output_tokens, cache_read_tokens,
             cache_creation_tokens, reasoning_tokens, total_tokens, cost_usd, first_observed_at, last_changed_at, last_observed_at) ",
        );
        query.push_values(chunk, |mut values, row| {
            values
                .push_bind(&source.id)
                .push_bind(row.day)
                .push_bind(&source.agent)
                .push_bind(&row.model)
                .push_bind(row.counts.input_tokens)
                .push_bind(row.counts.output_tokens)
                .push_bind(row.counts.cache_read_tokens)
                .push_bind(row.counts.cache_creation_tokens)
                .push_bind(row.counts.reasoning_tokens)
                .push_bind(row.counts.total_tokens)
                .push_bind(row.counts.cost_usd)
                .push_bind(now)
                .push_bind(now)
                .push_bind(now);
        });
        query.build().execute(&mut **tx).await?;
    }
    Ok(())
}

async fn insert_revisions(tx: &mut Transaction<'_, MySql>, ingest_id: u64, source: &Source, rows: &[&Change], now: DateTime<Utc>) -> sqlx::Result<()> {
    for chunk in rows.chunks(INSERT_CHUNK) {
        let mut query = QueryBuilder::<MySql>::new(
            "INSERT INTO usage_revisions (ingest_id, source_id, day, agent, model, input_tokens, output_tokens, cache_read_tokens,
             cache_creation_tokens, reasoning_tokens, total_tokens, cost_usd, recorded_at) ",
        );
        query.push_values(chunk, |mut values, row| {
            values
                .push_bind(ingest_id)
                .push_bind(&source.id)
                .push_bind(row.day)
                .push_bind(&source.agent)
                .push_bind(&row.model)
                .push_bind(row.counts.input_tokens)
                .push_bind(row.counts.output_tokens)
                .push_bind(row.counts.cache_read_tokens)
                .push_bind(row.counts.cache_creation_tokens)
                .push_bind(row.counts.reasoning_tokens)
                .push_bind(row.counts.total_tokens)
                .push_bind(row.counts.cost_usd)
                .push_bind(now);
        });
        query.build().execute(&mut **tx).await?;
    }
    Ok(())
}
