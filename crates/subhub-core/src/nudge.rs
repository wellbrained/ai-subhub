//! Run-out nudge (pure): a tray notice about 15 minutes before an account's 5-hour window runs
//! out at its current pace, when another account of the same provider has a fresh window. Off by
//! default (owner, 2026-11-08); it only tells, nothing switches. Quiet while hub steering moves
//! hub accounts by itself.
use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::{Account, AllowanceWindow, Reading};

/// How long before running out the notice comes (at least; see `lead`).
pub const LEAD_MINUTES: i64 = 15;
/// "Fresh" for the other account: at most this much of its 5-hour window used.
pub const FRESH_USED_PERCENT: f64 = 50.0;
/// The length of a 5-hour window.
const WINDOW: i64 = 5 * 60;
/// Recent pace: readings within this span before the latest one.
const RECENT_MINUTES: i64 = 45;
/// Recent pace needs readings at least this far apart; otherwise the window's average.
const MIN_SPAN_MINUTES: i64 = 10;
/// The window's average needs this much of the window behind it.
const MIN_ELAPSED_MINUTES: i64 = 30;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NudgeSettings {
    pub enabled: bool,
    /// Per account: the reset time of the window that already had its notice (one per window).
    pub shown: HashMap<Uuid, DateTime<Utc>>,
}

/// One 5-hour reading of an account, kept in memory for the pace.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub at: DateTime<Utc>,
    pub used: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Nudge {
    pub account: Uuid,
    /// Reset of the window running out (marks the notice as shown).
    pub resets_at: DateTime<Utc>,
    pub minutes: i64,
    pub other: Uuid,
    pub other_left: f64,
}

/// The notice comes this many minutes ahead: 15, or one refresh interval when readings come
/// less often (with 15-minute refreshes a 15-minute lead would often be missed).
pub fn lead(refresh_minutes: u32) -> i64 {
    LEAD_MINUTES.max(refresh_minutes as i64)
}

fn session(r: &Reading) -> Option<&AllowanceWindow> {
    r.windows.iter().find(|w| w.is_session())
}

fn weekly(r: &Reading) -> Option<&AllowanceWindow> {
    r.windows.iter().find(|w| w.is_weekly())
}

fn same_window(a: Option<DateTime<Utc>>, b: DateTime<Utc>) -> bool {
    a.is_some_and(|a| (a - b).num_seconds().abs() <= 120)
}

/// Minutes until the 5-hour window is used up at the current pace: the recent readings of this
/// window when they span at least 10 minutes, else the window's average since it started. None
/// when nothing is being used.
pub fn minutes_left(w: &AllowanceWindow, trail: &[Point], now: DateTime<Utc>) -> Option<f64> {
    let resets = w.resets_at?;
    let left = 100.0 - w.used_percent;
    if left <= 0.0 {
        return Some(0.0);
    }
    let pts: Vec<&Point> = trail
        .iter()
        .filter(|p| {
            same_window(p.resets_at, resets) && now - p.at <= Duration::minutes(RECENT_MINUTES)
        })
        .collect();
    let recent = match (pts.first(), pts.last()) {
        (Some(a), Some(b)) if b.at - a.at >= Duration::minutes(MIN_SPAN_MINUTES) => {
            let mins = (b.at - a.at).num_seconds() as f64 / 60.0;
            Some((b.used - a.used) / mins)
        }
        _ => None,
    };
    let rate = recent.or_else(|| {
        let start = resets - Duration::minutes(WINDOW);
        let elapsed = now - start;
        (elapsed >= Duration::minutes(MIN_ELAPSED_MINUTES))
            .then(|| w.used_percent / (elapsed.num_seconds() as f64 / 60.0))
    })?;
    (rate > 0.0).then(|| left / rate)
}

/// The account with the most room in its 5-hour window among the same provider's others:
/// visible, not benched, fresh reading, ≤ 50 % used (or its window has ended) and weekly room.
fn fresh_other(
    me: &Account,
    accounts: &[Account],
    readings: &HashMap<Uuid, Reading>,
    stale: &dyn Fn(Uuid) -> bool,
    now: DateTime<Utc>,
) -> Option<(Uuid, f64)> {
    accounts
        .iter()
        .filter(|a| a.id != me.id && a.provider == me.provider && usable(a) && !stale(a.id))
        .filter_map(|a| {
            let r = readings.get(&a.id)?;
            if weekly(r).is_some_and(|w| w.used_percent >= 100.0) {
                return None;
            }
            let used = match session(r) {
                Some(w) if w.resets_at.is_none_or(|t| t > now) => w.used_percent,
                _ => 0.0,
            };
            (used <= FRESH_USED_PERCENT).then_some((a.id, 100.0 - used))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

fn usable(a: &Account) -> bool {
    !a.hidden && !a.hub.as_ref().is_some_and(|h| h.benched)
}

pub struct Input<'a> {
    pub now: DateTime<Utc>,
    pub accounts: &'a [Account],
    pub readings: &'a HashMap<Uuid, Reading>,
    pub trails: &'a HashMap<Uuid, Vec<Point>>,
    pub stale: &'a dyn Fn(Uuid) -> bool,
    pub settings: &'a NudgeSettings,
    /// Hub steering is on and the hub answers: hub accounts are moved by it, no notice for them.
    pub steering: bool,
    pub refresh_minutes: u32,
}

/// The notices due now (at most one per account and window).
pub fn due(i: &Input) -> Vec<Nudge> {
    if !i.settings.enabled {
        return vec![];
    }
    let lead = lead(i.refresh_minutes) as f64;
    let mut out = vec![];
    for a in i.accounts {
        if !usable(a) || (i.stale)(a.id) || (i.steering && a.hub.is_some()) {
            continue;
        }
        let Some(w) = i.readings.get(&a.id).and_then(session) else {
            continue;
        };
        let Some(resets) = w.resets_at else { continue };
        if resets <= i.now || w.used_percent >= 100.0 {
            continue;
        }
        if i.settings
            .shown
            .get(&a.id)
            .is_some_and(|t| same_window(Some(*t), resets))
        {
            continue;
        }
        let trail = i.trails.get(&a.id).map(Vec::as_slice).unwrap_or(&[]);
        let Some(mins) = minutes_left(w, trail, i.now) else {
            continue;
        };
        // Runs out soon, and before the window resets anyway.
        let until_reset = (resets - i.now).num_seconds() as f64 / 60.0;
        if mins > lead || mins >= until_reset {
            continue;
        }
        let Some((other, other_left)) = fresh_other(a, i.accounts, i.readings, i.stale, i.now)
        else {
            continue;
        };
        out.push(Nudge {
            account: a.id,
            resets_at: resets,
            minutes: mins.round().max(1.0) as i64,
            other,
            other_left,
        });
    }
    out
}

/// "about 15 min" (rounded to 5 above 10 minutes).
pub fn about(minutes: i64) -> String {
    let m = if minutes > 10 {
        ((minutes as f64 / 5.0).round() * 5.0) as i64
    } else {
        minutes.max(1)
    };
    format!("about {m} min")
}

/// Drops marks of windows that have reset (keeps the settings small).
pub fn prune(s: &mut NudgeSettings, now: DateTime<Utc>) {
    s.shown.retain(|_, t| *t > now);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{HubLink, ProviderKind};

    fn t(min: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-08T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + Duration::minutes(min)
    }

    fn acc(id: u128, p: ProviderKind) -> Account {
        Account {
            id: Uuid::from_u128(id),
            provider: p,
            label: format!("A{id}"),
            email: None,
            profile_dir: None,
            hub: None,
            group: None,
            hidden: false,
            hidden_windows: vec![],
            pinned_windows: vec![],
        }
    }

    fn reading(used: f64, resets: DateTime<Utc>, weekly: f64) -> Reading {
        Reading {
            fetched_at: t(0),
            windows: vec![
                AllowanceWindow {
                    key: "five_hour".into(),
                    label: "Session".into(),
                    used_percent: used,
                    resets_at: Some(resets),
                },
                AllowanceWindow {
                    key: "seven_day".into(),
                    label: "Weekly".into(),
                    used_percent: weekly,
                    resets_at: Some(t(3 * 24 * 60)),
                },
            ],
        }
    }

    struct World {
        accounts: Vec<Account>,
        readings: HashMap<Uuid, Reading>,
        trails: HashMap<Uuid, Vec<Point>>,
        settings: NudgeSettings,
        steering: bool,
    }

    impl World {
        /// Account 1 at 90 % used, climbing 1 %/min over the last 20 min; window resets in 2 h.
        /// Account 2 of the same provider at 10 % used.
        fn new() -> Self {
            let resets = t(120);
            let a = acc(1, ProviderKind::Claude);
            let b = acc(2, ProviderKind::Claude);
            let trail = (0..=20)
                .map(|m| Point {
                    at: t(-20 + m),
                    used: 70.0 + m as f64,
                    resets_at: Some(resets),
                })
                .collect();
            World {
                readings: HashMap::from([
                    (a.id, reading(90.0, resets, 40.0)),
                    (b.id, reading(10.0, t(200), 20.0)),
                ]),
                trails: HashMap::from([(a.id, trail)]),
                accounts: vec![a, b],
                settings: NudgeSettings {
                    enabled: true,
                    shown: HashMap::new(),
                },
                steering: false,
            }
        }

        fn due(&self) -> Vec<Nudge> {
            due(&Input {
                now: t(0),
                accounts: &self.accounts,
                readings: &self.readings,
                trails: &self.trails,
                stale: &|_| false,
                settings: &self.settings,
                steering: self.steering,
                refresh_minutes: 5,
            })
        }
    }

    #[test]
    fn nudges_when_running_out_and_another_is_fresh() {
        let n = World::new().due();
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].account, Uuid::from_u128(1));
        assert_eq!(n[0].other, Uuid::from_u128(2));
        assert_eq!(n[0].minutes, 10);
        assert_eq!(n[0].other_left, 90.0);
    }

    #[test]
    fn off_by_default() {
        let mut w = World::new();
        w.settings = NudgeSettings::default();
        assert!(w.due().is_empty());
    }

    #[test]
    fn once_per_window() {
        let mut w = World::new();
        w.settings.shown.insert(Uuid::from_u128(1), t(120));
        assert!(w.due().is_empty());
    }

    #[test]
    fn quiet_when_the_window_resets_first() {
        let mut w = World::new();
        let id = Uuid::from_u128(1);
        w.readings.insert(id, reading(90.0, t(8), 40.0));
        for p in w.trails.get_mut(&id).unwrap() {
            p.resets_at = Some(t(8));
        }
        assert!(w.due().is_empty());
    }

    #[test]
    fn quiet_when_no_other_account_has_room() {
        let mut w = World::new();
        w.readings
            .insert(Uuid::from_u128(2), reading(60.0, t(200), 20.0));
        assert!(w.due().is_empty());
        w.readings
            .insert(Uuid::from_u128(2), reading(10.0, t(200), 100.0));
        assert!(w.due().is_empty());
    }

    #[test]
    fn other_provider_does_not_count() {
        let mut w = World::new();
        w.accounts[1].provider = ProviderKind::Codex;
        assert!(w.due().is_empty());
    }

    #[test]
    fn quiet_for_hub_accounts_while_steering() {
        let mut w = World::new();
        w.steering = true;
        assert_eq!(w.due().len(), 1, "a profile-only account still gets it");
        w.accounts[0].hub = Some(HubLink {
            auth_index: serde_json::Value::Null,
            file_name: "a.json".into(),
            benched: false,
            account_id: None,
            priority: None,
        });
        assert!(w.due().is_empty());
    }

    #[test]
    fn quiet_when_hidden_or_benched_or_slow() {
        let mut w = World::new();
        w.accounts[0].hidden = true;
        assert!(w.due().is_empty());
        let mut w = World::new();
        // 0.2 %/min: 50 minutes left.
        let id = Uuid::from_u128(1);
        for (i, p) in w.trails.get_mut(&id).unwrap().iter_mut().enumerate() {
            p.used = 86.0 + i as f64 * 0.2;
        }
        assert!(w.due().is_empty());
    }

    #[test]
    fn falls_back_to_the_window_average() {
        // 80 % used 4 h into the window: 1/3 % per minute, 20 % left = 60 min.
        let w = AllowanceWindow {
            key: "five_hour".into(),
            label: "Session".into(),
            used_percent: 80.0,
            resets_at: Some(t(60)),
        };
        let m = minutes_left(&w, &[], t(0)).unwrap();
        assert!((m - 60.0).abs() < 0.01, "{m}");
    }

    #[test]
    fn lead_follows_slow_refreshes() {
        assert_eq!(lead(5), 15);
        assert_eq!(lead(30), 30);
    }

    #[test]
    fn about_rounds() {
        assert_eq!(about(14), "about 15 min");
        assert_eq!(about(7), "about 7 min");
        assert_eq!(about(0), "about 1 min");
    }
}
