use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Map, Value};

/// The agents this tool reads, in display order.
pub const AGENTS: [&str; 4] = ["claude", "codex", "kimi", "opencode"];

/// One model request, as logged by an agent. Field names match the server's `/api/events`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Event {
    #[serde(skip)]
    pub agent: &'static str,
    /// Unique per agent, stable across runs.
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub model: String,
    pub provider: String,
    pub session_id: String,
    pub project: String,
    /// Uncached input.
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    /// All cache writes; the 5m/1h split is below when the agent logs it.
    pub cache_creation_tokens: u64,
    pub cache_creation_5m_tokens: u64,
    pub cache_creation_1h_tokens: u64,
    /// Part of `output_tokens`, not on top of it.
    pub reasoning_tokens: u64,
    pub total_tokens: u64,
    pub cost_usd: f64,
    /// `log` (written by the agent), `calculated` (LiteLLM prices) or `none`.
    pub cost_source: &'static str,
    /// Everything else the log had about the request (never prompts or replies).
    pub details: Value,
    /// Cost the agent itself logged, used instead of a calculated one.
    #[serde(skip)]
    pub logged_cost: Option<f64>,
}

impl Event {
    /// Prompt size used to pick long-context price tiers.
    pub fn prompt_tokens(&self) -> u64 {
        self.input_tokens + self.cache_read_tokens + self.cache_creation_tokens
    }

    pub fn fill_total(&mut self) {
        if self.total_tokens == 0 {
            self.total_tokens = self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_creation_tokens;
        }
    }
}

pub fn u64_at(value: &Value, key: &str) -> u64 {
    match value.get(key) {
        Some(Value::Number(number)) => number.as_u64().or_else(|| number.as_f64().map(|f| f.max(0.0) as u64)).unwrap_or(0),
        _ => 0,
    }
}

pub fn str_at<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

/// The scalar (string, number, bool) fields of an object: ids, versions, flags. Nested objects
/// are left out, which is where prompts, replies and tool output live.
pub fn scalars(value: &Value, skip: &[&str]) -> Map<String, Value> {
    let mut out = Map::new();
    if let Some(object) = value.as_object() {
        for (key, field) in object {
            if skip.contains(&key.as_str()) {
                continue;
            }
            if matches!(field, Value::String(_) | Value::Number(_) | Value::Bool(_)) {
                out.insert(key.clone(), field.clone());
            }
        }
    }
    out
}
