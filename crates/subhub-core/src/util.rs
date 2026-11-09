//! Small shared helpers: JSON number/time parsing, Retry-After, status mapping, HTTP client.
//! Nothing here logs or returns tokens, bodies or emails in messages.
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::model::{ReadSource, UsageResult, UsageState};

pub(crate) const USER_AGENT: &str = concat!("AI-SubHub-GPUI/", env!("CARGO_PKG_VERSION"));
/// Provider cooldown used when a 429 carries no usable `Retry-After` (same as the C# app).
pub(crate) const DEFAULT_COOLDOWN_SECS: u64 = 600;

pub(crate) fn http_client(timeout: Duration) -> Option<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(timeout)
        .user_agent(USER_AGENT)
        .build()
        .ok()
}

/// A finite number in 0..=100 under `key`; anything else (missing, null, negative, >100) is None.
pub(crate) fn percent(v: &Value, key: &str) -> Option<f64> {
    let n = v.get(key)?.as_f64()?;
    (n.is_finite() && (0.0..=100.0).contains(&n)).then_some(n)
}

pub(crate) fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key)?.as_str()
}

pub(crate) fn parse_rfc3339(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// Unix seconds (integer or float JSON number) to a time; out-of-range is None.
pub(crate) fn unix_time(v: &Value) -> Option<DateTime<Utc>> {
    let secs = match v.as_i64() {
        Some(s) => s,
        None => {
            let f = v.as_f64()?;
            if !f.is_finite() {
                return None;
            }
            f as i64
        }
    };
    DateTime::from_timestamp(secs, 0)
}

/// `Retry-After` as seconds: either a delay in seconds or an HTTP date.
pub(crate) fn parse_retry_after(value: &str, now: DateTime<Utc>) -> Option<u64> {
    let value = value.trim();
    if let Ok(secs) = value.parse::<u64>() {
        return Some(secs);
    }
    let date = DateTime::parse_from_rfc2822(value)
        .ok()?
        .with_timezone(&Utc);
    Some((date - now).num_seconds().max(0) as u64)
}

/// A plausible email address or None (length, no whitespace/control characters).
pub(crate) fn valid_email(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() || s.len() > 254 || s.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let (local, domain) = s.split_once('@')?;
    if local.is_empty() || domain.is_empty() || domain.contains('@') || !domain.contains('.') {
        return None;
    }
    Some(s.to_string())
}

pub(crate) fn failure(state: UsageState, message: &str, source: ReadSource) -> UsageResult {
    UsageResult {
        state,
        message: message.to_string(),
        reading: None,
        retry_after_secs: None,
        email: None,
        plan: None,
        source,
    }
}

/// Maps a provider HTTP status to a failure result; None for 2xx (caller parses the body).
/// 429 -> RateLimited with a cooldown, 401/403 -> SignInRequired, others -> Unavailable.
pub(crate) fn failure_for_status(
    provider: &str,
    status: u16,
    retry_after: Option<u64>,
    source: ReadSource,
) -> Option<UsageResult> {
    match status {
        200..=299 => None,
        429 => {
            let mut r = failure(
                UsageState::RateLimited,
                "Usage checks are rate-limited by the provider. This does not mean your chat allowance is exhausted.",
                source,
            );
            r.retry_after_secs = Some(retry_after.unwrap_or(DEFAULT_COOLDOWN_SECS));
            Some(r)
        }
        401 | 403 => Some(failure(
            UsageState::SignInRequired,
            &match source {
                // The hub owns and renews hub logins; SubHub never refreshes tokens itself.
                ReadSource::Hub => format!(
                    "The hub's {provider} login was not accepted (often an expired token the hub has not renewed yet). Use it once through the hub or sign in again in the hub."
                ),
                ReadSource::Profile => {
                    format!(
                        "{provider} could not authorize usage access. Sign in again, then refresh."
                    )
                }
            },
            source,
        )),
        n => Some(failure(
            UsageState::Unavailable,
            &format!("{provider} usage is unavailable (HTTP {n}). The endpoint may have changed."),
            source,
        )),
    }
}

/// The claims of a JWT (payload only; never verified, never kept).
pub(crate) fn jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    serde_json::from_slice(&crate::hub::base64url_decode(payload)?).ok()
}

#[cfg(test)]
pub(crate) fn base64url_encode(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
        let chars = chunk.len() + 1;
        for i in 0..chars {
            out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

/// Runs `cmd` (stdout piped) and returns its stdout, or None when it fails to start, exits
/// with an error or runs longer than `timeout` (then it is killed with its process tree).
pub(crate) fn output_with_timeout(
    mut cmd: std::process::Command,
    timeout: Duration,
) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    // The read ends when every process holding the pipe closed it; a helper the child started
    // may keep it open after the child exited, so the read is bounded by the same deadline.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = vec![];
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    let deadline = std::time::Instant::now() + timeout;
    let kill_tree = |child: &mut std::process::Child| {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &child.id().to_string(), "/T", "/F"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .creation_flags(0x0800_0000)
                .status();
        }
        let _ = child.kill();
        let _ = child.wait();
    };
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100))
            }
            _ => break None,
        }
    };
    let Some(status) = status else {
        kill_tree(&mut child);
        return None;
    };
    let left = deadline.saturating_duration_since(std::time::Instant::now());
    match rx.recv_timeout(left.max(Duration::from_millis(500))) {
        Ok(out) => status.success().then_some(out),
        Err(_) => {
            // The child is done; its helpers hold the pipe. The tree is gone with the child, so
            // only a detached helper is left; the reader thread ends when it does.
            kill_tree(&mut child);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn percent_rejects_missing_null_and_out_of_range() {
        let v = json!({"a": 12.5, "b": null, "c": -5, "d": 101, "e": "7"});
        assert_eq!(percent(&v, "a"), Some(12.5));
        for k in ["b", "c", "d", "e", "zz"] {
            assert_eq!(percent(&v, k), None, "{k}");
        }
    }

    #[test]
    fn retry_after_seconds_and_date() {
        let now = parse_rfc3339("2026-11-06T10:00:00Z").unwrap();
        assert_eq!(parse_retry_after(" 120 ", now), Some(120));
        assert_eq!(
            parse_retry_after("Fri, 06 Nov 2026 10:05:00 GMT", now),
            Some(300)
        );
        assert_eq!(
            parse_retry_after("Fri, 06 Nov 2026 09:00:00 GMT", now),
            Some(0)
        );
        assert_eq!(parse_retry_after("soon", now), None);
    }

    #[test]
    fn email_validation() {
        assert_eq!(valid_email("a@b.example").as_deref(), Some("a@b.example"));
        for bad in ["", "a", "a@", "@b.c", "a b@c.d", "a@b", "a@@b.c"] {
            assert_eq!(valid_email(bad), None, "{bad}");
        }
    }

    #[test]
    fn status_mapping() {
        let s = ReadSource::Hub;
        assert!(failure_for_status("X", 200, None, s).is_none());
        let r = failure_for_status("X", 429, Some(90), s).unwrap();
        assert_eq!(
            (r.state, r.retry_after_secs),
            (UsageState::RateLimited, Some(90))
        );
        let r = failure_for_status("X", 429, None, s).unwrap();
        assert_eq!(r.retry_after_secs, Some(DEFAULT_COOLDOWN_SECS));
        assert_eq!(
            failure_for_status("X", 401, None, s).unwrap().state,
            UsageState::SignInRequired
        );
        assert_eq!(
            failure_for_status("X", 403, None, s).unwrap().state,
            UsageState::SignInRequired
        );
        assert_eq!(
            failure_for_status("X", 500, None, s).unwrap().state,
            UsageState::Unavailable
        );
    }
}
