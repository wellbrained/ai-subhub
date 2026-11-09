//! Reads one account: hub first when linked and reachable, profile as fallback (ADR 0001).
//! Blocking; the app calls it on a background thread.
use std::path::Path;

use crate::hub::{HubAccount, HubClient};
use crate::model::{Account, ProviderKind, ReadSource, UsageResult, UsageState};
use crate::util::failure;
use crate::{claude, codex};

/// Benched hub-only account -> Unavailable "Benched in hub". Hub error on a merged account ->
/// profile read. Never reports a reading it did not get.
///
/// A hub answer of `RateLimited` is returned as is. The cooldown belongs to the hub's login;
/// the app may then read the account's CLI profile (a different login, seen not throttled on
/// 2026-11-06) and skips the hub for that account until the cooldown ends (`AppState::refresh`).
pub fn read_account(
    account: &Account,
    hub: Option<&HubClient>,
    codex_exe: Option<&Path>,
) -> UsageResult {
    if let Some(link) = &account.hub {
        if link.benched {
            // A benched login is disabled in the hub; only a profile can still be read.
            if account.profile_dir.is_none() {
                return failure(UsageState::Unavailable, "Benched in hub", ReadSource::Hub);
            }
        } else if let Some(client) = hub {
            let login = HubAccount {
                provider: account.provider,
                email: account.email.clone(),
                file_name: link.file_name.clone(),
                auth_index: link.auth_index.clone(),
                disabled: link.benched,
                account_id: link.account_id.clone(),
                priority: link.priority,
            };
            let result = client.read_usage(&login);
            if result.state == UsageState::Ready
                || result.state == UsageState::RateLimited
                || account.profile_dir.is_none()
            {
                return result;
            }
        } else if account.profile_dir.is_none() {
            return failure(
                UsageState::Unavailable,
                "The hub is not connected. Check the hub settings.",
                ReadSource::Hub,
            );
        }
    }

    let Some(dir) = &account.profile_dir else {
        return failure(
            UsageState::Unavailable,
            "This account has no profile or hub login to read.",
            ReadSource::Profile,
        );
    };
    match account.provider {
        ProviderKind::Claude => claude::read_profile(dir),
        ProviderKind::Codex => codex::read_profile(dir, codex_exe),
        ProviderKind::Copilot => crate::copilot::read_profile(dir),
        ProviderKind::Cursor => crate::cursor::read_profile(dir),
        ProviderKind::Antigravity => crate::antigravity::read_profile(dir, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::HubLink;
    use serde_json::json;
    use uuid::Uuid;

    fn account(profile: Option<&Path>, hub: Option<HubLink>) -> Account {
        Account {
            id: Uuid::new_v4(),
            provider: ProviderKind::Claude,
            label: "A".into(),
            email: None,
            profile_dir: profile.map(Path::to_path_buf),
            hub,
            group: None,
            hidden: false,
            hidden_windows: vec![],
            pinned_windows: vec![],
        }
    }

    fn link(benched: bool) -> HubLink {
        HubLink {
            auth_index: json!(1),
            file_name: "c.json".into(),
            benched,
            account_id: None,
            priority: None,
        }
    }

    #[test]
    fn benched_hub_only_account_is_unavailable() {
        let r = read_account(&account(None, Some(link(true))), None, None);
        assert_eq!(
            (r.state, r.message.as_str()),
            (UsageState::Unavailable, "Benched in hub")
        );
        assert!(r.reading.is_none());
    }

    #[test]
    fn hub_only_account_without_hub_client_is_unavailable() {
        let r = read_account(&account(None, Some(link(false))), None, None);
        assert_eq!(
            (r.state, r.source),
            (UsageState::Unavailable, ReadSource::Hub)
        );
    }

    #[test]
    fn account_with_nothing_to_read_is_unavailable() {
        let r = read_account(&account(None, None), None, None);
        assert_eq!(r.state, UsageState::Unavailable);
    }

    #[test]
    fn merged_account_falls_back_to_profile_when_hub_is_missing_or_benched() {
        // Empty synthetic profile dir: the Claude profile read stops at "sign in" before any network.
        let dir = tempfile::tempdir().unwrap();
        for hub_link in [link(false), link(true)] {
            let r = read_account(&account(Some(dir.path()), Some(hub_link)), None, None);
            assert_eq!(
                (r.state, r.source),
                (UsageState::SignInRequired, ReadSource::Profile)
            );
        }
    }

    #[test]
    fn profile_only_account_reads_its_profile() {
        let dir = tempfile::tempdir().unwrap();
        let r = read_account(&account(Some(dir.path()), None), None, None);
        assert_eq!(
            (r.state, r.source),
            (UsageState::SignInRequired, ReadSource::Profile)
        );
    }

    #[test]
    fn codex_profile_with_missing_cli_is_cli_missing() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = account(Some(dir.path()), None);
        a.provider = ProviderKind::Codex;
        let r = read_account(&a, None, Some(&dir.path().join("no-such-codex.exe")));
        assert_eq!(r.state, UsageState::CliMissing);
    }
}
