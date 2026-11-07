//! Demo mode fixtures (`ai-subhub --demo`, owner 2026-11-07): made-up accounts, readings, reading
//! history, CLI profile metadata and session logs, so screenshots and UI tests run the real app on
//! data that belongs to nobody. Everything is deterministic for a given `now`; [`seed`] writes it
//! into the sandbox folder (`%TEMP%\AI SubHub Demo`), which [`reset`] wipes first.
//! Emails are `@example.com` only. No token or credential file is ever written: Claude profiles
//! get a `.claude.json` with just the login email, Codex profiles only session logs.
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, NaiveDate, SecondsFormat, Utc};
use uuid::Uuid;

use crate::billing::MonthlyBilling;
use crate::history::HistoryLine;
use crate::model::{
    Account, AllowanceWindow, HubLink, ProviderKind, ReadSource, Reading, UsageResult, UsageState,
};
use crate::projects::Project;
use crate::settings::{self, Settings};

/// Name of the sandbox folder in the temp folder. [`safe_to_wipe`] insists on it.
pub const SANDBOX_NAME: &str = "AI SubHub Demo";

pub const CLAUDE_WORK: Uuid = Uuid::from_u128(0x00000000_0000_4000_8000_0000000de001);
pub const CLAUDE_PERSONAL: Uuid = Uuid::from_u128(0x00000000_0000_4000_8000_0000000de002);
pub const CLAUDE_TEAM: Uuid = Uuid::from_u128(0x00000000_0000_4000_8000_0000000de003);
pub const CLAUDE_SIDE: Uuid = Uuid::from_u128(0x00000000_0000_4000_8000_0000000de004);
pub const CODEX_WORK: Uuid = Uuid::from_u128(0x00000000_0000_4000_8000_0000000de005);
pub const CODEX_PERSONAL: Uuid = Uuid::from_u128(0x00000000_0000_4000_8000_0000000de006);

/// The message the stale account's checks fail with.
pub const STALE_MESSAGE: &str = "The provider did not answer. Showing the last reading.";

/// One allowance window of a fixture: key, label, % used, minutes from `now` to its reset.
type WindowSpec = (&'static str, &'static str, f64, i64);

struct Spec {
    id: Uuid,
    provider: ProviderKind,
    label: &'static str,
    email: &'static str,
    /// Profile folder name in the sandbox home.
    profile: Option<&'static str>,
    /// Hub auth file name and whether the login is benched.
    hub: Option<(&'static str, bool)>,
    group: &'static str,
    tag: &'static str,
    plan: &'static str,
    windows: &'static [WindowSpec],
    /// How old the seeded reading is.
    fetched_mins_ago: i64,
    /// Every check fails (the stale account).
    failing: bool,
}

/// The interesting states: fresh (Work), running out (Personal, 19% left), used up (Team, 5-hour
/// at 0%), benched hub login (Side project), hub-only (Team), local-only (Personal, Codex
/// Personal) and stale (Codex Personal, 3 h old reading).
const SPECS: &[Spec] = &[
    Spec {
        id: CLAUDE_WORK,
        provider: ProviderKind::Claude,
        label: "Work",
        email: "work@example.com",
        profile: Some(".claude-work"),
        hub: Some(("claude-work@example.com.json", false)),
        group: "Work",
        tag: "WRK",
        plan: "Max 20x",
        windows: &[
            ("five_hour", "Session", 38.0, 133),
            ("seven_day", "Weekly", 46.0, 4560),
            ("seven_day_opus", "Weekly · Opus", 22.0, 4560),
        ],
        fetched_mins_ago: 2,
        failing: false,
    },
    Spec {
        id: CLAUDE_PERSONAL,
        provider: ProviderKind::Claude,
        label: "Personal",
        email: "personal@example.com",
        profile: Some(".claude-personal"),
        hub: None,
        group: "Personal",
        tag: "PER",
        plan: "Pro",
        windows: &[
            ("five_hour", "Session", 81.0, 65),
            ("seven_day", "Weekly", 58.0, 7320),
        ],
        fetched_mins_ago: 4,
        failing: false,
    },
    Spec {
        id: CLAUDE_TEAM,
        provider: ProviderKind::Claude,
        label: "Team",
        email: "team@example.com",
        profile: None,
        hub: Some(("claude-team@example.com.json", false)),
        group: "Team",
        tag: "TM",
        plan: "Max 5x",
        windows: &[
            ("five_hour", "Session", 100.0, 47),
            ("seven_day", "Weekly", 71.0, 1980),
        ],
        fetched_mins_ago: 3,
        failing: false,
    },
    Spec {
        id: CLAUDE_SIDE,
        provider: ProviderKind::Claude,
        label: "Side project",
        email: "side@example.com",
        profile: None,
        hub: Some(("claude-side@example.com.json", true)),
        group: "Side project",
        tag: "SP",
        plan: "Pro",
        windows: &[
            ("five_hour", "Session", 12.0, 250),
            ("seven_day", "Weekly", 30.0, 8100),
        ],
        fetched_mins_ago: 5,
        failing: false,
    },
    Spec {
        id: CODEX_WORK,
        provider: ProviderKind::Codex,
        label: "Work",
        email: "dev@example.com",
        profile: Some(".codex"),
        hub: Some(("codex-dev@example.com.json", false)),
        group: "Work",
        tag: "CXW",
        plan: "Pro",
        windows: &[
            ("codex:primary", "5-hour", 22.0, 170),
            ("codex:secondary", "Weekly", 35.0, 6000),
        ],
        fetched_mins_ago: 2,
        failing: false,
    },
    Spec {
        id: CODEX_PERSONAL,
        provider: ProviderKind::Codex,
        label: "Personal",
        email: "me@example.com",
        profile: Some(".codex-personal"),
        hub: None,
        group: "Personal",
        tag: "CXP",
        plan: "Plus",
        windows: &[
            ("codex:primary", "5-hour", 64.0, 95),
            ("codex:secondary", "Weekly", 41.0, 3000),
        ],
        fetched_mins_ago: 185,
        failing: true,
    },
];

/// Saved project folders (in the sandbox home's `Projects` folder): id, folder, label.
const PROJECTS: &[(u128, &str, &str)] = &[
    (
        0x00000000_0000_4000_8000_0000000de101,
        "acme-web",
        "Acme web app",
    ),
    (
        0x00000000_0000_4000_8000_0000000de102,
        "api-gateway",
        "API gateway",
    ),
    (
        0x00000000_0000_4000_8000_0000000de103,
        "docs-site",
        "Docs site",
    ),
];

/// Claude models in the logs (all in the built-in price table) and their share of sessions.
const CLAUDE_MODELS: &[(&str, u64)] = &[
    ("claude-opus-5-5", 60),
    ("claude-sonnet-5-5", 30),
    ("claude-haiku-5-5", 10),
];
const CODEX_MODEL: &str = "gpt-6.1-sol";

/// Days of session logs and of reading history.
pub const LOG_DAYS: i64 = 60;
pub const HISTORY_DAYS: i64 = 14;

fn spec(id: Uuid) -> Option<&'static Spec> {
    SPECS.iter().find(|s| s.id == id)
}

/// The plan badge of a demo account.
pub fn plan(id: Uuid) -> Option<&'static str> {
    spec(id).map(|s| s.plan)
}

/// `%TEMP%\AI SubHub Demo`.
pub fn sandbox_dir(temp: &Path) -> PathBuf {
    temp.join(SANDBOX_NAME)
}

/// Only the demo sandbox may be wiped: named exactly [`SANDBOX_NAME`], directly in `temp`, and
/// not a link (deleting through a junction would reach elsewhere).
pub fn safe_to_wipe(dir: &Path, temp: &Path) -> bool {
    let named = dir.file_name().is_some_and(|n| n == SANDBOX_NAME);
    let placed = dir.parent().is_some_and(|p| p == temp) && !temp.as_os_str().is_empty();
    let link = std::fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink());
    named && placed && !link
}

/// Deletes the sandbox (only if [`safe_to_wipe`]) and creates it empty with `home` and `local`.
pub fn reset(dir: &Path, temp: &Path) -> anyhow::Result<()> {
    if !safe_to_wipe(dir, temp) {
        anyhow::bail!("refusing to wipe {}: not the demo sandbox", dir.display());
    }
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    std::fs::create_dir_all(dir.join("home"))?;
    std::fs::create_dir_all(dir.join("local"))?;
    Ok(())
}

/// A sandbox path as a screenshot should show it: `C:\Users\demo\…` instead of the temp folder
/// (which carries the real user name). None when `path` is not in the sandbox `root`.
pub fn display_path(path: &Path, root: &Path) -> Option<String> {
    let roots = [
        root.to_path_buf(),
        crate::profile::plain(root.to_path_buf()),
    ];
    for r in roots.iter().chain(
        std::fs::canonicalize(root)
            .ok()
            .map(crate::profile::plain)
            .iter(),
    ) {
        for (sub, shown) in [
            ("home", r"C:\Users\demo"),
            ("local", r"C:\Users\demo\AppData\Local"),
        ] {
            if let Ok(rest) = path.strip_prefix(r.join(sub)) {
                let rest = rest.to_string_lossy();
                return Some(if rest.is_empty() {
                    shown.to_string()
                } else {
                    format!("{shown}\\{rest}")
                });
            }
        }
    }
    None
}

fn window_len(key: &str) -> Duration {
    if key == "five_hour" || key == "codex:primary" {
        Duration::hours(5)
    } else {
        Duration::days(7)
    }
}

fn spec_windows(s: &Spec, now: DateTime<Utc>) -> Vec<AllowanceWindow> {
    s.windows
        .iter()
        .map(|&(key, label, used, mins)| AllowanceWindow {
            key: key.into(),
            label: label.into(),
            used_percent: used,
            resets_at: Some(now + Duration::minutes(mins)),
        })
        .collect()
}

/// The demo settings: accounts, readings, tags, groups, steering, a renewal, projects. Paths
/// are under `home` (the sandbox's user folder); `version` is recorded as seen, so no patch
/// notes show.
pub fn settings(home: &Path, now: DateTime<Utc>, version: &str) -> Settings {
    let mut s = Settings {
        refresh_minutes: 15,
        groups: ["Work", "Personal", "Side project", "Team"]
            .map(String::from)
            .to_vec(),
        ..Settings::default()
    };
    for (i, sp) in SPECS.iter().enumerate() {
        s.accounts.push(Account {
            id: sp.id,
            provider: sp.provider,
            label: sp.label.into(),
            email: Some(sp.email.into()),
            profile_dir: sp.profile.map(|p| home.join(p)),
            hub: sp.hub.map(|(file, benched)| HubLink {
                auth_index: serde_json::json!(i + 1),
                file_name: file.into(),
                benched,
                account_id: None,
                priority: (sp.id == CLAUDE_WORK).then_some(100),
            }),
            group: Some(sp.group.into()),
            hidden: false,
            hidden_windows: vec![],
            pinned_windows: vec![],
        });
        s.readings.insert(
            sp.id,
            Reading {
                fetched_at: now - Duration::minutes(sp.fetched_mins_ago),
                windows: spec_windows(sp, now),
            },
        );
        s.account_tags.insert(sp.id, sp.tag.into());
    }
    s.display.taskbar_favorites = vec![CLAUDE_WORK];
    // By the widgets, so it never sits on the installed app's display next to the tray (owner,
    // 2026-11-07: the two were drawn over each other).
    s.display.taskbar_position = settings::TaskbarPosition::Left;
    s.hub.enabled = true;
    s.steering.enabled = true;
    s.steering.current = Some(CLAUDE_WORK);
    s.steering.original_priorities =
        HashMap::from([("claude-work@example.com.json".to_string(), None)]);
    let today: NaiveDate = now.date_naive();
    s.billing
        .insert(CLAUDE_WORK, MonthlyBilling::new(today + Duration::days(4)));
    s.projects = PROJECTS
        .iter()
        .map(|&(id, dir, label)| Project {
            id: Uuid::from_u128(id),
            label: label.into(),
            directory: home.join("Projects").join(dir),
            hidden: false,
        })
        .collect();
    s.updates.last_seen_version = version.into();
    s
}

/// What a refresh of a demo account returns instead of asking a provider or the hub: the last
/// reading re-stamped as fresh, resets that passed rolled on by one window (used % unchanged, so
/// the used-up account stays used up). The stale account always fails, so it stays stale.
pub fn refresh_result(
    account: &Account,
    previous: Option<&Reading>,
    now: DateTime<Utc>,
) -> UsageResult {
    let sp = spec(account.id);
    let source = if account.hub.is_some() {
        ReadSource::Hub
    } else {
        ReadSource::Profile
    };
    let fail = |message: &str| UsageResult {
        state: UsageState::Unavailable,
        message: message.into(),
        reading: None,
        retry_after_secs: None,
        email: None,
        plan: None,
        source,
    };
    if sp.is_some_and(|s| s.failing) {
        return fail(STALE_MESSAGE);
    }
    let Some(mut reading) = previous.cloned().or_else(|| {
        sp.map(|s| Reading {
            fetched_at: now,
            windows: spec_windows(s, now),
        })
    }) else {
        return fail("Demo mode has no usage for this account.");
    };
    reading.fetched_at = now;
    for w in &mut reading.windows {
        let len = window_len(&w.key);
        if let Some(at) = w.resets_at.as_mut() {
            while *at <= now {
                *at += len;
            }
        }
    }
    UsageResult {
        state: UsageState::Ready,
        message: String::new(),
        reading: Some(reading),
        retry_after_secs: None,
        email: account.email.clone(),
        plan: sp.map(|s| s.plan.to_string()),
        source,
    }
}

// ---------- generated files ----------

/// splitmix64: small, stable, good enough for plausible noise.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0x9e37_79b9_7f4a_7c15)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    /// In `lo..=hi`.
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo + 1)
    }
    fn uuid(&mut self) -> Uuid {
        Uuid::from_u128(((self.next() as u128) << 64) | self.next() as u128)
    }
}

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Fewer sessions on weekends and further back (usage grew over the two months).
fn day_factor(now: DateTime<Utc>, day: i64) -> f64 {
    use chrono::Datelike;
    let date = (now - Duration::days(day)).date_naive();
    let weekend = matches!(date.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun);
    let trend = 0.55 + 0.45 * (1.0 - day as f64 / LOG_DAYS as f64);
    if weekend { trend * 0.35 } else { trend }
}

/// Login metadata of the fake CLI profiles (relative to the sandbox home): Claude's
/// `.claude.json` with the email only. No credentials, no tokens.
pub fn profile_files() -> Vec<(PathBuf, String)> {
    SPECS
        .iter()
        .filter(|s| s.provider == ProviderKind::Claude)
        .filter_map(|s| {
            let dir = s.profile?;
            let body = serde_json::json!({
                "numStartups": 42,
                "oauthAccount": { "emailAddress": s.email },
            });
            Some((Path::new(dir).join(".claude.json"), body.to_string()))
        })
        .collect()
}

/// Synthetic Claude Code (`<profile>\projects\<project>\<session>.jsonl`) and Codex
/// (`<profile>\sessions\YYYY\MM\DD\rollout-….jsonl`) logs over [`LOG_DAYS`], relative to the
/// sandbox home, in the exact shape `stats::parse` reads. Usage fields only; no message text.
pub fn session_logs(now: DateTime<Utc>) -> Vec<(PathBuf, String)> {
    let mut out = vec![];
    for day in 0..LOG_DAYS {
        let factor = day_factor(now, day);
        let mut rng = Rng::new(day as u64);
        // Claude Code.
        let sessions = ((rng.range(3, 6) as f64) * factor).round().max(1.0) as u64;
        for _ in 0..sessions {
            let profile = if rng.range(0, 99) < 70 {
                ".claude-work"
            } else {
                ".claude-personal"
            };
            let project = PROJECTS[rng.range(0, PROJECTS.len() as u64 - 1) as usize].1;
            let pick = rng.range(0, 99);
            let mut acc = 0;
            let model = CLAUDE_MODELS
                .iter()
                .find(|(_, share)| {
                    acc += share;
                    pick < acc
                })
                .map_or(CLAUDE_MODELS[0].0, |m| m.0);
            let session = rng.uuid();
            let mut t =
                now - Duration::days(day) - Duration::minutes(rng.range(30, 14 * 60) as i64);
            let msgs = rng.range(15, 60);
            let mut text = String::new();
            for i in 0..msgs {
                let first = i == 0;
                let input = rng.range(1, 40);
                let cache_write = if first {
                    rng.range(15_000, 30_000)
                } else {
                    rng.range(300, 6_000)
                };
                let cache_read = if first {
                    0
                } else {
                    18_000 + i * 2_400 + rng.range(0, 4_000)
                };
                let output = rng.range(80, 2_500);
                let _ = writeln!(
                    text,
                    r#"{{"type":"assistant","timestamp":"{}","sessionId":"{session}","requestId":"req_demo_{:016x}","message":{{"id":"msg_demo_{:016x}","model":"{model}","usage":{{"input_tokens":{input},"output_tokens":{output},"cache_creation_input_tokens":{cache_write},"cache_read_input_tokens":{cache_read}}}}}}}"#,
                    ts(t),
                    rng.next(),
                    rng.next(),
                );
                t += Duration::seconds(rng.range(40, 180) as i64);
            }
            let encoded = format!("C--Users-demo-Projects-{project}");
            out.push((
                Path::new(profile)
                    .join("projects")
                    .join(encoded)
                    .join(format!("{session}.jsonl")),
                text,
            ));
        }
        // Codex.
        let sessions = ((rng.range(1, 3) as f64) * factor).round() as u64;
        for _ in 0..sessions {
            let profile = if rng.range(0, 99) < 75 {
                ".codex"
            } else {
                ".codex-personal"
            };
            let session = rng.uuid();
            let start =
                now - Duration::days(day) - Duration::minutes(rng.range(20, 12 * 60) as i64);
            let mut t = start;
            let mut text = String::new();
            let _ = writeln!(
                text,
                r#"{{"timestamp":"{}","type":"turn_context","payload":{{"model":"{CODEX_MODEL}","cwd":"C:\\Users\\demo\\Projects"}}}}"#,
                ts(t)
            );
            let (mut ti, mut tc, mut to) = (0u64, 0u64, 0u64);
            for turn in 0..rng.range(8, 30) {
                let input = 20_000 + turn * 3_500 + rng.range(0, 8_000);
                let cached = input * rng.range(85, 92) / 100;
                let output = rng.range(200, 3_000);
                ti += input;
                tc += cached;
                to += output;
                t += Duration::seconds(rng.range(30, 150) as i64);
                let _ = writeln!(
                    text,
                    r#"{{"timestamp":"{}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{ti},"cached_input_tokens":{tc},"output_tokens":{to}}},"last_token_usage":{{"input_tokens":{input},"cached_input_tokens":{cached},"output_tokens":{output}}}}}}}}}"#,
                    ts(t)
                );
            }
            let name = format!(
                "rollout-{}-{session}.jsonl",
                start.format("%Y-%m-%dT%H-%M-%S")
            );
            out.push((
                Path::new(profile)
                    .join("sessions")
                    .join(start.format("%Y").to_string())
                    .join(start.format("%m").to_string())
                    .join(start.format("%d").to_string())
                    .join(name),
                text,
            ));
        }
    }
    out
}

/// Hourly readings over [`HISTORY_DAYS`] that end at each account's seeded reading: % used climbs
/// within each window cycle and drops at its reset. Oldest first.
pub fn history_lines(now: DateTime<Utc>) -> Vec<HistoryLine> {
    let mut out = vec![];
    let start = now - Duration::days(HISTORY_DAYS);
    let mut at = start;
    let mut rng = Rng::new(0x5eed);
    while at <= now {
        for sp in SPECS {
            let last = now - Duration::minutes(sp.fetched_mins_ago);
            if at > last {
                continue;
            }
            for &(key, _, used_now, mins) in sp.windows {
                let len = window_len(key);
                let reset_now = now + Duration::minutes(mins);
                // Time since the window's last reset, now and at `at`.
                let cycle = |t: DateTime<Utc>| {
                    let behind = (reset_now - t).num_seconds();
                    let n = behind.div_euclid(len.num_seconds());
                    let reset = reset_now - Duration::seconds(n * len.num_seconds());
                    (
                        reset,
                        n,
                        1.0 - (reset - t).num_seconds() as f64 / len.num_seconds() as f64,
                    )
                };
                let (_, _, frac_now) = cycle(last);
                let (reset, n, frac) = cycle(at);
                let scale = if n == 0 {
                    1.0
                } else {
                    0.6 + rng.range(0, 45) as f64 / 100.0
                };
                let used = (used_now * frac / frac_now.max(0.05) * scale)
                    .clamp(0.0, 100.0)
                    .round();
                out.push(HistoryLine {
                    account: sp.id,
                    fetched_at: at,
                    window: key.into(),
                    used_percent: used,
                    resets_at: Some(reset),
                });
            }
        }
        at += Duration::hours(1);
    }
    // The seeded reading itself closes each account's history.
    for sp in SPECS {
        let last = now - Duration::minutes(sp.fetched_mins_ago);
        for w in spec_windows(sp, now) {
            out.push(HistoryLine {
                account: sp.id,
                fetched_at: last,
                window: w.key,
                used_percent: w.used_percent,
                resets_at: w.resets_at,
            });
        }
    }
    out
}

/// Writes the whole demo into the sandbox `root` (as left by [`reset`]): profile metadata, logs
/// and project folders under `root\home`, settings and reading history under
/// `root\local\AI SubHub`. Returns the settings written.
pub fn seed(root: &Path, now: DateTime<Utc>, version: &str) -> anyhow::Result<Settings> {
    let home = root.join("home");
    let write = |rel: &Path, body: &str| -> anyhow::Result<()> {
        let path = home.join(rel);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, body)?;
        Ok(())
    };
    for sp in SPECS {
        if let Some(p) = sp.profile {
            std::fs::create_dir_all(home.join(p))?;
        }
    }
    for (rel, body) in profile_files().iter().chain(session_logs(now).iter()) {
        write(rel, body)?;
    }
    for (_, dir, _) in PROJECTS {
        std::fs::create_dir_all(home.join("Projects").join(dir))?;
    }
    // Stored folders are canonical, like the ones profile detection compares against.
    let home = crate::profile::plain(std::fs::canonicalize(&home)?);
    let s = settings(&home, now, version);
    let data = root.join("local").join("AI SubHub");
    settings::save(&data.join("settings.json"), &s)?;
    let mut text = String::new();
    for line in history_lines(now) {
        text.push_str(&serde_json::to_string(&line)?);
        text.push('\n');
    }
    std::fs::write(data.join("readings-history.jsonl"), text)?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::{self, DetectStatus};
    use crate::profile::ProfileEnv;
    use crate::stats::{self, PriceBook, Range};

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-07T15:30:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn left(s: &Settings, id: Uuid, key: &str) -> f64 {
        s.readings[&id]
            .windows
            .iter()
            .find(|w| w.key == key)
            .unwrap()
            .remaining_percent()
    }

    #[test]
    fn settings_show_the_interesting_states() {
        let s = settings(Path::new(r"C:\demo\home"), now(), "0.1.2");
        let claude = s
            .accounts
            .iter()
            .filter(|a| a.provider == ProviderKind::Claude)
            .count();
        assert_eq!((claude, s.accounts.len() - claude), (4, 2));
        assert!(
            s.accounts
                .iter()
                .all(|a| a.email.as_deref().unwrap().ends_with("@example.com"))
        );
        // Running out, used up, benched, hub-only, local-only, stale.
        assert!(left(&s, CLAUDE_PERSONAL, "five_hour") < 25.0);
        assert_eq!(left(&s, CLAUDE_TEAM, "five_hour"), 0.0);
        let acc = |id| s.accounts.iter().find(|a| a.id == id).unwrap();
        assert!(acc(CLAUDE_SIDE).hub.as_ref().unwrap().benched);
        assert!(acc(CLAUDE_TEAM).profile_dir.is_none() && acc(CLAUDE_TEAM).hub.is_some());
        assert!(acc(CLAUDE_PERSONAL).hub.is_none() && acc(CLAUDE_PERSONAL).profile_dir.is_some());
        assert!(now() - s.readings[&CODEX_PERSONAL].fetched_at > Duration::hours(2));
        // Favourite, hub + Auto steering with a target, renewal ahead, projects, no patch notes.
        assert_eq!(s.display.taskbar_favorites, vec![CLAUDE_WORK]);
        assert_eq!(s.display.taskbar_position, settings::TaskbarPosition::Left);
        assert!(s.hub.enabled && s.steering.enabled && s.steering.preferred.is_none());
        assert_eq!(s.steering.current, Some(CLAUDE_WORK));
        let days = crate::billing::days_until(&s.billing[&CLAUDE_WORK], now().date_naive());
        assert!((1..7).contains(&days));
        assert_eq!(s.projects.len(), 3);
        assert_eq!(s.refresh_minutes, 15);
        assert_eq!(s.updates.last_seen_version, "0.1.2");
        assert_eq!(plan(CLAUDE_WORK), Some("Max 20x"));
    }

    #[test]
    fn steering_auto_keeps_the_seeded_target() {
        let s = settings(Path::new(r"C:\demo\home"), now(), "0.1.2");
        let stale = |id| id == CODEX_PERSONAL;
        let target = crate::steer::target(&s, &s.accounts, &s.readings, &stale, now());
        assert_eq!(target, Some(CLAUDE_WORK));
        let (changes, _) = crate::steer::plan(&s.accounts, &s.steering, target);
        assert!(changes.is_empty(), "{changes:?}");
    }

    #[test]
    fn refresh_restamps_rolls_resets_and_keeps_the_stale_one_failing() {
        let s = settings(Path::new(r"C:\demo\home"), now(), "0.1.2");
        let later = now() + Duration::hours(6);
        let team = s.accounts.iter().find(|a| a.id == CLAUDE_TEAM).unwrap();
        let r = refresh_result(team, s.readings.get(&CLAUDE_TEAM), later);
        assert_eq!(r.state, UsageState::Ready);
        assert_eq!(r.plan.as_deref(), Some("Max 5x"));
        assert_eq!(r.source, ReadSource::Hub);
        let reading = r.reading.unwrap();
        assert_eq!(reading.fetched_at, later);
        let five = reading
            .windows
            .iter()
            .find(|w| w.key == "five_hour")
            .unwrap();
        assert_eq!(five.used_percent, 100.0);
        assert!(five.resets_at.unwrap() > later);

        let stale = s.accounts.iter().find(|a| a.id == CODEX_PERSONAL).unwrap();
        let r = refresh_result(stale, s.readings.get(&CODEX_PERSONAL), later);
        assert_eq!(r.state, UsageState::Unavailable);
        assert!(r.reading.is_none());
    }

    #[test]
    fn generated_files_are_deterministic() {
        assert_eq!(session_logs(now()), session_logs(now()));
        assert_eq!(history_lines(now()), history_lines(now()));
        let h = history_lines(now());
        assert!(h.iter().all(|l| (0.0..=100.0).contains(&l.used_percent)));
        assert!(h.iter().all(|l| l.fetched_at <= now()));
        // The newest line per window matches the seeded reading.
        let last = h
            .iter()
            .rev()
            .find(|l| l.account == CLAUDE_PERSONAL && l.window == "five_hour")
            .unwrap();
        assert_eq!(last.used_percent, 81.0);
        // No secrets or message text in what is written.
        for (_, body) in profile_files().iter().chain(session_logs(now()).iter()) {
            for bad in [
                "token\":\"",
                "access_token",
                "refresh_token",
                "content",
                "apiKey",
            ] {
                assert!(!body.contains(bad), "{bad}");
            }
        }
    }

    #[test]
    fn only_the_sandbox_can_be_wiped() {
        let temp = tempfile::tempdir().unwrap();
        let t = temp.path();
        assert!(safe_to_wipe(&sandbox_dir(t), t));
        assert!(!safe_to_wipe(t, t));
        assert!(!safe_to_wipe(&t.join("other"), t));
        assert!(!safe_to_wipe(&t.join("x").join(SANDBOX_NAME), t));
        assert!(!safe_to_wipe(Path::new(SANDBOX_NAME), Path::new("")));
        assert!(reset(&t.join("other"), t).is_err());
        // Reset clears what an earlier run left.
        let dir = sandbox_dir(t);
        reset(&dir, t).unwrap();
        std::fs::write(dir.join("home").join("left.txt"), "x").unwrap();
        reset(&dir, t).unwrap();
        assert!(!dir.join("home").join("left.txt").exists());
        assert!(dir.join("local").is_dir());
    }

    #[test]
    fn display_paths_hide_the_temp_folder() {
        let root = Path::new(r"C:\Users\someone\AppData\Local\Temp\AI SubHub Demo");
        assert_eq!(
            display_path(&root.join("home").join("Projects").join("acme-web"), root).as_deref(),
            Some(r"C:\Users\demo\Projects\acme-web")
        );
        assert_eq!(
            display_path(
                &root.join("local").join("AI SubHub").join("settings.json"),
                root
            )
            .as_deref(),
            Some(r"C:\Users\demo\AppData\Local\AI SubHub\settings.json")
        );
        assert_eq!(display_path(Path::new(r"D:\elsewhere"), root), None);
    }

    /// The seeded sandbox through the real code paths: detection finds the profiles as already
    /// added, the statistics scan reads the logs and prices every model with the built-in table.
    #[test]
    fn seeded_sandbox_feeds_detection_and_statistics() {
        let temp = tempfile::tempdir().unwrap();
        let root = sandbox_dir(temp.path());
        reset(&root, temp.path()).unwrap();
        let s = seed(&root, now(), "0.1.2").unwrap();
        let loaded =
            settings::load(&root.join("local").join("AI SubHub").join("settings.json")).unwrap();
        assert_eq!(loaded.accounts, s.accounts);
        let home = root.join("home");
        assert!(s.projects.iter().all(|p| p.directory.is_dir()));
        // No credential files anywhere.
        for p in [
            ".claude-work",
            ".claude-personal",
            ".codex",
            ".codex-personal",
        ] {
            assert!(!home.join(p).join(".credentials.json").exists());
            assert!(!home.join(p).join("auth.json").exists());
        }

        let found = detect::detect(&home, &ProfileEnv::default(), &s.accounts);
        assert_eq!(found.len(), 4, "{found:?}");
        assert!(
            found
                .iter()
                .all(|d| matches!(d.status, DetectStatus::Added { .. })),
            "{found:?}"
        );

        let roots = stats::discover_roots(&home, &ProfileEnv::default());
        let scan = stats::scan(&roots, None).unwrap();
        assert!(scan.files > 100);
        let prices = PriceBook::default();
        let local_now = now().with_timezone(&chrono::Local).naive_local();
        for range in [Range::Day, Range::Week, Range::Month, Range::Quarter] {
            let sum = stats::summarize(&scan, range, local_now, &prices);
            assert!(sum.work() > 0, "{range:?}");
            assert_eq!(sum.unpriced_models, 0, "{range:?}");
        }
    }
}
