//! Codex: `$CODEX_HOME/{sessions,archived_sessions}/**/*.jsonl` rollouts.
//!
//! Current Codex writes a `token_usage_record` per model response. Older rollouts only have
//! `token_count` events carrying the running total and the last turn's usage. Codex counts
//! cached reads inside `input_tokens`; here input is uncached and cached reads are separate.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rayon::prelude::*;
use serde_json::{json, Map, Value};

use super::{files, lines, user_home};
use crate::event::{str_at, u64_at, Event};

pub fn root() -> PathBuf {
    std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| user_home().join(".codex"))
}

pub fn load() -> Vec<Event> {
    let root = root();
    let paths: Vec<PathBuf> = ["sessions", "archived_sessions"]
        .iter()
        .flat_map(|dir| files(&root.join(dir), |name| name.ends_with(".jsonl")))
        .collect();

    let per_file: Vec<Vec<Event>> = paths.par_iter().map(|path| read_file(path)).collect();
    let mut seen = HashSet::new();
    per_file.into_iter().flatten().filter(|event| seen.insert(event.id.clone())).collect()
}

/// What the rollout says about the session, carried onto each event.
#[derive(Default)]
struct Context {
    session_id: String,
    cwd: String,
    model: String,
    provider: String,
    meta: Map<String, Value>,
}

fn read_file(path: &Path) -> Vec<Event> {
    let all: Vec<String> = lines(path).collect();
    let records = all.iter().any(|line| line.contains("\"token_usage_record\""));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_owned();

    let mut context = Context::default();
    let mut previous_total: Option<Value> = None;
    let mut out = Vec::new();

    for (index, line) in all.iter().enumerate() {
        let interesting = line.contains("\"session_meta\"")
            || line.contains("\"turn_context\"")
            || (records && line.contains("\"token_usage_record\""))
            || (!records && line.contains("\"token_count\""));
        if !interesting {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(line) else { continue };
        let payload = entry.get("payload").cloned().unwrap_or(Value::Null);
        let timestamp = entry.get("timestamp").and_then(Value::as_str).and_then(|t| t.parse::<DateTime<Utc>>().ok());

        match str_at(&entry, "type") {
            "session_meta" => {
                context.session_id = str_at(&payload, "id").to_owned();
                context.cwd = str_at(&payload, "cwd").to_owned();
                context.provider = str_at(&payload, "model_provider").to_owned();
                if let Some(model) = payload.get("model").and_then(Value::as_str) {
                    context.model = model.to_owned();
                }
                for key in ["cli_version", "originator", "thread_source", "agent_nickname", "agent_path", "parent_thread_id"] {
                    if let Some(value) = payload.get(key).filter(|v| !v.is_null()) {
                        context.meta.insert(key.to_owned(), value.clone());
                    }
                }
            }
            "turn_context" => {
                if let Some(model) = payload.get("model").and_then(Value::as_str) {
                    context.model = model.to_owned();
                }
                if let Some(cwd) = payload.get("cwd").and_then(Value::as_str) {
                    context.cwd = cwd.to_owned();
                }
                for key in ["effort", "summary", "approval_policy", "collaboration_mode"] {
                    if let Some(value) = payload.get(key).filter(|v| v.is_string()) {
                        context.meta.insert(key.to_owned(), value.clone());
                    }
                }
            }
            "token_usage_record" => {
                let Some(usage) = payload.get("usage") else { continue };
                let id = match str_at(&payload, "response_id") {
                    "" => format!("{stem}:{index}"),
                    response => response.to_owned(),
                };
                let mut details = context.meta.clone();
                for key in ["thread_id", "turn_id", "root_turn_id", "response_id"] {
                    if let Some(value) = payload.get(key) {
                        details.insert(key.to_owned(), value.clone());
                    }
                }
                details.insert("usage".to_owned(), usage.clone());
                if let Some(timestamp) = timestamp {
                    out.push(event(&context, id, timestamp, usage, details));
                }
            }
            "event_msg" if str_at(&payload, "type") == "token_count" => {
                let Some(info) = payload.get("info").filter(|i| i.is_object()) else { continue };
                let Some(total) = info.get("total_token_usage").cloned() else { continue };
                // The same total is re-sent with rate limit updates; only a change is usage.
                if previous_total.as_ref() == Some(&total) {
                    continue;
                }
                let usage = match info.get("last_token_usage").filter(|u| u.is_object()) {
                    Some(last) => last.clone(),
                    None => delta(&total, previous_total.as_ref()),
                };
                previous_total = Some(total);
                let mut details = context.meta.clone();
                details.insert("usage".to_owned(), usage.clone());
                let session = if context.session_id.is_empty() { stem.clone() } else { context.session_id.clone() };
                if let Some(timestamp) = timestamp {
                    out.push(event(&context, format!("{session}:{index}"), timestamp, &usage, details));
                }
            }
            _ => {}
        }
    }
    out
}

fn delta(total: &Value, previous: Option<&Value>) -> Value {
    let mut out = Map::new();
    if let Some(object) = total.as_object() {
        for key in object.keys() {
            let before = previous.map_or(0, |p| u64_at(p, key));
            out.insert(key.clone(), json!(u64_at(total, key).saturating_sub(before)));
        }
    }
    Value::Object(out)
}

/// Logged model names that ccusage reports under another name. Days already stored from ccusage
/// use these names; a different one would add a second row for the same usage.
const RENAMES: &[(&str, &str)] = &[("codex-auto-review", "gpt-5.6-luna")];

fn event(context: &Context, id: String, timestamp: DateTime<Utc>, usage: &Value, mut details: Map<String, Value>) -> Event {
    let logged = if context.model.is_empty() { "unknown" } else { context.model.as_str() };
    let model = match RENAMES.iter().find(|(from, _)| *from == logged) {
        Some((_, to)) => {
            details.insert("logged_model".to_owned(), json!(logged));
            (*to).to_owned()
        }
        None => logged.to_owned(),
    };

    let input = u64_at(usage, "input_tokens");
    let cached = u64_at(usage, "cached_input_tokens").min(input);
    let output = u64_at(usage, "output_tokens");
    let cache_write = u64_at(usage, "cache_write_input_tokens");

    let mut event = Event {
        agent: "codex",
        id,
        timestamp,
        model,
        provider: if context.provider.is_empty() { "openai".to_owned() } else { context.provider.clone() },
        session_id: context.session_id.clone(),
        project: context.cwd.clone(),
        input_tokens: input - cached,
        output_tokens: output,
        cache_read_tokens: cached,
        cache_creation_tokens: cache_write,
        cache_creation_5m_tokens: cache_write,
        reasoning_tokens: u64_at(usage, "reasoning_output_tokens"),
        total_tokens: u64_at(usage, "total_tokens"),
        details: Value::Object(details),
        ..Event::default()
    };
    event.fill_total();
    event
}
