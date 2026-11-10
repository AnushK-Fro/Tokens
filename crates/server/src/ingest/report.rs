//! Reads a `ccusage … daily --json` report into per-day, per-model counts.
//!
//! Handles the shapes the ccusage family emits:
//! - Claude Code / OpenCode: `daily[].modelBreakdowns[]` with `modelName` and a per-model `cost`.
//! - Codex: `daily[].models{name: usage}` with `cachedInputTokens` and only a day-level cost,
//!   which is split across models by token share.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde_json::{Map, Value};

use crate::usage::Counts;

pub const MAX_DAYS: usize = 5000;
const MAX_MODEL_LENGTH: usize = 160;

#[derive(Debug, Clone, PartialEq)]
pub struct ReportRow {
    pub day: NaiveDate,
    pub model: String,
    pub counts: Counts,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReportError {
    NotADailyReport,
    TooManyDays,
    /// ccusage v20+ `daily` without an agent subcommand: every agent in one report.
    MixedAgents,
}

impl std::fmt::Display for ReportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotADailyReport => f.write_str("Expected a ccusage daily JSON report"),
            Self::TooManyDays => write!(f, "Too many daily records (limit {MAX_DAYS})"),
            Self::MixedAgents => f.write_str(
                "This report mixes every agent (\"agent\": \"all\"). Send one per agent, e.g. `ccusage claude daily --json`",
            ),
        }
    }
}

pub fn parse(payload: &Value) -> Result<Vec<ReportRow>, ReportError> {
    let days = payload.get("daily").and_then(Value::as_array).ok_or(ReportError::NotADailyReport)?;

    if days.len() > MAX_DAYS {
        return Err(ReportError::TooManyDays);
    }

    // Its model rows carry no agent, so they could only be filed under the wrong one.
    if days.iter().any(|entry| entry.get("agent").and_then(Value::as_str) == Some("all")) {
        return Err(ReportError::MixedAgents);
    }

    // Keyed so a model listed twice for one day is summed rather than overwriting itself.
    let mut rows: BTreeMap<(NaiveDate, String), Counts> = BTreeMap::new();

    for entry in days.iter().filter_map(Value::as_object) {
        let Some(day) = entry.get("date").or_else(|| entry.get("period")).and_then(Value::as_str).and_then(parse_day) else {
            continue;
        };

        for (model, counts) in breakdowns(entry) {
            rows.entry((day, model)).or_default().add(&counts);
        }
    }

    Ok(rows.into_iter().map(|((day, model), counts)| ReportRow { day, model, counts }).collect())
}

fn breakdowns(entry: &Map<String, Value>) -> Vec<(String, Counts)> {
    if let Some(list) = entry.get("modelBreakdowns").and_then(Value::as_array) {
        return list
            .iter()
            .filter_map(Value::as_object)
            .map(|usage| (model_name(usage.get("modelName")), counts(usage, None)))
            .collect();
    }

    let Some(models) = entry.get("models").and_then(Value::as_object) else {
        return Vec::new();
    };

    let day_total = number(entry.get("totalTokens")).max(1) as f64;
    let day_cost = cost(entry).unwrap_or(0.0);

    models
        .iter()
        .filter_map(|(name, usage)| usage.as_object().map(|usage| (name, usage)))
        .map(|(name, usage)| {
            let share = number(usage.get("totalTokens")) as f64 / day_total;
            (model_name(Some(&Value::String(name.clone()))), counts(usage, Some(day_cost * share)))
        })
        .collect()
}

fn counts(usage: &Map<String, Value>, fallback_cost: Option<f64>) -> Counts {
    let input_tokens = number(usage.get("inputTokens"));
    let output_tokens = number(usage.get("outputTokens"));
    let cache_read_tokens = number(usage.get("cacheReadTokens").or_else(|| usage.get("cachedInputTokens")));
    let cache_creation_tokens = number(usage.get("cacheCreationTokens"));
    let reasoning_tokens = number(usage.get("reasoningOutputTokens").or_else(|| usage.get("reasoningTokens")));

    let mut total_tokens = number(usage.get("totalTokens"));
    if total_tokens == 0 {
        total_tokens = input_tokens + output_tokens + cache_read_tokens + cache_creation_tokens;
    }

    Counts {
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
        reasoning_tokens,
        total_tokens,
        cost_usd: cost(usage).or(fallback_cost).unwrap_or(0.0),
    }
}

fn cost(object: &Map<String, Value>) -> Option<f64> {
    ["cost", "costUSD", "totalCost"]
        .iter()
        .find_map(|key| object.get(*key).and_then(as_f64))
        .map(|value| if value.is_finite() { value.max(0.0) } else { 0.0 })
}

fn model_name(value: Option<&Value>) -> String {
    let name: String = value.and_then(Value::as_str).unwrap_or("").chars().filter(|c| !c.is_control()).collect();
    let name = name.trim();

    if name.is_empty() || name.len() > MAX_MODEL_LENGTH {
        "unknown".to_owned()
    } else {
        name.to_owned()
    }
}

fn parse_day(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .filter(|day| day.format("%Y-%m-%d").to_string() == value)
        .or_else(|| NaiveDate::parse_from_str(value, "%b %d, %Y").ok())
}

fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// Non-negative whole number from a JSON number or numeric string; anything else is zero.
fn number(value: Option<&Value>) -> u64 {
    match value.and_then(as_f64) {
        Some(n) if n.is_finite() && n > 0.0 => n as u64,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn claude_breakdowns() {
        let rows = parse(&json!({
            "daily": [{
                "date": "2026-11-09",
                "modelBreakdowns": [
                    { "modelName": "claude-opus-5-5", "inputTokens": 10, "outputTokens": 20, "cacheCreationTokens": 30, "cacheReadTokens": 40, "cost": 1.25 },
                    { "modelName": "claude-haiku-5-5", "inputTokens": 1, "outputTokens": 2, "cost": "0.5" }
                ]
            }]
        }))
        .unwrap();

        assert_eq!(rows.len(), 2);
        let opus = rows.iter().find(|row| row.model == "claude-opus-5-5").unwrap();
        assert_eq!(opus.counts.total_tokens, 100);
        assert_eq!(opus.counts.cache_read_tokens, 40);
        assert!((opus.counts.cost_usd - 1.25).abs() < 1e-12);
        let haiku = rows.iter().find(|row| row.model == "claude-haiku-5-5").unwrap();
        assert!((haiku.counts.cost_usd - 0.5).abs() < 1e-12);
    }

    #[test]
    fn codex_models_split_day_cost_and_read_cached_input() {
        let rows = parse(&json!({
            "daily": [{
                "date": "2026-11-09",
                "totalTokens": 400,
                "costUSD": 4.0,
                "models": {
                    "gpt-5-codex": { "inputTokens": 250, "cachedInputTokens": 200, "outputTokens": 50, "reasoningOutputTokens": 10, "totalTokens": 300 },
                    "gpt-5": { "inputTokens": 80, "outputTokens": 20, "totalTokens": 100 }
                }
            }]
        }))
        .unwrap();

        let codex = rows.iter().find(|row| row.model == "gpt-5-codex").unwrap();
        assert_eq!(codex.counts.cache_read_tokens, 200);
        assert_eq!(codex.counts.reasoning_tokens, 10);
        assert!((codex.counts.cost_usd - 3.0).abs() < 1e-12);
    }

    #[test]
    fn bad_days_and_negative_numbers_are_dropped() {
        let rows = parse(&json!({
            "daily": [
                { "date": "2026-02-30", "modelBreakdowns": [{ "modelName": "x", "inputTokens": 5 }] },
                { "date": "2026-11-09", "modelBreakdowns": [{ "modelName": "x", "inputTokens": -5, "outputTokens": 3 }] }
            ]
        }))
        .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].counts.input_tokens, 0);
        assert_eq!(rows[0].counts.total_tokens, 3);
    }

    #[test]
    fn rejects_other_shapes() {
        assert_eq!(parse(&json!({ "monthly": [] })), Err(ReportError::NotADailyReport));
        assert_eq!(parse(&json!({ "daily": [{ "agent": "all", "period": "2026-11-09" }] })), Err(ReportError::MixedAgents));
    }

    #[test]
    fn current_codex_report_with_separate_cache() {
        let rows = parse(&json!({
            "daily": [{
                "date": "2026-11-07", "costUSD": 10.0, "totalTokens": 1000,
                "models": { "gpt-x": { "inputTokens": 100, "cacheReadTokens": 800, "outputTokens": 100, "totalTokens": 1000, "isFallback": true } }
            }]
        }))
        .unwrap();
        assert_eq!(rows[0].counts.cache_read_tokens, 800);
        assert!((rows[0].counts.cost_usd - 10.0).abs() < 1e-12);
    }
}
