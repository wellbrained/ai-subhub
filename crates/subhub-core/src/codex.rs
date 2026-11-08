//! Codex: official CLI app-server account + rate-limit reads, or `wham/usage` through the hub.
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::model::{AllowanceWindow, ReadSource, Reading, UsageResult, UsageState};
use crate::util::{self, failure, failure_for_status};

pub const HUB_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";

/// Whole app-server session (spawn to last answer) must finish within this.
const SESSION_TIMEOUT: Duration = Duration::from_secs(25);

/// Window label from its length in minutes: "5-hour", "Weekly", "2-week", "30-minute"; falls back
/// to the slot name when the length is unknown.
fn duration_label(minutes: Option<i64>, slot: &str) -> String {
    match minutes {
        Some(10080) => "Weekly".into(),
        Some(m) if m > 0 && m % 10080 == 0 => format!("{}-week", m / 10080),
        Some(m) if m > 0 && m % 1440 == 0 => format!("{}-day", m / 1440),
        Some(m) if m > 0 && m % 60 == 0 => format!("{}-hour", m / 60),
        Some(m) if m > 0 => format!("{m}-minute"),
        _ => slot.to_string(),
    }
}

fn as_minutes(v: Option<&Value>) -> Option<i64> {
    let v = v?;
    v.as_i64()
        .or_else(|| v.as_f64().filter(|f| f.is_finite()).map(|f| f as i64))
}

fn add_app_server_bucket(out: &mut Vec<AllowanceWindow>, bucket: &Value, id: &str) {
    if !bucket.is_object() {
        return;
    }
    let name = util::text(bucket, "limitName").filter(|n| !n.trim().is_empty());
    // The main "codex" limit needs no prefix; other limits are named.
    let prefix = if id.eq_ignore_ascii_case("codex") {
        None
    } else {
        Some(name.unwrap_or(id))
    };
    for slot in ["primary", "secondary"] {
        let Some(window) = bucket.get(slot).filter(|w| w.is_object()) else {
            continue;
        };
        let Some(used) = util::percent(window, "usedPercent") else {
            continue;
        };
        let label = duration_label(as_minutes(window.get("windowDurationMins")), slot);
        out.push(AllowanceWindow {
            key: format!("{id}:{slot}"),
            label: match prefix {
                Some(p) => format!("{p} · {label}"),
                None => label,
            },
            used_percent: used,
            resets_at: window.get("resetsAt").and_then(util::unix_time),
        });
    }
}

/// Parses the app-server `rateLimits` / `rateLimitsByLimitId` shape. Keys: "<limitId>:primary" etc.
/// `rateLimitsByLimitId` wins; `rateLimits` is only the fallback (key id "codex").
pub fn parse_app_server(root: &Value) -> Vec<AllowanceWindow> {
    let mut out = vec![];
    if let Some(buckets) = root.get("rateLimitsByLimitId").and_then(Value::as_object) {
        // "codex" first, then the rest in the map's (alphabetical) order.
        let mut ids: Vec<&String> = buckets.keys().collect();
        ids.sort_by_key(|k| !k.eq_ignore_ascii_case("codex"));
        for id in ids {
            add_app_server_bucket(&mut out, &buckets[id], id);
        }
    }
    if out.is_empty()
        && let Some(fallback) = root.get("rateLimits")
    {
        add_app_server_bucket(&mut out, fallback, "codex");
    }
    out
}

fn parse_wham_window(window: &Value, slot: &str, now: DateTime<Utc>) -> Option<AllowanceWindow> {
    if !window.is_object() {
        return None;
    }
    let used = util::percent(window, "used_percent")?;
    let minutes = window
        .get("limit_window_seconds")
        .and_then(Value::as_i64)
        .filter(|s| *s > 0)
        .map(|s| s / 60);
    let resets_at = window
        .get("reset_at")
        .and_then(util::unix_time)
        .or_else(|| {
            let secs = window
                .get("reset_after_seconds")?
                .as_i64()
                .filter(|s| *s >= 0)?;
            now.checked_add_signed(chrono::Duration::seconds(secs))
        });
    Some(AllowanceWindow {
        key: format!("codex:{slot}"),
        label: duration_label(minutes, slot),
        used_percent: used,
        resets_at,
    })
}

/// Parses the `wham/usage` body returned through the hub (shape checked against the Codex CLI's
/// `RateLimitStatusPayload`, 2026-11-08): `rate_limit.primary_window` / `secondary_window` with
/// a valid `used_percent`, then each `additional_rate_limits` entry as a named limit, keyed and
/// labelled like the app-server read (`<metered_feature>:primary`, "<limit_name> · 5-hour").
/// Anything else gives no windows.
pub fn parse_wham(body: &Value) -> Vec<AllowanceWindow> {
    parse_wham_at(body, Utc::now())
}

/// Like [`parse_wham`] with an explicit clock (for `reset_after_seconds`).
pub fn parse_wham_at(body: &Value, now: DateTime<Utc>) -> Vec<AllowanceWindow> {
    let Some(limit) = body.get("rate_limit").filter(|l| l.is_object()) else {
        return vec![];
    };
    let slots = |limit: &Value| -> Vec<AllowanceWindow> {
        [
            ("primary_window", "primary"),
            ("secondary_window", "secondary"),
        ]
        .into_iter()
        .filter_map(|(field, slot)| parse_wham_window(limit.get(field)?, slot, now))
        .collect()
    };
    let mut out = slots(limit);
    let extra = body
        .get("additional_rate_limits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    for e in extra {
        let Some(id) = util::text(e, "metered_feature").filter(|s| !s.trim().is_empty()) else {
            continue;
        };
        let Some(limit) = e.get("rate_limit").filter(|l| l.is_object()) else {
            continue;
        };
        if id.eq_ignore_ascii_case("codex") {
            continue;
        }
        let name = util::text(e, "limit_name")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(id);
        for mut w in slots(limit) {
            let slot = w.key.rsplit(':').next().unwrap_or("primary").to_string();
            w.key = format!("{id}:{slot}");
            w.label = format!("{name} · {}", w.label);
            out.push(w);
        }
    }
    out
}

/// Plan from a `wham/usage` body (`plan_type`), known values only.
pub fn parse_wham_plan(body: &Value) -> Option<String> {
    crate::plan::label(util::text(body, "plan_type")?)
}

/// Pure core of a Codex read through the hub: provider HTTP status, `Retry-After`, parsed body.
pub fn wham_result(
    status: u16,
    retry_after: Option<u64>,
    body: &Value,
    email: Option<String>,
    source: ReadSource,
    now: DateTime<Utc>,
) -> UsageResult {
    if let Some(f) = failure_for_status("Codex", status, retry_after, source) {
        return f;
    }
    let windows = parse_wham_at(body, now);
    if windows.is_empty() {
        return failure(
            UsageState::Unavailable,
            "Codex returned no subscription allowances for this account.",
            source,
        );
    }
    let mut r = failure(UsageState::Ready, "Connected", source);
    r.reading = Some(Reading {
        fetched_at: now,
        windows,
    });
    r.plan = parse_wham_plan(body);
    r.email = email.and_then(|e| util::valid_email(&e));
    r
}

// ---------------------------------------------------------------------------------------------
// Direct read: Codex CLI app-server over stdio (JSON-RPC lines).

/// Line transport to the app-server; a fake in tests.
trait Transport {
    fn send(&mut self, message: &Value) -> std::io::Result<()>;
    fn recv(&mut self, timeout: Duration) -> Result<String, RpcError>;
}

#[derive(Debug, PartialEq)]
enum RpcError {
    Timeout,
    Closed,
    Remote,
    Io,
}

fn call(
    t: &mut dyn Transport,
    next_id: &mut u64,
    method: &str,
    params: Value,
    deadline: Instant,
) -> Result<Value, RpcError> {
    *next_id += 1;
    let id = *next_id;
    t.send(&json!({"id": id, "method": method, "params": params}))
        .map_err(|_| RpcError::Io)?;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(RpcError::Timeout);
        }
        let line = t.recv(remaining)?;
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if msg.get("id").and_then(Value::as_u64) != Some(id) {
            continue; // notifications and other ids
        }
        if msg.get("error").is_some() {
            return Err(RpcError::Remote);
        }
        return msg.get("result").cloned().ok_or(RpcError::Remote);
    }
}

fn rpc_failure(method: &str, e: RpcError) -> UsageResult {
    let src = ReadSource::Profile;
    let message = match e {
        RpcError::Timeout => "Codex did not answer in time. Try again.".to_string(),
        RpcError::Closed => {
            "Codex closed its usage connection. Check the CLI installation and try again."
                .to_string()
        }
        RpcError::Remote => format!(
            "Codex could not complete {method}. Check its login and CLI version, then refresh."
        ),
        RpcError::Io => "Could not talk to the Codex CLI.".to_string(),
    };
    failure(UsageState::Unavailable, &message, src)
}

/// The usage conversation as the C# app does it: initialize, initialized, account/read,
/// account/rateLimits/read.
fn run_session(t: &mut dyn Transport, deadline: Instant, now: DateTime<Utc>) -> UsageResult {
    let src = ReadSource::Profile;
    let mut id = 0u64;
    let sign_in = || {
        failure(
            UsageState::SignInRequired,
            "Sign in with ChatGPT to connect this profile.",
            src,
        )
    };

    let init = json!({"clientInfo": {"name": "ai_subhub", "title": "AI SubHub", "version": env!("CARGO_PKG_VERSION")}});
    if let Err(e) = call(t, &mut id, "initialize", init, deadline) {
        return rpc_failure("initialize", e);
    }
    if t.send(&json!({"method": "initialized", "params": {}}))
        .is_err()
    {
        return rpc_failure("initialized", RpcError::Io);
    }
    let identity = match call(
        t,
        &mut id,
        "account/read",
        json!({"refreshToken": false}),
        deadline,
    ) {
        Ok(v) => v,
        Err(e) => return rpc_failure("account/read", e),
    };
    let Some(account) = identity.get("account").filter(|a| a.is_object()) else {
        return sign_in();
    };
    if util::text(account, "type") == Some("apiKey") {
        return failure(
            UsageState::Unavailable,
            "This profile uses an API key. Sign in with a ChatGPT subscription for subscription usage.",
            src,
        );
    }
    let data = match call(t, &mut id, "account/rateLimits/read", Value::Null, deadline) {
        Ok(v) => v,
        Err(e) => return rpc_failure("account/rateLimits/read", e),
    };
    let windows = parse_app_server(&data);
    if windows.is_empty() {
        return failure(
            UsageState::Unavailable,
            "Codex returned no subscription allowances for this account.",
            src,
        );
    }
    let plan = data
        .get("rateLimits")
        .and_then(|l| util::text(l, "planType"))
        .and_then(crate::plan::label)
        .or_else(|| util::text(account, "planType").and_then(crate::plan::label));
    let mut r = failure(UsageState::Ready, "Connected", src);
    r.reading = Some(Reading {
        fetched_at: now,
        windows,
    });
    r.plan = plan;
    r.email = util::text(account, "email").and_then(util::valid_email);
    r
}

/// Env vars that would make the CLI use an API key or another endpoint instead of the login.
const ENV_OVERRIDES: [&str; 3] = ["OPENAI_API_KEY", "OPENAI_BASE_URL", "CODEX_API_KEY"];

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn build_command(exe: &Path, profile_dir: &Path) -> Command {
    let args = ["app-server", "--listen", "stdio://"];
    let direct = !cfg!(windows)
        || exe
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe"));
    let mut cmd = if direct {
        let mut c = Command::new(exe);
        c.args(args);
        c
    } else {
        // .cmd/.bat shims (npm installs) are started through PowerShell, like the C# app.
        let script = format!(
            "& {} {}",
            ps_quote(&exe.to_string_lossy()),
            args.map(ps_quote).join(" ")
        );
        let mut c = Command::new("powershell.exe");
        c.args(["-NoLogo", "-NoProfile", "-Command", &script]);
        c
    };
    for key in ENV_OVERRIDES {
        cmd.env_remove(key);
    }
    cmd.env("CODEX_HOME", profile_dir)
        .current_dir(profile_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// The spawned app-server. Killed (with its process tree on Windows) when dropped.
struct Proc {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
}

impl Proc {
    fn spawn(exe: &Path, profile_dir: &Path) -> std::io::Result<Self> {
        let mut child = build_command(exe, profile_dir).spawn()?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::other("no stdio"));
        };
        let (tx, lines) = mpsc::channel();
        // Ends on its own when the pipe closes (child killed or exited).
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(l) => {
                        if tx.send(l).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            lines,
        })
    }
}

impl Transport for Proc {
    fn send(&mut self, message: &Value) -> std::io::Result<()> {
        writeln!(self.stdin, "{message}")?;
        self.stdin.flush()
    }

    fn recv(&mut self, timeout: Duration) -> Result<String, RpcError> {
        self.lines.recv_timeout(timeout).map_err(|e| match e {
            RecvTimeoutError::Timeout => RpcError::Timeout,
            RecvTimeoutError::Disconnected => RpcError::Closed,
        })
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let _ = Command::new("taskkill")
                .args(["/PID", &self.child.id().to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .status();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(crate) fn find_in(dirs: &[PathBuf], name: &str, extensions: &[&str]) -> Option<PathBuf> {
    for dir in dirs {
        for ext in extensions {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Same search order as the C# app: ~/.local/bin, npm global, PATH, bundled desktop build.
pub(crate) fn candidate_dirs(
    home: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
    path_var: Option<OsString>,
) -> Vec<PathBuf> {
    let mut dirs = vec![];
    if let Some(home) = home {
        dirs.push(home.join(".local").join("bin"));
        dirs.push(home.join("AppData").join("Roaming").join("npm"));
    }
    if let Some(path) = path_var {
        dirs.extend(std::env::split_paths(&path).filter(|p| !p.as_os_str().is_empty()));
    }
    if let Some(bundled) = local_app_data.map(|l| l.join("OpenAI").join("Codex").join("bin"))
        && let Ok(read) = std::fs::read_dir(&bundled)
    {
        let mut versions: Vec<PathBuf> = read
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        versions.sort();
        versions.reverse();
        dirs.extend(versions);
    }
    dirs
}

/// Finds the Codex CLI (`codex.exe`, `.cmd`, `.bat` on Windows).
pub fn find_codex_exe() -> Option<PathBuf> {
    let dirs = candidate_dirs(
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from),
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        std::env::var_os("PATH"),
    );
    let exts: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    find_in(&dirs, "codex", exts)
}

/// Direct read via the Codex CLI app-server for this profile (CODEX_HOME = profile_dir).
/// `codex_exe` is the configured executable; None searches the usual places. The child is
/// killed on timeout and on drop.
pub fn read_profile(profile_dir: &Path, codex_exe: Option<&Path>) -> UsageResult {
    let src = ReadSource::Profile;
    let exe = match codex_exe {
        Some(p) => p.is_file().then(|| p.to_path_buf()),
        None => find_codex_exe(),
    };
    let Some(exe) = exe else {
        return failure(
            UsageState::CliMissing,
            "Codex CLI was not found. Set its executable in Settings.",
            src,
        );
    };
    if !profile_dir.is_dir() {
        return failure(
            UsageState::SignInRequired,
            "Sign in with ChatGPT to connect this profile.",
            src,
        );
    }
    let deadline = Instant::now() + SESSION_TIMEOUT;
    match Proc::spawn(&exe, profile_dir) {
        Ok(mut proc) => run_session(&mut proc, deadline, Utc::now()),
        Err(_) => failure(
            UsageState::Unavailable,
            "Could not start the Codex CLI.",
            src,
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    fn now() -> DateTime<Utc> {
        "2026-11-06T10:00:00Z".parse().unwrap()
    }

    #[test]
    fn app_server_uses_every_bucket_and_its_duration() {
        let root = json!({"rateLimitsByLimitId": {
            "codex": {"primary": {"usedPercent": 20, "windowDurationMins": 300, "resetsAt": 1792000000}, "secondary": null},
            "review": {"limitName": "Review", "primary": {"usedPercent": 45, "windowDurationMins": 10080, "resetsAt": null}}},
            "rateLimits": {"primary": {"usedPercent": 99}}});
        let w = parse_app_server(&root);
        assert_eq!(w.len(), 2);
        assert_eq!(
            (w[0].key.as_str(), w[0].label.as_str()),
            ("codex:primary", "5-hour")
        );
        assert_eq!(w[0].remaining_percent(), 80.0);
        assert_eq!(w[0].resets_at.unwrap().timestamp(), 1792000000);
        assert_eq!(
            (w[1].key.as_str(), w[1].label.as_str()),
            ("review:primary", "Review · Weekly")
        );
        assert!(w[1].resets_at.is_none());
    }

    #[test]
    fn app_server_falls_back_to_rate_limits_and_rejects_bad_numbers() {
        let root = json!({"rateLimits": {"primary": {"usedPercent": 10, "windowDurationMins": 10080},
                                         "secondary": {"usedPercent": 30}}});
        let w = parse_app_server(&root);
        assert_eq!(
            w.iter().map(|w| w.key.as_str()).collect::<Vec<_>>(),
            ["codex:primary", "codex:secondary"]
        );
        assert_eq!(w[0].label, "Weekly");
        assert_eq!(w[1].label, "secondary");
        let bad = json!({"rateLimits": {"primary": {"usedPercent": null}, "secondary": {"usedPercent": -5}}});
        assert!(parse_app_server(&bad).is_empty());
        assert!(parse_app_server(&json!(null)).is_empty());
    }

    #[test]
    fn duration_labels() {
        assert_eq!(duration_label(Some(300), "p"), "5-hour");
        assert_eq!(duration_label(Some(1440), "p"), "1-day");
        assert_eq!(duration_label(Some(20160), "p"), "2-week");
        assert_eq!(duration_label(Some(45), "p"), "45-minute");
        assert_eq!(duration_label(Some(0), "p"), "p");
        assert_eq!(duration_label(None, "p"), "p");
    }

    #[test]
    fn wham_parses_both_windows_defensively() {
        let body = json!({"plan_type": "plus", "rate_limit": {
            "allowed": true,
            "primary_window": {"used_percent": 12, "limit_window_seconds": 18000, "reset_after_seconds": 600, "reset_at": 1792000000},
            "secondary_window": {"used_percent": 40.5, "limit_window_seconds": 604800, "reset_after_seconds": 3600}}});
        let w = parse_wham_at(&body, now());
        assert_eq!(w.len(), 2);
        assert_eq!(
            (w[0].key.as_str(), w[0].label.as_str()),
            ("codex:primary", "5-hour")
        );
        assert_eq!(w[0].resets_at.unwrap().timestamp(), 1792000000);
        assert_eq!(w[1].label, "Weekly");
        assert_eq!(
            w[1].resets_at,
            Some(now() + chrono::Duration::seconds(3600))
        );
        assert_eq!(parse_wham_plan(&body).as_deref(), Some("Plus"));
    }

    #[test]
    fn wham_reads_additional_limits_like_the_app_server() {
        let body = json!({
            "plan_type": "pro",
            "rate_limit": {"primary_window": {"used_percent": 10, "limit_window_seconds": 18000, "reset_after_seconds": 60, "reset_at": 1_900_000_000}},
            "additional_rate_limits": [
                {"limit_name": "Spark", "metered_feature": "codex_spark", "rate_limit": {
                    "primary_window": {"used_percent": 40, "limit_window_seconds": 604800, "reset_after_seconds": 60, "reset_at": 1_900_000_000}}},
                {"limit_name": "", "metered_feature": "codex_x", "rate_limit": {
                    "secondary_window": {"used_percent": 5, "limit_window_seconds": 18000, "reset_after_seconds": 60, "reset_at": 1_900_000_000}}},
                {"limit_name": "dup", "metered_feature": "codex", "rate_limit": {
                    "primary_window": {"used_percent": 99, "limit_window_seconds": 18000, "reset_after_seconds": 60, "reset_at": 1_900_000_000}}},
                {"limit_name": "No id", "rate_limit": {}}
            ]
        });
        let w = parse_wham_at(&body, now());
        let got: Vec<(&str, &str)> = w
            .iter()
            .map(|w| (w.key.as_str(), w.label.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("codex:primary", "5-hour"),
                ("codex_spark:primary", "Spark · Weekly"),
                ("codex_x:secondary", "codex_x · 5-hour"),
            ]
        );
        // Named limits are neither the 5-hour nor the weekly window of the account.
        assert!(!w[1].is_weekly() && !w[2].is_session());
    }

    #[test]
    fn wham_unknown_shapes_give_no_windows() {
        for body in [
            json!(null),
            json!({}),
            json!({"rate_limit": null}),
            json!({"rate_limit": {"primary_window": {"used_percent": "x"}}}),
            json!({"rate_limit": {"primary_window": {"used_percent": 400}}}),
            json!({"rate_limit": {"primary_window": {"limit_window_seconds": 18000}}}),
        ] {
            assert!(parse_wham_at(&body, now()).is_empty(), "{body}");
        }
        assert_eq!(
            parse_wham_plan(&json!({"plan_type": "private-thing"})),
            None
        );
    }

    #[test]
    fn wham_result_maps_status_and_never_invents_readings() {
        let body = json!({"rate_limit": {"primary_window": {"used_percent": 25}}});
        let r = wham_result(
            200,
            None,
            &body,
            Some("someone@example.test".into()),
            ReadSource::Hub,
            now(),
        );
        assert_eq!(r.state, UsageState::Ready);
        assert_eq!(r.email.as_deref(), Some("someone@example.test"));
        let r = wham_result(
            200,
            None,
            &json!({"unexpected": true}),
            None,
            ReadSource::Hub,
            now(),
        );
        assert_eq!((r.state, r.reading), (UsageState::Unavailable, None));
        let r = wham_result(429, Some(77), &Value::Null, None, ReadSource::Hub, now());
        assert_eq!(
            (r.state, r.retry_after_secs),
            (UsageState::RateLimited, Some(77))
        );
        assert_eq!(
            wham_result(401, None, &Value::Null, None, ReadSource::Hub, now()).state,
            UsageState::SignInRequired
        );
        assert_eq!(
            wham_result(500, None, &Value::Null, None, ReadSource::Hub, now()).state,
            UsageState::Unavailable
        );
    }

    /// Scripted app-server: answers requests by method, plus an unrelated notification first.
    struct Fake {
        sent: Vec<Value>,
        queue: VecDeque<String>,
        account: Value,
        limits: Value,
        fail_method: Option<&'static str>,
        silent: bool,
    }

    impl Fake {
        fn new(account: Value, limits: Value) -> Self {
            Self {
                sent: vec![],
                queue: VecDeque::new(),
                account,
                limits,
                fail_method: None,
                silent: false,
            }
        }
    }

    impl Transport for Fake {
        fn send(&mut self, m: &Value) -> std::io::Result<()> {
            self.sent.push(m.clone());
            let (Some(id), Some(method)) = (m.get("id"), m.get("method").and_then(Value::as_str))
            else {
                return Ok(());
            };
            if self.silent {
                return Ok(());
            }
            self.queue
                .push_back(json!({"method": "thread/started", "params": {}}).to_string());
            let reply = if self.fail_method == Some(method) {
                json!({"id": id, "error": {"code": -1, "message": "secret detail"}})
            } else {
                let result = match method {
                    "account/read" => self.account.clone(),
                    "account/rateLimits/read" => self.limits.clone(),
                    _ => json!({}),
                };
                json!({"id": id, "result": result})
            };
            self.queue.push_back(reply.to_string());
            Ok(())
        }

        fn recv(&mut self, _timeout: Duration) -> Result<String, RpcError> {
            self.queue.pop_front().ok_or(RpcError::Closed)
        }
    }

    fn soon() -> Instant {
        Instant::now() + Duration::from_secs(5)
    }

    #[test]
    fn session_reads_windows_plan_and_email() {
        let account = json!({"account": {"type": "chatgpt", "email": "someone@example.test", "planType": "plus"}});
        let limits = json!({"rateLimits": {"planType": "pro", "primary": {"usedPercent": 30, "windowDurationMins": 300}}});
        let mut fake = Fake::new(account, limits);
        let r = run_session(&mut fake, soon(), now());
        assert_eq!(r.state, UsageState::Ready);
        assert_eq!(r.plan.as_deref(), Some("Pro"));
        assert_eq!(r.email.as_deref(), Some("someone@example.test"));
        assert_eq!(r.source, ReadSource::Profile);
        let methods: Vec<_> = fake
            .sent
            .iter()
            .map(|m| m["method"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            methods,
            [
                "initialize",
                "initialized",
                "account/read",
                "account/rateLimits/read"
            ]
        );
        assert!(fake.sent[1].get("id").is_none());
    }

    #[test]
    fn session_handles_signed_out_api_key_remote_error_and_silence() {
        let limits = json!({});
        let mut f = Fake::new(json!({"account": null}), limits.clone());
        assert_eq!(
            run_session(&mut f, soon(), now()).state,
            UsageState::SignInRequired
        );

        let mut f = Fake::new(json!({"account": {"type": "apiKey"}}), limits.clone());
        let r = run_session(&mut f, soon(), now());
        assert_eq!(r.state, UsageState::Unavailable);
        assert!(
            f.sent
                .iter()
                .all(|m| m["method"] != "account/rateLimits/read")
        );

        let mut f = Fake::new(
            json!({"account": {"type": "chatgpt"}}),
            json!({"rateLimits": {}}),
        );
        let r = run_session(&mut f, soon(), now());
        assert_eq!((r.state, r.reading), (UsageState::Unavailable, None));

        let mut f = Fake::new(json!({"account": {"type": "chatgpt"}}), limits);
        f.fail_method = Some("account/rateLimits/read");
        let r = run_session(&mut f, soon(), now());
        assert_eq!(r.state, UsageState::Unavailable);
        assert!(!r.message.contains("secret"));

        let mut f = Fake::new(json!({}), json!({}));
        f.silent = true;
        assert_eq!(
            run_session(&mut f, soon(), now()).state,
            UsageState::Unavailable
        );
    }

    #[test]
    fn expired_deadline_times_out_without_reading() {
        let mut f = Fake::new(json!({}), json!({}));
        let r = run_session(&mut f, Instant::now(), now());
        assert_eq!(r.state, UsageState::Unavailable);
        assert!(r.message.contains("in time"));
    }

    #[test]
    fn missing_cli_and_profile_are_reported_without_spawning() {
        let dir = tempfile::tempdir().unwrap();
        let r = read_profile(dir.path(), Some(&dir.path().join("no-such-codex.exe")));
        assert_eq!(r.state, UsageState::CliMissing);
        // A real file as "exe" but a missing profile dir: sign in, still no process started.
        let exe = dir.path().join("codex.exe");
        std::fs::write(&exe, "").unwrap();
        let r = read_profile(&dir.path().join("missing-profile"), Some(&exe));
        assert_eq!(r.state, UsageState::SignInRequired);
    }

    #[test]
    fn executable_search_order_and_extensions() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let npm = home.join("AppData").join("Roaming").join("npm");
        let local = root.path().join("local");
        let v1 = local.join("OpenAI").join("Codex").join("bin").join("1.0.0");
        let v2 = local.join("OpenAI").join("Codex").join("bin").join("2.0.0");
        for d in [&npm, &v1, &v2] {
            std::fs::create_dir_all(d).unwrap();
        }
        let dirs = candidate_dirs(Some(home.clone()), Some(local.clone()), None);
        assert_eq!(find_in(&dirs, "codex", &[".exe", ".cmd"]), None);
        std::fs::write(v1.join("codex.exe"), "").unwrap();
        std::fs::write(v2.join("codex.exe"), "").unwrap();
        // Newest bundled version first.
        assert_eq!(
            find_in(&dirs, "codex", &[".exe", ".cmd"]),
            Some(v2.join("codex.exe"))
        );
        std::fs::write(npm.join("codex.cmd"), "").unwrap();
        // npm global comes before the bundled build.
        assert_eq!(
            find_in(&dirs, "codex", &[".exe", ".cmd"]),
            Some(npm.join("codex.cmd"))
        );
    }

    #[test]
    fn powershell_quote_escapes_single_quotes() {
        assert_eq!(ps_quote("a'b"), "'a''b'");
    }
}
