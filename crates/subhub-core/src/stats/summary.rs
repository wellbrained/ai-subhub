//! Range summaries for the Statistics window: totals, per provider, daily series, per model and
//! per day, with cost as an API estimate from the price table.
use std::collections::{BTreeMap, HashMap};

use chrono::{Datelike, NaiveDate, NaiveDateTime};

use super::price::{cache_savings, cost_parts};
use super::prices::PriceBook;
use super::scan::Scan;
use super::{ModelPrice, Tokens, day_index, hour_date, hour_index, hour_of_day};
use crate::model::ProviderKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    /// The last 24 hours (hourly points).
    Day,
    Week,
    Month,
    Quarter,
}

impl Range {
    pub const ALL: [Range; 4] = [Range::Day, Range::Week, Range::Month, Range::Quarter];

    /// Days shown (the 24-hour range shows 24 hourly points).
    pub fn days(self) -> i64 {
        match self {
            Range::Day => 1,
            Range::Week => 7,
            Range::Month => 30,
            Range::Quarter => 90,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Range::Day => "Past 24h",
            Range::Week => "7 days",
            Range::Month => "30 days",
            Range::Quarter => "90 days",
        }
    }
}

/// Cost split by kind of token (USD).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CostParts {
    pub input: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub output: f64,
}

impl CostParts {
    pub fn total(&self) -> f64 {
        self.input + self.cache_read + self.cache_write + self.output
    }

    fn add(&mut self, o: &CostParts) {
        self.input += o.input;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
        self.output += o.output;
    }
}

/// One chart point; index 0 = Claude, 1 = Codex.
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    pub label: String,
    pub tokens: [u64; 2],
    pub cost: [f64; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderSummary {
    pub provider: ProviderKind,
    pub sessions: usize,
    pub tokens: Tokens,
    /// Priced models only.
    pub cost: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelRow {
    pub model: String,
    pub provider: ProviderKind,
    pub tokens: Tokens,
    /// None = Unpriced.
    pub cost: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DayRow {
    pub day: NaiveDate,
    pub tokens: u64,
    pub cost: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub range: Range,
    pub sessions: usize,
    pub tokens: Tokens,
    pub cost: CostParts,
    /// What the cached input would have cost uncached, minus its cache-read cost (priced models).
    pub cache_savings: f64,
    /// Codex first, then Claude (the order of the usage board).
    pub providers: Vec<ProviderSummary>,
    pub points: Vec<Point>,
    pub models: Vec<ModelRow>,
    /// Newest first.
    pub days: Vec<DayRow>,
}

fn slot(p: ProviderKind) -> usize {
    match p {
        ProviderKind::Claude => 0,
        ProviderKind::Codex => 1,
    }
}

fn day_label(d: NaiveDate) -> String {
    format!("{} {}", d.format("%b"), d.day())
}

/// Summarizes `scan` for `range` ending at local time `now`, pricing with the built-in table and
/// the user's overrides.
pub fn summarize(scan: &Scan, range: Range, now: NaiveDateTime, prices: &PriceBook) -> Summary {
    let now_hour = hour_index(now);
    let (lo, hi) = match range {
        Range::Day => (now_hour - 23, now_hour),
        r => ((day_index(now.date()) - (r.days() - 1)) * 24, now_hour),
    };
    // Chart points: hourly for 24h, daily otherwise.
    let (step, first) = match range {
        Range::Day => (1, lo),
        _ => (24, lo),
    };
    let n_points = ((hi - first) / step + 1) as usize;
    let mut points: Vec<Point> = (0..n_points as i64)
        .map(|i| {
            let h = first + i * step;
            Point {
                label: match range {
                    Range::Day => format!("{:02}:00", hour_of_day(h)),
                    _ => day_label(hour_date(h)),
                },
                tokens: [0; 2],
                cost: [0.0; 2],
            }
        })
        .collect();

    let mut price_cache: HashMap<&str, Option<ModelPrice>> = HashMap::new();
    let mut tokens = Tokens::default();
    let mut cost = CostParts::default();
    let mut savings = 0.0;
    let mut prov: [(Tokens, f64); 2] = [(Tokens::default(), 0.0), (Tokens::default(), 0.0)];
    let mut models: HashMap<(&str, ProviderKind), (Tokens, Option<f64>)> = HashMap::new();
    let mut days: BTreeMap<NaiveDate, (u64, f64)> = BTreeMap::new();

    for b in scan.buckets.iter().filter(|b| b.hour >= lo && b.hour <= hi) {
        let price = *price_cache
            .entry(b.model.as_str())
            .or_insert_with(|| prices.price(&b.model));
        let c = price.map(|p| cost_parts(&b.tokens, &p));
        let usd = c.map_or(0.0, |c| c.total());
        let s = slot(b.provider);
        tokens.add(&b.tokens);
        prov[s].0.add(&b.tokens);
        prov[s].1 += usd;
        if let Some(c) = c {
            cost.add(&c);
        }
        if let Some(p) = price {
            savings += cache_savings(&b.tokens, &p);
        }
        let m = models
            .entry((b.model.as_str(), b.provider))
            .or_insert((Tokens::default(), price.map(|_| 0.0)));
        m.0.add(&b.tokens);
        if let Some(x) = m.1.as_mut() {
            *x += usd;
        }
        let d = days.entry(hour_date(b.hour)).or_default();
        d.0 += b.tokens.processed();
        d.1 += usd;
        let idx = ((b.hour - first) / step) as usize;
        if let Some(p) = points.get_mut(idx) {
            p.tokens[s] += b.tokens.processed();
            p.cost[s] += usd;
        }
    }

    let mut sess = [0usize; 2];
    for s in &scan.sessions {
        if s.hours.iter().any(|h| *h >= lo && *h <= hi) {
            sess[slot(s.provider)] += 1;
        }
    }
    let providers = [ProviderKind::Codex, ProviderKind::Claude]
        .into_iter()
        .map(|p| ProviderSummary {
            provider: p,
            sessions: sess[slot(p)],
            tokens: prov[slot(p)].0,
            cost: prov[slot(p)].1,
        })
        .filter(|p| p.sessions > 0 || !p.tokens.is_zero())
        .collect();
    let mut models: Vec<ModelRow> = models
        .into_iter()
        .map(|((model, provider), (tokens, cost))| ModelRow {
            model: model.to_string(),
            provider,
            tokens,
            cost,
        })
        .collect();
    models.sort_by(|a, b| {
        b.cost
            .unwrap_or(0.0)
            .total_cmp(&a.cost.unwrap_or(0.0))
            .then(b.tokens.processed().cmp(&a.tokens.processed()))
            .then(a.model.cmp(&b.model))
    });
    Summary {
        range,
        sessions: sess[0] + sess[1],
        tokens,
        cost,
        cache_savings: savings,
        providers,
        points,
        models,
        days: days
            .into_iter()
            .rev()
            .map(|(day, (tokens, cost))| DayRow { day, tokens, cost })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::scan::{Bucket, ScanSession};
    use super::*;

    fn at(y: i32, m: u32, d: u32, h: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, 0, 0)
            .unwrap()
    }

    fn bucket(t: NaiveDateTime, provider: ProviderKind, model: &str, tokens: Tokens) -> Bucket {
        Bucket {
            hour: hour_index(t),
            provider,
            model: model.into(),
            tokens,
        }
    }

    fn sample() -> Scan {
        let opus = Tokens {
            input: 1_000_000,
            cache_read: 10_000_000,
            cache_write: 1_000_000,
            output: 100_000,
        };
        let unknown = Tokens {
            input: 500,
            cache_read: 0,
            cache_write: 0,
            output: 500,
        };
        Scan {
            buckets: vec![
                bucket(
                    at(2026, 11, 7, 9),
                    ProviderKind::Claude,
                    "claude-opus-5-5",
                    opus,
                ),
                bucket(
                    at(2026, 11, 6, 9),
                    ProviderKind::Claude,
                    "claude-opus-5-5",
                    opus,
                ),
                bucket(
                    at(2026, 11, 7, 8),
                    ProviderKind::Codex,
                    "codex-auto-review",
                    unknown,
                ),
                bucket(
                    at(2026, 8, 20, 8),
                    ProviderKind::Claude,
                    "claude-opus-5-5",
                    opus,
                ),
            ],
            sessions: vec![
                ScanSession {
                    provider: ProviderKind::Claude,
                    id: "a".into(),
                    hours: vec![
                        hour_index(at(2026, 11, 7, 9)),
                        hour_index(at(2026, 11, 6, 9)),
                    ],
                },
                ScanSession {
                    provider: ProviderKind::Codex,
                    id: "b".into(),
                    hours: vec![hour_index(at(2026, 11, 7, 8))],
                },
                ScanSession {
                    provider: ProviderKind::Claude,
                    id: "old".into(),
                    hours: vec![hour_index(at(2026, 8, 20, 8))],
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn ranges_cost_unpriced_and_savings() {
        let scan = sample();
        let now = at(2026, 11, 7, 12);
        let none = PriceBook::default();

        let day = summarize(&scan, Range::Day, now, &none);
        assert_eq!(day.points.len(), 24);
        assert_eq!(day.sessions, 2);
        // One opus bucket: 1M*4 + 10M*0.2 + 1M*5 + 0.1M*20 = 4 + 2 + 5 + 2.
        assert!((day.cost.total() - 13.0).abs() < 1e-9);
        // 10M cached * (4 - 0.2) = 38.
        assert!((day.cache_savings - 38.0).abs() < 1e-9);
        let unpriced = day
            .models
            .iter()
            .find(|m| m.model == "codex-auto-review")
            .unwrap();
        assert_eq!(unpriced.cost, None);
        assert_eq!(day.tokens.processed(), 12_100_000 + 1000);
        // Codex first.
        assert_eq!(day.providers[0].provider, ProviderKind::Codex);

        let week = summarize(&scan, Range::Week, now, &none);
        assert_eq!(week.points.len(), 7);
        assert_eq!(week.sessions, 2);
        assert!((week.cost.total() - 26.0).abs() < 1e-9);
        assert_eq!(week.days.len(), 2);
        assert_eq!(week.days[0].day, now.date());

        let quarter = summarize(&scan, Range::Quarter, now, &none);
        assert_eq!(quarter.points.len(), 90);
        assert_eq!(quarter.sessions, 3);
        // Models sort by cost, unpriced last.
        assert_eq!(quarter.models[0].model, "claude-opus-5-5");

        // A fetched price for the unknown model wins and the totals follow.
        let mut o = PriceBook::default();
        o.prices.insert(
            "codex-auto-review".to_string(),
            ModelPrice {
                input: 1000.0,
                output: 1000.0,
                cache_write: 0.0,
                cache_read: 0.0,
            },
        );
        let priced = summarize(&scan, Range::Day, now, &o);
        assert!((priced.cost.total() - 13.0 - 1.0).abs() < 1e-9);
    }
}
