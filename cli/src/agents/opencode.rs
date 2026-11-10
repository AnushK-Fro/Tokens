//! OpenCode: assistant messages carry `modelID`, `providerID`, `tokens` and `cost`. Newer
//! versions keep them in `opencode.db` (table `message`, JSON in `data`); older ones as
//! `storage/message/<session>/<message>.json`. Both are read; a message id counts once.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{TimeZone, Utc};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};

use super::{files, user_home};
use crate::event::{scalars, str_at, u64_at, Event};

pub fn root() -> PathBuf {
    if let Some(dir) = std::env::var_os("OPENCODE_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| user_home().join(".local").join("share"));
    data.join("opencode")
}

pub fn load() -> Vec<Event> {
    let root = root();
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    for (id, session, message) in from_database(&root.join("opencode.db")).into_iter().chain(from_files(&root.join("storage").join("message"))) {
        if seen.insert(id.clone()) {
            if let Some(event) = event(id, session, &message) {
                out.push(event);
            }
        }
    }
    out
}

fn from_database(path: &Path) -> Vec<(String, String, Value)> {
    if !path.exists() {
        return Vec::new();
    }
    let read = || -> rusqlite::Result<Vec<(String, String, Value)>> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        let mut statement = connection.prepare("SELECT id, session_id, data FROM message")?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))?;
        Ok(rows.filter_map(Result::ok).filter_map(|(id, session, data)| serde_json::from_str(&data).ok().map(|v| (id, session, v))).collect())
    };
    read().unwrap_or_else(|error| {
        eprintln!("opencode: could not read {}: {error}", path.display());
        Vec::new()
    })
}

fn from_files(dir: &Path) -> Vec<(String, String, Value)> {
    files(dir, |name| name.ends_with(".json"))
        .into_iter()
        .filter_map(|path| {
            let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
            let id = str_at(&value, "id").to_owned();
            let session = str_at(&value, "sessionID").to_owned();
            Some((if id.is_empty() { path.file_stem()?.to_string_lossy().into_owned() } else { id }, session, value))
        })
        .collect()
}

fn event(id: String, session: String, message: &Value) -> Option<Event> {
    if str_at(message, "role") != "assistant" {
        return None;
    }
    let tokens = message.get("tokens")?;
    let cache = tokens.get("cache").cloned().unwrap_or(Value::Null);
    let created = message.get("time").and_then(|t| t.get("created")).and_then(Value::as_i64)?;
    let timestamp = Utc.timestamp_millis_opt(created).single()?;
    let cost = message.get("cost").and_then(Value::as_f64).filter(|c| *c > 0.0);

    let mut details = scalars(message, &["role"]);
    details.insert("tokens".to_owned(), tokens.clone());
    for key in ["path", "time"] {
        if let Some(value) = message.get(key) {
            details.insert(key.to_owned(), value.clone());
        }
    }

    let project = message.get("path").map(|p| str_at(p, "cwd")).unwrap_or("").to_owned();
    let mut event = Event {
        agent: "opencode",
        id,
        timestamp,
        model: str_at(message, "modelID").to_owned(),
        provider: str_at(message, "providerID").to_owned(),
        session_id: if session.is_empty() { str_at(message, "sessionID").to_owned() } else { session },
        project,
        input_tokens: u64_at(tokens, "input"),
        output_tokens: u64_at(tokens, "output"),
        cache_read_tokens: u64_at(&cache, "read"),
        cache_creation_tokens: u64_at(&cache, "write"),
        cache_creation_5m_tokens: u64_at(&cache, "write"),
        reasoning_tokens: u64_at(tokens, "reasoning"),
        logged_cost: cost,
        details: json!(details),
        ..Event::default()
    };
    if event.model.is_empty() {
        event.model = "unknown".to_owned();
    }
    event.fill_total();
    Some(event)
}
