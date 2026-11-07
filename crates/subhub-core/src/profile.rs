//! Login email of a CLI profile, read from the profile's own metadata so a profile can be matched
//! to its hub login before any usage read (import/add never shows duplicates).
//! Claude: `<dir>/.claude.json` -> `oauthAccount.emailAddress` (no secrets in that file).
//! Codex: `<dir>/auth.json` -> `tokens.id_token` payload `email`; only that claim is read, the
//! token is neither kept nor verified.
use std::path::Path;

use serde_json::Value;

use crate::model::ProviderKind;
use crate::util::valid_email;

pub fn profile_email(provider: ProviderKind, dir: &Path) -> Option<String> {
    match provider {
        ProviderKind::Claude => {
            let v: Value =
                serde_json::from_str(&std::fs::read_to_string(dir.join(".claude.json")).ok()?)
                    .ok()?;
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
    }
}

/// Folder path normalised for comparison (case, separators, trailing slash).
pub fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        p.to_string_lossy()
            .replace('/', "\\")
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
    fn dirs_compare_case_and_slash_insensitive() {
        assert!(same_dir(
            Path::new("C:\\Users\\X\\.claude\\"),
            Path::new("c:/users/x/.claude")
        ));
        assert!(!same_dir(Path::new("C:\\a"), Path::new("C:\\b")));
    }
}
