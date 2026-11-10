//! Kimi Code: `~/.kimi-code/sessions/<workspace>/<session>/**/wire.jsonl`, one `usage.record`
//! per model call (`time` in epoch ms, model like `kimi-code/k3`). The older kimi-cli layout
//! (`~/.kimi/sessions`, `StatusUpdate` messages with `token_usage`) is read too.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeZone, Utc};
use rayon::prelude::*;
use serde_json::{json, Map, Value};

use super::{files, lines, user_home};
use crate::event::{str_at, u64_at, Event};

pub fn roots() -> Vec<PathBuf> {
    let home = user_home();
    vec![home.join(".kimi-code"), home.join(".kimi")]
}

pub fn load() -> Vec<Event> {
    roots()
        .iter()
        .flat_map(|root| {
            let sessions = root.join("sessions");
            let workspaces = workspace_roots(root);
            let paths = files(&sessions, |name| name == "wire.jsonl");
            paths.par_iter().map(|path| read_file(&sessions, path, &workspaces)).collect::<Vec<_>>()
        })
        .flatten()
        .collect()
}

/// `workspaces.json` maps workspace ids (`wd_…`) to their directories.
fn workspace_roots(root: &Path) -> HashMap<String, String> {
    let Ok(text) = std::fs::read_to_string(root.join("workspaces.json")) else { return HashMap::new() };
    let Ok(value) = serde_json::from_str::<Value>(&text) else { return HashMap::new() };
    value
        .get("workspaces")
        .and_then(Value::as_object)
        .map(|map| map.iter().map(|(id, w)| (id.clone(), str_at(w, "root").to_owned())).collect())
        .unwrap_or_default()
}

fn read_file(sessions: &Path, path: &Path, workspaces: &HashMap<String, String>) -> Vec<Event> {
    let relative = path.strip_prefix(sessions).unwrap_or(path);
    let parts: Vec<String> = relative.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let workspace = parts.first().cloned().unwrap_or_default();
    let session = parts.get(1).cloned().unwrap_or_default();
    let file_key = parts[..parts.len().saturating_sub(1)].join("/");
    let project = workspaces.get(&workspace).cloned().unwrap_or(workspace);

    let mut request = Map::new();
    let mut out = Vec::new();

    for (index, line) in lines(path).enumerate() {
        let is_usage = line.contains("\"usage.record\"");
        let is_request = line.contains("\"llm.request\"");
        let is_legacy = line.contains("\"token_usage\"");
        if !is_usage && !is_request && !is_legacy {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(&line) else { continue };
        let id = format!("{file_key}:{index}");

        match str_at(&entry, "type") {
            "llm.request" => {
                request = Map::new();
                for key in ["provider", "modelAlias", "thinkingEffort", "maxTokens", "kind", "turnStep"] {
                    if let Some(value) = entry.get(key) {
                        request.insert(key.to_owned(), value.clone());
                    }
                }
            }
            "usage.record" => {
                let Some(usage) = entry.get("usage") else { continue };
                let Some(timestamp) = entry.get("time").and_then(Value::as_i64).and_then(|ms| Utc.timestamp_millis_opt(ms).single()) else {
                    continue;
                };
                let (provider, model) = split_model(str_at(&entry, "model"));
                let mut details = request.clone();
                for key in ["agentId", "usageScope", "model"] {
                    if let Some(value) = entry.get(key) {
                        details.insert(key.to_owned(), value.clone());
                    }
                }
                details.insert("usage".to_owned(), usage.clone());
                let counts = [u64_at(usage, "inputOther"), u64_at(usage, "output"), u64_at(usage, "inputCacheRead"), u64_at(usage, "inputCacheCreation")];
                out.push(event(id, timestamp, provider, model, &session, &project, counts, details));
            }
            _ => {
                // kimi-cli: {"timestamp": <secs>, "message": {"type": "StatusUpdate", "payload": {"token_usage": {...}}}}
                let Some(message) = entry.get("message") else { continue };
                let Some(usage) = message.get("payload").and_then(|p| p.get("token_usage")).filter(|u| u.is_object()) else { continue };
                let Some(timestamp) = entry.get("timestamp").and_then(Value::as_f64).and_then(|s| Utc.timestamp_millis_opt((s * 1000.0) as i64).single()) else {
                    continue;
                };
                let mut details = Map::new();
                details.insert("usage".to_owned(), usage.clone());
                let counts = [u64_at(usage, "input_other"), u64_at(usage, "output"), u64_at(usage, "input_cache_read"), u64_at(usage, "input_cache_creation")];
                out.push(event(id, timestamp, "moonshot".to_owned(), "kimi-for-coding".to_owned(), &session, &project, counts, details));
            }
        }
    }
    out
}

/// `kimi-code/k3` → (`kimi-code`, `k3`), as ccusage names it.
fn split_model(name: &str) -> (String, String) {
    match name.rsplit_once('/') {
        Some((provider, model)) => (provider.to_owned(), model.to_owned()),
        None => ("kimi-code".to_owned(), if name.is_empty() { "unknown".to_owned() } else { name.to_owned() }),
    }
}

#[allow(clippy::too_many_arguments)]
fn event(id: String, timestamp: DateTime<Utc>, provider: String, model: String, session: &str, project: &str, counts: [u64; 4], details: Map<String, Value>) -> Event {
    let [input, output, cache_read, cache_creation] = counts;
    let mut event = Event {
        agent: "kimi",
        id,
        timestamp,
        model,
        provider,
        session_id: session.to_owned(),
        project: project.to_owned(),
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cache_read,
        cache_creation_tokens: cache_creation,
        cache_creation_5m_tokens: cache_creation,
        details: json!(details),
        ..Event::default()
    };
    event.fill_total();
    event
}
