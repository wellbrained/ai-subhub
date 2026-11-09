//! Google Antigravity (owner, 2026-11-09): limits only, through the official CLI. Runs
//! `agy -p /usage --output-format json` (agy 1.1.11+) and reads its groups. SubHub never touches
//! Antigravity's OAuth token: Google's terms forbid third-party tools using it, and accounts were
//! banned for that (docs/research/gemini-antigravity.md). The only thing read from the login is
//! its email claim, for the account's name. `agy` only runs when a login exists, so a signed-out
//! PC never gets a browser window from it. Not yet checked against a real account.
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::model::{AllowanceWindow, ReadSource, Reading, UsageResult, UsageState};
use crate::util::{self, failure};

/// Credential Manager target of agy's login (agy 1.4.3+).
const CRED_TARGET: &str = "gemini:antigravity";
/// File fallback inside the profile (`~/.gemini/antigravity-cli`).
const TOKEN_FILE: &str = "antigravity-oauth-token";

/// Finds `agy` (`%LOCALAPPDATA%\agy\bin`, then PATH).
pub fn find_exe() -> Option<PathBuf> {
    let mut dirs = vec![];
    if let Some(l) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(l).join("agy").join("bin"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path).filter(|p| !p.as_os_str().is_empty()));
    }
    let exts: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    crate::codex::find_in(&dirs, "agy", exts)
}

/// The login's JSON (Credential Manager, else the token file), only to tell that a login exists
/// and to read its email claim.
fn login_json(profile_dir: &Path) -> Option<Value> {
    let text = crate::wincred::read_generic(CRED_TARGET)
        .and_then(|b| crate::wincred::blob_text(&b))
        .or_else(|| std::fs::read_to_string(profile_dir.join(TOKEN_FILE)).ok())?;
    serde_json::from_str(&text).ok().or(Some(Value::Null))
}

/// The login email (`id_token` claim), when agy keeps one.
pub fn profile_email(profile_dir: &Path) -> Option<String> {
    let v = login_json(profile_dir)?;
    let id = v.get("id_token")?.as_str()?;
    util::valid_email(util::jwt_claims(id)?.get("email")?.as_str()?)
}

/// A short slug of a group name for window keys ("Claude and GPT models" -> "claude-gpt").
fn slug(name: &str) -> String {
    let words: Vec<String> = name
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !matches!(*w, "and" | "models" | "model"))
        .map(str::to_string)
        .collect();
    if words.is_empty() {
        "other".into()
    } else {
        words.join("-")
    }
}

/// A group's short display name ("Gemini Models" -> "Gemini", "Claude and GPT models" ->
/// "Claude & GPT").
fn group_name(name: &str) -> String {
    let n = name.trim();
    let n = n
        .strip_suffix(" Models")
        .or_else(|| n.strip_suffix(" models"))
        .unwrap_or(n);
    n.replace(" and ", " & ")
}

/// "5h" / "weekly" from a bucket (`window`, `bucketId`, `displayName`).
fn bucket_kind(b: &Value) -> Option<&'static str> {
    let text = ["window", "bucketId", "displayName"]
        .iter()
        .filter_map(|k| b.get(*k).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if text.contains("5h") || text.contains("5-hour") || text.contains("5 hour") {
        Some("5h")
    } else if text.contains("week") || text.contains("7d") {
        Some("weekly")
    } else {
        None
    }
}

fn remaining_fraction(b: &Value) -> Option<f64> {
    b.get("remaining")
        .and_then(|r| r.get("remainingFraction"))
        .or_else(|| b.get("remainingFraction"))
        .and_then(Value::as_f64)
}

/// The quota groups of agy's `/usage` JSON (`command.data.groups`, also accepted bare).
fn groups(root: &Value) -> Option<&Vec<Value>> {
    [
        root.pointer("/command/data/groups"),
        root.pointer("/data/groups"),
        root.pointer("/response/groups"),
        root.get("groups"),
    ]
    .into_iter()
    .flatten()
    .find_map(Value::as_array)
}

/// Windows from agy's `/usage`: the first group (Gemini) as `agy:5h` / `agy:weekly` ("5-hour",
/// "Weekly": the account's main windows), other groups as `agy:<slug>:5h|weekly`
/// ("5-hour · Claude & GPT").
pub fn parse_usage(root: &Value) -> Vec<AllowanceWindow> {
    let mut out = vec![];
    for (gi, g) in groups(root).into_iter().flatten().enumerate() {
        let name = g.get("displayName").and_then(Value::as_str).unwrap_or("");
        for b in g
            .get("buckets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if b.get("disabled").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            let (Some(kind), Some(left)) = (bucket_kind(b), remaining_fraction(b)) else {
                continue;
            };
            let title = if kind == "5h" { "5-hour" } else { "Weekly" };
            let (key, label) = if gi == 0 {
                (format!("agy:{kind}"), title.to_string())
            } else {
                (
                    format!("agy:{}:{kind}", slug(name)),
                    format!("{title} · {}", group_name(name)),
                )
            };
            if out.iter().any(|w: &AllowanceWindow| w.key == key) {
                continue;
            }
            out.push(AllowanceWindow {
                key,
                label,
                used_percent: ((1.0 - left) * 100.0).clamp(0.0, 100.0),
                resets_at: util::text(b, "resetTime").and_then(util::parse_rfc3339),
            });
        }
    }
    out
}

/// The JSON object in agy's output (it may print other lines around it).
fn json_in(stdout: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str::<Value>(stdout.trim()) {
        return Some(v);
    }
    let start = stdout.find('{')?;
    let end = stdout.rfind('}')?;
    serde_json::from_str(stdout.get(start..=end)?).ok()
}

pub fn usage_result(stdout: &str, now: DateTime<Utc>) -> UsageResult {
    let src = ReadSource::Profile;
    let Some(root) = json_in(stdout) else {
        return failure(
            UsageState::Unavailable,
            "Antigravity's CLI gave no usage. Update agy (1.1.11 or newer reports usage).",
            src,
        );
    };
    let status = util::text(&root, "status")
        .unwrap_or("SUCCESS")
        .to_uppercase();
    if status != "SUCCESS" {
        let msg = root
            .pointer("/error/message")
            .or_else(|| root.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();
        let signed_out = msg.contains("sign") || msg.contains("login") || msg.contains("auth");
        return failure(
            if signed_out {
                UsageState::SignInRequired
            } else {
                UsageState::Unavailable
            },
            if signed_out {
                "Antigravity's CLI is signed out. Run `agy` once and sign in."
            } else {
                "Antigravity's CLI could not read usage."
            },
            src,
        );
    }
    let windows = parse_usage(&root);
    if windows.is_empty() {
        return failure(
            UsageState::Unavailable,
            "Antigravity returned no usage data. The CLI's output may have changed.",
            src,
        );
    }
    // Free plans only get weekly quotas.
    let plan = (!windows.iter().any(|w| w.key.ends_with("5h"))).then(|| "Free".to_string());
    UsageResult {
        state: UsageState::Ready,
        message: String::new(),
        reading: Some(Reading {
            fetched_at: now,
            windows,
        }),
        retry_after_secs: None,
        email: None,
        plan,
        source: src,
    }
}

/// Reads Antigravity's limits through `agy` for the login on this PC.
pub fn read_profile(profile_dir: &Path, exe: Option<&Path>) -> UsageResult {
    let src = ReadSource::Profile;
    let Some(exe) = exe.map(Path::to_path_buf).or_else(find_exe) else {
        return failure(
            UsageState::CliMissing,
            "Antigravity's CLI (agy) was not found. Install it to see Antigravity limits.",
            src,
        );
    };
    if login_json(profile_dir).is_none() {
        return failure(
            UsageState::SignInRequired,
            "Antigravity's CLI is not signed in. Run `agy` once and sign in.",
            src,
        );
    }
    // Started directly, also a .cmd shim: Rust's std quotes the path and arguments for cmd.exe
    // safely (a hand-built `cmd /C <path>` would let `&` or `%VAR%` in a folder name run code).
    let mut cmd = Command::new(&exe);
    cmd.args(["-p", "/usage", "--output-format", "json"])
        .env("AGY_CLI_DISABLE_AUTO_UPDATE", "true")
        // A login that stopped working must not open a sign-in page on every refresh.
        .env("NO_BROWSER", "true")
        .current_dir(profile_dir.parent().unwrap_or(profile_dir))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    match util::output_with_timeout(cmd, Duration::from_secs(45)) {
        Some(out) => {
            let mut r = usage_result(&String::from_utf8_lossy(&out), Utc::now());
            if r.state == UsageState::Ready {
                r.email = profile_email(profile_dir);
            }
            r
        }
        None => failure(
            UsageState::Unavailable,
            "Antigravity's CLI did not answer in time.",
            src,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        "2026-11-09T10:00:00Z".parse().unwrap()
    }

    fn sample() -> String {
        json!({"status": "SUCCESS", "command": {"name": "usage", "data": {"groups": [
            {"displayName": "Gemini Models", "buckets": [
                {"bucketId": "gemini-5h", "displayName": "5-hour limit", "window": "5h", "remaining": {"remainingFraction": 0.7}, "resetTime": "2026-11-09T13:00:00Z"},
                {"bucketId": "gemini-weekly", "displayName": "Weekly limit", "remaining": {"remainingFraction": 0.45}, "resetTime": "2026-11-14T00:00:00Z"}
            ]},
            {"displayName": "Claude and GPT models", "buckets": [
                {"bucketId": "3p-5h", "window": "5h", "remainingFraction": 0.9, "resetTime": "2026-11-09T12:00:00Z"},
                {"bucketId": "3p-weekly", "window": "weekly", "remaining": {"remainingFraction": 0.8}},
                {"bucketId": "off", "window": "5h", "disabled": true, "remainingFraction": 0.0}
            ]}
        ]}}})
        .to_string()
    }

    #[test]
    fn groups_become_main_and_per_model_windows() {
        let r = usage_result(&format!("agy 1.4.3\n{}\n", sample()), now());
        assert_eq!(r.state, UsageState::Ready);
        let w = r.reading.unwrap().windows;
        assert_eq!(
            w.iter()
                .map(|w| (
                    w.key.as_str(),
                    w.label.as_str(),
                    w.used_percent.round() as i64
                ))
                .collect::<Vec<_>>(),
            vec![
                ("agy:5h", "5-hour", 30),
                ("agy:weekly", "Weekly", 55),
                ("agy:claude-gpt:5h", "5-hour · Claude & GPT", 10),
                ("agy:claude-gpt:weekly", "Weekly · Claude & GPT", 20),
            ]
        );
        assert_eq!(
            w[0].resets_at,
            Some("2026-11-09T13:00:00Z".parse().unwrap())
        );
        assert_eq!(r.plan, None);
    }

    #[test]
    fn weekly_only_means_free_and_failures_are_clear() {
        let free = json!({"status": "SUCCESS", "command": {"data": {"groups": [
            {"displayName": "Gemini Models", "buckets": [{"window": "weekly", "remainingFraction": 1.0}]}
        ]}}});
        assert_eq!(
            usage_result(&free.to_string(), now()).plan.as_deref(),
            Some("Free")
        );
        let out = json!({"status": "ERROR", "error": {"message": "Please sign in"}}).to_string();
        assert_eq!(usage_result(&out, now()).state, UsageState::SignInRequired);
        assert_eq!(
            usage_result("not json", now()).state,
            UsageState::Unavailable
        );
    }

    #[test]
    fn no_login_never_runs_agy() {
        let dir = tempfile::tempdir().unwrap();
        // A fake exe path: the missing login stops the read before anything is started.
        let fake = dir.path().join("agy.exe");
        let r = read_profile(&dir.path().join("antigravity-cli"), Some(&fake));
        // On a PC with a real agy login in Credential Manager this would differ; CI has none.
        if crate::wincred::read_generic(CRED_TARGET).is_none() {
            assert_eq!(r.state, UsageState::SignInRequired);
        }
    }
}
