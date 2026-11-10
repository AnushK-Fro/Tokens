//! Sends usage to a Tokens server: each agent's daily totals to `/api/ingest` (what the
//! dashboard reads) and every request to `/api/events` (kept for later analysis).
//!
//! Both endpoints merge by maximum, so sending the same data again changes nothing. Events are
//! sent from a little before the last successful push; `--all` sends every one again.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::agents;
use crate::event::Event;
use crate::report::{self, Daily};

const EVENT_BATCH: usize = 1000;
/// Re-sent on every push: Claude Code rewrites a message's usage while it streams.
const OVERLAP: chrono::Duration = chrono::Duration::hours(1);

pub struct Target {
    pub base: String,
    pub token: String,
    pub name: Option<String>,
    pub project: Option<String>,
}

impl Target {
    pub fn new(url: &str, token: String, name: Option<String>, project: Option<String>) -> Result<Self> {
        if token.is_empty() {
            bail!("no ingest token: pass --token or set TOKENS_TOKEN");
        }
        Ok(Self { base: base_url(url)?, token, name, project })
    }
}

/// `https://host/`, `https://host/index.php?api=ingest` and `https://host/api/ingest` all mean
/// the same server.
pub fn base_url(url: &str) -> Result<String> {
    let url = url.trim();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        bail!("server URL must start with http:// or https:// (got `{url}`)");
    }
    let mut base = url.split(['?', '#']).next().unwrap_or(url).trim_end_matches('/').to_owned();
    for suffix in ["/api/ingest", "/api/events", "/index.php", "/tokens.php", "/api"] {
        if let Some(stripped) = base.strip_suffix(suffix) {
            base = stripped.trim_end_matches('/').to_owned();
        }
    }
    Ok(base)
}

pub fn push(target: &Target, events: &[Event], daily: &Daily, agents_to_send: &[&'static str], all: bool, dry_run: bool) -> Result<()> {
    let mut state = State::load();

    for &agent in agents_to_send {
        let agent_events: Vec<&Event> = events.iter().filter(|e| e.agent == agent).collect();
        if agent_events.is_empty() {
            continue;
        }
        let source = source_id(agent)?;
        let state_key = format!("{}|{agent}|{source}", target.base);
        let since = if all { None } else { state.since(&state_key) };
        let fresh: Vec<&Event> = agent_events.iter().copied().filter(|e| since.is_none_or(|s| e.timestamp >= s)).collect();
        let report = report::ingest_payload(daily, agent);
        let days = report["daily"].as_array().map_or(0, Vec::len);

        if dry_run {
            println!("{agent}: would send {days} days and {} of {} requests as source {source}", fresh.len(), agent_events.len());
            continue;
        }

        let reply = post(target, agent, &source, "/api/ingest", &report)?;
        let mut line = format!(
            "{agent}: {days} days ({} new, {} raised)",
            reply["inserted"].as_u64().unwrap_or(0),
            reply["increased"].as_u64().unwrap_or(0)
        );

        let mut sent = 0;
        let mut supported = true;
        for batch in fresh.chunks(EVENT_BATCH) {
            match post(target, agent, &source, "/api/events", &json!({ "events": batch })) {
                Ok(_) => sent += batch.len(),
                Err(error) if error.downcast_ref::<NotFound>().is_some() => {
                    supported = false;
                    break;
                }
                Err(error) => return Err(error),
            }
        }
        if supported {
            line.push_str(&format!(", {sent} requests"));
            if let Some(latest) = agent_events.iter().map(|e| e.timestamp).max() {
                state.set(state_key, latest);
                state.save();
            }
        } else {
            line.push_str(", requests not sent (server has no /api/events yet)");
        }
        println!("{line}");
    }
    Ok(())
}

#[derive(Debug)]
struct NotFound;

impl std::fmt::Display for NotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not found")
    }
}

impl std::error::Error for NotFound {}

fn post(target: &Target, agent: &str, source: &str, path: &str, body: &Value) -> Result<Value> {
    let host = gethostname::gethostname().to_string_lossy().into_owned();
    let name = match &target.name {
        Some(name) => format!("{name}@{host}:{agent}"),
        None => format!("{host}:{agent}"),
    };
    let mut request = ureq::post(&format!("{}{path}", target.base))
        .timeout(Duration::from_secs(120))
        .set("Authorization", &format!("Bearer {}", target.token))
        .set("Content-Type", "application/json")
        .set("User-Agent", concat!("tokens-cli/", env!("CARGO_PKG_VERSION")))
        .set("X-Usage-Agent", agent)
        .set("X-Usage-Source-Id", source)
        .set("X-Usage-Source-Name", &name);
    if let Some(project) = &target.project {
        request = request.set("X-Usage-Project", project);
    }

    match request.send_string(&body.to_string()) {
        Ok(response) => Ok(response.into_string().ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or(Value::Null)),
        Err(ureq::Error::Status(404 | 405, _)) if path == "/api/events" => Err(NotFound.into()),
        Err(ureq::Error::Status(status, response)) => {
            let text = response.into_string().unwrap_or_default();
            bail!("{path} answered HTTP {status}: {}", text.trim())
        }
        Err(error) => Err(error).with_context(|| format!("sending to {}{path}", target.base)),
    }
}

/// One id per machine and agent, kept in the agent's own folder. It is the same file the
/// devcontainer sync script used, so a machine keeps its source id when it switches to this tool.
fn source_id(agent: &str) -> Result<String> {
    let variable = format!("TOKEN_USAGE_{}_SOURCE_ID", agent.to_ascii_uppercase());
    if let Ok(id) = std::env::var(&variable) {
        if !id.trim().is_empty() {
            return Ok(id.trim().to_owned());
        }
    }
    let home = agents::home(agent).context("no data folder for this agent")?;
    let file = home.join(".token-usage-source-id");
    if let Ok(id) = std::fs::read_to_string(&file) {
        if !id.trim().is_empty() {
            return Ok(id.trim().to_owned());
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    std::fs::create_dir_all(&home).with_context(|| format!("creating {}", home.display()))?;
    std::fs::write(&file, &id).with_context(|| format!("writing {}", file.display()))?;
    Ok(id)
}

/// When each server/agent/source last received events.
struct State {
    path: Option<PathBuf>,
    pushed: HashMap<String, DateTime<Utc>>,
}

impl State {
    fn load() -> Self {
        let path = dirs::data_local_dir().map(|dir| dir.join("tokens-cli").join("state.json"));
        let pushed = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self { path, pushed }
    }

    fn since(&self, key: &str) -> Option<DateTime<Utc>> {
        self.pushed.get(key).map(|last| *last - OVERLAP)
    }

    fn set(&mut self, key: String, latest: DateTime<Utc>) {
        self.pushed.insert(key, latest);
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(path));
        if let Ok(text) = serde_json::to_string_pretty(&self.pushed) {
            let _ = std::fs::write(path, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::base_url;

    #[test]
    fn normalises_server_urls() {
        assert_eq!(base_url("https://t.example/").unwrap(), "https://t.example");
        assert_eq!(base_url("https://t.example/index.php?api=ingest").unwrap(), "https://t.example");
        assert_eq!(base_url("https://t.example/api/ingest").unwrap(), "https://t.example");
        assert!(base_url("t.example").is_err());
    }
}
