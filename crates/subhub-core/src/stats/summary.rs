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

    /// Label of the range switch.
    pub fn short(self) -> &'static str {
        match self {
            Range::Day => "24h",
            Range::Week => "7d",
            Range::Month => "30d",
            Range::Quarter => "90d",
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

/// One chart point (an hour in the 24h range, otherwise a day); index by [`slot`] (0 = Claude, 1 = Codex, 2 = Copilot).
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    /// Short axis label ("Nov 7", "14:00").
    pub label: String,
    /// Tooltip title ("Sat, Nov 7", "Nov 7, 14:00").
    pub title: String,
    /// Work tokens (input + output + cache writes).
    pub work: [u64; SLOTS],
    /// Everything processed, cache reads included.
    pub tokens: [u64; SLOTS],
    pub cost: [f64; SLOTS],
}

impl Point {
    pub fn has_data(&self) -> bool {
        self.tokens.iter().any(|t| *t > 0)
    }
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
    /// Work tokens (input + output + cache writes).
    pub work: u64,
    /// Everything processed, cache reads included.
    pub tokens: u64,
    pub cost: f64,
    /// Sessions with activity on this day (a session spanning days counts on each).
    pub sessions: usize,
}

/// The current calendar month so far and its linear extrapolation (the run-rate).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonthPace {
    /// First day of the month.
    pub month: NaiveDate,
    /// API estimate from the 1st until now.
    pub cost_so_far: f64,
    /// `cost_so_far` scaled linearly to the end of the month; None until a full day has passed
    /// or while nothing has been spent.
    pub projected: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub range: Range,
    pub sessions: usize,
    pub tokens: Tokens,
    pub cost: CostParts,
    /// What the cached input would have cost uncached, minus its cache-read cost (priced models).
    pub cache_savings: f64,
    /// Cache reads as a share of all input read (uncached input + cache reads + cache writes);
    /// None when nothing was read.
    pub cache_hit_rate: Option<f64>,
    /// Models in the range without a price.
    pub unpriced_models: usize,
    /// Codex first, then Claude, then Copilot (the order of the usage board).
    pub providers: Vec<ProviderSummary>,
    /// Chart points. For day ranges they start at the first day with data (see `trimmed`).
    pub points: Vec<Point>,
    /// First day with data in the range (day ranges).
    pub first_day: Option<NaiveDate>,
    /// True when leading days without data were cut from `points`.
    pub trimmed: bool,
    pub models: Vec<ModelRow>,
    /// Newest first.
    pub days: Vec<DayRow>,
    /// Calendar month to date, independent of the range.
    pub month: MonthPace,
}

impl Summary {
    /// Work tokens of the range.
    pub fn work(&self) -> u64 {
        self.tokens.work()
    }
}

/// Providers with usage logs: Claude, Codex, Copilot CLI. The others never have buckets.
pub const SLOTS: usize = 3;

/// Chart slot of a provider (see [`Point`]).
pub fn slot(p: ProviderKind) -> usize {
    match p {
        ProviderKind::Claude => 0,
        ProviderKind::Copilot => 2,
        _ => 1,
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
            let day = hour_date(h);
            let (label, title) = match range {
                Range::Day => (
                    format!("{:02}:00", hour_of_day(h)),
                    format!("{}, {:02}:00", day_label(day), hour_of_day(h)),
                ),
                _ => (
                    day_label(day),
                    format!("{}, {}", day.format("%a"), day_label(day)),
                ),
            };
            Point {
                label,
                title,
                work: [0; SLOTS],
                tokens: [0; SLOTS],
                cost: [0.0; SLOTS],
            }
        })
        .collect();

    let mut price_cache: HashMap<&str, Option<ModelPrice>> = HashMap::new();
    let mut tokens = Tokens::default();
    let mut cost = CostParts::default();
    let mut savings = 0.0;
    let mut prov = [(Tokens::default(), 0.0); SLOTS];
    let mut models: HashMap<(&str, ProviderKind), (Tokens, Option<f64>)> = HashMap::new();
    // Per day: work tokens, processed tokens, cost.
    let mut days: BTreeMap<NaiveDate, (u64, u64, f64)> = BTreeMap::new();

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
        d.0 += b.tokens.work();
        d.1 += b.tokens.processed();
        d.2 += usd;
        let idx = ((b.hour - first) / step) as usize;
        if let Some(p) = points.get_mut(idx) {
            p.work[s] += b.tokens.work();
            p.tokens[s] += b.tokens.processed();
            p.cost[s] += usd;
        }
    }

    let mut sess = [0usize; SLOTS];
    let mut day_sessions: BTreeMap<NaiveDate, usize> = BTreeMap::new();
    for s in &scan.sessions {
        let in_range: Vec<NaiveDate> = s
            .hours
            .iter()
            .filter(|h| **h >= lo && **h <= hi)
            .map(|h| hour_date(*h))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        if !in_range.is_empty() {
            sess[slot(s.provider)] += 1;
            for d in in_range {
                *day_sessions.entry(d).or_default() += 1;
            }
        }
    }

    // Day ranges start at the first day with data; the 24h range keeps all its hours.
    let first_day = days.keys().next().copied();
    let mut trimmed = false;
    if range != Range::Day
        && let Some(i) = points.iter().position(Point::has_data)
        && i > 0
    {
        points.drain(..i);
        trimmed = true;
    }
    let providers = [
        ProviderKind::Codex,
        ProviderKind::Claude,
        ProviderKind::Copilot,
    ]
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
    let read = tokens.input + tokens.cache_read + tokens.cache_write;
    Summary {
        range,
        sessions: sess.iter().sum(),
        tokens,
        cost,
        cache_savings: savings,
        cache_hit_rate: (read > 0).then(|| tokens.cache_read as f64 / read as f64),
        unpriced_models: models.iter().filter(|m| m.cost.is_none()).count(),
        providers,
        points,
        first_day,
        trimmed,
        models,
        days: days
            .into_iter()
            .rev()
            .map(|(day, (work, tokens, cost))| DayRow {
                day,
                work,
                tokens,
                cost,
                sessions: day_sessions.get(&day).copied().unwrap_or(0),
            })
            .collect(),
        month: month_pace(scan, now, prices),
    }
}

/// Cost of the current calendar month up to `now` and its linear extrapolation to month end:
/// `cost so far * hours in month / hours elapsed` (the current hour counts as elapsed). No
/// projection before a full day has passed, or while nothing has been spent.
fn month_pace(scan: &Scan, now: NaiveDateTime, prices: &PriceBook) -> MonthPace {
    let month = now.date().with_day(1).unwrap_or(now.date());
    let next = if month.month() == 12 {
        NaiveDate::from_ymd_opt(month.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(month.year(), month.month() + 1, 1)
    }
    .unwrap_or(month);
    let month_hours = (next - month).num_days() * 24;
    let (lo, hi) = (day_index(month) * 24, hour_index(now));
    let mut cache: HashMap<&str, Option<ModelPrice>> = HashMap::new();
    let mut cost_so_far = 0.0;
    for b in scan.buckets.iter().filter(|b| b.hour >= lo && b.hour <= hi) {
        let price = *cache
            .entry(b.model.as_str())
            .or_insert_with(|| prices.price(&b.model));
        if let Some(p) = price {
            cost_so_far += cost_parts(&b.tokens, &p).total();
        }
    }
    let elapsed = hi - lo + 1;
    let projected = (elapsed >= 24 && cost_so_far > 0.0)
        .then(|| cost_so_far * month_hours as f64 / elapsed as f64);
    MonthPace {
        month,
        cost_so_far,
        projected,
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
            kind: super::super::TaskKind::Talking,
            sidechain: false,
            cwd: None,
            thread: "t".into(),
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
        // Trimmed to the first day with data (Nov 6).
        assert_eq!(week.points.len(), 2);
        assert_eq!(week.sessions, 2);
        assert!((week.cost.total() - 26.0).abs() < 1e-9);
        assert_eq!(week.days.len(), 2);
        assert_eq!(week.days[0].day, now.date());

        let quarter = summarize(&scan, Range::Quarter, now, &none);
        assert_eq!(quarter.points.len(), 80);
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

    #[test]
    fn copilot_has_its_own_slot_provider_and_model_row() {
        let now = at(2026, 11, 7, 12);
        let t = Tokens {
            input: 1_000_000,
            output: 100_000,
            ..Tokens::default()
        };
        let scan = Scan {
            buckets: vec![
                bucket(
                    at(2026, 11, 7, 9),
                    ProviderKind::Copilot,
                    "claude-sonnet-4.5",
                    t,
                ),
                bucket(
                    at(2026, 11, 7, 9),
                    ProviderKind::Claude,
                    "claude-sonnet-4-5",
                    t,
                ),
            ],
            sessions: vec![ScanSession {
                provider: ProviderKind::Copilot,
                id: "c1".into(),
                hours: vec![hour_index(at(2026, 11, 7, 9))],
            }],
            ..Scan::default()
        };
        let s = summarize(&scan, Range::Week, now, &PriceBook::default());
        let last = s.points.last().unwrap();
        assert_eq!(last.work, [1_100_000, 0, 1_100_000]);
        // The dotted Copilot id finds the built-in Sonnet 4.5 price: 3 + 1.5 USD per 1M / 0.1M.
        assert!((last.cost[2] - 4.5).abs() < 1e-9);
        assert_eq!(s.unpriced_models, 0);
        let copilot = s
            .providers
            .iter()
            .find(|p| p.provider == ProviderKind::Copilot)
            .unwrap();
        assert_eq!((copilot.sessions, copilot.tokens.work()), (1, 1_100_000));
        assert!(
            s.models
                .iter()
                .any(|m| m.provider == ProviderKind::Copilot && m.model == "claude-sonnet-4.5")
        );
        assert_eq!(s.sessions, 1);
    }

    #[test]
    fn work_tokens_per_provider_day_model_and_type() {
        let s = summarize(
            &sample(),
            Range::Week,
            at(2026, 11, 7, 12),
            &PriceBook::default(),
        );
        // Work = input + output + cache writes; cache reads are only in "processed".
        assert_eq!(s.work(), 2 * 2_100_000 + 1000);
        assert_eq!(s.tokens.processed(), 2 * 12_100_000 + 1000);
        assert_eq!(s.tokens.cache_read, 20_000_000);
        // Cache hit rate: cache reads / (uncached input + cache reads + cache writes).
        let read = 2_000_500.0 + 20_000_000.0 + 2_000_000.0;
        assert!((s.cache_hit_rate.unwrap() - 20_000_000.0 / read).abs() < 1e-12);

        let claude = s
            .providers
            .iter()
            .find(|p| p.provider == ProviderKind::Claude)
            .unwrap();
        assert_eq!(claude.tokens.work(), 4_200_000);
        assert_eq!(claude.sessions, 1);
        assert!((claude.cost - 26.0).abs() < 1e-9);

        // Per model: work, incl. cache and cost; the unknown model is Unpriced.
        let opus = s
            .models
            .iter()
            .find(|m| m.model == "claude-opus-5-5")
            .unwrap();
        assert_eq!(opus.tokens.work(), 4_200_000);
        assert_eq!(opus.tokens.processed(), 24_200_000);
        assert!((opus.cost.unwrap() - 26.0).abs() < 1e-9);
        assert_eq!(s.unpriced_models, 1);

        // Per type: tokens and cost (opus: 2M in * 4, 20M cache read * 0.2, 2M write * 5, 0.2M out * 20).
        assert!((s.cost.input - 8.0).abs() < 1e-9);
        assert!((s.cost.cache_read - 4.0).abs() < 1e-9);
        assert!((s.cost.cache_write - 10.0).abs() < 1e-9);
        assert!((s.cost.output - 4.0).abs() < 1e-9);

        // Days newest first, with work, incl. cache and the sessions active on each day.
        assert_eq!(s.days.len(), 2);
        assert_eq!(s.days[0].day, NaiveDate::from_ymd_opt(2026, 11, 7).unwrap());
        assert_eq!(s.days[0].work, 2_100_000 + 1000);
        assert_eq!(s.days[0].tokens, 12_100_000 + 1000);
        assert_eq!(s.days[0].sessions, 2);
        assert_eq!(s.days[1].sessions, 1);

        // Chart points carry work and cost per provider (0 = Claude, 1 = Codex, 2 = Copilot).
        assert_eq!(s.points.len(), 2);
        assert_eq!(s.points[1].work, [2_100_000, 1000, 0]);
        assert_eq!(s.points[1].tokens, [12_100_000, 1000, 0]);
        assert!((s.points[0].cost[0] - 13.0).abs() < 1e-9);
        assert_eq!(s.points[0].title, "Fri, Nov 6");
    }

    #[test]
    fn chart_starts_at_the_first_day_with_data() {
        let scan = sample();
        let now = at(2026, 11, 7, 12);
        let none = PriceBook::default();

        let q = summarize(&scan, Range::Quarter, now, &none);
        let first = NaiveDate::from_ymd_opt(2026, 8, 20).unwrap();
        assert_eq!(q.first_day, Some(first));
        assert!(q.trimmed);
        assert_eq!(q.points.len() as i64, (now.date() - first).num_days() + 1);
        assert_eq!(q.points[0].label, "Aug 20");
        assert!(q.points[0].has_data());

        // A range that already starts with data is not trimmed.
        let m = summarize(&scan, Range::Month, at(2026, 8, 25, 12), &none);
        assert!(!m.trimmed || m.points[0].has_data());

        // The 24h range keeps all 24 hours.
        let d = summarize(&scan, Range::Day, now, &none);
        assert_eq!(d.points.len(), 24);
        assert!(!d.trimmed);

        // Nothing in the range: nothing to trim, no first day.
        let empty = summarize(&Scan::default(), Range::Quarter, now, &none);
        assert_eq!(empty.points.len(), 90);
        assert_eq!(empty.first_day, None);
        assert_eq!(empty.cache_hit_rate, None);
    }

    #[test]
    fn run_rate_extrapolates_the_month_to_date_linearly() {
        let scan = sample();
        let none = PriceBook::default();
        // Nov 1 - Nov 7 12:00: two opus buckets at 13.00 each; the August one is outside.
        let s = summarize(&scan, Range::Day, at(2026, 11, 7, 12), &none);
        assert_eq!(s.month.month, NaiveDate::from_ymd_opt(2026, 11, 1).unwrap());
        assert!((s.month.cost_so_far - 26.0).abs() < 1e-9);
        // 6 full days + hours 0..=12 of the 7th = 157 h of 30 * 24 = 720 h.
        let expected = 26.0 * 720.0 / 157.0;
        assert!((s.month.projected.unwrap() - expected).abs() < 1e-9);

        // Independent of the range.
        let q = summarize(&scan, Range::Quarter, at(2026, 11, 7, 12), &none);
        assert_eq!(q.month, s.month);

        // Less than a day into the month: no projection yet.
        let mut first_day = Scan::default();
        first_day.buckets.push(bucket(
            at(2026, 8, 1, 9),
            ProviderKind::Claude,
            "claude-opus-5-5",
            Tokens {
                input: 1_000_000,
                ..Default::default()
            },
        ));
        let early = summarize(&first_day, Range::Day, at(2026, 8, 1, 10), &none);
        assert!(early.month.cost_so_far > 0.0);
        assert_eq!(early.month.projected, None);

        // Nothing spent: no projection.
        let none_spent = summarize(&scan, Range::Day, at(2026, 9, 15, 12), &none);
        assert_eq!(none_spent.month.cost_so_far, 0.0);
        assert_eq!(none_spent.month.projected, None);

        // December has 31 days and rolls into the next year.
        let mut dec = Scan::default();
        dec.buckets.push(bucket(
            at(2026, 12, 1, 5),
            ProviderKind::Claude,
            "claude-opus-5-5",
            Tokens {
                input: 1_000_000,
                ..Default::default()
            },
        ));
        let d = summarize(&dec, Range::Week, at(2026, 12, 3, 23), &none);
        // 2 full days + 24 h = 72 h elapsed of 744 h; 1M input * 4 = 4.
        assert!((d.month.projected.unwrap() - 4.0 * 744.0 / 72.0).abs() < 1e-9);
    }
}
