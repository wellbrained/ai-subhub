//! Cursor (owner, 2026-11-09): limits only. Reads `GetCurrentPeriodUsage` on api2.cursor.sh
//! (unofficial; the call Cursor's own `agent` CLI makes for its usage screen) with the Cursor
//! CLI's login in `<profile>\auth.json` (`%APPDATA%\Cursor`). The IDE's own token is encrypted
//! since Cursor 3.2x, so the CLI login is the one source; SubHub never refreshes it (an expired
//! token asks to sign in again). Shapes: docs/research/cursor.md. Not yet checked against a real
//! account (owner has none).
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use serde_json::{Value, json};

use crate::model::{AllowanceWindow, ReadSource, Reading, UsageResult, UsageState};
use crate::util::{self, failure, failure_for_status};

pub const USAGE_URL: &str =
    "https://api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage";
pub const PLAN_URL: &str = "https://api2.cursor.sh/aiserver.v1.DashboardService/GetPlanInfo";

/// A time as epoch milliseconds (number or string) or RFC 3339.
fn time(v: &Value, key: &str) -> Option<DateTime<Utc>> {
    match v.get(key)? {
        Value::Number(n) => Utc.timestamp_millis_opt(n.as_i64()?).single(),
        Value::String(s) => match s.trim().parse::<i64>() {
            Ok(ms) => Utc.timestamp_millis_opt(ms).single(),
            Err(_) => util::parse_rfc3339(s),
        },
        _ => None,
    }
}

fn num(v: &Value, key: &str) -> Option<f64> {
    match v.get(key)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|n: &f64| n.is_finite())
}

/// "Weekly" for a cycle of about a week, else "Monthly" (from the span, never hardcoded: new
/// plans reset weekly, legacy ones monthly).
fn cycle_label(start: Option<DateTime<Utc>>, end: Option<DateTime<Utc>>) -> &'static str {
    match (start, end) {
        (Some(s), Some(e)) if (e - s) <= chrono::Duration::days(10) => "Weekly",
        _ => "Monthly",
    }
}

/// Windows from a `GetCurrentPeriodUsage` body: the plan's total (`cursor:plan`), then Auto
/// (Cursor's models) and API (other models) when reported.
pub fn parse_usage(body: &Value) -> Vec<AllowanceWindow> {
    let Some(plan) = body.get("planUsage").filter(|p| p.is_object()) else {
        return vec![];
    };
    let (start, end) = (
        time(body, "billingCycleStart"),
        time(body, "billingCycleEnd"),
    );
    let cycle = cycle_label(start, end);
    let total = num(plan, "totalPercentUsed").or_else(|| {
        let limit = num(plan, "limit").filter(|l| *l > 0.0)?;
        Some(num(plan, "includedSpend").or_else(|| num(plan, "totalSpend"))? / limit * 100.0)
    });
    let mut out = vec![];
    let mut push = |key: &str, label: String, used: Option<f64>| {
        if let Some(u) = used {
            out.push(AllowanceWindow {
                key: key.into(),
                label,
                used_percent: u.clamp(0.0, 100.0),
                resets_at: end,
            });
        }
    };
    push("cursor:plan", cycle.to_string(), total);
    push(
        "cursor:auto",
        format!("{cycle} · Auto"),
        num(plan, "autoPercentUsed"),
    );
    push(
        "cursor:api",
        format!("{cycle} · API"),
        num(plan, "apiPercentUsed"),
    );
    out
}

/// "Pro", "Pro+", "Ultra", ... from `GetPlanInfo` (`planInfo.planName`).
pub fn parse_plan(body: &Value) -> Option<String> {
    let name = body.get("planInfo")?.get("planName")?.as_str()?.trim();
    (!name.is_empty() && name.len() <= 40).then(|| name.to_string())
}

pub fn usage_result(status: u16, body: &Value, now: DateTime<Utc>) -> UsageResult {
    let src = ReadSource::Profile;
    if let Some(f) = failure_for_status("Cursor", status, None, src) {
        return f;
    }
    let windows = parse_usage(body);
    if windows.is_empty() {
        // Team-billed seats have no personal plan usage (start == end).
        let team = body.get("planUsage").is_none()
            && time(body, "billingCycleStart").is_some()
            && time(body, "billingCycleStart") == time(body, "billingCycleEnd");
        return failure(
            UsageState::Unavailable,
            if team {
                "This Cursor seat is billed to a team; SubHub cannot show team limits yet."
            } else {
                "Cursor returned no usage data. The endpoint may have changed."
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
        email: None,
        plan: None,
        source: src,
    }
}

/// The CLI's access token from `auth.json`, and whether it has expired (JWT `exp`).
fn read_token(profile_dir: &Path) -> Option<(String, bool)> {
    let v: Value =
        serde_json::from_str(&std::fs::read_to_string(profile_dir.join("auth.json")).ok()?).ok()?;
    let token = v.get("accessToken")?.as_str()?.trim().to_string();
    if token.is_empty() || token.starts_with("cursor-safe-storage:") {
        return None;
    }
    let expired = util::jwt_claims(&token)
        .and_then(|c| c.get("exp").and_then(Value::as_i64))
        .is_some_and(|exp| exp <= Utc::now().timestamp());
    Some((token, expired))
}

/// The login email from the token's claims, when it carries one.
pub fn profile_email(profile_dir: &Path) -> Option<String> {
    let (token, _) = read_token(profile_dir)?;
    let claims = util::jwt_claims(&token)?;
    util::valid_email(claims.get("email")?.as_str()?)
}

fn post(http: &reqwest::blocking::Client, url: &str, token: &str) -> Option<(u16, Value)> {
    let r = http
        .post(url)
        .bearer_auth(token)
        .header("Content-Type", "application/json")
        .header("Connect-Protocol-Version", "1")
        .header("Accept", "application/json")
        .json(&json!({}))
        .send()
        .ok()?;
    let status = r.status().as_u16();
    Some((status, r.json().unwrap_or(Value::Null)))
}

/// Reads the Cursor account behind this PC's Cursor CLI login.
pub fn read_profile(profile_dir: &Path) -> UsageResult {
    let src = ReadSource::Profile;
    let sign_in = "Sign in once with the Cursor CLI (`cursor-agent login`); SubHub reads Cursor through that login.";
    let Some((token, expired)) = read_token(profile_dir) else {
        return failure(UsageState::SignInRequired, sign_in, src);
    };
    if expired {
        return failure(
            UsageState::SignInRequired,
            "The Cursor CLI login has expired. Run `cursor-agent login` again.",
            src,
        );
    }
    let Some(http) = util::http_client(Duration::from_secs(20)) else {
        return failure(
            UsageState::Unavailable,
            "Could not start the network client.",
            src,
        );
    };
    let Some((status, body)) = post(&http, USAGE_URL, &token) else {
        return failure(UsageState::Unavailable, "Cursor could not be reached.", src);
    };
    let mut result = usage_result(status, &body, Utc::now());
    if result.state == UsageState::Ready {
        result.email = profile_email(profile_dir);
        result.plan = post(&http, PLAN_URL, &token)
            .filter(|(s, _)| (200..300).contains(s))
            .and_then(|(_, b)| parse_plan(&b));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        "2026-11-09T10:00:00Z".parse().unwrap()
    }

    #[test]
    fn weekly_unified_plan_with_sub_pools() {
        let b = json!({
            "billingCycleStart": "1794211200000",
            "billingCycleEnd": "1794816000000",
            "planUsage": {"totalSpend": 123, "limit": 2000, "autoPercentUsed": 1.5, "apiPercentUsed": 0, "totalPercentUsed": 41.2}
        });
        let w = parse_usage(&b);
        assert_eq!(
            w.iter()
                .map(|w| (w.key.as_str(), w.label.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("cursor:plan", "Weekly"),
                ("cursor:auto", "Weekly · Auto"),
                ("cursor:api", "Weekly · API")
            ]
        );
        assert!((w[0].used_percent - 41.2).abs() < 1e-9);
        assert_eq!(
            w[0].resets_at,
            Utc.timestamp_millis_opt(1_794_816_000_000).single()
        );
        assert_eq!(usage_result(200, &b, now()).state, UsageState::Ready);
    }

    #[test]
    fn monthly_legacy_plan_from_spend_and_iso_dates() {
        let b = json!({
            "billingCycleStart": "2026-11-01T00:00:00.000Z",
            "billingCycleEnd": "2026-12-01T00:00:00.000Z",
            "planUsage": {"includedSpend": 500, "limit": 2000}
        });
        let w = parse_usage(&b);
        assert_eq!(w.len(), 1);
        assert_eq!((w[0].label.as_str(), w[0].used_percent), ("Monthly", 25.0));
    }

    #[test]
    fn team_seat_and_errors() {
        let team =
            json!({"billingCycleStart": "1794211200000", "billingCycleEnd": "1794211200000"});
        assert!(usage_result(200, &team, now()).message.contains("team"));
        assert_eq!(
            usage_result(401, &Value::Null, now()).state,
            UsageState::SignInRequired
        );
        assert_eq!(
            parse_plan(&json!({"planInfo": {"planName": "Pro+"}})).as_deref(),
            Some("Pro+")
        );
    }

    #[test]
    fn token_file_missing_encrypted_or_expired() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_profile(dir.path()).state, UsageState::SignInRequired);
        std::fs::write(
            dir.path().join("auth.json"),
            r#"{"accessToken":"cursor-safe-storage:v1:xx"}"#,
        )
        .unwrap();
        assert_eq!(read_profile(dir.path()).state, UsageState::SignInRequired);
        // header.{"exp":1}.sig: long expired, so no network call.
        let jwt = format!(
            "e30.{}.c2ln",
            util::base64url_encode(br#"{"exp":1,"email":"me@example.com"}"#)
        );
        std::fs::write(
            dir.path().join("auth.json"),
            json!({"accessToken": jwt}).to_string(),
        )
        .unwrap();
        let r = read_profile(dir.path());
        assert_eq!(r.state, UsageState::SignInRequired);
        assert!(r.message.contains("expired"));
        assert_eq!(profile_email(dir.path()).as_deref(), Some("me@example.com"));
    }
}
