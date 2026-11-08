//! Statistics -> Overview analysis (plan `docs/features/overview.md`, owner 2026-11-07): plan fit,
//! 5-hour windows and warm-up, renewals, where usage goes, and tips. Pure: everything comes in
//! through [`Input`] (reading history, the stats scan, settings, plans, prices, `now` and the
//! time zone), so it is tested without a clock and the app runs [`analyze`] off the UI thread.
//!
//! Results that need more data than there is are [`Collect::Collecting`] with the day the first
//! result is expected, never a guess. Benched hub logins are shown but never count as capacity
//! (header numbers, "another account was free", what-if).
//!
//! Layout: `zone` (local time), `readings` (history -> cycles and windows), `fit` (plan fit,
//! tiers, prices, what-if), `rhythm` (work rhythm, warm-up suggestion and simulation), `usage`
//! (task kind / project / thread / model shares), `tips`.
mod fit;
mod readings;
mod rhythm;
mod tips;
mod usage;
mod zone;

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use uuid::Uuid;

use crate::billing;
use crate::history::HistoryLine;
use crate::limits::{window_duration, window_title};
use crate::model::{Account, ProviderKind};
use crate::settings::Settings;
use crate::stats::{PriceBook, Scan, day_index, hour_date, hour_index, hour_of_day};
use crate::steer::{is_session_window, is_weekly_window};

pub use fit::{
    AT_LIMIT_HITS, AT_LIMIT_PEAK, DOWNGRADE_HITS, DOWNGRADE_PEAK, FIT_MIN_CYCLES, FitVerdict,
    Member, PLAN_PRICES_AS_OF, PlanFit, PlanTier, UNUSED_AVG, WhatIf, WhatIfChange, WhatIfShare,
    plan_fit, what_if,
};
pub use readings::{
    AccountObs, CYCLE_MIN_DAYS, END_SEEN_HOURS, FREE_NEIGHBOUR_MIN, FREE_SESSION_USED,
    FREE_WEEKLY_LEFT, HIT_PERCENT, Obs, SAME_RESET, SessionWindow, WeeklyCycle, WindowEnd,
    by_account, cycles, free_at, windows,
};
pub use rhythm::{
    DayStart, ExampleDay, FirstWindow, MAX_BEFORE_START_H, RECENT_DAYS, RECENT_HITS, RHYTHM_DAYS,
    ROUND_MIN, Rhythm, SimDay, Simulation, Span, example_day, first_accounts, is_workday, rhythm,
    simulate, suggest_time,
};
pub use tips::{
    ENDING_DAYS, EXPIRING_HOURS, EXPIRING_LEFT, TASK_INSIGHT_SHARE, Tip, TipAction, TipKind, TipRow,
};
pub use usage::{MAX_THREADS, ProjectKey, Share, ThreadShare, UsageReport, project_of, usage};
pub use zone::Zone;

/// Workdays with 5-hour windows the window analysis needs.
pub const WINDOW_MIN_WORKDAYS: usize = 5;
/// Plan fit looks back at least this far (a 7-day range holds only one cycle).
pub const FIT_LOOKBACK_DAYS: i64 = 28;

/// The Overview's range switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsightRange {
    Week,
    FourWeeks,
    Quarter,
}

impl InsightRange {
    pub const ALL: [InsightRange; 3] = [
        InsightRange::Week,
        InsightRange::FourWeeks,
        InsightRange::Quarter,
    ];

    pub fn days(self) -> i64 {
        match self {
            InsightRange::Week => 7,
            InsightRange::FourWeeks => 28,
            InsightRange::Quarter => 90,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            InsightRange::Week => "7 days",
            InsightRange::FourWeeks => "4 weeks",
            InsightRange::Quarter => "3 months",
        }
    }
}

/// A result, or the local day the first result is expected.
#[derive(Debug, Clone, PartialEq)]
pub enum Collect<T> {
    Ready(T),
    Collecting { ready_on: NaiveDate },
}

impl<T> Collect<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Collect::Ready(t) => Some(t),
            Collect::Collecting { .. } => None,
        }
    }

    pub fn ready_on(&self) -> Option<NaiveDate> {
        match self {
            Collect::Ready(_) => None,
            Collect::Collecting { ready_on } => Some(*ready_on),
        }
    }
}

/// Everything the analysis reads. `plans` are the plan labels of the last readings
/// (`UsageResult::plan`, "Max 20x", "Plus", ...); accounts without one have an unknown plan.
pub struct Input<'a> {
    pub history: &'a [HistoryLine],
    /// None while the first scan runs (Where usage goes is then empty).
    pub scan: Option<&'a Scan>,
    pub settings: &'a Settings,
    pub plans: &'a HashMap<Uuid, String>,
    pub prices: &'a PriceBook,
    pub range: InsightRange,
    pub now: DateTime<Utc>,
    pub zone: &'a Zone,
}

/// A window's latest reading (the "now" values, independent of the range).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NowWindow {
    pub used: f64,
    pub resets_at: Option<DateTime<Utc>>,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccountInsight {
    pub account: Uuid,
    pub provider: ProviderKind,
    pub label: String,
    pub plan: Option<String>,
    pub tier: Option<PlanTier>,
    pub benched: bool,
    /// A hub login without a CLI profile (warm-up cannot run it in v1).
    pub hub_only: bool,
    pub now_session: Option<NowWindow>,
    pub now_weekly: Option<NowWindow>,
    /// Ever seen a 5-hour window (Codex plans can have only a weekly one).
    pub has_session_window: bool,
    /// Weekly cycles that ended in or still run in the range, oldest first.
    pub cycles: Vec<WeeklyCycle>,
    /// Average and peak of the counted cycles in the range.
    pub weekly_avg: Option<f64>,
    pub weekly_peak: Option<f64>,
    /// 5-hour windows in the range, oldest first.
    pub windows: Vec<SessionWindow>,
    pub hits: usize,
    pub hits_per_week: f64,
    pub waited: Duration,
    pub fit: Collect<PlanFit>,
    /// For an ending plan or a suggested downgrade.
    pub what_if: Option<WhatIf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WarmupSuggestion {
    /// Local time of day.
    pub at: NaiveTime,
    /// Accounts whose window usually comes first on the hit days, most often first.
    pub accounts: Vec<Uuid>,
    pub simulation: Simulation,
    /// A past day drawn as it went and with the warm-up.
    pub example: Option<ExampleDay>,
}

/// The last 7 days of one account's windows, for the week timeline.
#[derive(Debug, Clone, PartialEq)]
pub struct WeekLane {
    pub account: Uuid,
    pub spans: Vec<Span>,
}

/// Days the week timeline shows (today included).
pub const WEEK_DAYS: i64 = 7;

#[derive(Debug, Clone, PartialEq)]
pub struct WindowsReport {
    /// In the range, over accounts that count (not benched).
    pub hits: usize,
    pub hits_per_week: f64,
    pub waited: Duration,
    /// Hits while another account of the provider was free.
    pub hits_with_free_account: usize,
    /// Workdays with windows in the whole history.
    pub workdays: usize,
    pub rhythm: Rhythm,
    pub warmup: Option<WarmupSuggestion>,
    /// Local midnight [`WEEK_DAYS`] - 1 days ago and local now: the week timeline's ends.
    pub week_from: NaiveDateTime,
    pub week_to: NaiveDateTime,
    /// Accounts that count and have 5-hour windows, in settings order.
    pub week: Vec<WeekLane>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Renewal {
    pub account: Uuid,
    pub date: NaiveDate,
    /// Days from today (negative: an access end date that passed).
    pub days: i64,
    pub auto_renew: bool,
    pub estimate: bool,
    pub reminders: bool,
}

/// Input for the verdict sentence; the UI words it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Verdict {
    /// Average weekly use over counted accounts, 0..=1.
    pub used_share: Option<f64>,
    pub unused: Vec<Uuid>,
    pub at_limit: Vec<Uuid>,
    pub downgrade: Vec<Uuid>,
    /// No plan fit is ready yet: when the first one is expected.
    pub collecting_until: Option<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub verdict: Verdict,
    /// Weekly limits used: average of the accounts' averages and the highest peak (%).
    pub weekly_avg: Option<f64>,
    pub weekly_peak: Option<f64>,
    pub hits_per_week: f64,
    pub waited: Duration,
    /// The soonest renewal or end today or later.
    pub next_renewal: Option<Renewal>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Overview {
    pub now: DateTime<Utc>,
    pub today: NaiveDate,
    pub range: InsightRange,
    pub header: Header,
    /// In settings order.
    pub accounts: Vec<AccountInsight>,
    pub windows: Collect<WindowsReport>,
    /// By date.
    pub renewals: Vec<Renewal>,
    pub usage: UsageReport,
    /// Highest priority first.
    pub tips: Vec<Tip>,
}

impl Overview {
    pub fn account(&self, id: Uuid) -> Option<&AccountInsight> {
        self.accounts.iter().find(|a| a.account == id)
    }
}

fn is_benched(a: &Account) -> bool {
    a.hub.as_ref().is_some_and(|h| h.benched)
}

/// The latest reading's 5-hour and weekly windows (Codex keys by their label's length).
pub fn now_windows(settings: &Settings, id: Uuid) -> (Option<NowWindow>, Option<NowWindow>) {
    let Some(r) = settings.readings.get(&id) else {
        return (None, None);
    };
    let (mut s, mut w) = (None, None);
    for x in &r.windows {
        let len = window_duration(&window_title(&x.label));
        let codex = x.key.starts_with("codex:");
        let nw = NowWindow {
            used: x.used_percent.clamp(0.0, 100.0),
            resets_at: x.resets_at,
            fetched_at: r.fetched_at,
        };
        if s.is_none() && (is_session_window(&x.key) || (codex && len == Some(Duration::hours(5))))
        {
            s = Some(nw);
        } else if w.is_none()
            && (is_weekly_window(&x.key) || (codex && len == Some(Duration::days(7))))
        {
            w = Some(nw);
        }
    }
    (s, w)
}

/// When a plan fit with `have` counted cycles will have [`FIT_MIN_CYCLES`]: the running cycle
/// counts if it can still be read on 3 days, then one more per week.
fn fit_ready_on(have: usize, all: &[WeeklyCycle], now: DateTime<Utc>, zone: &Zone) -> NaiveDate {
    let need = FIT_MIN_CYCLES.saturating_sub(have).max(1) as i32;
    let end = match all.iter().find(|c| !c.ended) {
        Some(c) => {
            let today_idx = (now - c.start).num_days().clamp(0, 6) as usize;
            let possible = c.days + (6 - today_idx);
            let first = if possible >= CYCLE_MIN_DAYS {
                c.resets_at
            } else {
                c.resets_at + Duration::days(7)
            };
            first + Duration::days(7 * (need as i64 - 1))
        }
        None => now + Duration::days(7 * need as i64),
    };
    zone.local(end).date()
}

/// The day the `need`-th further workday is reached (today counts when it has no window yet).
fn workdays_ready_on(need: usize, today: NaiveDate, today_counted: bool) -> NaiveDate {
    let mut d = today;
    let mut left = need.max(1);
    if today_counted || !is_workday(d) {
        d = d.succ_opt().unwrap_or(d);
    }
    loop {
        if is_workday(d) {
            left -= 1;
            if left == 0 {
                return d;
            }
        }
        d = d.succ_opt().unwrap_or(d);
    }
}

/// Runs the whole analysis.
pub fn analyze(input: &Input) -> Overview {
    let Input {
        history,
        scan,
        settings,
        plans,
        prices,
        range,
        now,
        zone,
    } = *input;
    let local_now = zone.local(now);
    let today = local_now.date();
    let range_start = now - Duration::days(range.days());
    let lookback = now - Duration::days(range.days().max(FIT_LOOKBACK_DAYS));
    let obs = by_account(history);
    let empty = AccountObs::default();

    // Per account: cycles and windows from the history.
    let mut all_windows: HashMap<Uuid, Vec<SessionWindow>> = HashMap::new();
    let mut all_cycles: HashMap<Uuid, Vec<WeeklyCycle>> = HashMap::new();
    for a in &settings.accounts {
        let o = obs.get(&a.id).unwrap_or(&empty);
        all_windows.insert(a.id, windows(a.id, &o.session, now));
        all_cycles.insert(a.id, cycles(&o.weekly, now));
    }
    // Another account was free at each hit.
    for a in &settings.accounts {
        let others: Vec<&Account> = settings
            .accounts
            .iter()
            .filter(|b| b.id != a.id && b.provider == a.provider && !is_benched(b))
            .collect();
        let ws = all_windows.get_mut(&a.id).expect("inserted");
        for w in ws.iter_mut() {
            if let Some(h) = w.hit_at {
                w.free_elsewhere = others
                    .iter()
                    .filter(|b| free_at(obs.get(&b.id).unwrap_or(&empty), h))
                    .map(|b| b.id)
                    .collect();
            }
        }
    }

    let mut accounts: Vec<AccountInsight> = vec![];
    for a in &settings.accounts {
        let o = obs.get(&a.id).unwrap_or(&empty);
        let plan = plans.get(&a.id).cloned();
        let tier = plan
            .as_deref()
            .and_then(|p| PlanTier::from_plan(a.provider, p));
        let cs = &all_cycles[&a.id];
        let ws = &all_windows[&a.id];
        let in_range: Vec<SessionWindow> = ws
            .iter()
            .filter(|w| w.resets_at > range_start)
            .cloned()
            .collect();
        let range_cycles: Vec<WeeklyCycle> = cs
            .iter()
            .filter(|c| c.resets_at > range_start)
            .cloned()
            .collect();
        let counted: Vec<&WeeklyCycle> = range_cycles.iter().filter(|c| c.counted).collect();
        let weekly_avg = (!counted.is_empty())
            .then(|| counted.iter().map(|c| c.peak).sum::<f64>() / counted.len() as f64);
        let weekly_peak = counted.iter().map(|c| c.peak).reduce(f64::max);
        let hits = in_range.iter().filter(|w| w.hit_at.is_some()).count();
        let waited = in_range.iter().filter_map(|w| w.waited).sum();
        let covered_from = o.first_at.map_or(now, |f| f.max(range_start));
        let weeks =
            ((now - covered_from).num_minutes() as f64 / (7.0 * 24.0 * 60.0)).max(1.0 / 7.0);
        let fit_cycles: Vec<&WeeklyCycle> = cs.iter().filter(|c| c.resets_at > lookback).collect();
        let fit = match plan_fit(tier, &fit_cycles, ws) {
            Some(f) => Collect::Ready(f),
            None => {
                let have = fit_cycles.iter().filter(|c| c.counted).count();
                Collect::Collecting {
                    ready_on: fit_ready_on(have, cs, now, zone),
                }
            }
        };
        let (now_session, now_weekly) = now_windows(settings, a.id);
        accounts.push(AccountInsight {
            account: a.id,
            provider: a.provider,
            label: a.label.clone(),
            plan,
            tier,
            benched: is_benched(a),
            hub_only: a.profile_dir.is_none() && a.hub.is_some(),
            now_session,
            now_weekly,
            has_session_window: o.has_session,
            cycles: range_cycles,
            weekly_avg,
            weekly_peak,
            hits,
            hits_per_week: hits as f64 / weeks,
            waited,
            windows: in_range,
            fit,
            what_if: None,
        });
    }

    let renewals = renewals(settings, today);
    add_what_ifs(&mut accounts, &renewals, zone);

    // 5-hour windows over the accounts that count.
    let counting: Vec<&AccountInsight> = accounts.iter().filter(|a| !a.benched).collect();
    let mut workdays: Vec<NaiveDate> = counting
        .iter()
        .flat_map(|a| all_windows[&a.account].iter())
        .map(|w| zone.local(w.start).date())
        .filter(|d| is_workday(*d) && *d <= today)
        .collect();
    workdays.sort();
    workdays.dedup();
    let windows_report = if workdays.len() >= WINDOW_MIN_WORKDAYS {
        let refs: Vec<&SessionWindow> = counting
            .iter()
            .flat_map(|a| all_windows[&a.account].iter())
            .collect();
        let logs = scan.map(log_starts).unwrap_or_default();
        let r = rhythm(&refs, &logs, zone, today);
        let warmup = suggest_time(&r).map(|at| {
            let of = |id: Uuid| {
                all_windows
                    .get(&id)
                    .map(|ws| ws.iter().map(|w| (w.start, w.resets_at)).collect())
                    .unwrap_or_default()
            };
            let simulation = simulate(at, &r, &of, zone);
            let ws_of = |id: Uuid| all_windows.get(&id).cloned().unwrap_or_default();
            WarmupSuggestion {
                at,
                accounts: first_accounts(&r),
                example: example_day(at, &r, &simulation, &ws_of, zone),
                simulation,
            }
        });
        let week_from = (today - Duration::days(WEEK_DAYS - 1)).and_time(NaiveTime::MIN);
        let week = week_lanes(&counting, &all_windows, week_from, zone);
        let hits: usize = counting.iter().map(|a| a.hits).sum();
        let weeks = range_weeks(&counting, &obs, range_start, now);
        Collect::Ready(WindowsReport {
            hits,
            hits_per_week: hits as f64 / weeks,
            waited: counting.iter().map(|a| a.waited).sum(),
            hits_with_free_account: counting
                .iter()
                .flat_map(|a| a.windows.iter())
                .filter(|w| w.hit_at.is_some() && !w.free_elsewhere.is_empty())
                .count(),
            workdays: workdays.len(),
            rhythm: r,
            warmup,
            week_from,
            week_to: local_now,
            week,
        })
    } else {
        Collect::Collecting {
            ready_on: workdays_ready_on(
                WINDOW_MIN_WORKDAYS - workdays.len(),
                today,
                workdays.last() == Some(&today),
            ),
        }
    };

    let usage_report = scan.map_or_else(UsageReport::default, |s| {
        let hi = hour_index(local_now);
        let lo = (day_index(today) - (range.days() - 1)) * 24;
        usage(s, lo, hi, &settings.projects, prices)
    });

    let header = header(&accounts, &obs, &renewals, range_start, now);
    let tips = tips::tips(&tips::TipInput {
        accounts: &accounts,
        settings,
        windows: windows_report.ready(),
        renewals: &renewals,
        usage: &usage_report,
        now,
    });
    Overview {
        now,
        today,
        range,
        header,
        accounts,
        windows: windows_report,
        renewals,
        usage: usage_report,
        tips,
    }
}

/// Windows that reach into the week timeline, per counting account with 5-hour windows.
fn week_lanes(
    counting: &[&AccountInsight],
    all_windows: &HashMap<Uuid, Vec<SessionWindow>>,
    from: NaiveDateTime,
    zone: &Zone,
) -> Vec<WeekLane> {
    counting
        .iter()
        .filter(|a| a.has_session_window)
        .map(|a| {
            let mut spans: Vec<Span> = all_windows[&a.account]
                .iter()
                .filter(|w| zone.local(w.resets_at) > from)
                .map(|w| Span {
                    start: zone.local(w.start),
                    reset: zone.local(w.resets_at),
                    hit: w.hit_at.map(|h| zone.local(h)),
                    warm: false,
                    unknown: w.end == WindowEnd::Unknown,
                })
                .collect();
            spans.sort_by_key(|s| s.start);
            WeekLane {
                account: a.account,
                spans,
            }
        })
        .collect()
}

/// Weeks of history the range covers for these accounts (at least a day).
fn range_weeks(
    accounts: &[&AccountInsight],
    obs: &BTreeMap<Uuid, AccountObs>,
    range_start: DateTime<Utc>,
    now: DateTime<Utc>,
) -> f64 {
    let first = accounts
        .iter()
        .filter_map(|a| obs.get(&a.account)?.first_at)
        .min()
        .map_or(now, |f| f.max(range_start));
    ((now - first).num_minutes() as f64 / (7.0 * 24.0 * 60.0)).max(1.0 / 7.0)
}

/// The first local hour with log activity per local day.
fn log_starts(scan: &Scan) -> BTreeMap<NaiveDate, NaiveTime> {
    let mut first: BTreeMap<NaiveDate, i64> = BTreeMap::new();
    for b in &scan.buckets {
        let e = first.entry(hour_date(b.hour)).or_insert(b.hour);
        *e = (*e).min(b.hour);
    }
    first
        .into_iter()
        .filter_map(|(d, h)| Some((d, NaiveTime::from_hms_opt(hour_of_day(h), 0, 0)?)))
        .collect()
}

fn renewals(settings: &Settings, today: NaiveDate) -> Vec<Renewal> {
    let mut v: Vec<Renewal> = settings
        .accounts
        .iter()
        .filter_map(|a| {
            let b = settings.billing.get(&a.id)?;
            let date = billing::next_date(b, today);
            Some(Renewal {
                account: a.id,
                date,
                days: (date - today).num_days(),
                auto_renew: b.auto_renew,
                estimate: billing::is_estimate(b, today),
                reminders: b.reminders,
            })
        })
        .collect();
    v.sort_by_key(|r| (r.date, r.account));
    v
}

/// What-ifs: an account whose access ends (today or later) moves all its use onto the provider's
/// other counted accounts; a suggested downgrade shows its own use on the smaller tier.
fn add_what_ifs(accounts: &mut [AccountInsight], renewals: &[Renewal], zone: &Zone) {
    let ending = |id: Uuid| {
        renewals
            .iter()
            .find(|r| r.account == id && !r.auto_renew && r.days >= 0)
    };
    let member = |a: &AccountInsight| {
        Some(Member {
            account: a.account,
            tier: a.tier?,
            avg: a.fit.ready()?.avg,
        })
    };
    let mut results: Vec<(usize, WhatIf)> = vec![];
    for (i, a) in accounts.iter().enumerate() {
        let Some(m) = member(a) else { continue };
        let change = if let Some(r) = ending(a.account) {
            let day = r.date.and_time(NaiveTime::MIN);
            WhatIfChange::Ends(zone.utc_of(day))
        } else if let Some(f) = a.fit.ready()
            && f.verdict == FitVerdict::Downgrade
            && let Some(t) = f.suggested
        {
            WhatIfChange::Tier(t)
        } else {
            continue;
        };
        let others: Vec<Member> = accounts
            .iter()
            .filter(|b| {
                b.account != a.account
                    && b.provider == a.provider
                    && !b.benched
                    && ending(b.account).is_none()
            })
            .filter_map(member)
            .collect();
        results.push((i, what_if(m, change, &others)));
    }
    for (i, w) in results {
        accounts[i].what_if = Some(w);
    }
}

fn header(
    accounts: &[AccountInsight],
    obs: &BTreeMap<Uuid, AccountObs>,
    renewals: &[Renewal],
    range_start: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Header {
    let counting: Vec<&AccountInsight> = accounts.iter().filter(|a| !a.benched).collect();
    let avgs: Vec<f64> = counting.iter().filter_map(|a| a.weekly_avg).collect();
    let weekly_avg = (!avgs.is_empty()).then(|| avgs.iter().sum::<f64>() / avgs.len() as f64);
    let hits: usize = counting.iter().map(|a| a.hits).sum();
    let ids = |f: FitVerdict| -> Vec<Uuid> {
        accounts
            .iter()
            .filter(|a| a.fit.ready().is_some_and(|x| x.verdict == f))
            .map(|a| a.account)
            .collect()
    };
    let any_ready = accounts.iter().any(|a| a.fit.ready().is_some());
    Header {
        verdict: Verdict {
            used_share: weekly_avg.map(|a| a / 100.0),
            unused: ids(FitVerdict::Unused),
            at_limit: ids(FitVerdict::AtLimit),
            downgrade: ids(FitVerdict::Downgrade),
            collecting_until: if any_ready {
                None
            } else {
                accounts.iter().filter_map(|a| a.fit.ready_on()).min()
            },
        },
        weekly_avg,
        weekly_peak: counting
            .iter()
            .filter_map(|a| a.weekly_peak)
            .reduce(f64::max),
        hits_per_week: hits as f64 / range_weeks(&counting, obs, range_start, now),
        waited: counting.iter().map(|a| a.waited).sum(),
        next_renewal: renewals.iter().find(|r| r.days >= 0).cloned(),
    }
}

#[cfg(test)]
mod tests;
