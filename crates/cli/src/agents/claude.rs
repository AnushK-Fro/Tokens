//! Claude Code: `<config>/projects/**/*.jsonl`, one line per message block.
//!
//! A message is logged once per content block, all with the same `message.id` + `requestId`;
//! early lines can carry a partial output count. Like ccusage, each message counts once, as its
//! line with the largest output (the first one on a tie), timestamp included.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use rayon::prelude::*;
use serde_json::{json, Value};

use super::{files, lines, user_home};
use crate::event::{scalars, str_at, u64_at, Event};

pub fn roots() -> Vec<PathBuf> {
    if let Ok(configured) = std::env::var("CLAUDE_CONFIG_DIR") {
        let roots: Vec<PathBuf> = configured.split(',').map(str::trim).filter(|p| !p.is_empty()).map(PathBuf::from).collect();
        if !roots.is_empty() {
            return roots;
        }
    }
    let home = user_home();
    vec![home.join(".claude"), home.join(".config").join("claude")]
}

pub fn load() -> Vec<Event> {
    let mut paths: Vec<PathBuf> = roots().iter().flat_map(|root| files(&root.join("projects"), |name| name.ends_with(".jsonl"))).collect();
    paths.dedup();

    let per_file: Vec<Vec<(String, Event)>> = paths.par_iter().map(|path| read_file(path)).collect();

    let mut order = Vec::new();
    let mut merged: HashMap<String, Event> = HashMap::new();
    for (key, event) in per_file.into_iter().flatten() {
        match merged.get_mut(&key) {
            None => {
                order.push(key.clone());
                merged.insert(key, event);
            }
            Some(kept) if event.output_tokens > kept.output_tokens => *kept = event,
            Some(_) => {}
        }
    }
    order.into_iter().filter_map(|key| merged.remove(&key)).collect()
}

fn read_file(path: &PathBuf) -> Vec<(String, Event)> {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_owned();
    let mut out = Vec::new();

    for (index, line) in lines(path).enumerate() {
        if !line.contains("\"usage\"") {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(message) = entry.get("message").filter(|m| m.is_object()) else { continue };
        let Some(usage) = message.get("usage").filter(|u| u.is_object()) else { continue };
        let model = str_at(message, "model");
        if model.is_empty() || model == "<synthetic>" {
            continue;
        }
        let Some(timestamp) = entry.get("timestamp").and_then(Value::as_str).and_then(|t| t.parse::<DateTime<Utc>>().ok()) else {
            continue;
        };

        let message_id = str_at(message, "id");
        let request_id = str_at(&entry, "requestId");
        let key = if !message_id.is_empty() && !request_id.is_empty() { format!("{message_id}:{request_id}") } else { format!("{stem}:{index}") };

        let cache_creation = u64_at(usage, "cache_creation_input_tokens");
        let split = usage.get("cache_creation");
        let one_hour = split.map_or(0, |s| u64_at(s, "ephemeral_1h_input_tokens")).min(cache_creation);

        let mut details = scalars(&entry, &[]);
        for (key, value) in scalars(message, &["type", "role"]) {
            details.insert(format!("message_{key}"), value);
        }
        details.insert("usage".to_owned(), usage.clone());

        let mut event = Event {
            agent: "claude",
            id: key.clone(),
            timestamp,
            model: model.to_owned(),
            provider: "anthropic".to_owned(),
            session_id: str_at(&entry, "sessionId").to_owned(),
            project: str_at(&entry, "cwd").to_owned(),
            input_tokens: u64_at(usage, "input_tokens"),
            output_tokens: u64_at(usage, "output_tokens"),
            cache_read_tokens: u64_at(usage, "cache_read_input_tokens"),
            cache_creation_tokens: cache_creation,
            cache_creation_5m_tokens: cache_creation - one_hour,
            cache_creation_1h_tokens: one_hour,
            reasoning_tokens: usage.get("output_tokens_details").map_or(0, |d| u64_at(d, "thinking_tokens")),
            logged_cost: entry.get("costUSD").and_then(Value::as_f64),
            details: json!(details),
            ..Event::default()
        };
        event.fill_total();
        out.push((key, event));
    }
    out
}
