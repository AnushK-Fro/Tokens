//! One-off import of the PHP version's SQLite database. Goes through the same monotonic merge as
//! live syncs, so it can be run before or after the new app starts receiving data, and re-running
//! it changes nothing.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{Connection, OpenFlags};
use sqlx::MySqlPool;

use crate::ingest::{ReportRow, Source};
use crate::store::record_report;
use crate::usage::Counts;

#[derive(Default)]
pub struct ImportSummary {
    groups: usize,
    rows: usize,
    inserted: usize,
    increased: usize,
    unchanged: usize,
    protected: usize,
}

impl fmt::Display for ImportSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "imported {} rows from {} source/agent groups: {} new, {} raised, {} unchanged, {} kept higher stored values",
            self.rows, self.groups, self.inserted, self.increased, self.unchanged, self.protected
        )
    }
}

struct SqliteSource {
    name: String,
    project: String,
    last_seen_at: DateTime<Utc>,
}

pub async fn from_sqlite(pool: &MySqlPool, path: &Path) -> Result<ImportSummary> {
    let (sources, groups) = read_sqlite(path)?;
    let mut summary = ImportSummary::default();

    for ((source_id, agent), rows) in groups {
        let meta = sources.get(&source_id);
        let source = Source {
            id: source_id.clone(),
            name: meta.map_or_else(|| "devcontainer".to_owned(), |meta| meta.name.clone()),
            agent,
            project: meta.map(|meta| meta.project.clone()).unwrap_or_default(),
        };
        let seen_at = meta.map_or_else(Utc::now, |meta| meta.last_seen_at);

        let outcome = record_report(pool, &source, &rows, seen_at).await.with_context(|| format!("importing source {source_id}"))?;
        summary.groups += 1;
        summary.rows += outcome.rows;
        summary.inserted += outcome.inserted;
        summary.increased += outcome.increased;
        summary.unchanged += outcome.unchanged;
        summary.protected += outcome.protected;
    }

    Ok(summary)
}

type Groups = BTreeMap<(String, String), Vec<ReportRow>>;

fn read_sqlite(path: &Path) -> Result<(BTreeMap<String, SqliteSource>, Groups)> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).with_context(|| format!("opening {}", path.display()))?;

    let has_project = connection.prepare("SELECT project FROM sources LIMIT 0").is_ok();
    let source_query = if has_project { "SELECT id, name, project, last_seen_at FROM sources" } else { "SELECT id, name, '', last_seen_at FROM sources" };

    let mut sources = BTreeMap::new();
    let mut statement = connection.prepare(source_query)?;
    let mut records = statement.query([])?;
    while let Some(record) = records.next()? {
        let last_seen: String = record.get(3)?;
        sources.insert(
            record.get::<_, String>(0)?,
            SqliteSource {
                name: record.get(1)?,
                project: record.get(2)?,
                last_seen_at: DateTime::parse_from_rfc3339(&last_seen).map(|at| at.with_timezone(&Utc)).unwrap_or_else(|_| Utc::now()),
            },
        );
    }

    let mut groups: Groups = BTreeMap::new();
    let mut statement = connection.prepare(
        "SELECT source_id, agent, day, model, input_tokens, output_tokens, cache_read_tokens,
                cache_creation_tokens, reasoning_tokens, total_tokens, cost_usd
         FROM daily_usage ORDER BY source_id, agent, day",
    )?;
    let mut records = statement.query([])?;
    while let Some(record) = records.next()? {
        let day: String = record.get(2)?;
        let Ok(day) = NaiveDate::parse_from_str(&day, "%Y-%m-%d") else {
            continue;
        };
        let count = |index: usize| -> rusqlite::Result<u64> { Ok(record.get::<_, i64>(index)?.max(0) as u64) };

        groups.entry((record.get(0)?, record.get(1)?)).or_default().push(ReportRow {
            day,
            model: record.get(3)?,
            counts: Counts {
                input_tokens: count(4)?,
                output_tokens: count(5)?,
                cache_read_tokens: count(6)?,
                cache_creation_tokens: count(7)?,
                reasoning_tokens: count(8)?,
                total_tokens: count(9)?,
                cost_usd: record.get::<_, f64>(10)?.max(0.0),
            },
        });
    }

    Ok((sources, groups))
}
