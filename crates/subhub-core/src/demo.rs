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
pub const COPILOT: Uuid = Uuid::from_u128(0x00000000_0000_4000_8000_0000000de007);
pub const CURSOR: Uuid = Uuid::from_u128(0x00000000_0000_4000_8000_0000000de008);
pub const ANTIGRAVITY: Uuid = Uuid::from_u128(0x00000000_0000_4000_8000_0000000de009);

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
            // A model's own weekly limit, as Claude reports it today (`limits` array).
            ("weekly_scoped:Fable", "Weekly · Fable", 22.0, 4560),
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
    // App logins (owner, 2026-11-09: no real accounts to test with, so the demo shows them):
    // Copilot's monthly AI credits, Cursor's weekly plan with its Auto/API pools, Antigravity's
    // two groups with 5-hour and weekly quotas each.
    Spec {
        id: COPILOT,
        provider: ProviderKind::Copilot,
        label: "Copilot",
        // Copilot shows the GitHub login, not an email.
        email: "demo-dev",
        profile: Some(".copilot"),
        hub: None,
        group: "Work",
        tag: "GH",
        plan: "Pro",
        windows: &[("copilot:premium", "Monthly · AI credits", 62.0, 30_240)],
        fetched_mins_ago: 3,
        failing: false,
    },
    Spec {
        id: CURSOR,
        provider: ProviderKind::Cursor,
        label: "Cursor",
        email: "cursor@example.com",
        profile: Some("AppData/Roaming/Cursor"),
        hub: None,
        group: "Personal",
        tag: "CUR",
        plan: "Pro+",
        windows: &[
            ("cursor:plan", "Weekly", 41.0, 4320),
            ("cursor:auto", "Weekly · Auto", 35.0, 4320),
            ("cursor:api", "Weekly · API", 12.0, 4320),
        ],
        fetched_mins_ago: 4,
        failing: false,
    },
    Spec {
        id: ANTIGRAVITY,
        provider: ProviderKind::Antigravity,
        label: "Antigravity",
        email: "ag@example.com",
        profile: Some(".gemini/antigravity-cli"),
        hub: None,
        group: "Personal",
        tag: "AG",
        plan: "Ultra",
        windows: &[
            ("agy:5h", "5-hour", 30.0, 180),
            ("agy:weekly", "Weekly", 55.0, 6500),
            ("agy:claude-gpt:5h", "5-hour · Claude & GPT", 10.0, 120),
            ("agy:claude-gpt:weekly", "Weekly · Claude & GPT", 20.0, 6500),
        ],
        fetched_mins_ago: 2,
        failing: false,
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
pub const HISTORY_DAYS: i64 = 35;

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
    if key == "five_hour" || key == "codex:primary" || key.ends_with("5h") {
        Duration::hours(5)
    } else if key.starts_with("copilot:") {
        Duration::days(30)
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
    s.display.taskbar_favorites = vec![CLAUDE_WORK, CLAUDE_PERSONAL, CODEX_WORK];
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
    // Canceled: access ends in 12 days (the Overview's ending-plan tip and what-if).
    s.billing.insert(
        CODEX_PERSONAL,
        MonthlyBilling {
            auto_renew: false,
            ..MonthlyBilling::new(today + Duration::days(12))
        },
    );
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
    let mut out: Vec<(PathBuf, String)> = SPECS
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
        .collect();
    // The app logins' folders, so Add accounts finds them (no tokens in any of them).
    out.push((Path::new(".copilot").join("config.json"), "{}".into()));
    out.push((
        Path::new("AppData")
            .join("Roaming")
            .join("Cursor")
            .join("User")
            .join("settings.json"),
        "{}".into(),
    ));
    out.push((
        Path::new(".gemini")
            .join("antigravity-cli")
            .join("settings.json"),
        "{}".into(),
    ));
    out
}

/// Made-up thread titles (Claude `custom-title` lines).
const TITLES: &[&str] = &[
    "Fix login redirect",
    "Speed up the search page",
    "Add CSV export",
    "Rate limits for the API",
    "Dark mode polish",
    "Flaky checkout test",
    "Docs: getting started",
    "Upgrade the build pipeline",
    "Refactor settings storage",
    "Release notes 2.4",
];

/// What a demo step does: tool calls as (name, shell command) pairs. Commands are made up and
/// only their first word is ever read.
/// Tool calls of one step: (name, shell command).
type Tools = &'static [(&'static str, Option<&'static str>)];

const STEP_KINDS: &[(u64, Tools)] = &[
    (24, &[]),
    (12, &[("Edit", None)]),
    (6, &[("Edit", None), ("Write", None)]),
    (3, &[("MultiEdit", None)]),
    (6, &[("Bash", Some("cargo test --workspace"))]),
    (3, &[("PowerShell", Some("npm run build"))]),
    (2, &[("Bash", Some("cd app && pnpm test"))]),
    (4, &[("Bash", Some("git status"))]),
    (2, &[("Bash", Some("gh pr view"))]),
    (10, &[("Read", None)]),
    (5, &[("Grep", None), ("Read", None)]),
    (3, &[("Glob", None)]),
    (2, &[("PowerShell", Some("Get-Content README.md"))]),
    (4, &[("Agent", None)]),
    (3, &[("WebSearch", None)]),
    (2, &[("WebFetch", None)]),
    (3, &[("mcp__preview__preview_screenshot", None)]),
    (6, &[("TodoWrite", None)]),
];

/// Codex tool calls before a token count: one JSON `response_item` payload each.
const CODEX_CALLS: &[&str] = &[
    r#"{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"cargo build\"}"}"#,
    r#"{"type":"function_call","name":"shell","arguments":"{\"command\":[\"bash\",\"-lc\",\"git diff\"]}"}"#,
    r#"{"type":"function_call","name":"shell_command","arguments":"{\"command\":\"rg TODO\"}"}"#,
    r#"{"type":"custom_tool_call","name":"apply_patch","input":""}"#,
    r#"{"type":"web_search_call","status":"completed"}"#,
    r#"{"type":"function_call","name":"update_plan","arguments":"{}"}"#,
];

fn pick<'a, T>(rng: &mut Rng, table: &'a [(u64, T)]) -> &'a T {
    let total: u64 = table.iter().map(|(w, _)| w).sum();
    let mut x = rng.range(0, total - 1);
    for (w, v) in table {
        if x < *w {
            return v;
        }
        x -= w;
    }
    &table[0].1
}

/// A JSON string literal (paths carry backslashes).
fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// One Claude Code assistant step in the log's shape: one line per tool use (same ids and usage,
/// as Claude Code writes content blocks), or one line without content for a step without tools.
#[allow(clippy::too_many_arguments)]
fn claude_step(
    text: &mut String,
    rng: &mut Rng,
    t: DateTime<Utc>,
    session: Uuid,
    model: &str,
    head: &str,
    tools: &[(&str, Option<&str>)],
    usage: (u64, u64, u64, u64),
) {
    let (input, output, cache_write, cache_read) = usage;
    let (req, msg) = (rng.next(), rng.next());
    let blocks: Vec<String> = if tools.is_empty() {
        vec![String::new()]
    } else {
        tools
            .iter()
            .map(|(name, cmd)| {
                let input = match cmd {
                    Some(c) => format!(r#"{{"command":{}}}"#, json_str(c)),
                    None => "{}".into(),
                };
                format!(
                    r#","content":[{{"type":"tool_use","id":"toolu_demo_{:012x}","name":"{name}","input":{input}}}]"#,
                    rng.next() >> 16
                )
            })
            .collect()
    };
    for block in blocks {
        let _ = writeln!(
            text,
            r#"{{"type":"assistant"{head},"timestamp":"{}","sessionId":"{session}","requestId":"req_demo_{req:016x}","message":{{"id":"msg_demo_{msg:016x}","model":"{model}"{block},"usage":{{"input_tokens":{input},"output_tokens":{output},"cache_creation_input_tokens":{cache_write},"cache_read_input_tokens":{cache_read}}}}}}}"#,
            ts(t),
        );
    }
}

/// Synthetic Claude Code (`<profile>\projects\<project>\<session>.jsonl`, subagents in
/// `<session>\subagents\agent-….jsonl`) and Codex (`<profile>\sessions\YYYY\MM\DD\rollout-….jsonl`)
/// logs over [`LOG_DAYS`], relative to the sandbox home, in the exact shape `stats::parse` reads:
/// usage, tool calls with made-up commands, `isSidechain`, `cwd` inside the demo projects under
/// `home` (some in a scratch folder, some without), thread titles. No message text.
pub fn session_logs(now: DateTime<Utc>, home: &Path) -> Vec<(PathBuf, String)> {
    let projects = home.join("Projects");
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
            let pick_model = rng.range(0, 99);
            let mut acc = 0;
            let model = CLAUDE_MODELS
                .iter()
                .find(|(_, share)| {
                    acc += share;
                    pick_model < acc
                })
                .map_or(CLAUDE_MODELS[0].0, |m| m.0);
            let session = rng.uuid();
            // Mostly in a project (sometimes a subfolder), some in a scratch folder, a few old
            // logs without a working folder.
            let cwd = match rng.range(0, 99) {
                0..=59 => Some(projects.join(project)),
                60..=79 => Some(projects.join(project).join("src")),
                80..=91 => Some(home.join("scratch").join("notes")),
                _ => None,
            };
            let head = |sidechain: bool| match &cwd {
                Some(c) => format!(
                    r#","isSidechain":{sidechain},"cwd":{}"#,
                    json_str(&c.to_string_lossy())
                ),
                None => String::new(),
            };
            let mut t =
                now - Duration::days(day) - Duration::minutes(rng.range(30, 14 * 60) as i64);
            let msgs = rng.range(15, 60);
            let mut text = String::new();
            if rng.range(0, 1) == 0 {
                let title = TITLES[rng.range(0, TITLES.len() as u64 - 1) as usize];
                let _ = writeln!(
                    text,
                    r#"{{"type":"custom-title","customTitle":"{title}","sessionId":"{session}"}}"#
                );
            }
            let mut agent_at: Option<DateTime<Utc>> = None;
            for i in 0..msgs {
                let first = i == 0;
                let usage = (
                    rng.range(1, 40),
                    rng.range(80, 2_500),
                    if first {
                        rng.range(15_000, 30_000)
                    } else {
                        rng.range(300, 6_000)
                    },
                    if first {
                        0
                    } else {
                        18_000 + i * 2_400 + rng.range(0, 4_000)
                    },
                );
                let tools = *pick(&mut rng, STEP_KINDS);
                if tools.iter().any(|(n, _)| *n == "Agent") {
                    agent_at.get_or_insert(t);
                }
                claude_step(
                    &mut text,
                    &mut rng,
                    t,
                    session,
                    model,
                    &head(false),
                    tools,
                    usage,
                );
                t += Duration::seconds(rng.range(40, 180) as i64);
            }
            let dir = Path::new(profile)
                .join("projects")
                .join(format!("C--Users-demo-Projects-{project}"));
            // A subagent started by an Agent call: its own file, same thread, marked sidechain.
            if let Some(mut at) = agent_at {
                let mut sub = String::new();
                for i in 0..rng.range(5, 15) {
                    let tools: &[(&str, Option<&str>)] = match i % 3 {
                        0 => &[("Grep", None)],
                        1 => &[("Read", None)],
                        _ => &[("Bash", Some("cargo check"))],
                    };
                    let usage = (
                        rng.range(1, 20),
                        rng.range(50, 900),
                        rng.range(300, 4_000),
                        12_000 + i * 1_500,
                    );
                    claude_step(
                        &mut sub,
                        &mut rng,
                        at,
                        session,
                        CLAUDE_MODELS[1].0,
                        &head(true),
                        tools,
                        usage,
                    );
                    at += Duration::seconds(rng.range(10, 60) as i64);
                }
                out.push((
                    dir.join(session.to_string())
                        .join("subagents")
                        .join(format!("agent-{:016x}.jsonl", rng.next())),
                    sub,
                ));
            }
            out.push((dir.join(format!("{session}.jsonl")), text));
        }
        // Codex.
        let sessions = ((rng.range(1, 3) as f64) * factor).round() as u64;
        for _ in 0..sessions {
            let profile = if rng.range(0, 99) < 75 {
                ".codex"
            } else {
                ".codex-personal"
            };
            let project = PROJECTS[rng.range(0, PROJECTS.len() as u64 - 1) as usize].1;
            let session = rng.uuid();
            let start =
                now - Duration::days(day) - Duration::minutes(rng.range(20, 12 * 60) as i64);
            let mut t = start;
            let mut text = String::new();
            let _ = writeln!(
                text,
                r#"{{"timestamp":"{}","type":"turn_context","payload":{{"model":"{CODEX_MODEL}","cwd":{}}}}}"#,
                ts(t),
                json_str(&projects.join(project).to_string_lossy())
            );
            let (mut ti, mut tc, mut to) = (0u64, 0u64, 0u64);
            for turn in 0..rng.range(8, 30) {
                for _ in 0..rng.range(0, 2) {
                    let call = CODEX_CALLS[rng.range(0, CODEX_CALLS.len() as u64 - 1) as usize];
                    let _ = writeln!(
                        text,
                        r#"{{"timestamp":"{}","type":"response_item","payload":{call}}}"#,
                        ts(t)
                    );
                }
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

/// The 5-hour and weekly window of a demo account and how it works: 5-hour windows per weekday
/// (UTC start hour and % used per hour; Mon = 0) and the target weekly peak of past cycles
/// (base + up to `spread`).
struct Pattern {
    id: Uuid,
    session_key: &'static str,
    weekly_key: &'static str,
    /// Scoped weekly windows that follow the all-models one (key, share of it).
    scoped: &'static [(&'static str, f64)],
    days: [&'static [(u32, f64)]; 7],
    weekly_base: f64,
    weekly_spread: u64,
    /// Days of history (an account added later has less).
    history_days: i64,
}

const NO_WINDOWS: &[(u32, f64)] = &[];

/// The story the Overview tells about the demo: Personal (Pro) runs out about 3 hours into its
/// first window on Mon/Wed/Fri while Work had room (at the limit, warm-up, "switch instead of
/// waiting"); Work (Max 20x) peaks near 20 % of its week (could drop to Max 5x); Team (Max 5x)
/// fits; Side project (benched, added 12 days ago) sits idle; Codex Work (Pro) fits; Codex
/// Personal (Plus, its access ends soon) uses afternoons, past the PC's evening switch-off
/// (windows with an unknown end).
const PATTERNS: &[Pattern] = &[
    Pattern {
        id: CLAUDE_PERSONAL,
        session_key: "five_hour",
        weekly_key: "seven_day",
        scoped: &[],
        days: [
            &[(8, 34.0), (14, 12.0)],
            &[(8, 22.0), (14, 10.0)],
            &[(8, 34.0), (14, 12.0)],
            &[(8, 22.0)],
            &[(8, 34.0), (14, 8.0)],
            NO_WINDOWS,
            NO_WINDOWS,
        ],
        weekly_base: 72.0,
        weekly_spread: 14,
        history_days: HISTORY_DAYS,
    },
    Pattern {
        id: CLAUDE_WORK,
        session_key: "five_hour",
        weekly_key: "seven_day",
        scoped: &[("seven_day_opus", 0.48)],
        days: [
            &[(9, 8.0), (15, 5.0)],
            &[(9, 9.0)],
            &[(9, 8.0), (15, 6.0)],
            &[(9, 7.0)],
            &[(9, 8.0)],
            &[(10, 4.0)],
            NO_WINDOWS,
        ],
        weekly_base: 18.0,
        weekly_spread: 4,
        history_days: HISTORY_DAYS,
    },
    Pattern {
        id: CLAUDE_TEAM,
        session_key: "five_hour",
        weekly_key: "seven_day",
        scoped: &[],
        days: [
            &[(10, 15.0)],
            &[(10, 30.0)],
            &[(10, 15.0)],
            &[(10, 30.0)],
            &[(10, 12.0)],
            NO_WINDOWS,
            NO_WINDOWS,
        ],
        weekly_base: 60.0,
        weekly_spread: 12,
        history_days: HISTORY_DAYS,
    },
    Pattern {
        id: CLAUDE_SIDE,
        session_key: "five_hour",
        weekly_key: "seven_day",
        scoped: &[],
        days: [
            NO_WINDOWS,
            &[(11, 3.0)],
            NO_WINDOWS,
            NO_WINDOWS,
            NO_WINDOWS,
            NO_WINDOWS,
            NO_WINDOWS,
        ],
        weekly_base: 2.0,
        weekly_spread: 2,
        history_days: 12,
    },
    Pattern {
        id: CODEX_WORK,
        session_key: "codex:primary",
        weekly_key: "codex:secondary",
        scoped: &[],
        days: [
            &[(13, 12.0)],
            &[(13, 10.0)],
            &[(13, 12.0)],
            &[(13, 9.0)],
            &[(13, 6.0)],
            NO_WINDOWS,
            NO_WINDOWS,
        ],
        weekly_base: 28.0,
        weekly_spread: 8,
        history_days: HISTORY_DAYS,
    },
    Pattern {
        id: CODEX_PERSONAL,
        session_key: "codex:primary",
        weekly_key: "codex:secondary",
        scoped: &[],
        days: [
            &[(16, 18.0)],
            NO_WINDOWS,
            &[(16, 18.0)],
            NO_WINDOWS,
            &[(16, 15.0)],
            NO_WINDOWS,
            NO_WINDOWS,
        ],
        weekly_base: 35.0,
        weekly_spread: 10,
        history_days: HISTORY_DAYS,
    },
];

/// The PC is on (SubHub reads every 15 minutes) 07:00-18:30 UTC on weekdays and 09:00-12:00 on
/// Saturdays; off at night and on Sundays (gaps).
fn pc_on(t: DateTime<Utc>) -> bool {
    use chrono::{Datelike, Timelike, Weekday};
    let m = t.hour() * 60 + t.minute();
    match t.weekday() {
        Weekday::Sun => false,
        Weekday::Sat => (9 * 60..=12 * 60).contains(&m),
        _ => (7 * 60..=18 * 60 + 30).contains(&m),
    }
}

/// % used of a window (start, % per hour) at `t`.
fn session_used(start: DateTime<Utc>, rate: f64, t: DateTime<Utc>) -> f64 {
    let h = (t - start).num_seconds() as f64 / 3600.0;
    (rate * h).clamp(0.0, 100.0)
}

/// Readings over [`HISTORY_DAYS`] that end at each account's seeded reading: every 15 minutes
/// while the PC is on, 5-hour windows per the account's weekday pattern (the seeded window is
/// the last one), weekly use summing up its windows and dropping at each weekly reset. Resets
/// carry a few seconds of jitter, as the providers' do. Oldest first per account.
pub fn history_lines(now: DateTime<Utc>) -> Vec<HistoryLine> {
    use chrono::{Datelike, DurationRound};
    let mut out = vec![];
    let five = Duration::hours(5);
    let week = Duration::days(7);
    for p in PATTERNS {
        let Some(sp) = spec(p.id) else { continue };
        let last = now - Duration::minutes(sp.fetched_mins_ago);
        let start = now - Duration::days(p.history_days);
        let seeded = |key: &str| sp.windows.iter().find(|w| w.0 == key).copied();
        let Some((_, _, cur_used, cur_mins)) = seeded(p.session_key) else {
            continue;
        };
        let Some((_, _, week_used, week_mins)) = seeded(p.weekly_key) else {
            continue;
        };
        // The running 5-hour window ends at the seeded reading's value.
        let cur_reset = now + Duration::minutes(cur_mins);
        let cur_start = cur_reset - five;
        let elapsed_h = ((last - cur_start).num_seconds() as f64 / 3600.0).max(0.05);
        let cur_rate = if cur_used >= 100.0 {
            100.0 / (elapsed_h * 0.75)
        } else {
            cur_used / elapsed_h
        };
        let mut wins: Vec<(DateTime<Utc>, f64)> = vec![];
        let mut day = start.date_naive();
        while day <= now.date_naive() {
            let pattern = p.days[day.weekday().num_days_from_monday() as usize];
            for &(hour, rate) in pattern {
                let Some(s) = day.and_hms_opt(hour, 0, 0).map(|d| d.and_utc()) else {
                    continue;
                };
                let overlaps = s < cur_reset && cur_start < s + five;
                if s >= start && !overlaps && s < last {
                    wins.push((s, rate));
                }
            }
            day = day.succ_opt().unwrap_or(day);
        }
        wins.push((cur_start, cur_rate));
        // Reading times: the PC's schedule, plus the running window up to the seeded reading.
        let first = start.duration_trunc(Duration::minutes(15)).unwrap_or(start);
        let mut times: Vec<DateTime<Utc>> =
            std::iter::successors(Some(first), |t| Some(*t + Duration::minutes(15)))
                .take_while(|t| *t < last)
                .filter(|t| *t >= start && (pc_on(*t) || *t >= cur_start))
                .collect();
        times.push(last);
        // Weekly cycles: the seeded reset and the ones before it.
        let week_reset = now + Duration::minutes(week_mins);
        let cycle_of = |t: DateTime<Utc>| {
            let n = (week_reset - t)
                .num_seconds()
                .div_euclid(week.num_seconds());
            week_reset - Duration::seconds(n * week.num_seconds())
        };
        let raw = |t: DateTime<Utc>| -> f64 {
            let cycle_start = cycle_of(t) - week;
            wins.iter()
                .filter(|(s, _)| *s >= cycle_start && *s <= t)
                .map(|(s, r)| session_used(*s, *r, t.min(*s + five)))
                .sum()
        };
        let mut rng = Rng::new(p.id.as_u128() as u64);
        let mut scale: HashMap<DateTime<Utc>, f64> = HashMap::new();
        for (n, t) in times.iter().enumerate() {
            let reset = cycle_of(*t);
            let k = *scale.entry(reset).or_insert_with(|| {
                let (target, at) = if reset == week_reset {
                    (week_used, last)
                } else {
                    let base = p.weekly_base + rng.range(0, p.weekly_spread) as f64;
                    (base, reset - Duration::seconds(1))
                };
                let r = raw(at);
                if r > 0.0 { target / r } else { 0.0 }
            });
            let jitter = Duration::seconds(n as i64 % 5 - 2);
            let session = wins
                .iter()
                .find(|(s, _)| *s <= *t && *t < *s + five)
                .map(|(s, r)| (session_used(*s, *r, *t).round(), Some(*s + five + jitter)));
            let (s_used, s_reset) = session.unwrap_or((0.0, None));
            let weekly = (k * raw(*t)).clamp(0.0, 100.0).round();
            let mut push = |key: &str, used: f64, resets_at: Option<DateTime<Utc>>| {
                out.push(HistoryLine {
                    account: p.id,
                    fetched_at: *t,
                    window: key.into(),
                    used_percent: used,
                    resets_at,
                })
            };
            if *t == last {
                // The seeded reading itself closes the history.
                for w in spec_windows(sp, now) {
                    push(&w.key, w.used_percent, w.resets_at);
                }
                continue;
            }
            push(p.session_key, s_used, s_reset);
            push(p.weekly_key, weekly, Some(reset + jitter));
            for (key, share) in p.scoped {
                push(key, (weekly * share).round(), Some(reset + jitter));
            }
        }
    }
    out
}

/// Writes the whole demo into the sandbox `root` (as left by [`reset`]): profile metadata, logs
/// and project folders under `root\home`, settings and reading history under
/// `root\local\AI SubHub`. Returns the settings written.
pub fn seed(root: &Path, now: DateTime<Utc>, version: &str) -> anyhow::Result<Settings> {
    let home = root.join("home");
    std::fs::create_dir_all(&home)?;
    // Stored folders and the logs' working folders are canonical, like the ones profile
    // detection and project matching compare against.
    let canonical = crate::profile::plain(std::fs::canonicalize(&home)?);
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
    for (rel, body) in profile_files()
        .iter()
        .chain(session_logs(now, &canonical).iter())
    {
        write(rel, body)?;
    }
    for (_, dir, _) in PROJECTS {
        std::fs::create_dir_all(home.join("Projects").join(dir))?;
    }
    let s = settings(&canonical, now, version);
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
    use crate::insights;
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
        let cli = s
            .accounts
            .iter()
            .filter(|a| a.provider.is_cli_profile())
            .count();
        assert_eq!((claude, cli - claude, s.accounts.len() - cli), (4, 2, 3));
        // Made-up identities only (Copilot shows a GitHub login, not an email).
        assert!(s.accounts.iter().all(|a| {
            let e = a.email.as_deref().unwrap();
            e.ends_with("@example.com") || (a.provider == ProviderKind::Copilot && e == "demo-dev")
        }));
        // Running out, used up, benched, hub-only, local-only, stale.
        assert!(left(&s, CLAUDE_PERSONAL, "five_hour") < 25.0);
        assert_eq!(left(&s, CLAUDE_TEAM, "five_hour"), 0.0);
        let acc = |id| s.accounts.iter().find(|a| a.id == id).unwrap();
        assert!(acc(CLAUDE_SIDE).hub.as_ref().unwrap().benched);
        assert!(acc(CLAUDE_TEAM).profile_dir.is_none() && acc(CLAUDE_TEAM).hub.is_some());
        assert!(acc(CLAUDE_PERSONAL).hub.is_none() && acc(CLAUDE_PERSONAL).profile_dir.is_some());
        assert!(now() - s.readings[&CODEX_PERSONAL].fetched_at > Duration::hours(2));
        // Favourite, hub + Auto steering with a target, renewal ahead, projects, no patch notes.
        assert_eq!(
            s.display.taskbar_favorites,
            vec![CLAUDE_WORK, CLAUDE_PERSONAL, CODEX_WORK]
        );
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
        let home = Path::new(r"C:\demo\home");
        assert_eq!(session_logs(now(), home), session_logs(now(), home));
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
        // No secrets or message text in what is written: content blocks are tool uses only (scan
        // v2; before it, no content at all).
        for (_, body) in profile_files()
            .iter()
            .chain(session_logs(now(), home).iter())
        {
            for bad in [
                "token\":\"",
                "access_token",
                "refresh_token",
                "\"text\"",
                "thinking",
                "apiKey",
            ] {
                assert!(!body.contains(bad), "{bad}");
            }
            assert_eq!(
                body.matches("\"content\":").count(),
                body.matches("\"content\":[{\"type\":\"tool_use\"").count()
            );
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
        // 4 CLI profiles and the 3 app logins (Copilot, Cursor, Antigravity).
        assert_eq!(found.len(), 7, "{found:?}");
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

        // Scan v2 dims: every task kind, subagent steps, the demo projects, titles, Codex too.
        let kinds: std::collections::BTreeSet<_> = scan.buckets.iter().map(|b| b.kind).collect();
        assert_eq!(kinds.len(), stats::TaskKind::ALL.len(), "{kinds:?}");
        assert!(scan.buckets.iter().any(|b| b.sidechain));
        assert!(!scan.titles.is_empty());
        assert!(scan.buckets.iter().any(|b| b.cwd.is_none()));
        let o = overview(&s, Some(&scan), now());
        let saved: Vec<_> = o
            .usage
            .projects
            .iter()
            .filter(|p| matches!(p.key, insights::ProjectKey::Saved(_)))
            .collect();
        assert_eq!(saved.len(), 3, "{:?}", o.usage.projects);
        assert!(
            o.usage
                .projects
                .iter()
                .any(|p| p.key == insights::ProjectKey::Folder("notes".into()))
        );
        assert!(o.usage.threads.iter().any(|t| t.title.is_some()));
        assert!(o.usage.sub_share() > 0.0);
    }

    fn overview(
        s: &Settings,
        scan: Option<&stats::Scan>,
        now: DateTime<Utc>,
    ) -> insights::Overview {
        let plans: HashMap<Uuid, String> = SPECS
            .iter()
            .map(|sp| (sp.id, sp.plan.to_string()))
            .collect();
        let history = history_lines(now);
        insights::analyze(&insights::Input {
            history: &history,
            scan,
            settings: s,
            plans: &plans,
            prices: &PriceBook::default(),
            range: insights::InsightRange::FourWeeks,
            now,
            zone: &insights::Zone::utc(),
        })
    }

    /// The Overview story the demo history tells (see `PATTERNS`).
    #[test]
    fn history_tells_the_overview_story() {
        use insights::{FitVerdict, PlanTier, TipKind};
        let s = settings(Path::new(r"C:\demo\home"), now(), "0.1.2");
        let o = overview(&s, None, now());
        let fit = |id| o.account(id).unwrap().fit.ready().map(|f| f.verdict);
        assert_eq!(fit(CLAUDE_PERSONAL), Some(FitVerdict::AtLimit));
        assert_eq!(fit(CLAUDE_WORK), Some(FitVerdict::Downgrade));
        assert_eq!(fit(CLAUDE_TEAM), Some(FitVerdict::Fits));
        assert_eq!(fit(CODEX_WORK), Some(FitVerdict::Fits));
        assert_eq!(fit(CODEX_PERSONAL), Some(FitVerdict::Fits));
        let work = o.account(CLAUDE_WORK).unwrap().fit.ready().unwrap();
        assert_eq!(work.suggested, Some(PlanTier::ClaudeMax5x));
        // Weekly cycles over 4+ weeks, several 5-hour hits, one while another account was free.
        assert!(o.account(CLAUDE_PERSONAL).unwrap().cycles.len() >= 4);
        let w = o.windows.ready().expect("enough workdays");
        assert!(w.hits >= 10, "{}", w.hits);
        assert!(w.hits_with_free_account >= 1);
        // PC off in the evening: Codex Personal's late windows have an unknown end.
        assert!(
            o.account(CODEX_PERSONAL)
                .unwrap()
                .windows
                .iter()
                .any(|w| w.end == insights::WindowEnd::Unknown)
        );
        // The benched login never counts as a free account.
        assert!(
            o.accounts
                .iter()
                .flat_map(|a| a.windows.iter())
                .all(|w| !w.free_elsewhere.contains(&CLAUDE_SIDE))
        );
        // Warm-up: Personal's first window runs out about 3 h in.
        let warm = w.warmup.as_ref().expect("warm-up suggestion");
        assert_eq!(warm.accounts.first(), Some(&CLAUDE_PERSONAL));
        assert!(warm.simulation.saved() > Duration::zero());
        // Codex Personal's access ends: its use moves to Codex Work.
        let wi = o.account(CODEX_PERSONAL).unwrap().what_if.as_ref().unwrap();
        assert_eq!(wi.others.len(), 1);
        assert_eq!(wi.others[0].account, CODEX_WORK);
        let kinds: Vec<_> = o.tips.iter().map(|t| &t.kind).collect();
        for want in ["ending", "upgrade", "switch", "warm-up", "downgrade"] {
            assert!(
                o.tips.iter().any(|t| t.id.starts_with(want)),
                "{want}: {kinds:?}"
            );
        }
        assert!(matches!(o.tips[0].kind, TipKind::EndingPlan { .. }));
        // Deterministic for a given now.
        assert_eq!(overview(&s, None, now()), o);
    }
}
