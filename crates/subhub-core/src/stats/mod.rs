//! Usage statistics from the CLIs' local session logs (ADR 0001, "Usage statistics").
//!
//! Privacy: the logs contain conversation text. Parsers read only usage numbers, model, timestamp,
//! session, subagent flag, working folder, tool names, the first word of shell commands and thread
//! titles; nothing else is deserialized, kept, logged or reported.
//!
//! Layout: `kind` (task kinds from tool calls), `parse` (pure line parsers), `price` (price
//! table, cost), `scan` (file discovery, incremental per-file cache, aggregation), `summary`
//! (range summaries for the UI).
mod kind;
mod parse;
mod price;
mod prices;
mod scan;
mod summary;

use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike};
use serde::{Deserialize, Serialize};

pub use kind::{TaskKind, ToolCall, classify_tool, command_word, step_kinds};
pub use parse::{
    ClaudeMessage, CodexEvent, RawUsage, parse_claude_line, parse_claude_title, parse_codex_line,
};
pub use price::{ModelPrice, builtin_lookup, builtin_prices};
pub use prices::{
    LITELLM_URL, PriceBook, PriceSource, default_path as prices_path, parse_litellm,
    refresh as refresh_prices,
};
pub use scan::{
    Bucket, FileEntry, Roots, Scan, ScanSession, cache_path, discover_roots, scan, scan_default,
};
pub use summary::{
    CostParts, DayRow, ModelRow, MonthPace, Point, ProviderSummary, Range, Summary, summarize,
};

/// Token counts of one kind of work, normalized across providers: `input` is uncached input,
/// `cache_read` cached input, `cache_write` input written to the cache. "Processed" is the sum.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub input: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub output: u64,
}

impl Tokens {
    pub fn add(&mut self, o: &Tokens) {
        self.input += o.input;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
        self.output += o.output;
    }

    /// Everything the model processed: input, cache reads and writes, output.
    pub fn processed(&self) -> u64 {
        self.input + self.cache_read + self.cache_write + self.output
    }

    /// Work tokens: input + output + cache writes, i.e. processed without the cache reads.
    pub fn work(&self) -> u64 {
        self.input + self.cache_write + self.output
    }

    pub fn is_zero(&self) -> bool {
        self.processed() == 0
    }
}

/// Hours are counted in local time as `days since 0001-01-01 * 24 + hour`.
pub fn hour_index(t: NaiveDateTime) -> i64 {
    t.date().num_days_from_ce() as i64 * 24 + t.hour() as i64
}

/// The local calendar day of an hour index.
pub fn hour_date(h: i64) -> NaiveDate {
    NaiveDate::from_num_days_from_ce_opt(h.div_euclid(24) as i32).unwrap_or(NaiveDate::MIN)
}

/// Local hour of day (0..24) of an hour index.
pub fn hour_of_day(h: i64) -> u32 {
    h.rem_euclid(24) as u32
}

pub fn day_index(d: NaiveDate) -> i64 {
    d.num_days_from_ce() as i64
}

/// API estimate (USD) of `t` at `price`.
pub fn cost(t: &Tokens, price: &ModelPrice) -> f64 {
    price::cost_parts(t, price).total()
}

/// Plain-text aggregate report of a scan for the last 90 days (`--stats-dump`): numbers only,
/// never any log content.
pub fn dump_report(scan: &Scan, now: NaiveDateTime, prices: &PriceBook) -> String {
    use std::fmt::Write;
    let s = summarize(scan, Range::Quarter, now, prices);
    let mut o = String::new();
    let _ = writeln!(
        o,
        "files: {} (read {}), scan time: {:.2?}",
        scan.files, scan.files_read, scan.elapsed
    );
    let _ = writeln!(
        o,
        "90 days: sessions {}, processed {}, cached input {}, uncached input {}, cache write {}, output {}",
        s.sessions,
        s.tokens.processed(),
        s.tokens.cache_read,
        s.tokens.input,
        s.tokens.cache_write,
        s.tokens.output
    );
    let _ = writeln!(
        o,
        "cost (API estimate): total {:.2}, input {:.2}, cache read {:.2}, cache write {:.2}, output {:.2}, cache savings {:.2}",
        s.cost.total(),
        s.cost.input,
        s.cost.cache_read,
        s.cost.cache_write,
        s.cost.output,
        s.cache_savings
    );
    for p in &s.providers {
        let name = p.provider.name().to_lowercase();
        let _ = writeln!(
            o,
            "provider {name}: sessions {}, tokens {}, cost {:.2}",
            p.sessions,
            p.tokens.processed(),
            p.cost
        );
    }
    for m in &s.models {
        let cost = m.cost.map_or("unpriced".to_string(), |c| format!("{c:.2}"));
        let _ = writeln!(
            o,
            "model {}: tokens {} (in {}, cache read {}, cache write {}, out {}), cost {cost}",
            m.model,
            m.tokens.processed(),
            m.tokens.input,
            m.tokens.cache_read,
            m.tokens.cache_write,
            m.tokens.output
        );
    }
    o
}
