//! Reading history -> weekly cycles and 5-hour windows per account (plan "Analysis", owner
//! 2026-11-07). Readings of one window are grouped by `resets_at`; providers move it by a few
//! seconds between readings, so a group takes every reset within [`SAME_RESET`] of its first one
//! (the plan says "minute precision"; a tolerance also survives a minute boundary).
use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Duration, Utc};
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

/// An account's readings of its 5-hour and its all-models weekly window, oldest first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccountObs {
    pub session: Vec<Obs>,
    pub weekly: Vec<Obs>,
    /// Whether a 5-hour window was ever seen (Codex plans without one have none).
    pub has_session: bool,
    pub first_at: Option<DateTime<Utc>>,
}

/// What a history window key is: the 5-hour or the all-models weekly window. Codex keys
/// (`codex:primary` / `codex:secondary`) carry no length, so it is read from how far ahead their
/// resets lie: at most 5 h (+10 min) is a 5-hour window, a day or more is weekly; a Codex plan
/// whose primary window is weekly thus has no 5-hour window.
fn kind_of(key: &str, lines: &[&HistoryLine]) -> Option<bool> {
    if is_session_window(key) {
        return Some(true);
    }
    if is_weekly_window(key) {
        return Some(false);
    }
    if key != "codex:primary" && key != "codex:secondary" {
        return None;
    }
    let ahead = lines
        .iter()
        .filter_map(|l| Some(l.resets_at? - l.fetched_at))
        .max()?;
    if ahead <= Duration::minutes(SESSION_LEN_H * 60 + 10) {
        Some(true)
    } else if ahead >= Duration::days(1) {
        Some(false)
    } else {
        None
    }
}

/// Groups history lines per account into 5-hour and weekly readings. Non-finite values and other
/// windows are skipped; used % is clamped to 0..=100.
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
        let Some(session) = kind_of(key, ls) else {
            continue;
        };
        // One window per kind and account (the first key in order wins).
        if !taken.insert((*account, session)) {
            continue;
        }
        let a = out.entry(*account).or_default();
        let list = if session {
            a.has_session = true;
            &mut a.session
        } else {
            &mut a.weekly
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

/// One weekly cycle of an account.
#[derive(Debug, Clone, PartialEq)]
pub struct WeeklyCycle {
    pub start: DateTime<Utc>,
    pub resets_at: DateTime<Utc>,
    /// Highest % used seen: the cycle's use.
    pub peak: f64,
    /// Of the cycle's 7 days, how many have a reading.
    pub days: usize,
    pub ended: bool,
    /// Ended and read on at least [`CYCLE_MIN_DAYS`] days.
    pub counted: bool,
}

pub fn cycles(weekly: &[Obs], now: DateTime<Utc>) -> Vec<WeeklyCycle> {
    group(weekly)
        .into_iter()
        .map(|(reset, obs)| {
            let start = reset - Duration::days(WEEK_LEN_D);
            let mut days: Vec<i64> = obs
                .iter()
                .map(|o| (o.at - start).num_seconds().div_euclid(86_400))
                .filter(|d| (0..WEEK_LEN_D).contains(d))
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
                counted: ended && days.len() >= CYCLE_MIN_DAYS,
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
