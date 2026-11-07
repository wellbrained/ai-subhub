//! Hub management key in Windows Credential Manager. Never written to settings, logs or errors.
use anyhow::Context;

pub const SERVICE: &str = "AI SubHub";
pub const HUB_KEY_USER: &str = "hub-management-key";

/// Accepts a bare key or pasted `name = value` lines and returns the management key.
/// With several lines the one mentioning "manag" wins, else the last line.
pub fn normalize_hub_key(raw: &str) -> String {
    let lines: Vec<&str> = raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.len() == 1 && !lines[0].contains(char::is_whitespace) {
        return lines[0].to_string();
    }
    let pick = lines
        .iter()
        .find(|l| l.to_ascii_lowercase().contains("manag"))
        .or(lines.last())
        .copied()
        .unwrap_or_default();
    let value = pick.rsplit(['=', ':']).next().unwrap_or(pick);
    value.trim().trim_matches(['"', '\'', '`']).to_string()
}

/// Service name before the rename to AI SubHub (2026-11-07); a key stored there is moved over.
const LEGACY_SERVICE: &str = "AI SubHub GPUI";

fn entry() -> anyhow::Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, HUB_KEY_USER).context("Credential Manager is not available")
}

/// Moves a key from the old service name to the current one (once; best effort).
fn migrate_legacy_key() {
    let Ok(legacy) = keyring::Entry::new(LEGACY_SERVICE, HUB_KEY_USER) else {
        return;
    };
    if let Ok(v) = legacy.get_password()
        && let Ok(e) = entry()
        && e.set_password(&v).is_ok()
    {
        let _ = legacy.delete_credential();
    }
}

/// The stored key (normalized), or None when none is stored.
pub fn hub_key() -> anyhow::Result<Option<String>> {
    if matches!(entry()?.get_password(), Err(keyring::Error::NoEntry)) {
        migrate_legacy_key();
    }
    match entry()?.get_password() {
        Ok(v) => {
            let key = normalize_hub_key(&v);
            Ok((!key.is_empty()).then_some(key))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => anyhow::bail!("Could not read the hub key from Credential Manager"),
    }
}

pub fn set_hub_key(key: &str) -> anyhow::Result<()> {
    let key = normalize_hub_key(key);
    anyhow::ensure!(!key.is_empty(), "The hub key is empty");
    entry()?
        .set_password(&key)
        .map_err(|_| anyhow::anyhow!("Could not store the hub key in Credential Manager"))
}

pub fn clear_hub_key() -> anyhow::Result<()> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => anyhow::bail!("Could not remove the hub key from Credential Manager"),
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_hub_key as n;

    #[test]
    fn bare_key_is_kept() {
        assert_eq!(n("  fake-key-123\n"), "fake-key-123");
    }

    #[test]
    fn management_line_wins_over_access_line() {
        let raw = "access-key = fake-access\nmanagement-key = fake-manage\n";
        assert_eq!(n(raw), "fake-manage");
    }

    #[test]
    fn single_assignment_and_quotes() {
        assert_eq!(n("management-key = \"fake-q\""), "fake-q");
        assert_eq!(n("Management Key: 'fake-c'"), "fake-c");
    }

    #[test]
    fn without_management_line_last_line_wins() {
        assert_eq!(n("a = one\nb = two"), "two");
        assert_eq!(n(""), "");
    }
}
