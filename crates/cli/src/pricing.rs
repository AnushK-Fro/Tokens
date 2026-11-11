//! API-equivalent cost from LiteLLM's public price list, the same source ccusage uses.
//!
//! The list is cached for a day. Long-context tiers (`…_above_200k_tokens`, `…_above_272k_tokens`)
//! apply per request when its prompt is over the threshold, and Claude's 1-hour cache writes use
//! their own rate.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde_json::{Map, Value};

use crate::event::Event;

const URL: &str = "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// Agent model names that LiteLLM lists under another name. LiteLLM has DeepSeek V4.1 Flash only
/// under resellers; OpenCode Go's entry charges DeepSeek's own V4 Flash rates.
const ALIASES: &[(&str, &str)] = &[
    ("k3", "moonshot/kimi-k3"),
    ("kimi-for-coding", "moonshot/kimi-k2-thinking"),
    ("deepseek-v4.1-flash", "opencode_go/deepseek-v4.1-flash"),
];

pub struct Prices {
    models: Map<String, Value>,
    resolved: HashMap<String, Option<String>>,
}

impl Prices {
    pub fn load(offline: bool) -> Self {
        let models = read(offline).unwrap_or_else(|| {
            eprintln!("warning: no LiteLLM prices available; costs are only those the agents logged");
            Map::new()
        });
        Self { models, resolved: HashMap::new() }
    }

    /// Sets `cost_usd` and `cost_source` on every event.
    pub fn apply(&mut self, events: &mut [Event]) {
        for event in events {
            if let Some(cost) = event.logged_cost {
                event.cost_usd = cost;
                event.cost_source = "log";
                continue;
            }
            match self.entry(&event.model).map(|entry| cost(entry, event)) {
                Some(cost) => {
                    event.cost_usd = cost;
                    event.cost_source = "calculated";
                }
                None => {
                    event.cost_usd = 0.0;
                    event.cost_source = "none";
                }
            }
        }
    }

    fn entry(&mut self, model: &str) -> Option<&Map<String, Value>> {
        if !self.resolved.contains_key(model) {
            let key = self.resolve(model);
            self.resolved.insert(model.to_owned(), key);
        }
        let key = self.resolved.get(model)?.as_ref()?;
        self.models.get(key)?.as_object()
    }

    fn priced(&self, key: &str) -> bool {
        self.models.get(key).and_then(|e| e.get("input_cost_per_token")).is_some()
    }

    /// Exact name, alias, common provider prefixes, then the longest listed name the model starts
    /// or ends with (`gpt-6-sol-2026-09-22` → `gpt-6-sol`, `gpt-daybreak-blue-latest` →
    /// `daybreak-blue-latest`).
    fn resolve(&self, model: &str) -> Option<String> {
        let bare = model.rsplit('/').next().unwrap_or(model);
        let mut candidates = vec![model.to_owned(), bare.to_owned()];
        candidates.extend(ALIASES.iter().filter(|(name, _)| *name == bare).map(|(_, key)| (*key).to_owned()));
        for prefix in ["anthropic/", "openai/", "moonshot/", "gemini/", "xai/", "deepseek/"] {
            candidates.push(format!("{prefix}{bare}"));
        }
        if let Some(found) = candidates.into_iter().find(|key| self.priced(key)) {
            return Some(found);
        }

        self.models
            .keys()
            .filter(|key| !key.contains('/') && key.len() >= 5 && self.priced(key))
            .filter(|key| bare.starts_with(&format!("{key}-")) || bare.ends_with(&format!("-{key}")))
            .max_by_key(|key| key.len())
            .cloned()
    }
}

fn cost(entry: &Map<String, Value>, event: &Event) -> f64 {
    let tier = tier_suffix(entry, event.prompt_tokens());
    let rate = |field: &str| -> f64 {
        tier.as_ref()
            .and_then(|suffix| entry.get(&format!("{field}{suffix}")))
            .or_else(|| entry.get(field))
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
    };

    let input = rate("input_cost_per_token");
    let output = rate("output_cost_per_token");
    let read = if entry.contains_key("cache_read_input_token_cost") { rate("cache_read_input_token_cost") } else { input };
    let write = if entry.contains_key("cache_creation_input_token_cost") { rate("cache_creation_input_token_cost") } else { input };
    let write_1h = if entry.contains_key("cache_creation_input_token_cost_above_1hr") { rate("cache_creation_input_token_cost_above_1hr") } else { write };

    event.input_tokens as f64 * input
        + event.output_tokens as f64 * output
        + event.cache_read_tokens as f64 * read
        + (event.cache_creation_tokens - event.cache_creation_1h_tokens.min(event.cache_creation_tokens)) as f64 * write
        + event.cache_creation_1h_tokens.min(event.cache_creation_tokens) as f64 * write_1h
}

/// `_above_<N>k_tokens` for the highest tier this prompt is over, if the model has tiers.
fn tier_suffix(entry: &Map<String, Value>, prompt: u64) -> Option<String> {
    entry
        .keys()
        .filter_map(|key| key.strip_prefix("input_cost_per_token_above_")?.strip_suffix("k_tokens")?.parse::<u64>().ok())
        .filter(|thousands| prompt > thousands * 1000)
        .max()
        .map(|thousands| format!("_above_{thousands}k_tokens"))
}

fn cache_path() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("tokens-cli").join("litellm-prices.json"))
}

fn read(offline: bool) -> Option<Map<String, Value>> {
    let path = cache_path();
    let cached = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok().map(|text| (text, p)));
    let fresh = path
        .as_ref()
        .and_then(|p| std::fs::metadata(p).ok()?.modified().ok())
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age < MAX_AGE);

    if !offline && !fresh {
        match ureq::get(URL).timeout(Duration::from_secs(30)).call().map(|r| r.into_string()) {
            Ok(Ok(text)) if serde_json::from_str::<Map<String, Value>>(&text).is_ok() => {
                if let Some(path) = &path {
                    let _ = std::fs::create_dir_all(path.parent().unwrap_or(path));
                    let _ = std::fs::write(path, &text);
                }
                return serde_json::from_str(&text).ok();
            }
            _ => eprintln!("warning: could not download LiteLLM prices; using the cached copy if there is one"),
        }
    }
    cached.and_then(|(text, _)| serde_json::from_str(&text).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn prices(value: Value) -> Prices {
        Prices { models: value.as_object().unwrap().clone(), resolved: HashMap::new() }
    }

    #[test]
    fn resolves_aliases_prefixes_and_suffixes() {
        let p = prices(json!({
            "gpt-6-sol": { "input_cost_per_token": 1.0 },
            "daybreak-blue-latest": { "input_cost_per_token": 1.0 },
            "moonshot/kimi-k3": { "input_cost_per_token": 1.0 },
            "anthropic/claude-x": { "input_cost_per_token": 1.0 },
            "opencode_go/deepseek-v4.1-flash": { "input_cost_per_token": 1.0 }
        }));
        assert_eq!(p.resolve("gpt-6-sol-2026-09-22").as_deref(), Some("gpt-6-sol"));
        assert_eq!(p.resolve("gpt-daybreak-blue-latest").as_deref(), Some("daybreak-blue-latest"));
        assert_eq!(p.resolve("k3").as_deref(), Some("moonshot/kimi-k3"));
        assert_eq!(p.resolve("claude-x").as_deref(), Some("anthropic/claude-x"));
        assert_eq!(p.resolve("deepseek/deepseek-v4.1-flash").as_deref(), Some("opencode_go/deepseek-v4.1-flash"));
        assert_eq!(p.resolve("nothing-like-it"), None);
    }

    #[test]
    fn applies_tier_and_one_hour_cache_rate() {
        let entry = json!({
            "input_cost_per_token": 1.0, "output_cost_per_token": 2.0,
            "cache_read_input_token_cost": 0.1, "cache_creation_input_token_cost": 1.25,
            "cache_creation_input_token_cost_above_1hr": 2.0,
            "input_cost_per_token_above_200k_tokens": 10.0
        });
        let entry = entry.as_object().unwrap();
        let small = Event { input_tokens: 10, output_tokens: 1, cache_read_tokens: 100, cache_creation_tokens: 4, cache_creation_1h_tokens: 3, ..Event::default() };
        assert!((cost(entry, &small) - (10.0 + 2.0 + 10.0 + 1.25 + 6.0)).abs() < 1e-9);
        let large = Event { input_tokens: 200_001, ..Event::default() };
        assert!((cost(entry, &large) - 2_000_010.0).abs() < 1e-6);
    }
}
