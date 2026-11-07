//! Merging hub logins into the account list (ADR 0001: merge by email, case-insensitive).
use uuid::Uuid;

use crate::hub::HubAccount;
use crate::model::{Account, HubLink};

#[derive(Debug, Default, PartialEq)]
pub struct MergeReport {
    pub added: usize,
    pub linked: usize,
    pub updated: usize,
    pub unlinked: usize,
}

/// Applies the hub's current list:
/// - same provider + email as an existing account: set/refresh its `hub` link (merged account);
/// - matching `hub.file_name`: refresh index/benched;
/// - new login: append a hub-only account labelled from the email's local part;
/// - account whose hub login vanished: drop the `hub` link (hub-only accounts stay, shown
///   unavailable; the user removes them).
///
/// Never touches labels, groups, pins or order of existing accounts. File-name matches are
/// resolved before email matches; each account and each hub login is used at most once.
pub fn apply_hub_accounts(accounts: &mut Vec<Account>, hub: &[HubAccount]) -> MergeReport {
    let mut report = MergeReport::default();
    let mut claimed_account = vec![false; accounts.len()];
    let mut claimed_hub = vec![false; hub.len()];

    // Pass 1: the account already linked to this hub file.
    for (hi, h) in hub.iter().enumerate() {
        if h.file_name.is_empty() {
            continue;
        }
        let found = accounts.iter().enumerate().position(|(ai, a)| {
            !claimed_account[ai]
                && a.provider == h.provider
                && a.hub.as_ref().is_some_and(|l| l.file_name == h.file_name)
        });
        if let Some(ai) = found {
            link(&mut accounts[ai], h, &mut report);
            claimed_account[ai] = true;
            claimed_hub[hi] = true;
        }
    }

    // Pass 2: same provider and email (case-insensitive).
    for (hi, h) in hub.iter().enumerate() {
        let Some(email) = h.email.as_deref().filter(|_| !claimed_hub[hi]) else {
            continue;
        };
        // An unlinked CLI-profile account is preferred over a hub-only one.
        let matches = |ai: usize, a: &Account| {
            !claimed_account[ai]
                && a.provider == h.provider
                && a.email
                    .as_deref()
                    .is_some_and(|e| e.eq_ignore_ascii_case(email))
        };
        let found = accounts
            .iter()
            .enumerate()
            .position(|(ai, a)| matches(ai, a) && a.profile_dir.is_some())
            .or_else(|| {
                accounts
                    .iter()
                    .enumerate()
                    .position(|(ai, a)| matches(ai, a))
            });
        if let Some(ai) = found {
            link(&mut accounts[ai], h, &mut report);
            claimed_account[ai] = true;
            claimed_hub[hi] = true;
        }
    }

    // Accounts whose hub login is gone.
    for (ai, account) in accounts.iter_mut().enumerate() {
        if !claimed_account[ai] && account.hub.take().is_some() {
            report.unlinked += 1;
        }
    }

    // New hub-only accounts. Default labels that collide with another account of the same
    // provider get the email domain; earlier collisions are only renamed if added in this pass.
    let mut added_ids: Vec<Uuid> = vec![];
    for (hi, h) in hub.iter().enumerate() {
        if claimed_hub[hi] {
            continue;
        }
        let mut label = label_for(h);
        let collides = |label: &str, accounts: &[Account]| {
            accounts
                .iter()
                .any(|a| a.provider == h.provider && a.label.eq_ignore_ascii_case(label))
        };
        if collides(&label, accounts) {
            if let Some(q) = qualified_label(h) {
                label = q;
            }
            let default = label_for(h);
            for a in accounts.iter_mut() {
                if added_ids.contains(&a.id)
                    && a.provider == h.provider
                    && a.label.eq_ignore_ascii_case(&default)
                    && let Some(email) = a.email.clone()
                {
                    a.label = qualify(&email);
                }
            }
        }
        let id = Uuid::new_v4();
        added_ids.push(id);
        accounts.push(Account {
            id,
            provider: h.provider,
            label,
            email: h.email.clone(),
            profile_dir: None,
            hub: Some(hub_link(h)),
            group: None,
            hidden: false,
            hidden_windows: vec![],
            pinned_windows: vec![],
        });
        report.added += 1;
    }
    report
}

fn hub_link(h: &HubAccount) -> HubLink {
    HubLink {
        auth_index: h.auth_index.clone(),
        file_name: h.file_name.clone(),
        benched: h.disabled,
        account_id: h.account_id.clone(),
        priority: h.priority,
    }
}

fn link(account: &mut Account, h: &HubAccount, report: &mut MergeReport) {
    let new = hub_link(h);
    match &account.hub {
        None => {
            account.hub = Some(new);
            report.linked += 1;
        }
        Some(old) if *old != new => {
            account.hub = Some(new);
            report.updated += 1;
        }
        Some(_) => {}
    }
    if account.email.is_none() {
        account.email = h.email.clone();
    }
}

/// "<local> · <first part of the domain>", cut to 60 characters.
pub(crate) fn qualify(email: &str) -> String {
    let (local, domain) = email.split_once('@').unwrap_or((email, ""));
    let first = domain.split('.').next().unwrap_or("");
    let label = if first.is_empty() {
        local.to_string()
    } else {
        format!("{local} · {first}")
    };
    label.chars().take(60).collect()
}

fn qualified_label(h: &HubAccount) -> Option<String> {
    h.email.as_deref().map(qualify)
}

/// Folds hub-only accounts into the CLI-profile account with the same provider and email
/// (case-insensitive): the profile account (no hub link yet) takes the hub link and the union of
/// pinned/hidden windows, and keeps its label, group, hidden flag and position. The hub-only
/// account is removed. Returns `(removed, kept)` ids so the caller can move readings/schedules.
pub fn fold_duplicates(accounts: &mut Vec<Account>) -> Vec<(Uuid, Uuid)> {
    let mut folded = vec![];
    let mut i = 0;
    while i < accounts.len() {
        let is_profile = {
            let a = &accounts[i];
            a.profile_dir.is_some() && a.hub.is_none() && a.email.is_some()
        };
        if is_profile {
            let email = accounts[i].email.clone().unwrap_or_default();
            let provider = accounts[i].provider;
            let dup = accounts.iter().position(|b| {
                b.profile_dir.is_none()
                    && b.provider == provider
                    && b.email
                        .as_deref()
                        .is_some_and(|e| e.eq_ignore_ascii_case(&email))
            });
            if let Some(j) = dup {
                let removed = accounts.remove(j);
                // Removing an earlier element shifts the profile account down by one.
                let k = if j < i { i - 1 } else { i };
                let kept = &mut accounts[k];
                kept.hub = removed.hub;
                for key in removed.pinned_windows {
                    if !kept.pinned_windows.contains(&key) {
                        kept.pinned_windows.push(key);
                    }
                }
                for key in removed.hidden_windows {
                    if !kept.hidden_windows.contains(&key) {
                        kept.hidden_windows.push(key);
                    }
                }
                folded.push((removed.id, kept.id));
                // Re-check the same profile account: more duplicates may exist.
                i = k;
                continue;
            }
        }
        i += 1;
    }
    folded
}

fn label_for(h: &HubAccount) -> String {
    let stem = h
        .file_name
        .rsplit_once('.')
        .map_or(h.file_name.as_str(), |(s, _)| s);
    let base = h
        .email
        .as_deref()
        .and_then(|e| e.split('@').next())
        .filter(|l| !l.is_empty())
        .or(Some(stem).filter(|s| !s.is_empty()))
        .unwrap_or("Hub account");
    base.chars().take(60).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ProviderKind;
    use crate::model::ProviderKind::{Claude, Codex};
    use serde_json::json;

    fn account(provider: ProviderKind, label: &str, email: Option<&str>, profile: bool) -> Account {
        Account {
            id: Uuid::new_v4(),
            provider,
            label: label.into(),
            email: email.map(Into::into),
            profile_dir: profile.then(|| "C:\\fake\\profile".into()),
            hub: None,
            group: Some("Work".into()),
            hidden: false,
            hidden_windows: vec![],
            pinned_windows: vec!["five_hour".into()],
        }
    }

    fn hub(
        provider: ProviderKind,
        email: Option<&str>,
        file: &str,
        index: i64,
        disabled: bool,
    ) -> HubAccount {
        HubAccount {
            provider,
            email: email.map(Into::into),
            file_name: file.into(),
            auth_index: json!(index),
            disabled,
            account_id: None,
            priority: None,
        }
    }

    #[test]
    fn email_match_is_case_insensitive_and_keeps_user_data() {
        let mut accounts = vec![account(
            Claude,
            "My Claude",
            Some("Owner@Example.test"),
            true,
        )];
        let before = accounts[0].clone();
        let r = apply_hub_accounts(
            &mut accounts,
            &[hub(Claude, Some("owner@example.test"), "c.json", 1, false)],
        );
        assert_eq!(
            r,
            MergeReport {
                linked: 1,
                ..Default::default()
            }
        );
        assert_eq!(accounts.len(), 1);
        assert!(accounts[0].is_merged());
        assert_eq!(accounts[0].id, before.id);
        assert_eq!(accounts[0].label, "My Claude");
        assert_eq!(accounts[0].group, before.group);
        assert_eq!(accounts[0].pinned_windows, before.pinned_windows);
        assert_eq!(accounts[0].email, before.email);
    }

    #[test]
    fn same_email_on_another_provider_does_not_merge() {
        let mut accounts = vec![account(Codex, "Codex", Some("owner@example.test"), true)];
        let r = apply_hub_accounts(
            &mut accounts,
            &[hub(Claude, Some("owner@example.test"), "c.json", 1, false)],
        );
        assert_eq!((r.added, r.linked), (1, 0));
        assert_eq!(accounts.len(), 2);
        assert!(accounts[0].hub.is_none());
    }

    #[test]
    fn new_login_becomes_hub_only_account_labelled_from_email() {
        let mut accounts = vec![];
        let r = apply_hub_accounts(
            &mut accounts,
            &[hub(Claude, Some("owner.d@example.test"), "c.json", 3, true)],
        );
        assert_eq!(r.added, 1);
        let a = &accounts[0];
        assert_eq!(
            (a.label.as_str(), a.profile_dir.is_none()),
            ("owner.d", true)
        );
        let link = a.hub.as_ref().unwrap();
        assert!(link.benched);
        assert_eq!(
            (link.auth_index.clone(), link.file_name.as_str()),
            (json!(3), "c.json")
        );
    }

    #[test]
    fn file_name_match_refreshes_index_and_benched_even_if_email_changed() {
        let mut accounts = vec![account(Claude, "A", Some("old@example.test"), false)];
        accounts[0].hub = Some(HubLink {
            auth_index: json!(1),
            file_name: "c.json".into(),
            benched: false,
            account_id: None,
            priority: None,
        });
        let r = apply_hub_accounts(
            &mut accounts,
            &[hub(Claude, Some("new@example.test"), "c.json", 9, true)],
        );
        assert_eq!(
            r,
            MergeReport {
                updated: 1,
                ..Default::default()
            }
        );
        assert_eq!(accounts.len(), 1);
        let link = accounts[0].hub.as_ref().unwrap();
        assert_eq!((link.auth_index.clone(), link.benched), (json!(9), true));
        // Running again with the same list changes nothing.
        let r = apply_hub_accounts(
            &mut accounts,
            &[hub(Claude, Some("new@example.test"), "c.json", 9, true)],
        );
        assert_eq!(r, MergeReport::default());
    }

    #[test]
    fn vanished_login_drops_link_but_keeps_account() {
        let mut accounts = vec![
            account(Claude, "Merged", Some("a@example.test"), true),
            account(Claude, "HubOnly", None, false),
        ];
        for a in &mut accounts {
            a.hub = Some(HubLink {
                auth_index: json!(1),
                file_name: format!("{}.json", a.label),
                benched: false,
                account_id: None,
                priority: None,
            });
        }
        let r = apply_hub_accounts(&mut accounts, &[]);
        assert_eq!(
            r,
            MergeReport {
                unlinked: 2,
                ..Default::default()
            }
        );
        assert_eq!(accounts.len(), 2);
        assert!(accounts.iter().all(|a| a.hub.is_none()));
    }

    #[test]
    fn each_account_and_login_is_used_once() {
        let mut accounts = vec![account(Claude, "A", Some("a@example.test"), true)];
        let list = [
            hub(Claude, Some("a@example.test"), "one.json", 1, false),
            hub(Claude, Some("a@example.test"), "two.json", 2, false),
        ];
        let r = apply_hub_accounts(&mut accounts, &list);
        assert_eq!((r.linked, r.added), (1, 1));
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].hub.as_ref().unwrap().file_name, "one.json");
    }

    #[test]
    fn missing_email_is_filled_and_long_labels_are_cut() {
        let mut accounts = vec![account(Claude, "A", None, true)];
        accounts[0].hub = Some(HubLink {
            auth_index: json!(1),
            file_name: "c.json".into(),
            benched: false,
            account_id: None,
            priority: None,
        });
        apply_hub_accounts(
            &mut accounts,
            &[hub(Claude, Some("a@example.test"), "c.json", 1, false)],
        );
        assert_eq!(accounts[0].email.as_deref(), Some("a@example.test"));
        let long = format!("{}@example.test", "x".repeat(100));
        let mut accounts = vec![];
        apply_hub_accounts(
            &mut accounts,
            &[hub(Claude, Some(&long), "c.json", 1, false)],
        );
        assert_eq!(accounts[0].label.chars().count(), 60);
        let mut accounts = vec![];
        apply_hub_accounts(
            &mut accounts,
            &[hub(Codex, None, "codex-main.json", 1, false)],
        );
        assert_eq!(accounts[0].label, "codex-main");
    }

    #[test]
    fn colliding_default_labels_get_the_domain_for_accounts_added_in_this_pass() {
        let mut accounts = vec![];
        let list = [
            hub(Claude, Some("alex.m@gmail.com"), "a.json", 1, false),
            hub(Claude, Some("alex.m@hotmail.de"), "b.json", 2, false),
        ];
        apply_hub_accounts(&mut accounts, &list);
        assert_eq!(accounts[0].label, "alex.m · gmail");
        assert_eq!(accounts[1].label, "alex.m · hotmail");
        // Other providers do not collide.
        let mut accounts = vec![];
        let list = [
            hub(Claude, Some("a@gmail.com"), "a.json", 1, false),
            hub(Codex, Some("a@gmail.com"), "b.json", 2, false),
        ];
        apply_hub_accounts(&mut accounts, &list);
        assert_eq!(
            (accounts[0].label.as_str(), accounts[1].label.as_str()),
            ("a", "a")
        );
    }

    #[test]
    fn collision_with_a_user_account_qualifies_only_the_new_one() {
        let mut accounts = vec![account(Claude, "alex.m", Some("other@example.test"), true)];
        apply_hub_accounts(
            &mut accounts,
            &[hub(Claude, Some("alex.m@gmail.com"), "a.json", 1, false)],
        );
        assert_eq!(accounts[0].label, "alex.m");
        assert_eq!(accounts[1].label, "alex.m · gmail");
    }

    #[test]
    fn account_id_is_carried_and_a_change_counts_as_updated() {
        let mut h = hub(Codex, Some("a@example.test"), "c.json", 1, false);
        h.account_id = Some("acc-1".into());
        let mut accounts = vec![];
        apply_hub_accounts(&mut accounts, std::slice::from_ref(&h));
        assert_eq!(
            accounts[0].hub.as_ref().unwrap().account_id.as_deref(),
            Some("acc-1")
        );
        assert_eq!(
            apply_hub_accounts(&mut accounts, std::slice::from_ref(&h)),
            MergeReport::default()
        );
        h.account_id = Some("acc-2".into());
        let r = apply_hub_accounts(&mut accounts, std::slice::from_ref(&h));
        assert_eq!(
            r,
            MergeReport {
                updated: 1,
                ..Default::default()
            }
        );
        assert_eq!(
            accounts[0].hub.as_ref().unwrap().account_id.as_deref(),
            Some("acc-2")
        );
    }

    #[test]
    fn pass_two_prefers_an_unlinked_profile_account_over_a_hub_only_one() {
        let hub_only = account(Claude, "HubOnly", Some("a@example.test"), false);
        let profile = account(Claude, "Profile", Some("A@example.test"), true);
        let profile_id = profile.id;
        let mut accounts = vec![hub_only, profile];
        let r = apply_hub_accounts(
            &mut accounts,
            &[hub(Claude, Some("a@example.test"), "c.json", 1, false)],
        );
        assert_eq!((r.linked, r.added), (1, 0));
        let linked = accounts.iter().find(|a| a.id == profile_id).unwrap();
        assert!(linked.is_merged());
    }

    #[test]
    fn fold_moves_hub_link_into_profile_account_and_unions_windows() {
        let mut profile = account(Claude, "Mine", Some("A@Example.test"), true);
        profile.pinned_windows = vec!["five_hour".into()];
        profile.hidden_windows = vec!["x".into()];
        let mut hub_only = account(Claude, "a", Some("a@example.test"), false);
        hub_only.hub = Some(HubLink {
            auth_index: json!(4),
            file_name: "a.json".into(),
            benched: false,
            account_id: None,
            priority: None,
        });
        hub_only.pinned_windows = vec!["five_hour".into(), "seven_day".into()];
        hub_only.hidden_windows = vec!["y".into()];
        hub_only.group = Some("Other".into());
        hub_only.hidden = true;
        let other = account(Claude, "Other", Some("b@example.test"), true);
        let (pid, hid) = (profile.id, hub_only.id);
        let mut accounts = vec![hub_only, other, profile];
        let folded = fold_duplicates(&mut accounts);
        assert_eq!(folded, vec![(hid, pid)]);
        assert_eq!(accounts.len(), 2);
        let kept = accounts.iter().find(|a| a.id == pid).unwrap();
        assert_eq!(kept.label, "Mine");
        assert_eq!(kept.group.as_deref(), Some("Work"));
        assert!(!kept.hidden);
        assert_eq!(kept.hub.as_ref().unwrap().file_name, "a.json");
        assert_eq!(kept.pinned_windows, ["five_hour", "seven_day"]);
        assert_eq!(kept.hidden_windows, ["x", "y"]);
        // Position of the profile account is kept (second now that the first was removed).
        assert_eq!(accounts[1].id, pid);
        assert!(fold_duplicates(&mut accounts).is_empty());
    }

    #[test]
    fn fold_leaves_other_providers_linked_profiles_and_emailless_accounts_alone() {
        let profile = account(Claude, "P", Some("a@example.test"), true);
        let other_provider = account(Codex, "H", Some("a@example.test"), false);
        let no_email = account(Claude, "N", None, true);
        let mut linked = account(Claude, "L", Some("l@example.test"), true);
        linked.hub = Some(HubLink {
            auth_index: json!(1),
            file_name: "l.json".into(),
            benched: false,
            account_id: None,
            priority: None,
        });
        let hub_only_same = account(Claude, "H2", Some("l@example.test"), false);
        let mut accounts = vec![profile, other_provider, no_email, linked, hub_only_same];
        assert!(fold_duplicates(&mut accounts).is_empty());
        assert_eq!(accounts.len(), 5);
    }
}
