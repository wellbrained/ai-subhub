//! GitHub Copilot (owner, 2026-11-09): limits only. Reads `GET api.github.com/copilot_internal/user`
//! (undocumented; the endpoint VS Code and the Copilot CLI's `/usage` use) with the GitHub token
//! the Copilot CLI already keeps: `COPILOT_GITHUB_TOKEN`, then Windows Credential Manager
//! (`copilot-cli`), then `<profile>\config.json`, then `gh auth token`. SubHub never signs in
//! itself and sends the token only to api.github.com. Shapes: docs/research/copilot.md.
//! Not yet checked against a real account (owner has none).
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use chrono::{DateTime, Datelike, TimeZone, Utc};
use serde_json::Value;

use crate::model::{AllowanceWindow, ReadSource, Reading, UsageResult, UsageState};
use crate::util::{self, failure, failure_for_status};

pub const USAGE_URL: &str = "https://api.github.com/copilot_internal/user";

/// A finite number that may come as a string (`entitlement` was seen both ways).
fn num(v: &Value, key: &str) -> Option<f64> {
    match v.get(key)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|n: &f64| n.is_finite())
}

/// The 1st of next month, 00:00 UTC (Copilot's documented reset).
fn next_month_start(now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let (y, m) = if now.month() == 12 {
        (now.year() + 1, 1)
    } else {
        (now.year(), now.month() + 1)
    };
    Utc.with_ymd_and_hms(y, m, 1, 0, 0, 0).single()
}

/// Reset of a snapshot: its own `quota_reset_at` (Unix seconds, often 0), else the top-level
/// `quota_reset_date_utc` / `quota_reset_date`, else the 1st of next month.
fn reset_of(snapshot: &Value, body: &Value, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    snapshot
        .get("quota_reset_at")
        .and_then(Value::as_i64)
        .filter(|s| *s > 0)
        .and_then(|s| Utc.timestamp_opt(s, 0).single())
        .or_else(|| util::text(body, "quota_reset_date_utc").and_then(util::parse_rfc3339))
        .or_else(|| {
            let d = util::text(body, "quota_reset_date")?;
            let date = chrono::NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d").ok()?;
            Some(Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0)?))
        })
        .or_else(|| next_month_start(now))
}

/// Used % of one snapshot; None when unlimited, without an allowance, or unreadable.
fn used_percent(s: &Value) -> Option<f64> {
    if s.get("unlimited").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let entitlement = num(s, "entitlement");
    if entitlement.is_some_and(|e| e <= 0.0) {
        return None;
    }
    if let Some(left) = num(s, "percent_remaining") {
        return Some((100.0 - left).clamp(0.0, 100.0));
    }
    let (e, r) = (entitlement?, num(s, "remaining")?);
    Some(((e - r) / e * 100.0).clamp(0.0, 100.0))
}

/// Windows from a `copilot_internal/user` body: premium requests / AI credits (`copilot:premium`,
/// "Monthly"), then the Free plan's chat and completions counts. Unlimited ones are left out.
pub fn parse_usage(body: &Value, now: DateTime<Utc>) -> Vec<AllowanceWindow> {
    let Some(snaps) = body.get("quota_snapshots") else {
        return vec![];
    };
    let credits = body.get("token_based_billing").and_then(Value::as_bool) == Some(true);
    let mut out = vec![];
    for (name, key, label) in [
        (
            "premium_interactions",
            "copilot:premium",
            if credits {
                "Monthly · AI credits"
            } else {
                "Monthly · Premium requests"
            },
        ),
        ("chat", "copilot:chat", "Monthly · Chat"),
        (
            "completions",
            "copilot:completions",
            "Monthly · Completions",
        ),
    ] {
        let Some(s) = snaps.get(name) else { continue };
        if let Some(used) = used_percent(s) {
            out.push(AllowanceWindow {
                key: key.into(),
                label: label.into(),
                used_percent: used,
                resets_at: reset_of(s, body, now),
            });
        }
    }
    out
}

/// Plan label: "Free", "Pro", "Pro+", "Business", "Enterprise", from `access_type_sku` /
/// `copilot_plan`.
pub fn parse_plan(body: &Value) -> Option<String> {
    let sku = util::text(body, "access_type_sku")
        .unwrap_or("")
        .to_lowercase();
    let plan = util::text(body, "copilot_plan")
        .unwrap_or("")
        .to_lowercase();
    let label = if sku.contains("free") || plan == "free" {
        "Free"
    } else if sku.contains("plus") || sku.contains("pro_plus") {
        "Pro+"
    } else if sku.contains("max") {
        "Max"
    } else if plan == "business" || sku.contains("business") {
        "Business"
    } else if plan == "enterprise" || sku.contains("enterprise") {
        "Enterprise"
    } else if plan == "individual" || sku.contains("pro") {
        "Pro"
    } else {
        return None;
    };
    Some(label.into())
}

/// The full result from a status and body.
pub fn usage_result(status: u16, body: &Value, now: DateTime<Utc>) -> UsageResult {
    let src = ReadSource::Profile;
    if let Some(f) = failure_for_status("Copilot", status, None, src) {
        return f;
    }
    if util::text(body, "access_type_sku") == Some("subscription_ended") {
        return failure(
            UsageState::Unavailable,
            "This GitHub account has no active Copilot plan.",
            src,
        );
    }
    let windows = parse_usage(body, now);
    if windows.is_empty() {
        let unlimited = body
            .get("quota_snapshots")
            .and_then(Value::as_object)
            .is_some_and(|m| {
                !m.is_empty()
                    && m.values()
                        .all(|s| s.get("unlimited").and_then(Value::as_bool) == Some(true))
            });
        return failure(
            UsageState::Unavailable,
            if unlimited {
                "Copilot reports no limits for this plan."
            } else {
                "Copilot returned no usage data. The endpoint may have changed."
            },
            src,
        );
    }
    UsageResult {
        state: UsageState::Ready,
        message: String::new(),
        reading: Some(Reading {
            fetched_at: now,
            windows,
        }),
        retry_after_secs: None,
        // GitHub login, not an email: shown as the account's identity.
        email: util::text(body, "login").map(str::to_string),
        plan: parse_plan(body),
        source: src,
    }
}

/// A GitHub token Copilot accepts (`gho_`, `ghu_`, fine-grained `github_pat_`; classic `ghp_`
/// is not supported by Copilot).
fn usable(t: &str) -> bool {
    let t = t.trim();
    (t.starts_with("gho_") || t.starts_with("ghu_") || t.starts_with("github_pat_"))
        && t.len() > 10
        && !t.contains(char::is_whitespace)
}

/// A host value that means github.com (`github.com`, `https://github.com/`).
fn is_github_com(host: &str) -> bool {
    let h = host.trim().trim_end_matches('/').to_lowercase();
    let h = h
        .strip_prefix("https://")
        .or_else(|| h.strip_prefix("http://"))
        .unwrap_or(&h);
    h == "github.com" || h == "api.github.com"
}

/// The first usable token anywhere in a JSON value (the CLI's config layout is undocumented).
/// An object naming another host (GitHub Enterprise, `*.ghe.com`) is skipped: its token must
/// never go to api.github.com.
fn token_in(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if usable(s) => Some(s.trim().to_string()),
        Value::Array(a) => a.iter().find_map(token_in),
        Value::Object(m) => {
            let other_host = ["host", "hostname", "server", "url"]
                .iter()
                .filter_map(|k| m.get(*k).and_then(Value::as_str))
                .any(|h| !is_github_com(h));
            if other_host {
                return None;
            }
            m.values().find_map(token_in)
        }
        _ => None,
    }
}

/// A Credential Manager target of the Copilot CLI for github.com: it names github.com, or no
/// host at all (anything with another domain belongs to an Enterprise host).
fn github_com_target(target: &str) -> bool {
    let t = target.to_lowercase();
    t.contains("github.com") && !t.contains("ghe.com") || !t.contains('.')
}

/// The Copilot CLI's token: env, Credential Manager, config file, then the gh CLI.
fn find_token(profile_dir: &Path) -> Option<String> {
    if let Ok(t) = std::env::var("COPILOT_GITHUB_TOKEN")
        && usable(&t)
    {
        return Some(t.trim().to_string());
    }
    for (target, blob) in crate::wincred::find_generic("copilot-cli") {
        if !github_com_target(&target) {
            continue;
        }
        let Some(text) = crate::wincred::blob_text(&blob) else {
            continue;
        };
        if usable(&text) {
            return Some(text);
        }
        if let Ok(v) = serde_json::from_str::<Value>(&text)
            && let Some(t) = token_in(&v)
        {
            return Some(t);
        }
    }
    if let Ok(text) = std::fs::read_to_string(profile_dir.join("config.json"))
        && let Ok(v) = serde_json::from_str::<Value>(&text)
        && let Some(t) = token_in(&v)
    {
        return Some(t);
    }
    gh_token()
}

/// `gh auth token` (no window; 10 s at most).
fn gh_token() -> Option<String> {
    let mut cmd = Command::new("gh");
    cmd.args(["auth", "token", "--hostname", "github.com"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = crate::util::output_with_timeout(cmd, Duration::from_secs(10))?;
    let t = String::from_utf8_lossy(&out).trim().to_string();
    usable(&t).then_some(t)
}

/// Reads the Copilot account behind this PC's Copilot CLI login.
pub fn read_profile(profile_dir: &Path) -> UsageResult {
    let src = ReadSource::Profile;
    let Some(token) = find_token(profile_dir) else {
        return failure(
            UsageState::SignInRequired,
            "No Copilot login found. Sign in once with the Copilot CLI (`copilot`, then /login) or `gh auth login`.",
            src,
        );
    };
    let Some(http) = util::http_client(Duration::from_secs(20)) else {
        return failure(
            UsageState::Unavailable,
            "Could not start the network client.",
            src,
        );
    };
    let response = http
        .get(USAGE_URL)
        .header("Authorization", format!("token {token}"))
        .header("Accept", "application/json")
        .header("X-GitHub-Api-Version", "2025-04-01")
        .header("User-Agent", util::USER_AGENT)
        .send();
    let response = match response {
        Ok(r) => r,
        Err(_) => {
            return failure(UsageState::Unavailable, "GitHub could not be reached.", src);
        }
    };
    let status = response.status().as_u16();
    let body: Value = response.json().unwrap_or(Value::Null);
    usage_result(status, &body, Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        "2026-11-09T10:00:00Z".parse().unwrap()
    }

    fn body() -> Value {
        json!({
            "login": "octocat",
            "copilot_plan": "individual",
            "access_type_sku": "copilot_pro",
            "quota_reset_date_utc": "2026-12-01T00:00:00.000Z",
            "token_based_billing": true,
            "quota_snapshots": {
                "premium_interactions": {"entitlement": "1500", "remaining": 1230, "percent_remaining": 82.0, "unlimited": false, "quota_reset_at": 0},
                "chat": {"entitlement": 0, "remaining": 0, "unlimited": true},
                "completions": {"entitlement": 0, "remaining": 0, "unlimited": true}
            }
        })
    }

    #[test]
    fn premium_credits_become_a_monthly_window() {
        let w = parse_usage(&body(), now());
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].key, "copilot:premium");
        assert_eq!(w[0].label, "Monthly · AI credits");
        assert!((w[0].used_percent - 18.0).abs() < 1e-9);
        assert_eq!(
            w[0].resets_at,
            Some("2026-12-01T00:00:00Z".parse().unwrap())
        );
        let r = usage_result(200, &body(), now());
        assert_eq!(r.state, UsageState::Ready);
        assert_eq!(r.email.as_deref(), Some("octocat"));
        assert_eq!(r.plan.as_deref(), Some("Pro"));
    }

    #[test]
    fn free_plan_counts_and_fallbacks() {
        // Legacy premium requests without percent: from entitlement/remaining; no reset date at
        // all: the 1st of next month.
        let b = json!({
            "access_type_sku": "free_limited_copilot",
            "quota_snapshots": {
                "premium_interactions": {"entitlement": 50, "remaining": 40},
                "chat": {"entitlement": 50, "remaining": 25, "percent_remaining": 50.0},
                "completions": {"entitlement": 2000, "remaining": 2000, "percent_remaining": 100.0}
            }
        });
        let w = parse_usage(&b, now());
        assert_eq!(
            w.iter()
                .map(|w| (w.key.as_str(), w.used_percent))
                .collect::<Vec<_>>(),
            vec![
                ("copilot:premium", 20.0),
                ("copilot:chat", 50.0),
                ("copilot:completions", 0.0)
            ]
        );
        assert_eq!(w[0].label, "Monthly · Premium requests");
        assert_eq!(
            w[0].resets_at,
            Some("2026-12-01T00:00:00Z".parse().unwrap())
        );
        assert_eq!(parse_plan(&b).as_deref(), Some("Free"));
    }

    #[test]
    fn ended_unlimited_and_errors() {
        let ended = json!({"access_type_sku": "subscription_ended", "quota_snapshots": {}});
        assert_eq!(
            usage_result(200, &ended, now()).state,
            UsageState::Unavailable
        );
        let unlimited = json!({"quota_snapshots": {"chat": {"unlimited": true}}});
        let r = usage_result(200, &unlimited, now());
        assert_eq!(r.message, "Copilot reports no limits for this plan.");
        assert_eq!(
            usage_result(401, &Value::Null, now()).state,
            UsageState::SignInRequired
        );
        assert_eq!(
            usage_result(429, &Value::Null, now()).state,
            UsageState::RateLimited
        );
    }

    #[test]
    fn tokens_are_picked_from_any_json_layout() {
        let v = json!({"logged_in_users": [
            {"host": "https://acme.ghe.com", "token": "gho_enterprise123"},
            {"host": "https://github.com", "token": "gho_abcdefghijk"}
        ]});
        assert_eq!(token_in(&v).as_deref(), Some("gho_abcdefghijk"));
        assert!(github_com_target("copilot-cli/https://github.com:octocat"));
        assert!(github_com_target("copilot-cli"));
        assert!(!github_com_target("copilot-cli/https://acme.ghe.com:me"));
        assert!(!github_com_target(
            "copilot-cli/https://git.corp.example:me"
        ));
        assert_eq!(token_in(&json!({"token": "ghp_classic12345"})), None);
        assert!(!usable("gho_ short"));
    }

    #[test]
    fn december_resets_in_january() {
        let dec: DateTime<Utc> = "2026-12-20T10:00:00Z".parse().unwrap();
        assert_eq!(
            next_month_start(dec),
            Some("2027-01-01T00:00:00Z".parse().unwrap())
        );
    }
}
