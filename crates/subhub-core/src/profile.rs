//! CLI profiles on disk: where they are (`discover_dirs`, shared by profile detection and the
//! usage statistics) and their login email, read from the profile's own metadata so a profile
//! can be matched to its hub login before any usage read (add/import never makes duplicates).
//! Claude: `<dir>/.claude.json` -> `oauthAccount.emailAddress` (no secrets in that file); the
//! default `~/.claude` folder falls back to `~/.claude.json`, where Claude Code keeps it when no
//! `CLAUDE_CONFIG_DIR` is set.
//! Codex: `<dir>/auth.json` -> `tokens.id_token` payload `email`; only that claim is read, the
//! token is neither kept nor verified.
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::model::ProviderKind;
use crate::util::valid_email;

pub fn profile_email(provider: ProviderKind, dir: &Path) -> Option<String> {
    match provider {
        ProviderKind::Claude => {
            let own = dir.join(".claude.json");
            let file = if own.is_file() {
                own
            } else if dir.file_name().is_some_and(|n| n == ".claude") {
                // Default profile: Claude Code keeps its config next to the folder.
                dir.parent()?.join(".claude.json")
            } else {
                return None;
            };
            let v: Value = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
            valid_email(v.get("oauthAccount")?.get("emailAddress")?.as_str()?)
        }
        ProviderKind::Codex => {
            let v: Value =
                serde_json::from_str(&std::fs::read_to_string(dir.join("auth.json")).ok()?).ok()?;
            let jwt = v.get("tokens")?.get("id_token")?.as_str()?;
            let payload = jwt.split('.').nth(1)?;
            let claims: Value =
                serde_json::from_slice(&crate::hub::base64url_decode(payload)?).ok()?;
            let email = claims
                .get("email")
                .or_else(|| claims.get("https://api.openai.com/profile")?.get("email"))?
                .as_str()?;
            valid_email(email)
        }
        // GitHub login comes with the first reading (`copilot_internal/user` -> login).
        ProviderKind::Copilot => None,
        ProviderKind::Cursor => crate::cursor::profile_email(dir),
        ProviderKind::Antigravity => crate::antigravity::profile_email(dir),
    }
}

/// Folders of the one-login-per-PC providers (Copilot, Cursor, Antigravity) that exist:
/// `<COPILOT_HOME or ~\.copilot>`, `%APPDATA%\Cursor` (the Cursor CLI's `auth.json` lives there),
/// `<GEMINI_CLI_HOME or ~\.gemini>\antigravity-cli`. Kept apart from `discover_dirs`, which also
/// feeds the log statistics (Claude and Codex only).
pub fn discover_app_dirs(home: &Path, env: &ProfileEnv) -> Vec<(ProviderKind, PathBuf)> {
    let var = |v: &Option<String>| v.as_deref().map(PathBuf::from);
    let mut out = vec![];
    let copilot = var(&env.copilot_home).unwrap_or_else(|| home.join(".copilot"));
    out.push((ProviderKind::Copilot, copilot));
    let roaming = var(&env.appdata).unwrap_or_else(|| home.join("AppData").join("Roaming"));
    out.push((ProviderKind::Cursor, roaming.join("Cursor")));
    let gemini = var(&env.gemini_cli_home).unwrap_or_else(|| home.join(".gemini"));
    out.push((ProviderKind::Antigravity, gemini.join("antigravity-cli")));
    out.into_iter()
        .filter(|(_, d)| d.is_dir())
        .filter_map(|(p, d)| Some((p, std::fs::canonicalize(&d).map(plain).ok()?)))
        .collect()
}

/// The environment variables that move a provider's profile folder. `CLAUDE_CONFIG_DIR` may
/// list several folders separated by commas.
#[derive(Debug, Clone, Default)]
pub struct ProfileEnv {
    pub claude_config_dir: Option<String>,
    pub codex_home: Option<String>,
    pub xdg_config_home: Option<String>,
    /// App logins: `COPILOT_HOME`, `APPDATA` (Cursor), `GEMINI_CLI_HOME` (Antigravity).
    pub copilot_home: Option<String>,
    pub appdata: Option<String>,
    pub gemini_cli_home: Option<String>,
}

impl ProfileEnv {
    pub fn from_process() -> Self {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        Self {
            claude_config_dir: var("CLAUDE_CONFIG_DIR"),
            codex_home: var("CODEX_HOME"),
            xdg_config_home: var("XDG_CONFIG_HOME"),
            copilot_home: var("COPILOT_HOME"),
            appdata: var("APPDATA"),
            gemini_cli_home: var("GEMINI_CLI_HOME"),
        }
    }
}

/// The current user's home folder.
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// Every folder that may be a CLI profile, canonical and without duplicates per provider:
/// `<home>\.claude*` / `<home>\.codex*` folders (sorted by name, so the defaults come first),
/// then the folders named by `CLAUDE_CONFIG_DIR` (comma-separated) and `CODEX_HOME`, then
/// `<XDG_CONFIG_HOME or home\.config>\claude`. Only existing folders; files such as
/// `.claude.json` are skipped. Callers decide which ones count (logs, login metadata).
pub fn discover_dirs(home: &Path, env: &ProfileEnv) -> Vec<(ProviderKind, PathBuf)> {
    let mut out: Vec<(ProviderKind, PathBuf)> = vec![];
    let mut push = |p: ProviderKind, dir: &Path| {
        if let Ok(c) = std::fs::canonicalize(dir).map(plain)
            && c.is_dir()
            && !out.iter().any(|(q, d)| *q == p && *d == c)
        {
            out.push((p, c));
        }
    };
    let mut names: Vec<_> = std::fs::read_dir(home)
        .map(|rd| rd.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    names.sort();
    for n in names {
        let name = n.to_string_lossy();
        if name.starts_with(".claude") {
            push(ProviderKind::Claude, &home.join(&n));
        } else if name.starts_with(".codex") {
            push(ProviderKind::Codex, &home.join(&n));
        }
    }
    let expand = |s: &str| -> PathBuf {
        let s = s.trim();
        match s.strip_prefix('~') {
            Some(rest) => home.join(rest.trim_start_matches(['/', '\\'])),
            None => PathBuf::from(s),
        }
    };
    if let Some(list) = &env.claude_config_dir {
        for part in list.split(',').filter(|p| !p.trim().is_empty()) {
            push(ProviderKind::Claude, &expand(part));
        }
    }
    if let Some(dir) = env.codex_home.as_deref().filter(|d| !d.trim().is_empty()) {
        push(ProviderKind::Codex, &expand(dir));
    }
    let config = env
        .xdg_config_home
        .as_deref()
        .map(expand)
        .unwrap_or_else(|| home.join(".config"));
    push(ProviderKind::Claude, &config.join("claude"));
    out
}

/// A canonical path without Windows' verbatim `\\?\` prefix when it is a plain drive path, so
/// stored profile folders stay usable as `CLAUDE_CONFIG_DIR` / `CODEX_HOME` and readable.
pub(crate) fn plain(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => p,
    }
}

/// Folder path normalised for comparison (case, separators, trailing slash).
pub fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        let s = p.to_string_lossy().replace('/', "\\");
        // Canonical Windows paths carry a verbatim prefix.
        s.strip_prefix(r"\\?\")
            .unwrap_or(&s)
            .trim_end_matches('\\')
            .to_lowercase()
    };
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_email_from_claude_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"a@example.test"}}"#,
        )
        .unwrap();
        assert_eq!(
            profile_email(ProviderKind::Claude, dir.path()).as_deref(),
            Some("a@example.test")
        );
        assert_eq!(profile_email(ProviderKind::Codex, dir.path()), None);
    }

    #[test]
    fn codex_email_from_id_token_payload_only() {
        let dir = tempfile::tempdir().unwrap();
        // header.payload.signature with payload {"email":"c@example.test"} (base64url, no padding)
        let payload = "eyJlbWFpbCI6ImNAZXhhbXBsZS50ZXN0In0";
        let auth = format!(r#"{{"tokens":{{"id_token":"x.{payload}.y","access_token":"fake"}}}}"#);
        std::fs::write(dir.path().join("auth.json"), auth).unwrap();
        assert_eq!(
            profile_email(ProviderKind::Codex, dir.path()).as_deref(),
            Some("c@example.test")
        );
    }

    #[test]
    fn missing_or_invalid_metadata_gives_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(profile_email(ProviderKind::Claude, dir.path()), None);
        std::fs::write(
            dir.path().join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"not an email"}}"#,
        )
        .unwrap();
        assert_eq!(profile_email(ProviderKind::Claude, dir.path()), None);
    }

    #[test]
    fn default_claude_folder_falls_back_to_home_claude_json() {
        let home = tempfile::tempdir().unwrap();
        let def = home.path().join(".claude");
        let other = home.path().join(".claude-work");
        std::fs::create_dir_all(&def).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(
            home.path().join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"home@example.test"}}"#,
        )
        .unwrap();
        assert_eq!(
            profile_email(ProviderKind::Claude, &def).as_deref(),
            Some("home@example.test")
        );
        // Only the default folder uses the home file.
        assert_eq!(profile_email(ProviderKind::Claude, &other), None);
        // The folder's own file wins.
        std::fs::write(
            def.join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"own@example.test"}}"#,
        )
        .unwrap();
        assert_eq!(
            profile_email(ProviderKind::Claude, &def).as_deref(),
            Some("own@example.test")
        );
    }

    #[test]
    fn discovers_prefixed_env_and_xdg_folders_once() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        for d in [
            ".claude",
            ".claude-work",
            ".codex",
            ".codex-b",
            ".config/claude",
            "elsewhere/c",
            "cx",
        ] {
            std::fs::create_dir_all(h.join(d)).unwrap();
        }
        std::fs::write(h.join(".claude.json"), "{}").unwrap(); // a file, not a profile
        let env = ProfileEnv {
            // Duplicates (also via ~) and a missing folder are dropped.
            claude_config_dir: Some(format!(
                "{}, ~/.claude-work ,{}",
                h.join("elsewhere").join("c").display(),
                h.join("missing").display()
            )),
            codex_home: Some(h.join("cx").display().to_string()),
            xdg_config_home: None,
            ..ProfileEnv::default()
        };
        let found = discover_dirs(h, &env);
        // Stored as plain paths (usable as CLAUDE_CONFIG_DIR), not verbatim `\\?\C:\…`.
        assert!(
            found
                .iter()
                .all(|(_, d)| !d.to_string_lossy().starts_with(r"\\?\C:")
                    && !d.to_string_lossy().starts_with(r"\\?\D:"))
        );
        let names: Vec<(ProviderKind, String)> = found
            .iter()
            .map(|(p, d)| (*p, d.file_name().unwrap().to_string_lossy().into_owned()))
            .collect();
        let want: Vec<(ProviderKind, String)> = [
            (ProviderKind::Claude, ".claude"),
            (ProviderKind::Claude, ".claude-work"),
            (ProviderKind::Codex, ".codex"),
            (ProviderKind::Codex, ".codex-b"),
            (ProviderKind::Claude, "c"),
            (ProviderKind::Codex, "cx"),
            (ProviderKind::Claude, "claude"),
        ]
        .into_iter()
        .map(|(p, n)| (p, n.to_string()))
        .collect();
        assert_eq!(names, want);
    }

    #[test]
    fn dirs_compare_case_and_slash_insensitive() {
        assert!(same_dir(
            Path::new("C:\\Users\\X\\.claude\\"),
            Path::new("c:/users/x/.claude")
        ));
        assert!(!same_dir(Path::new("C:\\a"), Path::new("C:\\b")));
    }
}
