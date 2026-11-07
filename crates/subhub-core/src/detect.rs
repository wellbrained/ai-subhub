//! Profile detection: finds the CLI profiles on this PC (`profile::discover_dirs`), tells for
//! each whether importing it would add a new account, join an existing one, or do nothing, and
//! imports the chosen ones. Also the one place that attaches a profile to the account list, so
//! "Add account" and the import share the no-duplicates rule (owner rule): a folder already
//! connected is skipped, a login (email from the profile's own metadata) that belongs to an
//! account without a profile is attached to that account, a login that already has a profile
//! folder is never added again. Only the email metadata is read, never a token.
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::merge::qualify;
use crate::model::{Account, ProviderKind};
use crate::profile::{ProfileEnv, discover_dirs, profile_email, same_dir};

/// What importing a detected profile would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectStatus {
    /// Becomes a new account.
    New,
    /// Attached to this existing account (same login, no profile yet, e.g. a hub account).
    Joins { account: Uuid, label: String },
    /// The folder is already connected to this account.
    Added { account: Uuid, label: String },
    /// Another folder (connected or found earlier) already has this login; never imported.
    SameLogin { dir: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedProfile {
    pub provider: ProviderKind,
    /// Canonical folder.
    pub dir: PathBuf,
    /// Login email from the profile's metadata; None = no login found.
    pub email: Option<String>,
    pub status: DetectStatus,
}

impl DetectedProfile {
    pub fn importable(&self) -> bool {
        matches!(self.status, DetectStatus::New | DetectStatus::Joins { .. })
    }

    /// Ticked when the list opens: importable with a known login.
    pub fn preselected(&self) -> bool {
        self.importable() && self.email.is_some()
    }
}

/// A folder counts as a profile when it has the provider's login file or its log folder.
fn is_profile(provider: ProviderKind, dir: &Path) -> bool {
    match provider {
        ProviderKind::Claude => dir.join(".claude.json").is_file() || dir.join("projects").is_dir(),
        ProviderKind::Codex => dir.join("auth.json").is_file() || dir.join("sessions").is_dir(),
    }
}

/// Same folder, also when one side is not canonical (or no longer exists).
pub fn same_folder(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    same_dir(a, b) || same_dir(&canon(a), &canon(b))
}

fn same_email(a: Option<&str>, b: &str) -> bool {
    a.is_some_and(|a| a.eq_ignore_ascii_case(b))
}

/// Finds the profiles on disk and their status against the current accounts.
pub fn detect(home: &Path, env: &ProfileEnv, accounts: &[Account]) -> Vec<DetectedProfile> {
    let found = discover_dirs(home, env)
        .into_iter()
        .filter(|(p, d)| is_profile(*p, d))
        .map(|(p, d)| {
            let email = profile_email(p, &d);
            (p, d, email)
        })
        .collect();
    classify(found, accounts)
}

/// Statuses for found folders, in order (the first folder of a login wins).
pub fn classify(
    found: Vec<(ProviderKind, PathBuf, Option<String>)>,
    accounts: &[Account],
) -> Vec<DetectedProfile> {
    let mut out: Vec<DetectedProfile> = vec![];
    for (provider, dir, email) in found {
        let status = status_for(provider, &dir, email.as_deref(), accounts, &out);
        out.push(DetectedProfile {
            provider,
            dir,
            email,
            status,
        });
    }
    out
}

fn status_for(
    provider: ProviderKind,
    dir: &Path,
    email: Option<&str>,
    accounts: &[Account],
    earlier: &[DetectedProfile],
) -> DetectStatus {
    let mine = || accounts.iter().filter(|a| a.provider == provider);
    if let Some(a) = mine().find(|a| {
        a.profile_dir
            .as_deref()
            .is_some_and(|p| same_folder(p, dir))
    }) {
        return DetectStatus::Added {
            account: a.id,
            label: a.label.clone(),
        };
    }
    let Some(email) = email else {
        return DetectStatus::New;
    };
    if let Some(p) = mine()
        .filter(|a| same_email(a.email.as_deref(), email))
        .find_map(|a| a.profile_dir.clone())
    {
        return DetectStatus::SameLogin { dir: p };
    }
    if let Some(e) = earlier
        .iter()
        .find(|e| e.provider == provider && e.importable() && same_email(e.email.as_deref(), email))
    {
        return DetectStatus::SameLogin { dir: e.dir.clone() };
    }
    if let Some(a) = mine().find(|a| same_email(a.email.as_deref(), email)) {
        return DetectStatus::Joins {
            account: a.id,
            label: a.label.clone(),
        };
    }
    DetectStatus::New
}

/// Result of attaching one profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attach {
    Added(Uuid),
    Linked { account: Uuid, label: String },
    Duplicate,
}

/// Adds a CLI-profile account without ever creating a duplicate (see the module comment). The
/// email is read from the profile's metadata here; a linked account keeps its name, tag, star,
/// order and readings.
pub fn attach_profile(accounts: &mut Vec<Account>, mut acc: Account) -> Attach {
    let Some(dir) = acc.profile_dir.clone() else {
        return Attach::Duplicate;
    };
    if accounts.iter().any(|a| {
        a.provider == acc.provider
            && a.profile_dir
                .as_deref()
                .is_some_and(|p| same_folder(p, &dir))
    }) {
        return Attach::Duplicate;
    }
    acc.email = profile_email(acc.provider, &dir);
    if let Some(email) = acc.email.clone() {
        let same =
            |a: &Account| a.provider == acc.provider && same_email(a.email.as_deref(), &email);
        if accounts.iter().any(|a| same(a) && a.profile_dir.is_some()) {
            // Same login already has another profile folder: keep the first.
            return Attach::Duplicate;
        }
        if let Some(existing) = accounts.iter_mut().find(|a| same(a)) {
            existing.profile_dir = Some(dir);
            return Attach::Linked {
                account: existing.id,
                label: existing.label.clone(),
            };
        }
    }
    let id = acc.id;
    accounts.push(acc);
    Attach::Added(id)
}

/// Name of a new account: the email's local part (with the domain when another account of the
/// provider already uses it, as for hub accounts), else the folder name without its dot.
pub fn default_label(
    accounts: &[Account],
    provider: ProviderKind,
    email: Option<&str>,
    dir: &Path,
) -> String {
    let folder = dir
        .file_name()
        .map(|n| n.to_string_lossy().trim_start_matches('.').to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Profile".into());
    let base = email
        .and_then(|e| e.split('@').next())
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .unwrap_or(folder);
    let base: String = base.chars().take(60).collect();
    let taken = |l: &str| {
        accounts
            .iter()
            .any(|a| a.provider == provider && a.label.eq_ignore_ascii_case(l))
    };
    match email {
        Some(e) if taken(&base) => qualify(e),
        _ => base,
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ImportReport {
    /// New accounts, in import order.
    pub added: Vec<Uuid>,
    /// Existing accounts that got the profile.
    pub linked: Vec<Uuid>,
    /// Picks that turned out to be duplicates.
    pub skipped: usize,
}

/// Imports the picked profiles (only importable ones; everything goes through `attach_profile`,
/// so a pick made stale by an earlier one in the same batch is skipped, not duplicated).
pub fn import(accounts: &mut Vec<Account>, picks: &[DetectedProfile]) -> ImportReport {
    let mut report = ImportReport::default();
    for p in picks.iter().filter(|p| p.importable()) {
        let label = default_label(accounts, p.provider, p.email.as_deref(), &p.dir);
        let acc = Account {
            id: Uuid::new_v4(),
            provider: p.provider,
            label,
            email: None,
            profile_dir: Some(p.dir.clone()),
            hub: None,
            group: None,
            hidden: false,
            hidden_windows: vec![],
            pinned_windows: vec![],
        };
        match attach_profile(accounts, acc) {
            Attach::Added(id) => report.added.push(id),
            Attach::Linked { account, .. } => report.linked.push(account),
            Attach::Duplicate => report.skipped += 1,
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::HubLink;

    fn claude_profile(dir: &Path, email: Option<&str>) {
        std::fs::create_dir_all(dir).unwrap();
        let body = match email {
            Some(e) => format!(r#"{{"oauthAccount":{{"emailAddress":"{e}"}}}}"#),
            None => "{}".into(),
        };
        std::fs::write(dir.join(".claude.json"), body).unwrap();
    }

    fn account(provider: ProviderKind, label: &str, email: Option<&str>) -> Account {
        Account {
            id: Uuid::new_v4(),
            provider,
            label: label.into(),
            email: email.map(Into::into),
            profile_dir: None,
            hub: None,
            group: None,
            hidden: false,
            hidden_windows: vec![],
            pinned_windows: vec![],
        }
    }

    fn hub_only(label: &str, email: &str) -> Account {
        let mut a = account(ProviderKind::Claude, label, Some(email));
        a.hub = Some(HubLink {
            auth_index: serde_json::Value::Null,
            file_name: format!("{email}.json"),
            benched: false,
            account_id: None,
            priority: None,
        });
        a
    }

    #[test]
    fn only_folders_with_login_file_or_logs_count() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        claude_profile(&h.join(".claude-a"), Some("a@example.test"));
        std::fs::create_dir_all(h.join(".claude-logs").join("projects")).unwrap();
        std::fs::create_dir_all(h.join(".claude-empty")).unwrap();
        std::fs::create_dir_all(h.join(".codex").join("sessions")).unwrap();
        std::fs::create_dir_all(h.join(".codex-empty")).unwrap();
        let found = detect(h, &ProfileEnv::default(), &[]);
        let names: Vec<String> = found
            .iter()
            .map(|d| d.dir.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![".claude-a", ".claude-logs", ".codex"]);
        assert_eq!(found[0].email.as_deref(), Some("a@example.test"));
        assert!(found[0].preselected());
        // No login found: importable, but not ticked.
        assert_eq!(found[1].email, None);
        assert!(found[1].importable() && !found[1].preselected());
    }

    #[test]
    fn statuses_new_joins_added_and_same_login() {
        let c = ProviderKind::Claude;
        let mut linked = account(c, "Work", Some("w@example.test"));
        linked.profile_dir = Some(PathBuf::from("C:\\p\\.claude-work"));
        let hub = hub_only("pers", "p@example.test");
        let hub_id = hub.id;
        let accounts = vec![linked.clone(), hub];
        let found = vec![
            (
                c,
                PathBuf::from("C:\\P\\.claude-work\\"),
                Some("w@example.test".into()),
            ),
            (
                c,
                PathBuf::from("C:\\p\\.claude-w2"),
                Some("W@example.test".into()),
            ),
            (
                c,
                PathBuf::from("C:\\p\\.claude-p"),
                Some("p@example.test".into()),
            ),
            (
                c,
                PathBuf::from("C:\\p\\.claude-p2"),
                Some("p@example.test".into()),
            ),
            (
                c,
                PathBuf::from("C:\\p\\.claude-n"),
                Some("n@example.test".into()),
            ),
            (
                ProviderKind::Codex,
                PathBuf::from("C:\\p\\.codex"),
                Some("w@example.test".into()),
            ),
        ];
        let st: Vec<DetectStatus> = classify(found, &accounts)
            .into_iter()
            .map(|d| d.status)
            .collect();
        assert_eq!(
            st,
            vec![
                DetectStatus::Added {
                    account: linked.id,
                    label: "Work".into()
                },
                DetectStatus::SameLogin {
                    dir: PathBuf::from("C:\\p\\.claude-work")
                },
                DetectStatus::Joins {
                    account: hub_id,
                    label: "pers".into()
                },
                DetectStatus::SameLogin {
                    dir: PathBuf::from("C:\\p\\.claude-p")
                },
                DetectStatus::New,
                // Another provider's login is a different account.
                DetectStatus::New,
            ]
        );
    }

    #[test]
    fn import_adds_links_and_never_duplicates() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        claude_profile(&h.join(".claude-a"), Some("a@example.test"));
        claude_profile(&h.join(".claude-b"), Some("b@example.test"));
        claude_profile(&h.join(".claude-x"), None);
        let hub = hub_only("b", "b@example.test");
        let hub_id = hub.id;
        let mut accounts = vec![hub];
        let found = detect(h, &ProfileEnv::default(), &accounts);
        let report = import(&mut accounts, &found);
        assert_eq!(report.added.len(), 2);
        assert_eq!(report.linked, vec![hub_id]);
        assert_eq!(accounts.len(), 3);
        assert!(
            accounts[0].profile_dir.is_some(),
            "hub account got the profile"
        );
        assert_eq!(accounts[1].label, "a");
        assert_eq!(accounts[1].email.as_deref(), Some("a@example.test"));
        assert_eq!(accounts[2].label, "claude-x");

        // Detecting again: everything is already added; importing again changes nothing.
        let again = detect(h, &ProfileEnv::default(), &accounts);
        assert!(again.iter().all(|d| !d.importable()));
        let before = accounts.clone();
        let report = import(&mut accounts, &found); // stale picks
        assert_eq!(report.skipped, 3);
        assert_eq!(accounts, before);
    }

    #[test]
    fn hub_listed_after_import_links_instead_of_duplicating() {
        // CLI import first (e.g. the hub key is entered later), then the hub's refresh.
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        claude_profile(&h.join(".claude-a"), Some("a@example.test"));
        let mut accounts = vec![];
        let found = detect(h, &ProfileEnv::default(), &accounts);
        import(&mut accounts, &found);
        let hub = vec![
            crate::hub::HubAccount {
                provider: ProviderKind::Claude,
                email: Some("A@example.test".into()),
                file_name: "a.json".into(),
                auth_index: serde_json::Value::Null,
                disabled: false,
                account_id: None,
                priority: None,
            },
            crate::hub::HubAccount {
                provider: ProviderKind::Claude,
                email: Some("only-hub@example.test".into()),
                file_name: "o.json".into(),
                auth_index: serde_json::Value::Null,
                disabled: false,
                account_id: None,
                priority: None,
            },
        ];
        crate::merge::apply_hub_accounts(&mut accounts, &hub);
        crate::merge::fold_duplicates(&mut accounts);
        assert_eq!(accounts.len(), 2, "one account per login");
        assert!(accounts[0].profile_dir.is_some() && accounts[0].hub.is_some());
        assert!(accounts[1].profile_dir.is_none() && accounts[1].hub.is_some());
        // And detecting again offers nothing.
        assert!(
            detect(h, &ProfileEnv::default(), &accounts)
                .iter()
                .all(|d| !d.importable())
        );
    }

    #[test]
    fn same_login_in_one_batch_is_imported_once() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        claude_profile(&h.join(".claude-1"), Some("s@example.test"));
        claude_profile(&h.join(".claude-2"), Some("s@example.test"));
        let mut accounts = vec![];
        let mut found = detect(h, &ProfileEnv::default(), &accounts);
        // Even if the second were (wrongly) marked importable, attach refuses it.
        found[1].status = DetectStatus::New;
        let report = import(&mut accounts, &found);
        assert_eq!((report.added.len(), report.skipped), (1, 1));
        assert_eq!(accounts.len(), 1);
    }

    #[test]
    fn labels_follow_the_hub_rule() {
        let c = ProviderKind::Claude;
        let accounts = vec![account(c, "me", None)];
        let dir = Path::new("C:\\u\\.claude-work");
        assert_eq!(default_label(&[], c, Some("me@corp.test"), dir), "me");
        assert_eq!(
            default_label(&accounts, c, Some("me@corp.test"), dir),
            "me · corp"
        );
        assert_eq!(
            default_label(&accounts, ProviderKind::Codex, Some("me@corp.test"), dir),
            "me"
        );
        assert_eq!(default_label(&[], c, None, dir), "claude-work");
    }
}
