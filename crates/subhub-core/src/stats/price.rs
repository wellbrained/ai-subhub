//! Built-in price table (USD per million tokens) and cost calculation. Costs are an "API
//! estimate": what the same tokens would cost on the pay-per-token API, not what a subscription
//! charges. Unknown models are "Unpriced": counted in tokens, not in cost.
use serde::{Deserialize, Serialize};

use super::Tokens;

/// USD per million tokens.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ModelPrice {
    pub input: f64,
    pub output: f64,
    pub cache_write: f64,
    pub cache_read: f64,
}

const fn p(input: f64, output: f64, cache_write: f64, cache_read: f64) -> ModelPrice {
    ModelPrice {
        input,
        output,
        cache_write,
        cache_read,
    }
}

// Built-in prices: Anthropic pricing page (platform.claude.com, fetched 2026-11-07; 5-minute cache
// write rate) and OpenAI pricing page for gpt-6.1-sol (standard tier, via web search 2026-11-07).
// VERIFY before trusting totals; they change. Not modelled: Haiku 5.5's higher tier for prompts
// over 100k tokens, OpenAI's long-prompt tier (over 272k), 1-hour cache writes. OpenAI models
// report no cache writes, so their write price only matters if a log ever shows some.
// Used only when the fetched LiteLLM prices (prices.rs) do not know a model; models in neither
// are Unpriced.
const BUILTIN: &[(&str, ModelPrice)] = &[
    // Claude Opus
    ("claude-opus-5-5", p(4.0, 20.0, 5.0, 0.20)),
    ("claude-opus-5", p(5.0, 25.0, 6.25, 0.50)),
    ("claude-opus-4-8", p(5.0, 25.0, 6.25, 0.50)),
    ("claude-opus-4-7", p(5.0, 25.0, 6.25, 0.50)),
    ("claude-opus-4-6", p(5.0, 25.0, 6.25, 0.50)),
    ("claude-opus-4-5", p(5.0, 25.0, 6.25, 0.50)),
    ("claude-opus-4-1", p(15.0, 75.0, 18.75, 1.50)),
    ("claude-opus-4", p(15.0, 75.0, 18.75, 1.50)),
    // Claude Sonnet
    ("claude-sonnet-5-5", p(2.0, 10.0, 2.50, 0.10)),
    ("claude-sonnet-5", p(2.0, 10.0, 2.50, 0.20)),
    ("claude-sonnet-4-6", p(3.0, 15.0, 3.75, 0.30)),
    ("claude-sonnet-4-5", p(3.0, 15.0, 3.75, 0.30)),
    ("claude-sonnet-4", p(3.0, 15.0, 3.75, 0.30)),
    // Claude Haiku
    ("claude-haiku-5-5", p(0.10, 0.50, 0.125, 0.01)),
    ("claude-haiku-4-5", p(1.0, 5.0, 1.25, 0.10)),
    // Claude Fable
    ("claude-fable-5-1", p(10.0, 50.0, 12.50, 0.25)),
    ("claude-fable-5", p(10.0, 50.0, 12.50, 1.0)),
    // OpenAI / Codex
    ("gpt-6.1-sol", p(2.0, 10.0, 2.50, 0.10)),
];

pub fn builtin_prices() -> Vec<(&'static str, ModelPrice)> {
    BUILTIN.to_vec()
}

/// Lower-cases and drops a trailing `-YYYYMMDD` snapshot date and a `[1m]` suffix.
pub(crate) fn normalize(model: &str) -> String {
    let mut m = model.trim().to_lowercase();
    if let Some(i) = m.find('[') {
        m.truncate(i);
    }
    if let Some((head, tail)) = m.rsplit_once('-')
        && tail.len() == 8
        && tail.bytes().all(|b| b.is_ascii_digit())
    {
        m = head.to_string();
    }
    m
}

/// The built-in price for `model` by exact id (snapshot dates ignored). None = no built-in price.
pub fn builtin_lookup(model: &str) -> Option<ModelPrice> {
    let n = normalize(model);
    BUILTIN.iter().find(|(k, _)| *k == n).map(|(_, v)| *v)
}

/// Cost of `t` at `price`, split by token kind (USD).
pub(crate) fn cost_parts(t: &Tokens, price: &ModelPrice) -> super::CostParts {
    let per = |n: u64, usd_per_m: f64| n as f64 * usd_per_m / 1_000_000.0;
    super::CostParts {
        input: per(t.input, price.input),
        cache_read: per(t.cache_read, price.cache_read),
        cache_write: per(t.cache_write, price.cache_write),
        output: per(t.output, price.output),
    }
}

/// What the cached input would have cost uncached, minus its cache-read cost.
pub(crate) fn cache_savings(t: &Tokens, price: &ModelPrice) -> f64 {
    t.cache_read as f64 * (price.input - price.cache_read) / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_lookup_exact_dated_and_unknown() {
        assert!(builtin_lookup("claude-opus-5-5").is_some());
        assert_eq!(
            builtin_lookup("claude-sonnet-4-5-20250929"),
            builtin_lookup("claude-sonnet-4-5")
        );
        assert!(builtin_lookup("claude-opus-5-5[1m]").is_some());
        assert!(builtin_lookup("codex-auto-review").is_none());
    }

    #[test]
    fn cost_and_savings() {
        let price = p(5.0, 25.0, 6.25, 0.5);
        let t = Tokens {
            input: 1_000_000,
            cache_read: 2_000_000,
            cache_write: 1_000_000,
            output: 100_000,
        };
        let c = cost_parts(&t, &price);
        assert!((c.input - 5.0).abs() < 1e-9);
        assert!((c.cache_read - 1.0).abs() < 1e-9);
        assert!((c.cache_write - 6.25).abs() < 1e-9);
        assert!((c.output - 2.5).abs() < 1e-9);
        // 2M cached tokens: 2 * (5 - 0.5) = 9.
        assert!((cache_savings(&t, &price) - 9.0).abs() < 1e-9);
    }
}
