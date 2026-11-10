//! Daily totals per agent and model, printed as a table or as ccusage-style JSON.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use chrono_tz::Tz;
use comfy_table::presets::UTF8_FULL_CONDENSED;
use comfy_table::{Cell, CellAlignment, Table};
use serde_json::{json, Value};

use crate::event::Event;

#[derive(Debug, Default, Clone, Copy)]
pub struct Counts {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_creation: u64,
    pub reasoning: u64,
    pub total: u64,
    pub cost: f64,
}

impl Counts {
    fn add(&mut self, event: &Event) {
        self.input += event.input_tokens;
        self.output += event.output_tokens;
        self.cache_read += event.cache_read_tokens;
        self.cache_creation += event.cache_creation_tokens;
        self.reasoning += event.reasoning_tokens;
        self.total += event.total_tokens;
        self.cost += event.cost_usd;
    }

    fn merge(&mut self, other: &Counts) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_creation += other.cache_creation;
        self.reasoning += other.reasoning;
        self.total += other.total;
        self.cost += other.cost;
    }

    fn json(&self) -> serde_json::Map<String, Value> {
        let value = json!({
            "inputTokens": self.input,
            "outputTokens": self.output,
            "cacheCreationTokens": self.cache_creation,
            "cacheReadTokens": self.cache_read,
            "reasoningTokens": self.reasoning,
            "totalTokens": self.total,
        });
        value.as_object().cloned().unwrap_or_default()
    }
}

/// day → agent → model → counts.
pub type Daily = BTreeMap<NaiveDate, BTreeMap<&'static str, BTreeMap<String, Counts>>>;

pub fn daily(events: &[Event], timezone: Tz) -> Daily {
    let mut out = Daily::new();
    for event in events {
        let day = event.timestamp.with_timezone(&timezone).date_naive();
        out.entry(day).or_default().entry(event.agent).or_default().entry(event.model.clone()).or_default().add(event);
    }
    out
}

/// One agent's days in the shape `/api/ingest` takes (ccusage's `modelBreakdowns`).
pub fn ingest_payload(daily: &Daily, agent: &str) -> Value {
    let days: Vec<Value> = daily
        .iter()
        .filter_map(|(day, agents)| {
            let models = agents.get(agent)?;
            let breakdowns: Vec<Value> = models
                .iter()
                .map(|(model, counts)| {
                    let mut row = counts.json();
                    row.insert("modelName".to_owned(), json!(model));
                    row.insert("cost".to_owned(), json!(counts.cost));
                    Value::Object(row)
                })
                .collect();
            Some(json!({ "date": day.to_string(), "modelBreakdowns": breakdowns }))
        })
        .collect();
    json!({ "daily": days })
}

/// `--json`: one entry per day and agent, with per-model breakdowns, plus totals.
pub fn to_json(daily: &Daily) -> Value {
    let mut totals = Counts::default();
    let mut days = Vec::new();
    for (day, agents) in daily {
        for (agent, models) in agents {
            let mut sum = Counts::default();
            let breakdowns: Vec<Value> = models
                .iter()
                .map(|(model, counts)| {
                    sum.merge(counts);
                    let mut row = counts.json();
                    row.insert("modelName".to_owned(), json!(model));
                    row.insert("cost".to_owned(), json!(counts.cost));
                    Value::Object(row)
                })
                .collect();
            totals.merge(&sum);
            let mut row = sum.json();
            row.insert("date".to_owned(), json!(day.to_string()));
            row.insert("agent".to_owned(), json!(agent));
            row.insert("totalCost".to_owned(), json!(sum.cost));
            row.insert("modelsUsed".to_owned(), json!(models.keys().collect::<Vec<_>>()));
            row.insert("modelBreakdowns".to_owned(), Value::Array(breakdowns));
            days.push(Value::Object(row));
        }
    }
    let mut total = totals.json();
    total.insert("totalCost".to_owned(), json!(totals.cost));
    json!({ "daily": days, "totals": total })
}

pub fn print_table(daily: &Daily, breakdown: bool) {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(["Date", "Agent", "Models", "Input", "Output", "Cache Create", "Cache Read", "Total Tokens", "Cost (USD)"]);

    let mut totals = Counts::default();
    for (day, agents) in daily {
        for (agent, models) in agents {
            let mut sum = Counts::default();
            models.values().for_each(|counts| sum.merge(counts));
            totals.merge(&sum);
            let split = breakdown && models.len() > 1;
            let names = if split { String::new() } else { models.keys().map(String::as_str).collect::<Vec<_>>().join("\n") };
            table.add_row(row(&day.to_string(), agent, &names, &sum));
            if split {
                for (model, counts) in models {
                    table.add_row(row("", "", &format!("  └ {model}"), counts));
                }
            }
        }
    }
    table.add_row(row("Total", "", "", &totals));

    for index in 3..9 {
        if let Some(column) = table.column_mut(index) {
            column.set_cell_alignment(CellAlignment::Right);
        }
    }
    println!("{table}");
}

fn row(date: &str, agent: &str, models: &str, counts: &Counts) -> Vec<Cell> {
    vec![
        Cell::new(date),
        Cell::new(agent),
        Cell::new(models),
        Cell::new(thousands(counts.input)),
        Cell::new(thousands(counts.output)),
        Cell::new(thousands(counts.cache_creation)),
        Cell::new(thousands(counts.cache_read)),
        Cell::new(thousands(counts.total)),
        Cell::new(dollars(counts.cost)),
    ]
}

fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn dollars(value: f64) -> String {
    let cents = (value * 100.0).round() as u64;
    format!("${}.{:02}", thousands(cents / 100), cents % 100)
}

#[cfg(test)]
mod tests {
    use super::{dollars, thousands};

    #[test]
    fn formats_dollars() {
        assert_eq!(dollars(1835.7928), "$1,835.79");
        assert_eq!(dollars(0.004), "$0.00");
    }

    #[test]
    fn groups_digits() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(1234), "1,234");
        assert_eq!(thousands(1234567), "1,234,567");
    }
}
