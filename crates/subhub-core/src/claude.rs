//! Claude: undocumented OAuth usage endpoint. Parsing is isolated here (CLAUDE.md rule).
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::model::{AllowanceWindow, ReadSource, Reading, UsageResult, UsageState};
use crate::util::{self, failure, failure_for_status};

pub const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
pub const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";

/// Parses a usage body (both the `limits` array and the legacy `five_hour`/`seven_day*` shape).
/// Window keys: "five_hour", "seven_day", "seven_day_opus", "seven_day_sonnet", or for `limits`
/// entries "<kind>" / "<kind>:<model display name>". Entries without a valid 0..=100 number
/// are skipped (missing is never 0%).
pub fn parse_usage(body: &Value) -> Vec<AllowanceWindow> {
    let mut windows: Vec<AllowanceWindow> = vec![];
    if let Some(limits) = body.get("limits").and_then(Value::as_array) {
        for limit in limits.iter().filter(|l| l.is_object()) {
            let Some(used) = util::percent(limit, "percent") else {
                continue;
            };
            let kind = util::text(limit, "kind");
            let base = match kind {
                Some("session") => "Session",
                Some("weekly_all") | Some("weekly_scoped") => "Weekly",
                Some(other) => other,
                None => "Allowance",
            };
            let model = limit
                .get("scope")
                .and_then(|s| s.get("model"))
                .and_then(|m| util::text(m, "display_name"));
            let mut key = kind.unwrap_or("allowance").to_string();
            let mut label = base.to_string();
            if let Some(model) = model {
                key = format!("{key}:{model}");
                label = format!("{label} · {model}");
            }
            if windows.iter().any(|w| w.key == key) {
                key = format!("{key}#{}", windows.len() + 1);
            }
            let resets_at = util::text(limit, "resets_at").and_then(util::parse_rfc3339);
            windows.push(AllowanceWindow {
                key,
                label,
                used_percent: used,
                resets_at,
            });
        }
        if !windows.is_empty() {
            return windows;
        }
    }
    for (key, label) in [
        ("five_hour", "Session"),
        ("seven_day", "Weekly"),
        ("seven_day_sonnet", "Weekly · Sonnet"),
        ("seven_day_opus", "Weekly · Opus"),
    ] {
        let Some(bucket) = body.get(key).filter(|b| b.is_object()) else {
            continue;
        };
        let Some(used) = util::percent(bucket, "utilization") else {
            continue;
        };
        let resets_at = util::text(bucket, "resets_at").and_then(util::parse_rfc3339);
        windows.push(AllowanceWindow {
            key: key.into(),
            label: label.into(),
            used_percent: used,
            resets_at,
        });
    }
    windows
}

/// Plan label from profile/credential metadata (e.g. "Max 20x", "Pro"). Unknown values give None.
pub fn parse_plan(profile: &Value) -> Option<String> {
    crate::plan::claude_plan(profile)
}

/// Email from an `oauth/profile` body (`account.email`).
pub fn parse_profile_email(profile: &Value) -> Option<String> {
    util::valid_email(util::text(profile.get("account")?, "email")?)
}

/// Pure core of a Claude read: HTTP status of the usage call (+ `Retry-After`), its parsed body,
/// and the optional profile body -> result. Used for the hub path and the direct path.
pub fn usage_result(
    status: u16,
    retry_after: Option<u64>,
    usage_body: &Value,
    profile_body: Option<&Value>,
    fallback_plan: Option<String>,
    source: ReadSource,
    now: DateTime<Utc>,
) -> UsageResult {
    if let Some(f) = failure_for_status("Claude", status, retry_after, source) {
        return f;
    }
    let windows = parse_usage(usage_body);
    if windows.is_empty() {
        return failure(
            UsageState::Unavailable,
            "Claude returned no recognized subscription allowances.",
            source,
        );
    }
    let mut r = failure(UsageState::Ready, "Connected", source);
    r.reading = Some(Reading {
        fetched_at: now,
        windows,
    });
    r.plan = profile_body.and_then(parse_plan).or(fallback_plan);
    r.email = profile_body.and_then(parse_profile_email);
    r
}

/// OAuth access token from the profile's `.credentials.json`, plus the plan from its metadata.
/// Err carries the user-facing result. The token is held in memory only.
pub(crate) struct Credentials {
    pub token: String,
    pub plan: Option<String>,
}

pub(crate) fn read_credentials(profile_dir: &Path) -> Result<Credentials, Box<UsageResult>> {
    let src = ReadSource::Profile;
    let sign_in = || {
        Box::new(failure(
            UsageState::SignInRequired,
            "Sign in with your Claude subscription to connect this profile.",
            src,
        ))
    };
    let text =
        std::fs::read_to_string(profile_dir.join(".credentials.json")).map_err(|_| sign_in())?;
    let json: Value = serde_json::from_str(&text).map_err(|_| sign_in())?;
    let token = json
        .get("claudeAiOauth")
        .and_then(|o| util::text(o, "accessToken"))
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| {
            Box::new(failure(
                UsageState::SignInRequired,
                "No Claude subscription login found in this profile.",
                src,
            ))
        })?;
    Ok(Credentials {
        token: token.to_string(),
        plan: parse_plan(&json),
    })
}

/// Direct read with the CLI profile's own credentials (fallback when the hub is down).
/// Reads the access token into memory only; no refresh, no copying.
pub fn read_profile(profile_dir: &Path) -> UsageResult {
    let src = ReadSource::Profile;
    let creds = match read_credentials(profile_dir) {
        Ok(c) => c,
        Err(r) => return *r,
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
        .bearer_auth(&creds.token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("anthropic-version", "2023-06-01")
        .header("x-app", "cli")
        .header("Accept", "application/json")
        .send();
    let response = match response {
        Ok(r) => r,
        Err(_) => {
            return failure(
                UsageState::Unavailable,
                "Could not reach Claude. Check your connection.",
                src,
            );
        }
    };
    let status = response.status().as_u16();
    let now = Utc::now();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| util::parse_retry_after(v, now));
    if !(200..300).contains(&status) {
        return usage_result(status, retry_after, &Value::Null, None, None, src, now);
    }
    let usage: Value = response.json().unwrap_or(Value::Null);
    // Plan and email are best effort and must not delay or fail the usage reading.
    let profile = http
        .get(PROFILE_URL)
        .timeout(Duration::from_secs(4))
        .bearer_auth(&creds.token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .send()
        .ok()
        .filter(|r| r.status().is_success())
        .and_then(|r| r.json::<Value>().ok());
    usage_result(
        status,
        retry_after,
        &usage,
        profile.as_ref(),
        creds.plan,
        src,
        now,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        "2026-11-06T10:00:00Z".parse().unwrap()
    }

    #[test]
    fn limits_include_inactive_and_model_scoped_windows() {
        let body = json!({"five_hour": null, "limits": [
            {"kind": "session", "percent": 12.5, "is_active": false, "scope": null},
            {"kind": "weekly_all", "percent": 77, "is_active": false},
            {"kind": "weekly_scoped", "percent": 100, "is_active": true,
             "scope": {"model": {"display_name": "Model A"}}}]});
        let w = parse_usage(&body);
        assert_eq!(w.len(), 3);
        assert_eq!(
            (w[0].key.as_str(), w[0].label.as_str()),
            ("session", "Session")
        );
        assert_eq!(w[0].remaining_percent(), 87.5);
        assert_eq!(w[1].remaining_percent(), 23.0);
        assert_eq!(w[2].key, "weekly_scoped:Model A");
        assert_eq!(w[2].label, "Weekly · Model A");
        assert_eq!(w[2].remaining_percent(), 0.0);
    }

    #[test]
    fn legacy_shape_skips_null_and_missing_buckets() {
        let body = json!({"five_hour": null, "seven_day": {"utilization": 27.5, "resets_at": "2026-10-20T10:00:00Z"}});
        let w = parse_usage(&body);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].key, "seven_day");
        assert_eq!(w[0].remaining_percent(), 72.5);
        assert!(w[0].resets_at.is_some());
    }

    #[test]
    fn invalid_numbers_are_not_zero() {
        let body = json!({"five_hour": {"utilization": "7"}, "seven_day": {"utilization": 130},
                          "seven_day_opus": {"utilization": null}, "limits": [{"kind": "session"}]});
        assert!(parse_usage(&body).is_empty());
        assert!(parse_usage(&json!(null)).is_empty());
        assert!(parse_usage(&json!([1, 2])).is_empty());
    }

    #[test]
    fn duplicate_limit_keys_stay_unique() {
        let body = json!({"limits": [{"kind": "weekly_scoped", "percent": 1}, {"kind": "weekly_scoped", "percent": 2}]});
        let w = parse_usage(&body);
        assert_ne!(w[0].key, w[1].key);
    }

    #[test]
    fn plans_use_known_values_only() {
        let org = |t: &str, tier: &str| json!({"organization": {"organization_type": t, "rate_limit_tier": tier}});
        assert_eq!(
            parse_plan(&org("claude_max", "default_claude_max_20x")).as_deref(),
            Some("Max 20x")
        );
        assert_eq!(
            parse_plan(&org("claude_max", "default_claude_max_5x")).as_deref(),
            Some("Max 5x")
        );
        assert_eq!(
            parse_plan(&org("claude_max", "other")).as_deref(),
            Some("Max")
        );
        assert_eq!(
            parse_plan(&org("claude_pro", "default_claude_ai")).as_deref(),
            Some("Pro")
        );
        assert_eq!(parse_plan(&org("private-value", "x")), None);
        assert_eq!(parse_plan(&json!({"organization": null})), None);
        assert_eq!(parse_plan(&json!(null)), None);
        let creds = json!({"claudeAiOauth": {"subscriptionType": "max", "rateLimitTier": "default_claude_max_5x"}});
        assert_eq!(parse_plan(&creds).as_deref(), Some("Max 5x"));
        let unknown = json!({"claudeAiOauth": {"subscriptionType": "future-private-value"}});
        assert_eq!(parse_plan(&unknown), None);
    }

    #[test]
    fn ready_result_has_windows_plan_and_email_without_leaking_anything() {
        let usage = json!({"five_hour": {"utilization": 40, "resets_at": "2026-11-06T12:00:00Z"}});
        let profile = json!({"account": {"email": "someone@example.test"},
                             "organization": {"organization_type": "claude_pro"}});
        let r = usage_result(
            200,
            None,
            &usage,
            Some(&profile),
            None,
            ReadSource::Hub,
            now(),
        );
        assert_eq!(r.state, UsageState::Ready);
        assert_eq!(r.source, ReadSource::Hub);
        assert_eq!(r.plan.as_deref(), Some("Pro"));
        assert_eq!(r.email.as_deref(), Some("someone@example.test"));
        assert_eq!(
            r.reading.as_ref().unwrap().windows[0].remaining_percent(),
            60.0
        );
        assert!(!r.message.contains("example.test"));
    }

    #[test]
    fn fallback_plan_used_when_profile_has_none_and_empty_usage_is_unavailable() {
        let usage = json!({"five_hour": {"utilization": 1}});
        let r = usage_result(
            200,
            None,
            &usage,
            None,
            Some("Pro".into()),
            ReadSource::Profile,
            now(),
        );
        assert_eq!(r.plan.as_deref(), Some("Pro"));
        let r = usage_result(
            200,
            None,
            &json!({"x": 1}),
            None,
            None,
            ReadSource::Profile,
            now(),
        );
        assert_eq!(r.state, UsageState::Unavailable);
        assert!(r.reading.is_none());
    }

    #[test]
    fn statuses_map_to_states() {
        let body = Value::Null;
        let r = usage_result(429, Some(120), &body, None, None, ReadSource::Hub, now());
        assert_eq!(
            (r.state, r.retry_after_secs),
            (UsageState::RateLimited, Some(120))
        );
        let r = usage_result(429, None, &body, None, None, ReadSource::Hub, now());
        assert_eq!(r.retry_after_secs, Some(600));
        for s in [401, 403] {
            assert_eq!(
                usage_result(s, None, &body, None, None, ReadSource::Hub, now()).state,
                UsageState::SignInRequired
            );
        }
        assert_eq!(
            usage_result(502, None, &body, None, None, ReadSource::Hub, now()).state,
            UsageState::Unavailable
        );
    }

    #[test]
    fn credentials_from_synthetic_profile_dir() {
        let dir = tempfile::tempdir().unwrap();
        // No file: sign in required.
        assert_eq!(
            read_credentials(dir.path()).err().unwrap().state,
            UsageState::SignInRequired
        );
        std::fs::write(dir.path().join(".credentials.json"), "garbage").unwrap();
        assert_eq!(
            read_credentials(dir.path()).err().unwrap().state,
            UsageState::SignInRequired
        );
        std::fs::write(
            dir.path().join(".credentials.json"),
            r#"{"claudeAiOauth": {"accessToken": " "}}"#,
        )
        .unwrap();
        assert_eq!(
            read_credentials(dir.path()).err().unwrap().state,
            UsageState::SignInRequired
        );
        let creds =
            r#"{"claudeAiOauth": {"accessToken": "fake-token", "subscriptionType": "pro"}}"#;
        std::fs::write(dir.path().join(".credentials.json"), creds).unwrap();
        let c = read_credentials(dir.path()).ok().unwrap();
        assert_eq!(c.token, "fake-token");
        assert_eq!(c.plan.as_deref(), Some("Pro"));
    }

    #[test]
    fn read_profile_without_credentials_is_sign_in_required_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let r = read_profile(dir.path());
        assert_eq!(
            (r.state, r.source),
            (UsageState::SignInRequired, ReadSource::Profile)
        );
        assert!(r.reading.is_none());
    }
}
