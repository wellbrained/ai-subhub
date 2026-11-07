//! Priority view classification and the tray metric. Pure functions, unit tested.
use std::cmp::Ordering;
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::model::{Account, AllowanceWindow, Reading};
use crate::settings::{DisplaySettings, TrayMetric};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Pinned,
    RunningOut,
    UsedUp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PriorityItem {
    pub account: Uuid,
    pub window: AllowanceWindow,
    pub section: Section,
    /// Reading is stale (caller decides: overdue or last check failed).
    pub stale: bool,
}

/// Visible (not hidden) windows of visible accounts, in sections:
/// Pinned (pinned and not used up), Running out (0 < remaining <= threshold, not pinned),
/// Used up (remaining == 0). A reset time that passed does NOT refill the window: it stays
/// Used up until a new reading says otherwise. Windows in none of these are omitted.
/// Sorted: Pinned in pin order, Running out by remaining asc, Used up by reset asc.
pub fn classify(
    accounts: &[Account],
    readings: &HashMap<Uuid, Reading>,
    stale: &dyn Fn(Uuid) -> bool,
    display: &DisplaySettings,
    now: DateTime<Utc>,
) -> Vec<PriorityItem> {
    // `now` is deliberately not compared with reset times (reset countdowns never refill locally).
    let _ = now;
    let threshold = f64::from(display.running_out_threshold);
    // (account position, pin position) keeps pin order across accounts.
    let mut pinned: Vec<((usize, usize), PriorityItem)> = vec![];
    let mut running: Vec<PriorityItem> = vec![];
    let mut used: Vec<PriorityItem> = vec![];

    for (position, account) in accounts.iter().enumerate().filter(|(_, a)| !a.hidden) {
        let Some(reading) = readings.get(&account.id) else {
            continue;
        };
        let is_stale = stale(account.id);
        for window in reading
            .windows
            .iter()
            .filter(|w| !account.hidden_windows.contains(&w.key))
        {
            let remaining = window.remaining_percent();
            let item = |section| PriorityItem {
                account: account.id,
                window: window.clone(),
                section,
                stale: is_stale,
            };
            if remaining <= 0.0 {
                used.push(item(Section::UsedUp));
            } else if let Some(pin) = account.pinned_windows.iter().position(|k| *k == window.key) {
                pinned.push(((position, pin), item(Section::Pinned)));
            } else if remaining <= threshold {
                running.push(item(Section::RunningOut));
            }
        }
    }

    pinned.sort_by_key(|(order, _)| *order);
    running.sort_by(|a, b| {
        a.window
            .remaining_percent()
            .total_cmp(&b.window.remaining_percent())
    });
    used.sort_by(|a, b| match (a.window.resets_at, b.window.resets_at) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    });

    pinned
        .into_iter()
        .map(|(_, item)| item)
        .chain(running)
        .chain(used)
        .collect()
}

/// Fraction 0..=1 remaining for the tray ring, or None when nothing is known.
/// `LowestPinned`: lowest across pinned windows; with nothing pinned, across all visible windows.
/// A pinned window without a reading is unknown, not replaced by another one.
pub fn tray_fraction(
    accounts: &[Account],
    readings: &HashMap<Uuid, Reading>,
    metric: &TrayMetric,
) -> Option<f64> {
    let visible_windows = |only_pinned: bool| {
        accounts
            .iter()
            .filter(|a| !a.hidden)
            .filter_map(|a| readings.get(&a.id).map(|r| (a, r)))
            .flat_map(move |(a, r)| {
                r.windows
                    .iter()
                    .filter(move |w| !a.hidden_windows.contains(&w.key))
                    .filter(move |w| !only_pinned || a.pinned_windows.contains(&w.key))
            })
            .map(AllowanceWindow::remaining_percent)
    };
    let lowest = |values: &mut dyn Iterator<Item = f64>| values.reduce(f64::min).map(|p| p / 100.0);

    match metric {
        TrayMetric::LowestAll => lowest(&mut visible_windows(false)),
        TrayMetric::LowestPinned => {
            let any_pinned = accounts.iter().filter(|a| !a.hidden).any(|a| {
                a.pinned_windows
                    .iter()
                    .any(|k| !a.hidden_windows.contains(k))
            });
            lowest(&mut visible_windows(any_pinned))
        }
        TrayMetric::Window { account, key } => {
            let a = accounts.iter().find(|a| a.id == *account && !a.hidden)?;
            if a.hidden_windows.contains(key) {
                return None;
            }
            let w = readings
                .get(&a.id)?
                .windows
                .iter()
                .find(|w| w.key == *key)?;
            Some(w.remaining_percent() / 100.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ProviderKind;

    fn now() -> DateTime<Utc> {
        "2026-11-06T10:00:00Z".parse().unwrap()
    }

    fn window(key: &str, used: f64, reset_hours: Option<i64>) -> AllowanceWindow {
        AllowanceWindow {
            key: key.into(),
            label: key.into(),
            used_percent: used,
            resets_at: reset_hours.map(|h| now() + chrono::Duration::hours(h)),
        }
    }

    fn account(pinned: &[&str]) -> Account {
        Account {
            id: Uuid::new_v4(),
            provider: ProviderKind::Claude,
            label: "A".into(),
            email: None,
            profile_dir: None,
            hub: None,
            group: None,
            hidden: false,
            hidden_windows: vec![],
            pinned_windows: pinned.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn readings(entries: Vec<(&Account, Vec<AllowanceWindow>)>) -> HashMap<Uuid, Reading> {
        entries
            .into_iter()
            .map(|(a, w)| {
                (
                    a.id,
                    Reading {
                        fetched_at: now(),
                        windows: w,
                    },
                )
            })
            .collect()
    }

    fn keys(items: &[PriorityItem]) -> Vec<(&str, Section)> {
        items
            .iter()
            .map(|i| (i.window.key.as_str(), i.section))
            .collect()
    }

    #[test]
    fn sections_and_sort_order() {
        let a = account(&["weekly", "session"]);
        let b = account(&[]);
        let r = readings(vec![
            (
                &a,
                vec![
                    window("session", 50.0, Some(1)),
                    window("weekly", 10.0, Some(5)),
                    window("opus", 80.0, None),
                ],
            ),
            (
                &b,
                vec![
                    window("x", 95.0, None),
                    window("y", 100.0, Some(3)),
                    window("z", 100.0, Some(1)),
                    window("ok", 10.0, None),
                ],
            ),
        ]);
        let items = classify(&[a, b], &r, &|_| false, &DisplaySettings::default(), now());
        assert_eq!(
            keys(&items),
            [
                ("weekly", Section::Pinned),
                ("session", Section::Pinned),
                ("x", Section::RunningOut),
                ("opus", Section::RunningOut),
                ("z", Section::UsedUp),
                ("y", Section::UsedUp),
            ]
        );
    }

    #[test]
    fn running_out_threshold_is_inclusive_and_configurable() {
        let a = account(&[]);
        let r = readings(vec![(
            &a,
            vec![window("at", 75.0, None), window("above", 74.0, None)],
        )]);
        let mut display = DisplaySettings::default();
        assert_eq!(
            keys(&classify(
                std::slice::from_ref(&a),
                &r,
                &|_| false,
                &display,
                now()
            )),
            [("at", Section::RunningOut)]
        );
        display.running_out_threshold = 10;
        assert!(classify(std::slice::from_ref(&a), &r, &|_| false, &display, now()).is_empty());
    }

    #[test]
    fn pinned_but_used_up_goes_to_used_up_and_passed_reset_does_not_refill() {
        let a = account(&["s"]);
        let mut w = window("s", 100.0, Some(-5));
        w.resets_at = Some(now() - chrono::Duration::hours(5));
        let r = readings(vec![(&a, vec![w])]);
        let items = classify(
            &[a],
            &r,
            &|_| false,
            &DisplaySettings::default(),
            now() + chrono::Duration::days(3),
        );
        assert_eq!(keys(&items), [("s", Section::UsedUp)]);
    }

    #[test]
    fn hidden_accounts_and_windows_and_missing_readings_are_omitted() {
        let mut hidden = account(&["s"]);
        hidden.hidden = true;
        let mut partly = account(&[]);
        partly.hidden_windows = vec!["gone".into()];
        let no_reading = account(&["s"]);
        let r = readings(vec![
            (&hidden, vec![window("s", 99.0, None)]),
            (
                &partly,
                vec![window("gone", 99.0, None), window("shown", 99.0, None)],
            ),
        ]);
        let items = classify(
            &[hidden, partly, no_reading],
            &r,
            &|_| false,
            &DisplaySettings::default(),
            now(),
        );
        assert_eq!(keys(&items), [("shown", Section::RunningOut)]);
    }

    #[test]
    fn stale_flag_comes_from_the_callback() {
        let a = account(&["s"]);
        let id = a.id;
        let r = readings(vec![(&a, vec![window("s", 10.0, None)])]);
        let items = classify(&[a], &r, &|u| u == id, &DisplaySettings::default(), now());
        assert!(items[0].stale);
    }

    #[test]
    fn tray_lowest_pinned_falls_back_to_all_only_when_nothing_is_pinned() {
        let pinned = account(&["p"]);
        let other = account(&[]);
        let r = readings(vec![
            (
                &pinned,
                vec![window("p", 40.0, None), window("q", 90.0, None)],
            ),
            (&other, vec![window("o", 70.0, None)]),
        ]);
        let accounts = [pinned.clone(), other.clone()];
        assert_eq!(
            tray_fraction(&accounts, &r, &TrayMetric::LowestPinned),
            Some(0.6)
        );
        assert_eq!(
            tray_fraction(&accounts, &r, &TrayMetric::LowestAll),
            Some(0.1)
        );
        let none_pinned = [account(&[]), other];
        let r2 = readings(vec![
            (&none_pinned[0], vec![window("a", 20.0, None)]),
            (&none_pinned[1], vec![window("o", 70.0, None)]),
        ]);
        assert_eq!(
            tray_fraction(&none_pinned, &r2, &TrayMetric::LowestPinned),
            Some(0.3)
        );
    }

    #[test]
    fn tray_unknown_is_none_never_zero_or_full() {
        let a = account(&["p"]);
        assert_eq!(
            tray_fraction(
                std::slice::from_ref(&a),
                &HashMap::new(),
                &TrayMetric::LowestPinned
            ),
            None
        );
        assert_eq!(
            tray_fraction(&[], &HashMap::new(), &TrayMetric::LowestAll),
            None
        );
        // Pinned window has no reading yet: unknown, not another window's value.
        let r = readings(vec![(&a, vec![window("other", 50.0, None)])]);
        assert_eq!(
            tray_fraction(std::slice::from_ref(&a), &r, &TrayMetric::LowestPinned),
            None
        );
    }

    #[test]
    fn tray_specific_window() {
        let mut a = account(&[]);
        let r = readings(vec![(&a, vec![window("s", 25.0, None)])]);
        let metric = TrayMetric::Window {
            account: a.id,
            key: "s".into(),
        };
        assert_eq!(
            tray_fraction(std::slice::from_ref(&a), &r, &metric),
            Some(0.75)
        );
        let missing = TrayMetric::Window {
            account: a.id,
            key: "nope".into(),
        };
        assert_eq!(tray_fraction(std::slice::from_ref(&a), &r, &missing), None);
        a.hidden_windows = vec!["s".into()];
        assert_eq!(tray_fraction(&[a], &r, &metric), None);
    }
}
