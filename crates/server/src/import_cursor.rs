//! Import of a Cursor usage export (`usage-events-*.csv` from cursor.com/dashboard → Usage →
//! Export). One CSV row is one request; they are summed into per-day, per-model rows and merged
//! through the same monotonic [`record_report`] as live syncs, so re-importing the same or an
//! overlapping export changes nothing, and a later export can only raise a day.
//!
//! Column mapping (Cursor's `Total Tokens` is the sum of the four token columns):
//! - `Input (w/o Cache Write)` → `input_tokens`
//! - `Input (w/ Cache Write)`  → `cache_creation_tokens`
//! - `Cache Read`              → `cache_read_tokens`
//! - `Output Tokens`           → `output_tokens`
//! - `Cost` → `cost_usd` when it is a dollar amount. Plan usage (`Included` / `Free`) carries no
//!   amount, so it is priced per request at Cursor's published API rates ([`rate`]); models with
//!   no published rate (e.g. `auto`) stay at 0.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use sqlx::MySqlPool;

use crate::ingest::{valid_identifier, ReportRow, Source};
use crate::store::record_report;
use crate::usage::Counts;

pub const AGENT: &str = "cursor";
pub const DEFAULT_SOURCE_ID: &str = "cursor-usage-export";
const SOURCE_NAME: &str = "Cursor (usage export)";
const MAX_MODEL_LENGTH: usize = 160;

#[derive(Debug, Default, PartialEq)]
pub struct Parsed {
    pub rows: Vec<ReportRow>,
    /// Requests that carried tokens.
    pub events: usize,
    /// Requests with no tokens at all (errored or free requests); nothing to record.
    pub empty: usize,
    pub latest: Option<DateTime<Utc>>,
    /// Requests whose cost was computed from [`rate`] because the export gave no amount.
    pub priced: usize,
    /// Models with plan usage but no known rate; their cost stays 0.
    pub unpriced_models: BTreeSet<String>,
}

/// USD per 1M tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rate {
    pub input: f64,
    /// Cursor lists no cache-write surcharge for Grok, Composer or Kimi: billed as input.
    pub cache_write: f64,
    pub cache_read: f64,
    pub output: f64,
    /// Prompts (input + cache write + cache read) above this many tokens are billed, whole
    /// request, at `multiplier` × these rates.
    pub long_context: Option<(u64, f64)>,
}

impl Rate {
    const fn flat(input: f64, cache_write: f64, cache_read: f64, output: f64) -> Self {
        Self { input, cache_write, cache_read, output, long_context: None }
    }

    pub fn cost(&self, counts: &Counts) -> f64 {
        let prompt = counts.input_tokens + counts.cache_creation_tokens + counts.cache_read_tokens;
        let multiplier = match self.long_context {
            Some((threshold, multiplier)) if prompt > threshold => multiplier,
            _ => 1.0,
        };
        let dollars = counts.input_tokens as f64 * self.input
            + counts.cache_creation_tokens as f64 * self.cache_write
            + counts.cache_read_tokens as f64 * self.cache_read
            + counts.output_tokens as f64 * self.output;
        dollars * multiplier / 1_000_000.0
    }
}

/// Cursor's published API list prices (cursor.com/docs/models, read 2026-11-10), by the model
/// names its usage export uses. Effort / thinking / Max Mode suffixes (`-high`, `-thinking`,
/// `-max`, …) don't change the per-token rate; `-fast` does.
///
/// Grok Bot (`grok-bot-*`) has no published per-token rate (it draws from the Cursor-model pool);
/// it is estimated at Grok 4.7 standard, the flagship model in that pool. `auto` has no fixed
/// rate and returns `None`.
pub fn rate(model: &str) -> Option<Rate> {
    let mut name = model.trim().to_ascii_lowercase();
    if name.starts_with("grok-bot") {
        return rate("grok-4.7");
    }
    if let Some(rest) = name.strip_prefix("cursor-") {
        name = rest.to_owned();
    }

    let mut fast = false;
    loop {
        let Some((base, suffix)) = name.rsplit_once('-') else { break };
        match suffix {
            "fast" => fast = true,
            "thinking" | "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" => {}
            _ => break,
        }
        name = base.to_owned();
    }

    // Grok 4.7: over 256k prompt tokens costs 2× standard, or 3× standard in Fast.
    const GROK_47_LONG: u64 = 256_000;
    Some(match (name.as_str(), fast) {
        ("grok-4.7", false) => Rate { long_context: Some((GROK_47_LONG, 2.0)), ..Rate::flat(2.0, 2.0, 0.50, 6.0) },
        ("grok-4.7", true) => Rate { long_context: Some((GROK_47_LONG, 1.5)), ..Rate::flat(4.0, 4.0, 1.0, 12.0) },
        ("grok-4.6", false) => Rate::flat(2.0, 2.0, 0.50, 6.0),
        ("grok-4.6", true) => Rate::flat(4.0, 4.0, 1.0, 12.0),
        ("grok-4.5", false) => Rate::flat(2.0, 2.0, 0.50, 6.0),
        ("grok-4.5", true) => Rate::flat(4.0, 4.0, 1.0, 18.0),
        ("composer-2.5", false) => Rate::flat(0.50, 0.50, 0.20, 2.50),
        ("composer-2.5", true) => Rate::flat(3.0, 3.0, 0.50, 15.0),
        ("claude-opus-4-8", false) => Rate::flat(5.0, 6.25, 0.50, 25.0),
        ("claude-sonnet-5", false) => Rate::flat(2.0, 2.50, 0.20, 10.0),
        ("kimi-k3", false) => Rate::flat(3.0, 3.0, 0.30, 15.0),
        // Older models, under the names older exports use (`claude-4.5-opus-high-thinking`, …).
        ("claude-4-sonnet" | "claude-4.5-sonnet", false) => Rate::flat(3.0, 3.75, 0.30, 15.0),
        ("claude-4.5-opus" | "claude-4.6-opus", false) => Rate::flat(5.0, 6.25, 0.50, 25.0),
        ("gpt-5" | "gpt-5-codex", false) => Rate::flat(1.25, 1.25, 0.125, 10.0),
        ("gpt-5.2-codex" | "gpt-5.3-codex", false) => Rate::flat(1.75, 1.75, 0.175, 14.0),
        _ => return None,
    })
}

pub struct CursorImportSummary {
    source_id: String,
    events: usize,
    empty: usize,
    first_day: Option<NaiveDate>,
    last_day: Option<NaiveDate>,
    rows: usize,
    inserted: usize,
    increased: usize,
    unchanged: usize,
    protected: usize,
    priced: usize,
    cost_usd: f64,
    unpriced_models: BTreeSet<String>,
}

impl fmt::Display for CursorImportSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let span = match (self.first_day, self.last_day) {
            (Some(first), Some(last)) => format!(" ({first} – {last})"),
            _ => String::new(),
        };
        write!(
            f,
            "imported {} Cursor requests as {} day/model rows{span} under source `{}`: {} new, {} raised, {} unchanged, {} kept higher stored values; skipped {} requests with no tokens\n\
             cost ${:.2} in this export ({} requests priced at Cursor API rates)",
            self.events, self.rows, self.source_id, self.inserted, self.increased, self.unchanged, self.protected, self.empty, self.cost_usd, self.priced
        )?;
        if !self.unpriced_models.is_empty() {
            let models: Vec<&str> = self.unpriced_models.iter().map(String::as_str).collect();
            write!(f, "; no published rate, left at $0: {}", models.join(", "))?;
        }
        Ok(())
    }
}

pub async fn from_csv(pool: &MySqlPool, path: &Path, source_id: &str, timezone: Tz) -> Result<CursorImportSummary> {
    if !valid_identifier(source_id, 128) {
        bail!("source id `{source_id}` must be 1–128 characters of A–Z a–z 0–9 . _ : @ + -");
    }

    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let parsed = parse(&text, timezone).with_context(|| format!("parsing {}", path.display()))?;
    if parsed.rows.is_empty() {
        bail!("{} has no usage rows", path.display());
    }

    // One source per Cursor account: totals are summed across sources, so two accounts stack,
    // while re-importing one account under its own id only ever raises its days.
    let name = if source_id == DEFAULT_SOURCE_ID { SOURCE_NAME.to_owned() } else { format!("Cursor ({source_id})") };
    let source = Source { id: source_id.to_owned(), name, agent: AGENT.to_owned(), project: String::new() };
    let seen_at = parsed.latest.unwrap_or_else(Utc::now);
    let outcome = record_report(pool, &source, &parsed.rows, seen_at).await.context("recording Cursor usage")?;

    Ok(CursorImportSummary {
        source_id: source_id.to_owned(),
        events: parsed.events,
        empty: parsed.empty,
        first_day: parsed.rows.iter().map(|row| row.day).min(),
        last_day: parsed.rows.iter().map(|row| row.day).max(),
        rows: outcome.rows,
        inserted: outcome.inserted,
        increased: outcome.increased,
        unchanged: outcome.unchanged,
        protected: outcome.protected,
        priced: parsed.priced,
        cost_usd: parsed.rows.iter().map(|row| row.counts.cost_usd).sum(),
        unpriced_models: parsed.unpriced_models,
    })
}

/// Sums a Cursor export into per-day, per-model rows. Days are taken in `timezone`, the same zone
/// the dashboard uses for "today".
pub fn parse(text: &str, timezone: Tz) -> Result<Parsed> {
    let mut records = read_csv(text.strip_prefix('\u{feff}').unwrap_or(text))?.into_iter();
    let header = records.next().context("the file is empty")?;
    let column = |name: &str| -> Result<usize> {
        header
            .iter()
            .position(|field| field.trim().eq_ignore_ascii_case(name))
            .with_context(|| format!("missing column `{name}`; is this a Cursor usage export?"))
    };

    let date = column("Date")?;
    let model = column("Model")?;
    let cache_write = column("Input (w/ Cache Write)")?;
    let input = column("Input (w/o Cache Write)")?;
    let cache_read = column("Cache Read")?;
    let output = column("Output Tokens")?;
    let total = column("Total Tokens")?;
    let cost = column("Cost").ok();
    let kind = column("Kind").ok();

    let mut sums: BTreeMap<(NaiveDate, String), Counts> = BTreeMap::new();
    let mut parsed = Parsed::default();

    for (index, record) in records.enumerate() {
        let line = index + 2;
        if record.iter().all(|field| field.trim().is_empty()) {
            continue;
        }
        let field = |at: usize| record.get(at).map_or("", |value| value.trim());
        let tokens = |at: usize, name: &str| -> Result<u64> {
            let value = field(at).replace(',', "");
            if value.is_empty() || value == "-" {
                return Ok(0);
            }
            value.parse().with_context(|| format!("line {line}: `{name}` is not a whole number: {:?}", field(at)))
        };

        let at = DateTime::parse_from_rfc3339(field(date))
            .with_context(|| format!("line {line}: `Date` is not an ISO 8601 timestamp: {:?}", field(date)))?
            .with_timezone(&Utc);

        let mut counts = Counts {
            input_tokens: tokens(input, "Input (w/o Cache Write)")?,
            output_tokens: tokens(output, "Output Tokens")?,
            cache_read_tokens: tokens(cache_read, "Cache Read")?,
            cache_creation_tokens: tokens(cache_write, "Input (w/ Cache Write)")?,
            reasoning_tokens: 0,
            total_tokens: tokens(total, "Total Tokens")?,
            cost_usd: cost.map_or(0.0, |at| dollars(field(at))),
        };
        if counts.total_tokens == 0 {
            counts.total_tokens = counts.input_tokens + counts.output_tokens + counts.cache_read_tokens + counts.cache_creation_tokens;
        }
        if counts.total_tokens == 0 && counts.cost_usd == 0.0 {
            parsed.empty += 1;
            continue;
        }

        let name = crate::ingest::clean_label(field(model), MAX_MODEL_LENGTH);
        let name = if name.is_empty() { "unknown".to_owned() } else { name };

        // "Errored, No Charge" / "Aborted, Not Charged": the tokens happened, nobody paid for them.
        let charged = !kind.is_some_and(|at| not_charged(field(at)));

        if charged && counts.cost_usd == 0.0 && counts.total_tokens > 0 {
            match rate(&name) {
                Some(rate) => {
                    counts.cost_usd = rate.cost(&counts);
                    parsed.priced += 1;
                }
                None => {
                    parsed.unpriced_models.insert(name.clone());
                }
            }
        }

        sums.entry((at.with_timezone(&timezone).date_naive(), name)).or_default().add(&counts);
        parsed.events += 1;
        parsed.latest = parsed.latest.max(Some(at));
    }

    parsed.rows = sums.into_iter().map(|((day, model), counts)| ReportRow { day, model, counts }).collect();
    Ok(parsed)
}

fn not_charged(kind: &str) -> bool {
    let kind = kind.to_ascii_lowercase();
    kind.contains("no charge") || kind.contains("not charged") || kind.starts_with("errored") || kind.starts_with("aborted")
}

/// `$1.23`, `1.23` → 1.23; `Included`, `Free`, `-` and anything else non-numeric → 0.
fn dollars(value: &str) -> f64 {
    value.trim().trim_start_matches('$').replace(',', "").parse::<f64>().ok().filter(|cost| cost.is_finite() && *cost > 0.0).unwrap_or(0.0)
}

/// RFC 4180: comma-separated, fields optionally double-quoted with `""` as an escaped quote,
/// quoted fields may span lines, LF or CRLF line endings.
pub(crate) fn read_csv(text: &str) -> Result<Vec<Vec<String>>> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => record.push(std::mem::take(&mut field)),
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            _ => field.push(c),
        }
    }
    if quoted {
        bail!("unterminated quoted field at the end of the file");
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "Date,Cloud Agent ID,Automation ID,Kind,Model,Max Mode,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost\n";

    fn day(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }

    fn close(actual: f64, expected: f64) -> bool {
        (actual - expected).abs() < 1e-12
    }

    #[test]
    fn sums_requests_per_day_and_model() {
        // `auto` has no rate, so costs here are only what the export states.
        let csv = format!(
            "{HEADER}\
\"2026-11-09T10:12:02.254Z\",\"\",\"\",\"Included\",\"auto\",\"No\",\"0\",\"1091\",\"120064\",\"274\",\"121429\",\"Included\"\r
\"2026-11-09T09:00:00.000Z\",\"\",\"\",\"Included\",\"auto\",\"No\",\"973\",\"2\",\"49270\",\"441\",\"50686\",\"Included\"\r
\"2026-11-08T23:59:59.000Z\",\"\",\"\",\"Included\",\"auto\",\"No\",\"0\",\"10\",\"0\",\"5\",\"15\",\"$0.25\"\r
\"2026-11-09T09:45:39.842Z\",\"\",\"\",\"Included\",\"grok-bot-default\",\"No\",\"\",\"\",\"\",\"\",\"\",\"Free\"\r
\"2026-11-09T08:00:00.000Z\",\"\",\"\",\"Included\",\"other-model\",\"No\",\"0\",\"100\",\"0\",\"50\",\"150\",\"Included\"\r
"
        );
        let parsed = parse(&csv, Tz::UTC).unwrap();

        assert_eq!(parsed.events, 4);
        assert_eq!(parsed.empty, 1);
        assert_eq!(parsed.priced, 0);
        assert_eq!(parsed.unpriced_models, BTreeSet::from(["auto".to_owned(), "other-model".to_owned()]));
        assert_eq!(parsed.latest, Some("2026-11-09T10:12:02.254Z".parse().unwrap()));
        assert_eq!(
            parsed.rows,
            vec![
                ReportRow {
                    day: day("2026-11-08"),
                    model: "auto".into(),
                    counts: Counts { input_tokens: 10, output_tokens: 5, total_tokens: 15, cost_usd: 0.25, ..Counts::default() },
                },
                ReportRow {
                    day: day("2026-11-09"),
                    model: "auto".into(),
                    counts: Counts {
                        input_tokens: 1093,
                        output_tokens: 715,
                        cache_read_tokens: 169_334,
                        cache_creation_tokens: 973,
                        reasoning_tokens: 0,
                        total_tokens: 172_115,
                        cost_usd: 0.0,
                    },
                },
                ReportRow {
                    day: day("2026-11-09"),
                    model: "other-model".into(),
                    counts: Counts { input_tokens: 100, output_tokens: 50, total_tokens: 150, ..Counts::default() },
                },
            ]
        );
    }

    #[test]
    fn plan_usage_is_priced_per_request_at_api_rates() {
        // 1M input + 1M cache write + 1M cache read + 1M output on Opus 4.8: 5 + 6.25 + 0.5 + 25.
        let csv = format!(
            "{HEADER}\
2026-11-09T10:00:00Z,,,Included,claude-opus-4-8-thinking-high,No,1000000,1000000,1000000,1000000,4000000,Included\n\
2026-11-09T10:00:01Z,,,Included,grok-4.7-medium,No,0,100000,100000,1000,201000,Included\n\
2026-11-09T10:00:02Z,,,Included,grok-4.7-medium,No,0,100000,200000,1000,301000,Included\n\
2026-11-09T10:00:03Z,,,Included,kimi-k3-max,No,0,0,0,1000000,1000000,$1.00\n"
        );
        let parsed = parse(&csv, Tz::UTC).unwrap();
        let cost = |model: &str| parsed.rows.iter().find(|row| row.model == model).unwrap().counts.cost_usd;

        assert_eq!(parsed.priced, 3);
        assert!(parsed.unpriced_models.is_empty());
        assert!(close(cost("claude-opus-4-8-thinking-high"), 36.75));
        // Under 256k prompt: 0.2 + 0.05 + 0.006. Over it, the whole request at 2x: 2 × (0.2 + 0.1 + 0.006).
        assert!(close(cost("grok-4.7-medium"), 0.256 + 0.612), "{}", cost("grok-4.7-medium"));
        // A stated dollar amount wins over the rate card.
        assert!(close(cost("kimi-k3-max"), 1.0));
    }

    #[test]
    fn uncharged_requests_keep_tokens_but_are_not_priced() {
        let csv = format!(
            "{HEADER}\
2026-01-01T10:00:00Z,,,\"Errored, No Charge\",gpt-5,No,0,1000,0,100,1100,-\n\
2026-01-01T10:00:01Z,,,\"Aborted, Not Charged\",gpt-5,No,0,1000,0,100,1100,0\n"
        );
        let parsed = parse(&csv, Tz::UTC).unwrap();
        assert_eq!(parsed.priced, 0);
        assert!(parsed.unpriced_models.is_empty());
        assert_eq!(parsed.rows[0].counts.total_tokens, 2200);
        assert_eq!(parsed.rows[0].counts.cost_usd, 0.0);
    }

    #[test]
    fn model_names_map_to_rates() {
        let grok_47 = rate("grok-4.7").unwrap();
        assert_eq!(rate("grok-4.7-medium"), Some(grok_47));
        assert_eq!(rate("grok-bot-default"), Some(grok_47));
        assert_eq!(rate("grok-bot-automation"), Some(grok_47));
        assert_eq!(rate("cursor-grok-4.5-high").unwrap().output, 6.0);
        assert_eq!(rate("grok-4.5-fast").unwrap().output, 18.0);
        assert_eq!(rate("grok-4.7-high-fast").unwrap().input, 4.0);
        assert_eq!(rate("claude-sonnet-5-thinking-high").unwrap().cache_write, 2.5);
        assert_eq!(rate("kimi-k3-max").unwrap().cache_read, 0.30);
        assert_eq!(rate("composer-2.5").unwrap().input, 0.50);
        assert_eq!(rate("claude-4.5-opus-high-thinking").unwrap().cache_write, 6.25);
        assert_eq!(rate("claude-4.6-opus-high-thinking").unwrap().output, 25.0);
        assert_eq!(rate("claude-4-sonnet-thinking").unwrap().input, 3.0);
        assert_eq!(rate("claude-4.5-sonnet-thinking").unwrap().cache_read, 0.30);
        assert_eq!(rate("gpt-5").unwrap().cache_read, 0.125);
        assert_eq!(rate("gpt-5-codex").unwrap().output, 10.0);
        assert_eq!(rate("gpt-5.3-codex").unwrap().input, 1.75);
        assert_eq!(rate("grok-code-fast-1"), None);
        assert_eq!(rate("composer-1"), None);
        // Different models that merely share a prefix are not priced as each other.
        assert_eq!(rate("claude-sonnet-5-5-thinking-high"), None);
        assert_eq!(rate("claude-opus-4-8-fast"), None);
        assert_eq!(rate("auto"), None);
    }

    #[test]
    fn days_follow_the_dashboard_timezone() {
        let csv = format!("{HEADER}2026-11-08T23:30:00.000Z,,,Included,auto,No,0,1,0,1,2,Included\n");
        let parsed = parse(&csv, "Asia/Kolkata".parse().unwrap()).unwrap();
        assert_eq!(parsed.rows[0].day, day("2026-11-09"));
    }

    #[test]
    fn missing_total_is_the_sum_of_the_parts() {
        let csv = format!("\u{feff}{HEADER}2026-11-08T12:00:00Z,,,Included,auto,No,\"1,000\",2,3,4,,Included");
        let parsed = parse(&csv, Tz::UTC).unwrap();
        assert_eq!(parsed.rows[0].counts.total_tokens, 1009);
    }

    #[test]
    fn rejects_files_that_are_not_cursor_exports() {
        let error = parse("date,tokens\n2026-11-08,5\n", Tz::UTC).unwrap_err();
        assert!(error.to_string().contains("missing column"), "{error}");
    }

    #[test]
    fn reports_the_line_of_a_bad_value() {
        let csv = format!("{HEADER}2026-11-08T12:00:00Z,,,Included,auto,No,0,x,0,0,0,Included\n");
        let error = parse(&csv, Tz::UTC).unwrap_err();
        assert!(error.to_string().contains("line 2"), "{error}");
    }

    #[test]
    fn csv_quoting() {
        let records = read_csv("a,\"b,\"\"c\"\"\",\"d\ne\"\r\nf,,\n").unwrap();
        assert_eq!(records, vec![vec!["a", "b,\"c\"", "d\ne"], vec!["f", "", ""]]);
        assert!(read_csv("\"open").is_err());
    }

    #[test]
    fn dollar_amounts() {
        assert_eq!(dollars("$1,234.50"), 1234.5);
        assert_eq!(dollars("0.04"), 0.04);
        assert_eq!(dollars("Included"), 0.0);
        assert_eq!(dollars("Free"), 0.0);
        assert_eq!(dollars("-"), 0.0);
    }
}
