//! Hub steering (pure): which hub login should be on top, and which credential priorities to
//! write to get there. The only hub write SubHub makes is each login's priority (ADR 0001).
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::model::{Account, AllowanceWindow, Reading};
use crate::settings::{Settings, SteeringSettings};

/// A 5-hour session window (`five_hour`, or the `session` limit kind).
pub fn is_session_window(key: &str) -> bool {
    matches!(key, "five_hour" | "session")
}

/// The all-models weekly window (`seven_day`, or the `weekly_all` limit kind).
pub fn is_weekly_window(key: &str) -> bool {
    matches!(key, "seven_day" | "weekly_all")
}

/// A new hub login must beat the current one by this factor to take over.
const HYSTERESIS: f64 = 1.25;
/// Minimum 5-hour room (% left) for a candidate.
const MIN_SESSION_LEFT: f64 = 5.0;

struct Candidate {
    id: Uuid,
    urgency: f64,
    session_left: f64,
}

fn candidate(
    account: &Account,
    readings: &HashMap<Uuid, Reading>,
    stale: &dyn Fn(Uuid) -> bool,
    now: DateTime<Utc>,
) -> Option<Candidate> {
    let link = account.hub.as_ref()?;
    if link.benched || account.hidden || stale(account.id) {
        return None;
    }
    let reading = readings.get(&account.id)?;
    let find = |f: fn(&str) -> bool| -> Option<&AllowanceWindow> {
        reading.windows.iter().find(|w| f(&w.key))
    };
    let session_left = match find(is_session_window) {
        Some(w) if w.remaining_percent() < MIN_SESSION_LEFT => return None,
        Some(w) => w.remaining_percent(),
        None => 100.0,
    };
    let weekly = find(is_weekly_window)?;
    let left = weekly.remaining_percent();
    if left <= 0.0 {
        return None;
    }
    let hours = weekly
        .resets_at
        .map_or(168.0, |at| (at - now).num_seconds() as f64 / 3600.0)
        .max(1.0);
    Some(Candidate {
        id: account.id,
        urgency: left / hours,
        session_left,
    })
}

/// The hub account whose weekly leftover is most at risk of expiring (weekly % left divided by
/// hours to its reset). Skips benched, hidden, stale, reading-less accounts, accounts with <5%
/// left in the 5-hour window and accounts with nothing left weekly. Ties go to more 5-hour room.
/// `current` is kept unless the best is at least 25% more urgent.
pub fn auto_pick(
    accounts: &[Account],
    readings: &HashMap<Uuid, Reading>,
    stale: &dyn Fn(Uuid) -> bool,
    current: Option<Uuid>,
    now: DateTime<Utc>,
) -> Option<Uuid> {
    let candidates: Vec<Candidate> = accounts
        .iter()
        .filter_map(|a| candidate(a, readings, stale, now))
        .collect();
    let best = candidates.iter().max_by(|a, b| {
        a.urgency
            .total_cmp(&b.urgency)
            .then(a.session_left.total_cmp(&b.session_left))
    })?;
    if let Some(cur) = current.and_then(|c| candidates.iter().find(|x| x.id == c))
        && best.urgency < cur.urgency * HYSTERESIS
    {
        return Some(cur.id);
    }
    Some(best.id)
}

/// Who should be on top: nobody when steering is off; the preferred account while it is still a
/// usable hub login; otherwise [`auto_pick`].
pub fn target(
    settings: &Settings,
    accounts: &[Account],
    readings: &HashMap<Uuid, Reading>,
    stale: &dyn Fn(Uuid) -> bool,
    now: DateTime<Utc>,
) -> Option<Uuid> {
    let s = &settings.steering;
    if !s.enabled {
        return None;
    }
    if let Some(p) = s.preferred
        && accounts
            .iter()
            .any(|a| a.id == p && a.hub.as_ref().is_some_and(|l| !l.benched))
    {
        return Some(p);
    }
    auto_pick(accounts, readings, stale, s.current, now)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorityChange {
    pub file_name: String,
    pub priority: Option<i64>,
}

/// Priority writes needed to put `target` on top and give every other login SubHub touched before
/// its original priority back. Returns the changes (target first) and the updated original
/// priorities to persist once the writes succeeded. `target` None (steering off) restores all and
/// clears the map. An account's original is recorded from `HubLink.priority` the first time it is
/// changed; no change is listed when the value is already right.
pub fn plan(
    accounts: &[Account],
    steering: &SteeringSettings,
    target: Option<Uuid>,
) -> (Vec<PriorityChange>, HashMap<String, Option<i64>>) {
    let mut originals = steering.original_priorities.clone();
    let mut changes = vec![];
    let link_of = |file: &str| {
        accounts
            .iter()
            .filter_map(|a| a.hub.as_ref())
            .find(|l| l.file_name == file)
    };

    let top_file = target
        .and_then(|t| accounts.iter().find(|a| a.id == t))
        .and_then(|a| a.hub.as_ref())
        .filter(|l| !l.file_name.is_empty());
    if let Some(link) = top_file {
        originals
            .entry(link.file_name.clone())
            .or_insert(link.priority);
        if link.priority != Some(steering.top_priority) {
            changes.push(PriorityChange {
                file_name: link.file_name.clone(),
                priority: Some(steering.top_priority),
            });
        }
    }

    let mut others: Vec<String> = originals
        .keys()
        .filter(|f| top_file.is_none_or(|l| l.file_name != **f))
        .cloned()
        .collect();
    others.sort();
    for file in others {
        let original = originals[&file];
        // A login no longer listed cannot be written; keep its entry for later.
        let Some(link) = link_of(&file) else { continue };
        if link.priority != original {
            changes.push(PriorityChange {
                file_name: file.clone(),
                priority: original,
            });
        }
        originals.remove(&file);
    }
    (changes, originals)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{HubLink, ProviderKind};

    fn now() -> DateTime<Utc> {
        "2026-11-06T10:00:00Z".parse().unwrap()
    }

    fn win(key: &str, used: f64, reset_hours: Option<i64>) -> AllowanceWindow {
        AllowanceWindow {
            key: key.into(),
            label: key.into(),
            used_percent: used,
            resets_at: reset_hours.map(|h| now() + chrono::Duration::hours(h)),
        }
    }

    fn hub_account(file: &str, priority: Option<i64>) -> Account {
        Account {
            id: Uuid::new_v4(),
            provider: ProviderKind::Claude,
            label: file.into(),
            email: None,
            profile_dir: None,
            hub: Some(HubLink {
                auth_index: serde_json::json!(1),
                file_name: file.into(),
                benched: false,
                account_id: None,
                priority,
            }),
            group: None,
            hidden: false,
            hidden_windows: vec![],
            pinned_windows: vec![],
        }
    }

    /// weekly: (used %, hours to reset); session: used % of the 5-hour window.
    fn reading(session_used: f64, weekly_used: f64, hours: Option<i64>) -> Reading {
        Reading {
            fetched_at: now(),
            windows: vec![
                win("five_hour", session_used, Some(2)),
                win("seven_day", weekly_used, hours),
            ],
        }
    }

    fn readings(entries: Vec<(&Account, Reading)>) -> HashMap<Uuid, Reading> {
        entries.into_iter().map(|(a, r)| (a.id, r)).collect()
    }

    fn fresh(_: Uuid) -> bool {
        false
    }

    #[test]
    fn window_key_helpers() {
        assert!(is_session_window("five_hour") && is_session_window("session"));
        assert!(!is_session_window("seven_day"));
        assert!(is_weekly_window("seven_day") && is_weekly_window("weekly_all"));
        assert!(!is_weekly_window("seven_day_opus") && !is_weekly_window("weekly_scoped:X"));
    }

    #[test]
    fn most_urgent_weekly_leftover_wins() {
        let (a, b, c) = (
            hub_account("a", None),
            hub_account("b", None),
            hub_account("c", None),
        );
        // a: 50% left in 100h = 0.5/h; b: 40% left in 20h = 2/h; c: 90% left, no reset = 90/168.
        let r = readings(vec![
            (&a, reading(10.0, 50.0, Some(100))),
            (&b, reading(10.0, 60.0, Some(20))),
            (&c, reading(10.0, 10.0, None)),
        ]);
        let accounts = [a, b.clone(), c];
        assert_eq!(auto_pick(&accounts, &r, &fresh, None, now()), Some(b.id));
    }

    #[test]
    fn tie_goes_to_more_five_hour_room_and_reset_under_an_hour_counts_as_one_hour() {
        let (a, b) = (hub_account("a", None), hub_account("b", None));
        let r = readings(vec![
            (&a, reading(60.0, 50.0, Some(0))),
            (&b, reading(20.0, 50.0, Some(1))),
        ]);
        let accounts = [a, b.clone()];
        assert_eq!(auto_pick(&accounts, &r, &fresh, None, now()), Some(b.id));
    }

    #[test]
    fn hysteresis_keeps_the_current_account_unless_clearly_beaten() {
        let (a, b) = (hub_account("a", None), hub_account("b", None));
        // a: 50 left / 10h = 5.0; b: 60 left / 10h = 6.0 (< 6.25): keep a.
        let r = readings(vec![
            (&a, reading(0.0, 50.0, Some(10))),
            (&b, reading(0.0, 40.0, Some(10))),
        ]);
        let accounts = [a.clone(), b.clone()];
        assert_eq!(
            auto_pick(&accounts, &r, &fresh, Some(a.id), now()),
            Some(a.id)
        );
        assert_eq!(auto_pick(&accounts, &r, &fresh, None, now()), Some(b.id));
        // b: 80 left / 10h = 8.0 >= 6.25: switch.
        let r = readings(vec![
            (&a, reading(0.0, 50.0, Some(10))),
            (&b, reading(0.0, 20.0, Some(10))),
        ]);
        assert_eq!(
            auto_pick(&accounts, &r, &fresh, Some(a.id), now()),
            Some(b.id)
        );
    }

    #[test]
    fn exclusions() {
        let mut benched = hub_account("benched", None);
        benched.hub.as_mut().unwrap().benched = true;
        let mut hidden = hub_account("hidden", None);
        hidden.hidden = true;
        let stale = hub_account("stale", None);
        let tired = hub_account("tired", None);
        let empty_week = hub_account("empty", None);
        let no_reading = hub_account("none", None);
        let mut cli_only = hub_account("cli", None);
        cli_only.hub = None;
        let ok = hub_account("ok", None);
        let good = |_: &Account| reading(0.0, 90.0, Some(5));
        let r = readings(vec![
            (&benched, good(&benched)),
            (&hidden, good(&hidden)),
            (&stale, good(&stale)),
            (&tired, reading(96.0, 90.0, Some(5))),
            (&empty_week, reading(0.0, 100.0, Some(5))),
            (&cli_only, good(&cli_only)),
            (&ok, reading(0.0, 10.0, Some(150))),
        ]);
        let stale_id = stale.id;
        let accounts = [
            benched,
            hidden,
            stale,
            tired,
            empty_week,
            no_reading,
            cli_only,
            ok.clone(),
        ];
        let is_stale = move |u: Uuid| u == stale_id;
        assert_eq!(
            auto_pick(&accounts, &r, &is_stale, None, now()),
            Some(ok.id)
        );
        assert_eq!(auto_pick(&accounts[..7], &r, &is_stale, None, now()), None);
        // Exactly 5% left in the 5-hour window is still allowed.
        let edge = hub_account("edge", None);
        let r = readings(vec![(&edge, reading(95.0, 50.0, Some(10)))]);
        assert_eq!(
            auto_pick(std::slice::from_ref(&edge), &r, &fresh, None, now()),
            Some(edge.id)
        );
    }

    #[test]
    fn limits_kind_keys_work_too() {
        let a = hub_account("a", None);
        let r = readings(vec![(
            &a,
            Reading {
                fetched_at: now(),
                windows: vec![
                    win("session", 10.0, None),
                    win("weekly_all", 20.0, Some(24)),
                ],
            },
        )]);
        assert_eq!(
            auto_pick(std::slice::from_ref(&a), &r, &fresh, None, now()),
            Some(a.id)
        );
    }

    #[test]
    fn target_rules() {
        let (a, mut b) = (hub_account("a", None), hub_account("b", None));
        let r = readings(vec![
            (&a, reading(0.0, 10.0, Some(5))),
            (&b, reading(0.0, 90.0, Some(100))),
        ]);
        let mut settings = Settings::default();
        let accounts = [a.clone(), b.clone()];
        assert_eq!(target(&settings, &accounts, &r, &fresh, now()), Some(a.id));
        settings.steering.preferred = Some(b.id);
        assert_eq!(target(&settings, &accounts, &r, &fresh, now()), Some(b.id));
        // Preferred but benched: back to Auto.
        b.hub.as_mut().unwrap().benched = true;
        let accounts = [a.clone(), b.clone()];
        assert_eq!(target(&settings, &accounts, &r, &fresh, now()), Some(a.id));
        // Preferred that is not a hub login at all: Auto.
        b.hub = None;
        let accounts = [a.clone(), b];
        assert_eq!(target(&settings, &accounts, &r, &fresh, now()), Some(a.id));
        settings.steering.enabled = false;
        assert_eq!(target(&settings, &accounts, &r, &fresh, now()), None);
    }

    #[test]
    fn plan_raises_target_remembers_originals_and_restores_others() {
        let (a, b) = (hub_account("a.json", Some(5)), hub_account("b.json", None));
        let mut steering = SteeringSettings::default();
        let accounts = [a.clone(), b.clone()];

        let (changes, originals) = plan(&accounts, &steering, Some(a.id));
        assert_eq!(
            changes,
            [PriorityChange {
                file_name: "a.json".into(),
                priority: Some(100)
            }]
        );
        assert_eq!(originals, HashMap::from([("a.json".to_string(), Some(5))]));
        steering.original_priorities = originals;

        // The hub now lists a at 100. Switching to b restores a and records b's original (none).
        let mut a_now = a.clone();
        a_now.hub.as_mut().unwrap().priority = Some(100);
        let accounts = [a_now.clone(), b.clone()];
        let (changes, originals) = plan(&accounts, &steering, Some(b.id));
        assert_eq!(
            changes,
            [
                PriorityChange {
                    file_name: "b.json".into(),
                    priority: Some(100)
                },
                PriorityChange {
                    file_name: "a.json".into(),
                    priority: Some(5)
                },
            ]
        );
        assert_eq!(originals, HashMap::from([("b.json".to_string(), None)]));
        steering.original_priorities = originals;

        // Same target again, hub already shows the result: nothing to do.
        let mut b_now = b.clone();
        b_now.hub.as_mut().unwrap().priority = Some(100);
        let mut a_back = a.clone();
        a_back.hub.as_mut().unwrap().priority = Some(5);
        let accounts = [a_back, b_now.clone()];
        let (changes, originals) = plan(&accounts, &steering, Some(b.id));
        assert!(changes.is_empty());
        assert_eq!(originals, steering.original_priorities);

        // Steering off: restore b to "no priority" and clear the map.
        let (changes, originals) = plan(&accounts, &steering, None);
        assert_eq!(
            changes,
            [PriorityChange {
                file_name: "b.json".into(),
                priority: None
            }]
        );
        assert!(originals.is_empty());
    }

    #[test]
    fn plan_keeps_originals_of_logins_that_are_not_listed_and_ignores_missing_target() {
        let a = hub_account("a.json", Some(1));
        let mut steering = SteeringSettings::default();
        steering
            .original_priorities
            .insert("gone.json".into(), Some(7));
        let (changes, originals) = plan(std::slice::from_ref(&a), &steering, Some(a.id));
        assert_eq!(changes.len(), 1);
        assert_eq!(originals.get("gone.json"), Some(&Some(7)));
        assert_eq!(originals.get("a.json"), Some(&Some(1)));
        // A target that is not a hub login restores like "off".
        let (changes, _) = plan(std::slice::from_ref(&a), &steering, Some(Uuid::new_v4()));
        assert!(changes.is_empty());
    }

    #[test]
    fn steering_settings_default_and_old_files_load() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert!(s.steering.enabled);
        assert_eq!(s.steering.top_priority, 100);
        assert!(s.steering.preferred.is_none());
    }
}
