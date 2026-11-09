//! Warm-up (plan `docs/features/overview.md`, "Warm-up", owner 2026-11-07): at a chosen time on
//! chosen days SubHub sends a one-word prompt with the smallest model for each chosen account,
//! so its 5-hour window starts early and resets around the time the user usually runs out.
//!
//! Pure parts: the settings, which accounts are due ([`due`]), when a run is skipped ([`skip`]),
//! the command ([`plan`]) and the scheduled task XML ([`task_xml`]). Impure parts: [`run`]
//! (starts the CLI hidden, 60 s timeout), the run log file and the `schtasks` calls.
//!
//! The app and the headless `--warm-up` mode (started by the wake task) share the run log
//! `warmup-runs.json` next to the settings; whoever runs holds a named mutex, so a warm-up never
//! runs twice.
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration as StdDuration, Instant};

use chrono::{DateTime, Datelike, Duration, NaiveTime, Utc, Weekday};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::insights::{Zone, now_windows};
use crate::launch::{OVERRIDE_VARS, profile_env_var};
use crate::model::{Account, ProviderKind};
use crate::settings::Settings;

/// A missed run (SubHub not running at the time) still runs this long after the time.
pub const CATCH_UP_MIN: i64 = 60;
/// A failed run is tried once more after this long.
pub const RETRY_MIN: i64 = 5;
/// The CLI gets this long, then it is stopped.
pub const TIMEOUT_SECS: u64 = 60;
/// The account is read again this long after a run.
pub const REFRESH_AFTER_SECS: u64 = 30;
/// A weekly window used at least this much counts as used up.
pub const WEEKLY_USED_UP: f64 = 99.5;
/// Name of the Windows scheduled task that wakes the PC.
pub const TASK_NAME: &str = "AI SubHub warm-up";
/// The prompt. One word: the reply is a word too.
pub const PROMPT: &str = "ok";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WarmupSettings {
    pub enabled: bool,
    /// Local time of day.
    pub time: NaiveTime,
    pub days: Vec<Weekday>,
    pub accounts: Vec<Uuid>,
    /// A Windows scheduled task wakes a sleeping PC for it.
    pub wake_pc: bool,
}

impl Default for WarmupSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            time: NaiveTime::from_hms_opt(7, 0, 0).expect("valid"),
            days: WORKDAYS.to_vec(),
            accounts: vec![],
            wake_pc: false,
        }
    }
}

pub const WORKDAYS: [Weekday; 5] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
];

pub const WEEK: [Weekday; 7] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
    Weekday::Sat,
    Weekday::Sun,
];

impl WarmupSettings {
    pub fn runs_on(&self, d: Weekday) -> bool {
        self.days.contains(&d)
    }

    /// "weekdays", "every day", "Mo, We, Fr".
    pub fn days_text(&self) -> String {
        let on: Vec<Weekday> = WEEK.iter().copied().filter(|d| self.runs_on(*d)).collect();
        if on.len() == 7 {
            return "every day".into();
        }
        if on == WORKDAYS {
            return "weekdays".into();
        }
        if on.is_empty() {
            return "no days".into();
        }
        on.iter()
            .map(|d| short_day(*d))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub fn short_day(d: Weekday) -> &'static str {
    match d {
        Weekday::Mon => "Mo",
        Weekday::Tue => "Tu",
        Weekday::Wed => "We",
        Weekday::Thu => "Th",
        Weekday::Fri => "Fr",
        Weekday::Sat => "Sa",
        Weekday::Sun => "Su",
    }
}

/// Whether an account can be warmed up: it needs a local CLI profile (hub-only logins cannot
/// in v1).
pub fn can_warm(a: &Account) -> bool {
    // Copilot, Cursor and Antigravity have no 5-hour CLI window to start.
    a.profile_dir.is_some() && a.provider.is_cli_profile()
}

// ---------- the run log ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Outcome {
    /// The CLI answered: a window started.
    Started,
    /// A window of the account was already running.
    SkippedRunning,
    /// The weekly window is used up.
    SkippedWeekly,
    /// No CLI profile (hub-only login).
    SkippedNoProfile,
    Failed(String),
    /// Demo mode: nothing was started.
    Demo,
}

impl Outcome {
    pub fn failed(&self) -> bool {
        matches!(self, Outcome::Failed(_))
    }

    /// Short words for the UI.
    pub fn text(&self) -> String {
        match self {
            Outcome::Started => "window started".into(),
            Outcome::SkippedRunning => "skipped, a window was already running".into(),
            Outcome::SkippedWeekly => "skipped, the weekly limit is used up".into(),
            Outcome::SkippedNoProfile => "skipped, needs a CLI profile".into(),
            Outcome::Failed(m) => format!("failed: {m}"),
            Outcome::Demo => "Demo: nothing was started".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub at: DateTime<Utc>,
    pub outcome: Outcome,
    /// "Start a window now" (not the schedule).
    pub manual: bool,
    /// 1 for the first try at a scheduled time, 2 for the retry.
    pub attempt: u8,
}

/// The last run per account.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunLog {
    pub last: HashMap<Uuid, Run>,
}

pub fn log_path(settings_path: &Path) -> PathBuf {
    settings_path.with_file_name("warmup-runs.json")
}

/// A missing or unreadable log is empty.
pub fn load_log(path: &Path) -> RunLog {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Temp file + rename.
pub fn save_log(path: &Path, log: &RunLog) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(log).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(&tmp, path)
}

// ---------- what is due ----------

/// Today's scheduled instant, when today is a chosen day.
pub fn scheduled_today(
    cfg: &WarmupSettings,
    now: DateTime<Utc>,
    zone: &Zone,
) -> Option<DateTime<Utc>> {
    let today = zone.local(now).date();
    cfg.runs_on(today.weekday())
        .then(|| zone.utc_of(today.and_time(cfg.time)))
}

/// One scheduled account to run now, and which attempt it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Due {
    pub account: Uuid,
    pub attempt: u8,
}

/// Accounts due at `now`: on a chosen day, from the time until [`CATCH_UP_MIN`] later, each
/// chosen account (that still exists and has a profile) with no run since the time; or whose
/// first try since then failed at least [`RETRY_MIN`] ago (the one retry).
pub fn due(
    cfg: &WarmupSettings,
    accounts: &[Account],
    log: &RunLog,
    now: DateTime<Utc>,
    zone: &Zone,
) -> Vec<Due> {
    if !cfg.enabled {
        return vec![];
    }
    let Some(at) = scheduled_today(cfg, now, zone) else {
        return vec![];
    };
    if now < at || now >= at + Duration::minutes(CATCH_UP_MIN) {
        return vec![];
    }
    cfg.accounts
        .iter()
        .filter(|id| accounts.iter().any(|a| a.id == **id && can_warm(a)))
        .filter_map(|id| match log.last.get(id).filter(|r| r.at >= at) {
            None => Some(Due {
                account: *id,
                attempt: 1,
            }),
            Some(r)
                if r.outcome.failed()
                    && r.attempt < 2
                    && !r.manual
                    && now >= r.at + Duration::minutes(RETRY_MIN) =>
            {
                Some(Due {
                    account: *id,
                    attempt: 2,
                })
            }
            _ => None,
        })
        .collect()
}

/// Why a run is skipped, from the account's last reading: its 5-hour window is running (resets
/// in the future and some of it is used) or its weekly window is used up.
pub fn skip(settings: &Settings, account: &Account, now: DateTime<Utc>) -> Option<Outcome> {
    if !can_warm(account) {
        return Some(Outcome::SkippedNoProfile);
    }
    let (session, weekly) = now_windows(settings, account.id);
    if let Some(s) = session
        && s.resets_at.is_some_and(|r| r > now)
        && s.used > 0.0
    {
        return Some(Outcome::SkippedRunning);
    }
    if let Some(w) = weekly
        && w.resets_at.is_some_and(|r| r > now)
        && w.used >= WEEKLY_USED_UP
    {
        return Some(Outcome::SkippedWeekly);
    }
    None
}

// ---------- running the CLI ----------

/// The command line for one warm-up: the smallest model, one word.
pub fn args(provider: ProviderKind) -> Vec<&'static str> {
    match provider {
        ProviderKind::Claude => vec!["-p", PROMPT, "--model", "haiku"],
        ProviderKind::Codex => vec!["exec", "--skip-git-repo-check", PROMPT],
        // No warm-up for app logins (`can_warm`).
        _ => vec![],
    }
}

fn provider_name(p: ProviderKind) -> &'static str {
    p.name()
}

/// Runs the CLI hidden for the profile (billing / API-key variables cleared, as for Launch) in
/// `cwd`, stops it after `timeout`. Ok when it exits with 0; otherwise the last line it printed
/// on stderr (shortened) or the exit code.
pub fn run(
    provider: ProviderKind,
    exe: Option<&Path>,
    profile_dir: &Path,
    cwd: &Path,
    timeout: StdDuration,
) -> Result<(), String> {
    let Some(exe) = exe else {
        return Err(format!("{} CLI not found", provider_name(provider)));
    };
    let _ = std::fs::create_dir_all(cwd);
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args(provider))
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    for key in OVERRIDE_VARS {
        cmd.env_remove(key);
    }
    cmd.env(profile_env_var(provider), profile_dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not start the CLI ({e})"))?;
    // Read stderr on a thread so a chatty CLI never blocks on a full pipe.
    let reader = child.stderr.take().map(|mut err| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = err.read_to_string(&mut s);
            s
        })
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if started.elapsed() >= timeout => {
                kill_tree(&mut child);
                break None;
            }
            Ok(None) => std::thread::sleep(StdDuration::from_millis(250)),
            Err(e) => return Err(format!("could not wait for the CLI ({e})")),
        }
    };
    let stderr = reader.and_then(|h| h.join().ok()).unwrap_or_default();
    match status {
        None => Err(format!("no answer within {} s", timeout.as_secs())),
        Some(s) if s.success() => Ok(()),
        Some(s) => Err(last_line(&stderr)
            .unwrap_or_else(|| format!("the CLI ended with code {}", s.code().unwrap_or(-1)))),
    }
}

/// Stops the CLI and everything it started (`claude.cmd` runs node under cmd.exe; killing only
/// cmd.exe would leave node running and holding the output pipe).
fn kill_tree(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("taskkill.exe")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// The last non-empty line, at most 120 characters.
fn last_line(s: &str) -> Option<String> {
    let line = s.lines().map(str::trim).rfind(|l| !l.is_empty())?;
    let mut out: String = line.chars().take(120).collect();
    if line.chars().count() > 120 {
        out.push('…');
    }
    Some(out)
}

// ---------- the wake task ----------

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn day_tag(d: Weekday) -> &'static str {
    match d {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

/// Task Scheduler XML: weekly on the chosen days at the time (local, as the task's start
/// boundary has no zone), wakes the PC, runs `exe --warm-up` for the logged-on user without
/// elevation, on battery too, stopped after 10 minutes (a run, a retry 5 minutes later, a run). `start_date` is a date in the past or
/// today (the boundary's date part).
pub fn task_xml(exe: &Path, cfg: &WarmupSettings, start_date: chrono::NaiveDate) -> String {
    let days: String = WEEK
        .iter()
        .filter(|d| cfg.runs_on(**d))
        .map(|d| format!("<{0} />", day_tag(*d)))
        .collect();
    let boundary = start_date.and_time(cfg.time).format("%Y-%m-%dT%H:%M:%S");
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Starts the 5-hour windows of the chosen AI SubHub accounts. Created and removed by AI SubHub (Statistics, 5-hour windows, Warm-up).</Description>
  </RegistrationInfo>
  <Triggers>
    <CalendarTrigger>
      <StartBoundary>{boundary}</StartBoundary>
      <Enabled>true</Enabled>
      <ScheduleByWeek>
        <DaysOfWeek>{days}</DaysOfWeek>
        <WeeksInterval>1</WeeksInterval>
      </ScheduleByWeek>
    </CalendarTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <WakeToRun>true</WakeToRun>
    <ExecutionTimeLimit>PT10M</ExecutionTimeLimit>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <Arguments>--warm-up</Arguments>
    </Exec>
  </Actions>
</Task>
"#,
        exe = xml_escape(&exe.to_string_lossy()),
    )
}

/// Creates or replaces the wake task (`schtasks /Create /XML`, UTF-16 file as it expects).
#[cfg(windows)]
pub fn install_task(
    exe: &Path,
    cfg: &WarmupSettings,
    today: chrono::NaiveDate,
) -> Result<(), String> {
    install_named(TASK_NAME, exe, cfg, today)
}

#[cfg(windows)]
fn install_named(
    name: &str,
    exe: &Path,
    cfg: &WarmupSettings,
    today: chrono::NaiveDate,
) -> Result<(), String> {
    let xml = task_xml(exe, cfg, today);
    let path = std::env::temp_dir().join(format!("ai-subhub-warmup-{}.xml", std::process::id()));
    let mut bytes = vec![0xFF, 0xFE];
    for u in xml.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    let out = schtasks(&[
        "/Create",
        "/TN",
        name,
        "/XML",
        &path.to_string_lossy(),
        "/F",
    ]);
    let _ = std::fs::remove_file(&path);
    out
}

/// Deletes the wake task; a missing task is fine.
#[cfg(windows)]
pub fn remove_task() -> Result<(), String> {
    if !task_exists() {
        return Ok(());
    }
    schtasks(&["/Delete", "/TN", TASK_NAME, "/F"])
}

#[cfg(windows)]
pub fn task_exists() -> bool {
    schtasks(&["/Query", "/TN", TASK_NAME]).is_ok()
}

#[cfg(windows)]
fn schtasks(args: &[&str]) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("schtasks.exe")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        let msg = String::from_utf8_lossy(&out.stderr);
        Err(last_line(&msg).unwrap_or_else(|| "schtasks failed".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AllowanceWindow, Reading};

    fn t(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    fn account(id: u128, profile: bool) -> Account {
        Account {
            id: Uuid::from_u128(id),
            provider: ProviderKind::Claude,
            label: format!("A{id}"),
            email: None,
            profile_dir: profile.then(|| PathBuf::from(format!("C:\\p{id}"))),
            hub: None,
            group: None,
            hidden: false,
            hidden_windows: vec![],
            pinned_windows: vec![],
        }
    }

    fn cfg(ids: &[u128]) -> WarmupSettings {
        WarmupSettings {
            enabled: true,
            accounts: ids.iter().map(|i| Uuid::from_u128(*i)).collect(),
            ..Default::default()
        }
    }

    fn run(at: &str, outcome: Outcome, attempt: u8) -> Run {
        Run {
            at: t(at),
            outcome,
            manual: false,
            attempt,
        }
    }

    #[test]
    fn due_from_the_time_for_an_hour_on_chosen_days() {
        let accounts = [account(1, true), account(2, false)];
        let c = cfg(&[1, 2, 3]);
        let z = Zone::utc();
        let log = RunLog::default();
        // Mon Nov 9 2026, 06:59: not yet; 07:00: account 1 (2 is hub-only, 3 is gone).
        assert!(due(&c, &accounts, &log, t("2026-11-09T06:59:00Z"), &z).is_empty());
        let d = due(&c, &accounts, &log, t("2026-11-09T07:00:00Z"), &z);
        assert_eq!(
            d,
            vec![Due {
                account: Uuid::from_u128(1),
                attempt: 1
            }]
        );
        // Catch-up until 07:59, not at 08:00.
        assert_eq!(
            due(&c, &accounts, &log, t("2026-11-09T07:59:00Z"), &z).len(),
            1
        );
        assert!(due(&c, &accounts, &log, t("2026-11-09T08:00:00Z"), &z).is_empty());
        // Saturday is not chosen.
        assert!(due(&c, &accounts, &log, t("2026-11-14T07:10:00Z"), &z).is_empty());
        // Off: nothing.
        let off = WarmupSettings {
            enabled: false,
            ..c.clone()
        };
        assert!(due(&off, &accounts, &log, t("2026-11-09T07:10:00Z"), &z).is_empty());
    }

    #[test]
    fn a_run_since_the_time_counts_and_a_failure_is_retried_once() {
        let accounts = [account(1, true)];
        let c = cfg(&[1]);
        let z = Zone::utc();
        let id = Uuid::from_u128(1);
        let mut log = RunLog::default();
        // Yesterday's run does not count for today.
        log.last
            .insert(id, run("2026-11-08T07:00:00Z", Outcome::Started, 1));
        assert_eq!(
            due(&c, &accounts, &log, t("2026-11-09T07:00:00Z"), &z).len(),
            1
        );
        // Today's skip counts as done.
        log.last
            .insert(id, run("2026-11-09T07:00:00Z", Outcome::SkippedRunning, 1));
        assert!(due(&c, &accounts, &log, t("2026-11-09T07:20:00Z"), &z).is_empty());
        // A failure: retried 5 minutes later, once.
        log.last.insert(
            id,
            run("2026-11-09T07:00:00Z", Outcome::Failed("x".into()), 1),
        );
        assert!(due(&c, &accounts, &log, t("2026-11-09T07:04:00Z"), &z).is_empty());
        assert_eq!(
            due(&c, &accounts, &log, t("2026-11-09T07:05:00Z"), &z),
            vec![Due {
                account: id,
                attempt: 2
            }]
        );
        log.last.insert(
            id,
            run("2026-11-09T07:05:00Z", Outcome::Failed("x".into()), 2),
        );
        assert!(due(&c, &accounts, &log, t("2026-11-09T07:30:00Z"), &z).is_empty());
    }

    #[test]
    fn dst_day_keeps_the_wall_time() {
        // Spring forward on Sun Mar 28 2027 (+1 -> +2); Mon Mar 29 07:00 local = 05:00 UTC.
        let z = crate::insights::Zone::Changes(vec![
            (
                t("2027-01-01T00:00:00Z"),
                chrono::FixedOffset::east_opt(3600).unwrap(),
            ),
            (
                t("2027-03-28T01:00:00Z"),
                chrono::FixedOffset::east_opt(7200).unwrap(),
            ),
        ]);
        let c = cfg(&[1]);
        let accounts = [account(1, true)];
        let log = RunLog::default();
        assert!(due(&c, &accounts, &log, t("2027-03-29T04:59:00Z"), &z).is_empty());
        assert_eq!(
            due(&c, &accounts, &log, t("2027-03-29T05:00:00Z"), &z).len(),
            1
        );
        // Friday before (+1): 07:00 local = 06:00 UTC.
        assert!(due(&c, &accounts, &log, t("2027-03-26T05:30:00Z"), &z).is_empty());
        assert_eq!(
            due(&c, &accounts, &log, t("2027-03-26T06:00:00Z"), &z).len(),
            1
        );
    }

    fn reading(session: Option<(f64, &str)>, weekly: Option<(f64, &str)>, at: &str) -> Reading {
        let mut windows = vec![];
        let mut push = |key: &str, label: &str, w: Option<(f64, &str)>| {
            if let Some((used, reset)) = w {
                windows.push(AllowanceWindow {
                    key: key.into(),
                    label: label.into(),
                    used_percent: used,
                    resets_at: Some(t(reset)),
                });
            }
        };
        push("five_hour", "5-hour", session);
        push("seven_day", "Weekly", weekly);
        Reading {
            fetched_at: t(at),
            windows,
        }
    }

    #[test]
    fn skips_a_running_window_and_a_used_up_week() {
        let a = account(1, true);
        let now = t("2026-11-09T07:00:00Z");
        let mut s = Settings {
            accounts: vec![a.clone()],
            ..Default::default()
        };
        // No reading: run.
        assert_eq!(skip(&s, &a, now), None);
        // A window that resets at 09:00 with 12 % used: running.
        s.readings.insert(
            a.id,
            reading(
                Some((12.0, "2026-11-09T09:00:00Z")),
                Some((40.0, "2026-11-12T00:00:00Z")),
                "2026-11-09T06:50:00Z",
            ),
        );
        assert_eq!(skip(&s, &a, now), Some(Outcome::SkippedRunning));
        // Its reset passed: run.
        s.readings.insert(
            a.id,
            reading(
                Some((80.0, "2026-11-09T06:30:00Z")),
                Some((40.0, "2026-11-12T00:00:00Z")),
                "2026-11-09T06:00:00Z",
            ),
        );
        assert_eq!(skip(&s, &a, now), None);
        // Weekly used up.
        s.readings.insert(
            a.id,
            reading(
                None,
                Some((100.0, "2026-11-12T00:00:00Z")),
                "2026-11-09T06:00:00Z",
            ),
        );
        assert_eq!(skip(&s, &a, now), Some(Outcome::SkippedWeekly));
        // Hub-only.
        assert_eq!(
            skip(&s, &account(2, false), now),
            Some(Outcome::SkippedNoProfile)
        );
    }

    #[test]
    fn commands_use_the_smallest_model_and_one_word() {
        assert_eq!(
            args(ProviderKind::Claude),
            vec!["-p", "ok", "--model", "haiku"]
        );
        assert_eq!(
            args(ProviderKind::Codex),
            vec!["exec", "--skip-git-repo-check", "ok"]
        );
    }

    #[test]
    fn task_xml_wakes_on_the_chosen_days() {
        let c = WarmupSettings {
            enabled: true,
            time: NaiveTime::from_hms_opt(6, 45, 0).unwrap(),
            days: vec![Weekday::Mon, Weekday::Wed],
            accounts: vec![],
            wake_pc: true,
        };
        let x = task_xml(
            Path::new(r"C:\Apps\AI SubHub & co\ai-subhub.exe"),
            &c,
            "2026-11-08".parse().unwrap(),
        );
        assert!(x.contains("<StartBoundary>2026-11-08T06:45:00</StartBoundary>"));
        assert!(x.contains("<DaysOfWeek><Monday /><Wednesday /></DaysOfWeek>"));
        assert!(x.contains("<WakeToRun>true</WakeToRun>"));
        assert!(x.contains("<Command>C:\\Apps\\AI SubHub &amp; co\\ai-subhub.exe</Command>"));
        assert!(x.contains("<Arguments>--warm-up</Arguments>"));
    }

    #[test]
    fn days_read_short() {
        let mut c = WarmupSettings::default();
        assert_eq!(c.days_text(), "weekdays");
        c.days = WEEK.to_vec();
        assert_eq!(c.days_text(), "every day");
        c.days = vec![Weekday::Fri, Weekday::Mon];
        assert_eq!(c.days_text(), "Mo, Fr");
    }

    #[test]
    fn log_round_trips_and_missing_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let p = log_path(&dir.path().join("settings.json"));
        assert_eq!(load_log(&p), RunLog::default());
        let mut log = RunLog::default();
        log.last.insert(
            Uuid::from_u128(1),
            run(
                "2026-11-09T07:00:00Z",
                Outcome::Failed("no answer".into()),
                1,
            ),
        );
        save_log(&p, &log).unwrap();
        assert_eq!(load_log(&p), log);
    }

    #[test]
    fn last_line_is_short() {
        assert_eq!(last_line("a\n\n  b  \n\n"), Some("b".into()));
        assert_eq!(last_line(" \n"), None);
        assert_eq!(last_line(&"x".repeat(130)).unwrap().chars().count(), 121);
    }

    #[cfg(windows)]
    #[test]
    fn run_passes_the_profile_and_reports_errors_and_timeouts() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.txt");
        let ok = dir.path().join("ok.cmd");
        std::fs::write(
            &ok,
            format!(
                "@echo off
echo %CLAUDE_CONFIG_DIR%^|%ANTHROPIC_API_KEY%^|%* > \"{}\"
exit /b 0
",
                out.display()
            ),
        )
        .unwrap();
        let bad = dir.path().join("bad.cmd");
        std::fs::write(
            &bad,
            "@echo off
echo first 1>&2
echo Rate limited 1>&2
exit /b 3
",
        )
        .unwrap();
        let slow = dir.path().join("slow.cmd");
        std::fs::write(
            &slow,
            "@echo off
ping -n 6 127.0.0.1 >nul
",
        )
        .unwrap();
        let profile = dir.path().join("prof");
        let cwd = dir.path().join("work");
        let secs = |s| StdDuration::from_secs(s);

        // SAFETY of the test: the variable is set only in this test process.
        unsafe { std::env::set_var("ANTHROPIC_API_KEY", "should-be-cleared") };
        super::run(ProviderKind::Claude, Some(&ok), &profile, &cwd, secs(20)).unwrap();
        let written = std::fs::read_to_string(&out).unwrap();
        assert!(
            written.contains(&format!("{}|", profile.display())),
            "{written}"
        );
        assert!(!written.contains("should-be-cleared"), "{written}");
        assert!(written.contains("-p ok --model haiku"), "{written}");
        assert!(cwd.is_dir());

        let e = super::run(ProviderKind::Claude, Some(&bad), &profile, &cwd, secs(20)).unwrap_err();
        assert_eq!(e, "Rate limited");
        let e = super::run(ProviderKind::Claude, Some(&slow), &profile, &cwd, secs(1)).unwrap_err();
        assert_eq!(e, "no answer within 1 s");
        let e = super::run(ProviderKind::Codex, None, &profile, &cwd, secs(1)).unwrap_err();
        assert_eq!(e, "Codex CLI not found");
    }

    /// Registers the task under a test name, reads it back and deletes it. Touches the PC's
    /// Task Scheduler, so only on request: `cargo test -p subhub-core -- --ignored task_registers`.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn task_registers_with_task_scheduler() {
        let name = "AI SubHub warm-up (test, safe to delete)";
        let c = WarmupSettings {
            enabled: true,
            wake_pc: true,
            ..Default::default()
        };
        install_named(
            name,
            Path::new(r"C:WindowsSystem32cmd.exe"),
            &c,
            "2026-11-08".parse().unwrap(),
        )
        .unwrap();
        let q = std::process::Command::new("schtasks.exe")
            .args(["/Query", "/TN", name, "/XML"])
            .output()
            .unwrap();
        let _ = schtasks(&["/Delete", "/TN", name, "/F"]);
        let xml = String::from_utf8_lossy(&q.stdout);
        assert!(q.status.success(), "{}", String::from_utf8_lossy(&q.stderr));
        assert!(xml.contains("<WakeToRun>true</WakeToRun>"), "{xml}");
        assert!(xml.contains("<Arguments>--warm-up</Arguments>"), "{xml}");
    }
}
