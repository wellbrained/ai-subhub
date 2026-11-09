//! Reading history -> weekly (or monthly) cycles and 5-hour windows per account (plan
//! "Analysis", owner 2026-11-07; monthly cycles for Copilot and Cursor's legacy plans, owner
//! 2026-11-09). Readings of one window are grouped by `resets_at`; providers move it by a few
//! seconds between readings, so a group takes every reset within [`SAME_RESET`] of its first one
//! (the plan says "minute precision"; a tolerance also survives a minute boundary).
use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Duration, Months, Utc};
use uuid::Uuid;

use crate::history::HistoryLine;
use crate::steer::{is_session_window, is_weekly_window};

/// Resets this close together are the same window.
pub const SAME_RESET: i64 = 10 * 60;
/// "Hit" = the first reading at or above this % used.
pub const HIT_PERCENT: f64 = 99.5;
/// A window without a reading in its last 2 hours has an unknown end (PC off).
pub const END_SEEN_HOURS: i64 = 2;
/// A weekly cycle counts once ended with readings on at least this many of its 7 days.
pub const CYCLE_MIN_DAYS: usize = 3;
/// A monthly cycle counts once ended with readings on at least this share of its days (18 of
/// 30): a month has room for a week away, but a verdict from a few days would be a guess.
pub const MONTH_MIN_DAY_SHARE: f64 = 0.6;
/// Cursor's plan cycle is weekly up to this many days, else monthly (as `cursor::cycle_label`).
pub const WEEKLY_MAX_DAYS: i64 = 10;

pub const SESSION_LEN_H: i64 = 5;
pub const WEEK_LEN_D: i64 = 7;

/// One reading of one window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Obs {
    pub at: DateTime<Utc>,
    pub used: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

impl Obs {
    /// % used as of the reading: a window whose reset already passed is empty.
    pub fn effective_used(&self) -> f64 {
        match self.resets_at {
            Some(r) if r <= self.at => 0.0,
            _ => self.used,
        }
    }
}

/// How long an account's main long window runs: a week (Claude, Codex, Antigravity, Cursor's
/// current plans) or a calendar month (Copilot, Cursor's legacy plans).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CycleLen {
    #[default]
    Week,
    Month,
}

impl CycleLen {
    /// Days of a cycle `len_days` long that need a reading for it to count.
    pub fn min_days(self, len_days: i64) -> usize {
        match self {
            CycleLen::Week => CYCLE_MIN_DAYS,
            CycleLen::Month => (len_days as f64 * MONTH_MIN_DAY_SHARE).ceil() as usize,
        }
    }

    /// Where a cycle that resets at `reset` started, when its previous reset was not seen.
    pub fn back(self, reset: DateTime<Utc>) -> DateTime<Utc> {
        match self {
            CycleLen::Week => reset - Duration::days(WEEK_LEN_D),
            CycleLen::Month => reset
                .checked_sub_months(Months::new(1))
                .unwrap_or(reset - Duration::days(30)),
        }
    }

    /// The reset one cycle after `reset`.
    pub fn next(self, reset: DateTime<Utc>) -> DateTime<Utc> {
        match self {
            CycleLen::Week => reset + Duration::days(WEEK_LEN_D),
            CycleLen::Month => reset
                .checked_add_months(Months::new(1))
                .unwrap_or(reset + Duration::days(30)),
        }
    }
}

/// An account's readings of its 5-hour and its main long window (the all-models weekly one,
/// Copilot's monthly credits, Cursor's plan total), oldest first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccountObs {
    pub session: Vec<Obs>,
    /// The main long window (named when it was always weekly).
    pub weekly: Vec<Obs>,
    /// Its length.
    pub cycle: CycleLen,
    /// Whether a 5-hour window was ever seen (Codex plans without one have none).
    pub has_session: bool,
    pub first_at: Option<DateTime<Utc>>,
}

/// What a history window key is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Session,
    Long(CycleLen),
}

/// What a history window key is: the 5-hour or the main long window. Codex keys
/// (`codex:primary` / `codex:secondary`) carry no length, so it is read from how far ahead their
/// resets lie: at most 5 h (+10 min) is a 5-hour window, a day or more is weekly; a Codex plan
/// whose primary window is weekly thus has no 5-hour window. Antigravity's first group
/// (`agy:5h`, `agy:weekly`) is its main pair, Copilot's credits (`copilot:premium`) are monthly,
/// Cursor's plan total (`cursor:plan`) is weekly or monthly ([`cursor_cycle`]). Per-model and
/// per-pool windows never count.
pub(crate) fn kind_of(key: &str, lines: &[&HistoryLine]) -> Option<Kind> {
    if is_session_window(key) || key == "agy:5h" {
        return Some(Kind::Session);
    }
    if is_weekly_window(key) || key == "agy:weekly" {
        return Some(Kind::Long(CycleLen::Week));
    }
    match key {
        "copilot:premium" => return Some(Kind::Long(CycleLen::Month)),
        "cursor:plan" => return Some(Kind::Long(cursor_cycle(lines))),
        "codex:primary" | "codex:secondary" => {}
        _ => return None,
    }
    let ahead = lines
        .iter()
        .filter_map(|l| Some(l.resets_at? - l.fetched_at))
        .max()?;
    if ahead <= Duration::minutes(SESSION_LEN_H * 60 + 10) {
        Some(Kind::Session)
    } else if ahead >= Duration::days(1) {
        Some(Kind::Long(CycleLen::Week))
    } else {
        None
    }
}

/// Cursor's plan cycle from its readings: monthly when a reset ever lay more than
/// [`WEEKLY_MAX_DAYS`] ahead or two resets in a row lie further apart than that (readings late
/// in a month alone look weekly), else weekly. `analyze` prefers the latest reading's label
/// ("Weekly" / "Monthly") when there is one.
fn cursor_cycle(lines: &[&HistoryLine]) -> CycleLen {
    let max = Duration::days(WEEKLY_MAX_DAYS);
    let far_ahead = lines
        .iter()
        .filter_map(|l| Some(l.resets_at? - l.fetched_at))
        .any(|d| d > max);
    let mut resets: Vec<DateTime<Utc>> = lines.iter().filter_map(|l| l.resets_at).collect();
    resets.sort();
    resets.dedup_by(|later, first| (*later - *first).num_seconds() <= SAME_RESET);
    let min_gap = resets.windows(2).map(|w| w[1] - w[0]).min();
    if far_ahead || min_gap.is_some_and(|g| g > max) {
        CycleLen::Month
    } else {
        CycleLen::Week
    }
}

/// Groups history lines per account into 5-hour and main long-window readings. Non-finite values
/// and other windows are skipped; used % is clamped to 0..=100.
pub fn by_account(lines: &[HistoryLine]) -> BTreeMap<Uuid, AccountObs> {
    let mut keys: BTreeMap<(Uuid, &str), Vec<&HistoryLine>> = BTreeMap::new();
    for l in lines.iter().filter(|l| l.used_percent.is_finite()) {
        keys.entry((l.account, l.window.as_str()))
            .or_default()
            .push(l);
    }
    let mut out: BTreeMap<Uuid, AccountObs> = BTreeMap::new();
    let mut taken: HashSet<(Uuid, bool)> = HashSet::new();
    for ((account, key), ls) in &keys {
        let Some(kind) = kind_of(key, ls) else {
            continue;
        };
        // One window per kind and account (the first key in order wins).
        if !taken.insert((*account, kind == Kind::Session)) {
            continue;
        }
        let a = out.entry(*account).or_default();
        let list = match kind {
            Kind::Session => {
                a.has_session = true;
                &mut a.session
            }
            Kind::Long(len) => {
                a.cycle = len;
                &mut a.weekly
            }
        };
        list.extend(ls.iter().map(|l| Obs {
            at: l.fetched_at,
            used: l.used_percent.clamp(0.0, 100.0),
            resets_at: l.resets_at,
        }));
    }
    for a in out.values_mut() {
        a.session.sort_by_key(|o| o.at);
        a.weekly.sort_by_key(|o| o.at);
        a.first_at = a
            .session
            .first()
            .into_iter()
            .chain(a.weekly.first())
            .map(|o| o.at)
            .min();
    }
    out
}

/// To the nearest minute.
fn round_minute(t: DateTime<Utc>) -> DateTime<Utc> {
    let s = t.timestamp();
    DateTime::from_timestamp((s + 30).div_euclid(60) * 60, 0).unwrap_or(t)
}

/// Readings of one window occurrence (same reset), oldest first.
fn group(obs: &[Obs]) -> Vec<(DateTime<Utc>, Vec<Obs>)> {
    let mut running: Vec<Obs> = obs
        .iter()
        .filter(|o| o.resets_at.is_some_and(|r| r > o.at))
        .copied()
        .collect();
    running.sort_by_key(|o| (o.resets_at, o.at));
    let mut out: Vec<(DateTime<Utc>, Vec<Obs>)> = vec![];
    let mut first: Option<DateTime<Utc>> = None;
    for o in running {
        let r = o.resets_at.expect("filtered");
        match first {
            Some(f) if (r - f).num_seconds() <= SAME_RESET => {
                out.last_mut().expect("group exists").1.push(o);
            }
            _ => {
                first = Some(r);
                out.push((r, vec![o]));
            }
        }
    }
    for g in &mut out {
        g.1.sort_by_key(|o| o.at);
        // The latest reading's reset, to the minute (the jitter is seconds).
        if let Some(r) = g.1.last().and_then(|o| o.resets_at) {
            g.0 = round_minute(r);
        }
    }
    out
}

/// One cycle of an account's main long window: weekly, or monthly for Copilot and Cursor's
/// legacy plans (the name is from when all were weekly).
#[derive(Debug, Clone, PartialEq)]
pub struct WeeklyCycle {
    pub start: DateTime<Utc>,
    pub resets_at: DateTime<Utc>,
    /// Highest % used seen: the cycle's use.
    pub peak: f64,
    /// Of the cycle's days, how many have a reading.
    pub days: usize,
    pub ended: bool,
    /// Ended and read on enough days ([`CycleLen::min_days`]).
    pub counted: bool,
}

/// Weekly cycles.
pub fn cycles(weekly: &[Obs], now: DateTime<Utc>) -> Vec<WeeklyCycle> {
    cycles_of(weekly, CycleLen::Week, now)
}

/// Cycles of a long window. A weekly cycle starts 7 days before its reset; a monthly one at the
/// previous reset when that was seen 20 to 35 days earlier (billing days move a little), else a
/// calendar month before.
pub fn cycles_of(obs: &[Obs], len: CycleLen, now: DateTime<Utc>) -> Vec<WeeklyCycle> {
    let mut prev: Option<DateTime<Utc>> = None;
    group(obs)
        .into_iter()
        .map(|(reset, obs)| {
            let start = match (len, prev) {
                (CycleLen::Month, Some(p))
                    if (Duration::days(20)..=Duration::days(35)).contains(&(reset - p)) =>
                {
                    p
                }
                _ => len.back(reset),
            };
            prev = Some(reset);
            let len_days = ((reset - start).num_seconds() as f64 / 86_400.0).round() as i64;
            let mut days: Vec<i64> = obs
                .iter()
                .map(|o| (o.at - start).num_seconds().div_euclid(86_400))
                .filter(|d| (0..len_days).contains(d))
                .collect();
            days.sort();
            days.dedup();
            let ended = reset <= now;
            WeeklyCycle {
                start,
                resets_at: reset,
                peak: obs.iter().map(|o| o.used).fold(0.0, f64::max),
                days: days.len(),
                ended,
                counted: ended && days.len() >= len.min_days(len_days),
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowEnd {
    /// Read during its last 2 hours.
    Seen,
    /// No reading in its last 2 hours (PC off): drawn as a gap, never counted as a hit.
    Unknown,
    /// Still running at `now`.
    Running,
}

/// One 5-hour window of an account.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionWindow {
    pub account: Uuid,
    pub start: DateTime<Utc>,
    pub resets_at: DateTime<Utc>,
    pub peak: f64,
    /// First reading at >= 99.5 % used (None for an unknown end).
    pub hit_at: Option<DateTime<Utc>>,
    /// Reset - hit.
    pub waited: Option<Duration>,
    pub end: WindowEnd,
    /// At the hit, other accounts of the provider that had room ([`free_at`]).
    pub free_elsewhere: Vec<Uuid>,
}

pub fn windows(account: Uuid, session: &[Obs], now: DateTime<Utc>) -> Vec<SessionWindow> {
    group(session)
        .into_iter()
        .map(|(reset, obs)| {
            let start = reset - Duration::hours(SESSION_LEN_H);
            let end = if reset > now {
                WindowEnd::Running
            } else if obs
                .iter()
                .any(|o| o.at >= reset - Duration::hours(END_SEEN_HOURS))
            {
                WindowEnd::Seen
            } else {
                WindowEnd::Unknown
            };
            let hit_at = (end != WindowEnd::Unknown)
                .then(|| obs.iter().find(|o| o.used >= HIT_PERCENT).map(|o| o.at))
                .flatten();
            SessionWindow {
                account,
                start,
                resets_at: reset,
                peak: obs.iter().map(|o| o.used).fold(0.0, f64::max),
                hit_at,
                waited: hit_at.map(|h| reset - h),
                end,
                free_elsewhere: vec![],
            }
        })
        .collect()
}

/// A reading of `obs` within `max` of `t`, the nearest one.
fn nearest(obs: &[Obs], t: DateTime<Utc>, max: Duration) -> Option<&Obs> {
    obs.iter()
        .filter(|o| (o.at - t).abs() <= max)
        .min_by_key(|o| (o.at - t).abs())
}

/// Within this of a hit, another account's reading says whether it was free.
pub const FREE_NEIGHBOUR_MIN: i64 = 30;
/// "Free": at most this % of the 5-hour window used ...
pub const FREE_SESSION_USED: f64 = 50.0;
/// ... and more than this % of the weekly window left.
pub const FREE_WEEKLY_LEFT: f64 = 10.0;

/// Whether an account had room at `t`: a reading within 30 min with <= 50 % of its 5-hour window
/// used and > 10 % of its weekly window left. A missing window kind (no 5-hour window on this
/// plan) does not block; a window the account has but was not read near `t` does (unknown).
pub fn free_at(a: &AccountObs, t: DateTime<Utc>) -> bool {
    let max = Duration::minutes(FREE_NEIGHBOUR_MIN);
    let s = nearest(&a.session, t, max);
    let w = nearest(&a.weekly, t, max);
    if s.is_none() && w.is_none() {
        return false;
    }
    let session_ok = match s {
        Some(o) => o.effective_used() <= FREE_SESSION_USED,
        None => !a.has_session,
    };
    let weekly_ok = match w {
        Some(o) => 100.0 - o.effective_used() > FREE_WEEKLY_LEFT,
        None => a.weekly.is_empty(),
    };
    session_ok && weekly_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn t(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    fn line(acc: u128, at: &str, window: &str, used: f64, reset: Option<&str>) -> HistoryLine {
        HistoryLine {
            account: Uuid::from_u128(acc),
            fetched_at: t(at),
            window: window.into(),
            used_percent: used,
            resets_at: reset.map(t),
        }
    }

    #[test]
    fn codex_windows_are_told_apart_by_how_far_their_resets_lie() {
        let lines = [
            // Account 1: Codex with a 5-hour primary and a weekly secondary.
            line(
                1,
                "2026-11-02T10:00:00Z",
                "codex:primary",
                10.0,
                Some("2026-11-02T14:00:00Z"),
            ),
            line(
                1,
                "2026-11-02T10:00:00Z",
                "codex:secondary",
                5.0,
                Some("2026-11-06T10:00:00Z"),
            ),
            // Account 2: Codex whose only window is weekly (no 5-hour window).
            line(
                2,
                "2026-11-02T10:00:00Z",
                "codex:primary",
                30.0,
                Some("2026-11-08T10:00:00Z"),
            ),
            // Scoped weekly and unrelated windows are ignored.
            line(3, "2026-11-02T10:00:00Z", "seven_day_opus", 30.0, None),
            line(3, "2026-11-02T10:00:00Z", "five_hour", f64::NAN, None),
        ];
        let m = by_account(&lines);
        let a1 = &m[&Uuid::from_u128(1)];
        assert!(a1.has_session && a1.session.len() == 1 && a1.weekly.len() == 1);
        let a2 = &m[&Uuid::from_u128(2)];
        assert!(!a2.has_session && a2.weekly.len() == 1);
        assert!(!m.contains_key(&Uuid::from_u128(3)));
    }

    #[test]
    fn resets_moving_by_seconds_stay_one_window() {
        let obs: Vec<Obs> = [
            ("2026-11-02T10:00:00Z", 20.0, "2026-11-02T14:00:59Z"),
            ("2026-11-02T11:00:00Z", 60.0, "2026-11-02T14:01:02Z"),
            ("2026-11-02T12:15:00Z", 100.0, "2026-11-02T14:00:58Z"),
            ("2026-11-02T13:30:00Z", 100.0, "2026-11-02T14:01:00Z"),
            // The next window.
            ("2026-11-02T15:00:00Z", 5.0, "2026-11-02T19:00:00Z"),
            // A reading after the reset passed (stale) belongs to no window.
            ("2026-11-02T19:30:00Z", 70.0, "2026-11-02T19:00:00Z"),
        ]
        .iter()
        .map(|&(at, used, r)| Obs {
            at: t(at),
            used,
            resets_at: Some(t(r)),
        })
        .collect();
        let w = windows(Uuid::nil(), &obs, t("2026-11-03T00:00:00Z"));
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].hit_at, Some(t("2026-11-02T12:15:00Z")));
        assert_eq!(w[0].resets_at, t("2026-11-02T14:01:00Z"));
        assert_eq!(w[0].waited, Some(Duration::seconds(105 * 60 + 60)));
        assert_eq!(w[0].end, WindowEnd::Seen);
        // The second window's last reading is 4 h before its reset: unknown end, no hit.
        assert_eq!(w[1].end, WindowEnd::Unknown);
        assert_eq!(w[1].hit_at, None);
    }

    #[test]
    fn unknown_ends_are_never_hits_and_running_windows_are_marked() {
        let obs = [
            Obs {
                at: t("2026-11-02T10:00:00Z"),
                used: 100.0,
                resets_at: Some(t("2026-11-02T14:00:00Z")),
            },
            Obs {
                at: t("2026-11-02T20:00:00Z"),
                used: 100.0,
                resets_at: Some(t("2026-11-02T23:00:00Z")),
            },
        ];
        let w = windows(Uuid::nil(), &obs, t("2026-11-02T21:00:00Z"));
        assert_eq!((w[0].end, w[0].hit_at), (WindowEnd::Unknown, None));
        assert_eq!(w[0].peak, 100.0);
        assert_eq!(w[1].end, WindowEnd::Running);
        assert_eq!(w[1].waited, Some(Duration::hours(3)));
    }

    #[test]
    fn cycles_count_when_ended_with_three_days_of_readings() {
        let reset = "2026-11-09T08:00:00Z";
        let mk = |at: &str, used| Obs {
            at: t(at),
            used,
            resets_at: Some(t(reset)),
        };
        let two_days = [
            mk("2026-11-03T09:00:00Z", 10.0),
            mk("2026-11-04T09:00:00Z", 30.0),
        ];
        let c = cycles(&two_days, t("2026-11-10T00:00:00Z"));
        assert_eq!((c[0].days, c[0].ended, c[0].counted), (2, true, false));
        let mut three = two_days.to_vec();
        three.push(mk("2026-11-08T20:00:00Z", 64.0));
        let c = cycles(&three, t("2026-11-10T00:00:00Z"));
        assert_eq!((c[0].days, c[0].counted, c[0].peak), (3, true, 64.0));
        assert_eq!(c[0].start, t("2026-11-02T08:00:00Z"));
        // Not ended yet: never counted.
        let c = cycles(&three, t("2026-11-09T07:00:00Z"));
        assert!(!c[0].ended && !c[0].counted);
    }

    #[test]
    fn monthly_cycles_count_with_sixty_percent_of_their_days() {
        // Copilot: resets on the 1st; the October cycle runs Oct 1 - Nov 1 (31 days, 19 needed).
        let reset = "2026-11-01T00:00:00Z";
        let read = |days: i64| -> Vec<Obs> {
            (0..days)
                .map(|d| Obs {
                    at: t("2026-10-01T09:00:00Z") + Duration::days(d),
                    used: 2.0 * d as f64,
                    resets_at: Some(t(reset)),
                })
                .collect()
        };
        let now = t("2026-11-03T00:00:00Z");
        let c = cycles_of(&read(18), CycleLen::Month, now);
        assert_eq!(c[0].start, t("2026-10-01T00:00:00Z"));
        assert_eq!((c[0].days, c[0].ended, c[0].counted), (18, true, false));
        let c = cycles_of(&read(19), CycleLen::Month, now);
        assert_eq!((c[0].days, c[0].counted, c[0].peak), (19, true, 36.0));
        // The previous reset, when seen, is where the next cycle starts (Cursor's billing day).
        let mut two = read(19);
        two.push(Obs {
            at: t("2026-11-02T09:00:00Z"),
            used: 5.0,
            resets_at: Some(t("2026-11-29T00:00:00Z")),
        });
        let c = cycles_of(&two, CycleLen::Month, now);
        assert_eq!(c[1].start, t("2026-11-01T00:00:00Z"));
        assert!(!c[1].ended && !c[1].counted);
    }

    #[test]
    fn app_login_windows_have_their_kinds() {
        let at = "2026-11-02T10:00:00Z";
        let lines = [
            line(1, at, "copilot:premium", 40.0, Some("2026-12-01T00:00:00Z")),
            line(1, at, "copilot:chat", 10.0, Some("2026-12-01T00:00:00Z")),
            line(2, at, "agy:5h", 30.0, Some("2026-11-02T13:00:00Z")),
            line(2, at, "agy:weekly", 20.0, Some("2026-11-06T00:00:00Z")),
            line(
                2,
                at,
                "agy:claude-gpt:5h",
                90.0,
                Some("2026-11-02T12:00:00Z"),
            ),
            // Cursor weekly (reset 4 days ahead) and legacy monthly (20 days ahead).
            line(3, at, "cursor:plan", 41.0, Some("2026-11-06T10:00:00Z")),
            line(3, at, "cursor:auto", 41.0, Some("2026-11-06T10:00:00Z")),
            line(4, at, "cursor:plan", 25.0, Some("2026-11-22T10:00:00Z")),
        ];
        let m = by_account(&lines);
        let a = |n| &m[&Uuid::from_u128(n)];
        assert_eq!(
            (a(1).cycle, a(1).weekly.len(), a(1).has_session),
            (CycleLen::Month, 1, false)
        );
        assert!(a(2).has_session && a(2).session[0].used == 30.0 && a(2).cycle == CycleLen::Week);
        assert_eq!((a(3).cycle, a(3).weekly.len()), (CycleLen::Week, 1));
        assert_eq!(a(4).cycle, CycleLen::Month);
        // Late in a legacy month the reset is near, but resets a month apart still say monthly.
        let late = [
            line(
                5,
                "2026-10-28T10:00:00Z",
                "cursor:plan",
                80.0,
                Some("2026-10-31T10:00:00Z"),
            ),
            line(
                5,
                "2026-11-02T10:00:00Z",
                "cursor:plan",
                5.0,
                Some("2026-11-30T10:00:00Z"),
            ),
        ];
        let refs: Vec<&HistoryLine> = late.iter().collect();
        assert_eq!(cursor_cycle(&refs[..1]), CycleLen::Week);
        assert_eq!(cursor_cycle(&refs), CycleLen::Month);
    }

    #[test]
    fn free_needs_a_close_reading_with_room() {
        let at = t("2026-11-02T12:00:00Z");
        let obs = |mins: i64, used: f64| Obs {
            at: at + Duration::minutes(mins),
            used,
            resets_at: Some(at + Duration::hours(3)),
        };
        let a = AccountObs {
            session: vec![obs(-20, 40.0)],
            weekly: vec![obs(-20, 50.0)],
            has_session: true,
            first_at: None,
            cycle: CycleLen::Week,
        };
        assert!(free_at(&a, at));
        // Too far away.
        assert!(!free_at(&a, at + Duration::minutes(60)));
        // Session too full.
        let busy = AccountObs {
            session: vec![obs(10, 60.0)],
            ..a.clone()
        };
        assert!(!free_at(&busy, at));
        // Weekly nearly used up (only 8 % left).
        let week_out = AccountObs {
            weekly: vec![obs(5, 92.0)],
            ..a.clone()
        };
        assert!(!free_at(&week_out, at));
        // A plan without a 5-hour window only needs weekly room.
        let codex = AccountObs {
            session: vec![],
            has_session: false,
            ..a.clone()
        };
        assert!(free_at(&codex, at));
    }
}
