//! Priority view classification. Pure functions, unit tested.
use std::cmp::Ordering;
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::model::{Account, AllowanceWindow, Reading};
use crate::settings::RUNNING_OUT_PERCENT;

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
    now: DateTime<Utc>,
) -> Vec<PriorityItem> {
    // `now` is deliberately not compared with reset times (reset countdowns never refill locally).
    let _ = now;
    let threshold = RUNNING_OUT_PERCENT;
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
        let items = classify(&[a, b], &r, &|_| false, now());
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
    fn running_out_is_fixed_at_25_percent_and_inclusive() {
        let a = account(&[]);
        let r = readings(vec![(
            &a,
            vec![window("at", 75.0, None), window("above", 74.0, None)],
        )]);
        assert_eq!(
            keys(&classify(std::slice::from_ref(&a), &r, &|_| false, now())),
            [("at", Section::RunningOut)]
        );
    }

    #[test]
    fn pinned_but_used_up_goes_to_used_up_and_passed_reset_does_not_refill() {
        let a = account(&["s"]);
        let mut w = window("s", 100.0, Some(-5));
        w.resets_at = Some(now() - chrono::Duration::hours(5));
        let r = readings(vec![(&a, vec![w])]);
        let items = classify(&[a], &r, &|_| false, now() + chrono::Duration::days(3));
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
        let items = classify(&[hidden, partly, no_reading], &r, &|_| false, now());
        assert_eq!(keys(&items), [("shown", Section::RunningOut)]);
    }

    #[test]
    fn stale_flag_comes_from_the_callback() {
        let a = account(&["s"]);
        let id = a.id;
        let r = readings(vec![(&a, vec![window("s", 10.0, None)])]);
        let items = classify(&[a], &r, &|u| u == id, now());
        assert!(items[0].stale);
    }
}
