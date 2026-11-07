//! Automatic prices: LiteLLM's public price list, fetched at most daily and kept in
//! `%LOCALAPPDATA%\AI SubHub\prices.json`. Layering: fetched LiteLLM prices, then the
//! built-in table (`price.rs`), else the model is Unpriced. There are no manual overrides.
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::price::{ModelPrice, builtin_lookup, normalize};

pub const LITELLM_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";

/// Fetched prices older than this are fetched again.
pub const MAX_AGE_HOURS: i64 = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceSource {
    LiteLlm,
    BuiltIn,
}

impl PriceSource {
    pub fn label(self) -> &'static str {
        match self {
            PriceSource::LiteLlm => "LiteLLM",
            PriceSource::BuiltIn => "built-in",
        }
    }
}

/// The fetched price list as stored on disk, plus the last fetch error.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PriceBook {
    pub fetched_at: Option<DateTime<Utc>>,
    pub source: String,
    pub prices: BTreeMap<String, ModelPrice>,
    /// Why the last update failed (the old list stays in use).
    pub error: Option<String>,
}

pub fn default_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("AI SubHub").join("prices.json")
}

impl PriceBook {
    /// A missing or unreadable file gives an empty book (built-in prices only).
    pub fn load(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Price and where it came from: fetched LiteLLM list first, then the built-in table.
    pub fn lookup(&self, model: &str) -> Option<(ModelPrice, PriceSource)> {
        if let Some(p) = self.prices.get(model) {
            return Some((*p, PriceSource::LiteLlm));
        }
        let n = normalize(model);
        if let Some((_, p)) = self.prices.iter().find(|(k, _)| normalize(k) == n) {
            return Some((*p, PriceSource::LiteLlm));
        }
        builtin_lookup(model).map(|p| (p, PriceSource::BuiltIn))
    }

    pub fn price(&self, model: &str) -> Option<ModelPrice> {
        self.lookup(model).map(|(p, _)| p)
    }

    pub fn is_stale(&self, now: DateTime<Utc>) -> bool {
        self.fetched_at
            .is_none_or(|t| now - t >= chrono::Duration::hours(MAX_AGE_HOURS))
    }
}

/// USD per million tokens from LiteLLM's USD per token, without float noise.
fn per_million(v: f64) -> f64 {
    (v * 1e12).round() / 1e6
}

fn num(v: &serde_json::Value, key: &str) -> Option<f64> {
    v.get(key)
        .and_then(|x| x.as_f64())
        .filter(|x| x.is_finite() && *x >= 0.0)
}

/// Parses LiteLLM's list: keeps Anthropic and OpenAI entries plus any model in `wanted`. Keys are
/// stored by plain model id (a `provider/` prefix is dropped; a plain key wins over a prefixed
/// one). An entry needs input and output prices; missing cache prices fall back to the input
/// price (no discount known). Anything malformed is skipped; this never panics.
pub fn parse_litellm(json: &str, wanted: &HashSet<String>) -> BTreeMap<String, ModelPrice> {
    let mut out: BTreeMap<String, ModelPrice> = BTreeMap::new();
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(json) else {
        return out;
    };
    for (key, v) in &map {
        if key == "sample_spec" || !v.is_object() {
            continue;
        }
        let provider = v.get("litellm_provider").and_then(|p| p.as_str());
        let plain = key.rsplit('/').next().unwrap_or(key);
        let keep = matches!(provider, Some("anthropic" | "openai"))
            || wanted.contains(key.as_str())
            || wanted.contains(plain);
        if !keep {
            continue;
        }
        let (Some(input), Some(output)) = (
            num(v, "input_cost_per_token"),
            num(v, "output_cost_per_token"),
        ) else {
            continue;
        };
        let price = ModelPrice {
            input: per_million(input),
            output: per_million(output),
            cache_write: num(v, "cache_creation_input_token_cost")
                .map_or(per_million(input), per_million),
            cache_read: num(v, "cache_read_input_token_cost")
                .map_or(per_million(input), per_million),
        };
        if key.contains('/') {
            out.entry(plain.to_string()).or_insert(price);
        } else {
            out.insert(plain.to_string(), price);
        }
    }
    out
}

/// Downloads the list (30 s timeout).
fn fetch(url: &str) -> anyhow::Result<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("ai-subhub")
        .build()?;
    Ok(client.get(url).send()?.error_for_status()?.text()?)
}

/// Updates the stored book when it is older than a day (or `force`). On failure the old prices
/// stay and the error is recorded. Blocking: call from a background thread.
pub fn refresh(
    path: &Path,
    wanted: &HashSet<String>,
    force: bool,
    now: DateTime<Utc>,
) -> PriceBook {
    refresh_from(path, wanted, force, now, || fetch(LITELLM_URL))
}

fn refresh_from(
    path: &Path,
    wanted: &HashSet<String>,
    force: bool,
    now: DateTime<Utc>,
    get: impl FnOnce() -> anyhow::Result<String>,
) -> PriceBook {
    let mut book = PriceBook::load(path);
    if !force && !book.is_stale(now) {
        return book;
    }
    match get() {
        Ok(text) => {
            let prices = parse_litellm(&text, wanted);
            if prices.is_empty() {
                book.error = Some("The price list had no usable entries".into());
            } else {
                book = PriceBook {
                    fetched_at: Some(now),
                    source: LITELLM_URL.into(),
                    prices,
                    error: None,
                };
            }
        }
        Err(e) => book.error = Some(short_error(&e)),
    }
    let _ = book.save(path);
    book
}

/// A short user-facing reason without URLs or internals.
fn short_error(e: &anyhow::Error) -> String {
    if let Some(r) = e.downcast_ref::<reqwest::Error>() {
        if r.is_timeout() {
            return "timed out".into();
        }
        if r.is_connect() {
            return "no connection".into();
        }
        if let Some(s) = r.status() {
            return format!("HTTP {}", s.as_u16());
        }
    }
    "download failed".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "sample_spec": {"litellm_provider": "x", "input_cost_per_token": 1},
      "claude-opus-5-5": {"litellm_provider": "anthropic", "input_cost_per_token": 4e-06,
        "output_cost_per_token": 2e-05, "cache_read_input_token_cost": 2e-07,
        "cache_creation_input_token_cost": 5e-06},
      "openai/gpt-6.1-sol": {"litellm_provider": "openai", "input_cost_per_token": 9e-06,
        "output_cost_per_token": 9e-05},
      "gpt-6.1-sol": {"litellm_provider": "openai", "input_cost_per_token": 2e-06,
        "output_cost_per_token": 1e-05, "cache_read_input_token_cost": 1e-07},
      "anthropic.claude-opus-5-5-v1:0": {"litellm_provider": "bedrock",
        "input_cost_per_token": 1e-06, "output_cost_per_token": 1e-06},
      "acme/special-model": {"litellm_provider": "acme", "input_cost_per_token": 1e-06,
        "output_cost_per_token": 3e-06},
      "claude-no-output": {"litellm_provider": "anthropic", "input_cost_per_token": 1e-06},
      "weird": 5,
      "claude-bad": {"litellm_provider": "anthropic", "input_cost_per_token": "x",
        "output_cost_per_token": 1e-06}
    }"#;

    fn wanted(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_anthropic_openai_and_wanted_entries_defensively() {
        let p = parse_litellm(SAMPLE, &wanted(&["special-model"]));
        let opus = p["claude-opus-5-5"];
        assert_eq!(
            (opus.input, opus.output, opus.cache_write, opus.cache_read),
            (4.0, 20.0, 5.0, 0.2)
        );
        // Plain key wins over the prefixed one; missing cache prices fall back to input.
        let sol = p["gpt-6.1-sol"];
        assert_eq!(
            (sol.input, sol.cache_write, sol.cache_read),
            (2.0, 2.0, 0.1)
        );
        // A seen model of another provider is kept (with its prefix dropped).
        assert_eq!(p["special-model"].output, 3.0);
        // Other providers, missing/garbled fields and junk are skipped.
        assert!(!p.contains_key("anthropic.claude-opus-5-5-v1:0"));
        assert!(!p.contains_key("claude-no-output") && !p.contains_key("claude-bad"));
        assert!(!p.contains_key("weird") && !p.contains_key("sample_spec"));
        assert!(parse_litellm("not json", &wanted(&[])).is_empty());
        assert!(parse_litellm("[1,2]", &wanted(&[])).is_empty());
    }

    #[test]
    fn lookup_layers_fetched_over_builtin() {
        let mut book = PriceBook::default();
        // Built-in only.
        let (_, src) = book.lookup("claude-opus-5-5").unwrap();
        assert_eq!(src, PriceSource::BuiltIn);
        assert!(book.lookup("codex-auto-review").is_none());
        book.prices.insert(
            "claude-opus-5-5".into(),
            ModelPrice {
                input: 1.0,
                output: 2.0,
                cache_write: 3.0,
                cache_read: 4.0,
            },
        );
        let (p, src) = book.lookup("claude-opus-5-5-20260101").unwrap();
        assert_eq!((p.input, src), (1.0, PriceSource::LiteLlm));
    }

    #[test]
    fn refresh_fetches_when_stale_keeps_old_copy_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prices.json");
        let now: DateTime<Utc> = "2026-11-07T12:00:00Z".parse().unwrap();
        let ok = refresh_from(&path, &wanted(&[]), false, now, || Ok(SAMPLE.to_string()));
        assert!(ok.error.is_none() && ok.prices.contains_key("claude-opus-5-5"));
        assert_eq!(ok.fetched_at, Some(now));

        // Fresh: no download at all.
        let later = now + chrono::Duration::hours(2);
        let same = refresh_from(&path, &wanted(&[]), false, later, || {
            panic!("must not download a fresh list")
        });
        assert_eq!(same.fetched_at, Some(now));

        // Stale and the download fails: old prices stay, the error is recorded and saved.
        let day = now + chrono::Duration::hours(25);
        let failed = refresh_from(&path, &wanted(&[]), false, day, || anyhow::bail!("offline"));
        assert!(failed.prices.contains_key("claude-opus-5-5"));
        assert!(failed.error.is_some());
        assert_eq!(failed.fetched_at, Some(now));
        assert!(PriceBook::load(&path).error.is_some());

        // A forced update with an empty list is an error, not a wipe.
        let empty = refresh_from(&path, &wanted(&[]), true, day, || Ok("{}".into()));
        assert!(empty.prices.contains_key("claude-opus-5-5") && empty.error.is_some());
    }
}
