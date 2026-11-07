//! Settings file `%LOCALAPPDATA%\AI SubHub\settings.json`. Holds metadata, preferences and
//! cached readings only, never secrets. The C# app's settings are only ever read (`import`).
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::billing::{MonthlyBilling, ReminderMark};
use crate::model::{Account, ProviderKind, Reading, UsageState};
use crate::projects::Project;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum UsageStyle {
    #[default]
    Remaining,
    Used,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Gauge {
    #[default]
    Ring,
    Bar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

/// Always-visible display on the taskbar (ADR 0001 refinements). Values are % left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TaskbarMode {
    /// Legacy "no display": older files load it as `taskbar_enabled = false` + Badge
    /// (`settings_store::sanitize`). The UI offers an on/off switch instead.
    Off,
    /// Per account a bordered badge like T3 Code's provider tags: provider glyph, coloured
    /// two-letter account tag, 5h and W mini bars with %. The removed styles "Numbers" and
    /// "Focus" (and "Table", which had become Numbers) load as this (owner, 2026-11-07).
    #[default]
    #[serde(alias = "Numbers", alias = "Focus", alias = "Table")]
    Badge,
    /// Per account: name, 5-hour bar with the % printed (preview R1). Old "Mix"/"Blocks"
    /// values load as this.
    #[serde(alias = "Mix", alias = "Blocks")]
    Bars,
}

/// Which accounts the taskbar display shows. The popover always shows all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TaskbarAccounts {
    /// Accounts starred "on taskbar" (at most 3); the steering target when none is starred.
    #[default]
    Favorites,
    /// Only the steering target (the account new hub threads go to).
    Focus,
    /// The steering target plus any account running low.
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TaskbarPosition {
    /// Right of the widgets/weather button (centred taskbars only).
    Left,
    /// Next to the tray icons (default; works with left-aligned taskbars too).
    #[default]
    Right,
}

/// Which monitors' taskbars get the taskbar display (owner 2026-11-07). Monitors are saved by
/// their stable display id (`placement::MonitorInfo::id`), not "Display 2".
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TaskbarMonitors {
    /// The main taskbar only (where the tray icon is).
    #[default]
    Main,
    /// Every taskbar Windows shows.
    All,
    /// These monitors; a missing one falls back to the main taskbar until it is back.
    Chosen(Vec<String>),
}

/// % left at or below which a window is "Running out" (warning colour, taskbar Auto). Fixed
/// since 2026-11-07 (owner); older files with a `running_out_threshold` still load, ignored.
pub const RUNNING_OUT_PERCENT: f64 = 25.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplaySettings {
    pub usage_style: UsageStyle,
    /// Gauge per view section.
    pub priority_gauge: Gauge,
    pub all_usage_gauge: Gauge,
    // Removed 2026-11-07 (owner, "Settings distilled"): `running_out_threshold` (now
    // `RUNNING_OUT_PERCENT`), `reset_format` (always relative, the exact time in a tooltip),
    // `density` (always Comfortable) and `tray_metric` (the tray icon is a static logo). Older
    // files that still have them load; unknown fields are ignored.
    pub show_plan: bool,
    pub show_email: bool,
    pub theme: ThemeMode,
    /// "#rrggbb"
    pub accent: String,
    /// Close the popover when another window takes focus. Off: it stays open until the tray
    /// icon, Esc or its close button (the owner's preference, 2026-11-06).
    pub close_popover_on_blur: bool,
    /// Popover height the user dragged its top edge to (logical px); None = fit the content.
    pub popover_height: Option<f32>,
    /// What the close buttons (popover ✕, settings window X) do: false = keep running in the
    /// tray (default), true = quit SubHub completely.
    #[serde(default)]
    pub close_quits: bool,
    /// Show the taskbar display next to the tray. Off leaves just the tray icon; the style is
    /// kept for when it is switched back on.
    pub taskbar_enabled: bool,
    pub taskbar_mode: TaskbarMode,
    pub taskbar_position: TaskbarPosition,
    /// Where the user dragged the taskbar display: logical px from the taskbar's right edge to
    /// the display's right edge. None = automatic placement for `taskbar_position`.
    /// Legacy single offset (before per-monitor offsets): still used for the main taskbar
    /// until the display is dragged there again.
    #[serde(default)]
    pub taskbar_offset: Option<f32>,
    /// Dragged spot per monitor id, same meaning as `taskbar_offset`.
    #[serde(default)]
    pub taskbar_offsets: BTreeMap<String, f32>,
    #[serde(default)]
    pub taskbar_monitors: TaskbarMonitors,
    /// Hover cards over the taskbar badges (off: nothing on hover). The tray icon always keeps
    /// Windows' plain text tooltip (owner, 2026-11-07).
    pub hover_details: bool,
    #[serde(default)]
    pub taskbar_accounts: TaskbarAccounts,
    /// Accounts starred for the taskbar, in star order (Favorites mode shows the first 3).
    #[serde(default)]
    pub taskbar_favorites: Vec<Uuid>,
    /// How many accounts the taskbar display shows at most (1–5).
    #[serde(default = "default_taskbar_count")]
    pub taskbar_count: u8,
}

fn default_taskbar_count() -> u8 {
    3
}

impl DisplaySettings {
    /// Dragged spot of the taskbar display on this monitor (None = automatic placement). The
    /// main taskbar falls back to the legacy single offset.
    pub fn taskbar_offset_for(&self, monitor: &str, is_main: bool) -> Option<f32> {
        self.taskbar_offsets.get(monitor).copied().or(if is_main {
            self.taskbar_offset
        } else {
            None
        })
    }

    /// Remembers a drag on one monitor; the main taskbar's legacy value is dropped then.
    pub fn set_taskbar_offset(&mut self, monitor: &str, is_main: bool, offset: f32) {
        self.taskbar_offsets
            .insert(monitor.to_string(), offset.max(0.0));
        if is_main {
            self.taskbar_offset = None;
        }
    }

    /// Back to automatic placement everywhere (Position Left/Right chosen).
    pub fn reset_taskbar_offsets(&mut self) {
        self.taskbar_offset = None;
        self.taskbar_offsets.clear();
    }
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            usage_style: UsageStyle::Remaining,
            priority_gauge: Gauge::Ring,
            all_usage_gauge: Gauge::Bar,
            show_plan: true,
            show_email: false,
            theme: ThemeMode::System,
            accent: "#3b82f6".into(),
            close_popover_on_blur: false,
            popover_height: None,
            close_quits: false,
            taskbar_enabled: true,
            taskbar_mode: TaskbarMode::Badge,
            taskbar_position: TaskbarPosition::Right,
            taskbar_offset: None,
            taskbar_offsets: BTreeMap::new(),
            taskbar_monitors: TaskbarMonitors::Main,
            hover_details: true,
            taskbar_accounts: TaskbarAccounts::Favorites,
            taskbar_favorites: vec![],
            taskbar_count: default_taskbar_count(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HubSettings {
    pub enabled: bool,
    pub base_url: String,
}

impl Default for HubSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            base_url: "http://127.0.0.1:8317".into(),
        }
    }
}

/// Hub steering: the only thing SubHub writes to the hub is each login's credential priority.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SteeringSettings {
    pub enabled: bool,
    /// Manual pick (clicked Hub tag); None = Auto.
    pub preferred: Option<Uuid>,
    /// Account SubHub last made top.
    pub current: Option<Uuid>,
    /// Hub file name -> priority before SubHub first changed it (None = it had none).
    pub original_priorities: HashMap<String, Option<i64>>,
    /// Priority given to the target login.
    pub top_priority: i64,
}

impl Default for SteeringSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            preferred: None,
            current: None,
            original_priorities: HashMap::new(),
            top_priority: 100,
        }
    }
}

/// Where the sign-in CLI login opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Terminal {
    /// Windows Terminal when installed, else PowerShell.
    #[default]
    WindowsTerminal,
    PowerShell,
}

/// Tools SubHub launches: the sign-in terminal and the provider CLIs. `None` = auto-detect.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolsSettings {
    pub terminal: Terminal,
    pub claude_exe: Option<PathBuf>,
    pub codex_exe: Option<PathBuf>,
}

impl ToolsSettings {
    pub fn exe_for(&self, provider: ProviderKind) -> Option<&Path> {
        match provider {
            ProviderKind::Claude => self.claude_exe.as_deref(),
            ProviderKind::Codex => self.codex_exe.as_deref(),
        }
    }
}

/// Self-update behavior (Settings -> About & updates). Checks run in both modes; Auto also
/// downloads and installs when SubHub is idle, Manual waits for the Install button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum UpdateMode {
    Auto,
    #[default]
    Manual,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateSettings {
    pub mode: UpdateMode,
    /// Last successful check.
    pub last_check: Option<DateTime<Utc>>,
    /// Version whose patch notes were shown (or that was installed fresh); empty = never run.
    pub last_seen_version: String,
}

/// What the Usage view breaks the numbers down by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatsBreakdown {
    #[default]
    Models,
    Providers,
    Days,
}

/// What the daily chart (and the share bars) measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatsMetric {
    #[default]
    Tokens,
    Cost,
}

/// Remembered choices of the Statistics window.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StatsSettings {
    pub breakdown: StatsBreakdown,
    pub metric: StatsMetric,
}

/// Persisted refresh schedule per account (failure backoff + provider cooldown survive restarts).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedSchedule {
    pub state: UsageState,
    pub failures: u32,
    pub next_attempt: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub schema_version: u32,
    /// In the user's order; the views group them by provider.
    pub accounts: Vec<Account>,
    /// User-defined group names in order (e.g. "Work", "Personal").
    pub groups: Vec<String>,
    pub display: DisplaySettings,
    pub hub: HubSettings,
    pub refresh_minutes: u32,
    /// Last successful reading per account (shown stale when overdue or after failure).
    pub readings: HashMap<Uuid, Reading>,
    pub schedules: HashMap<Uuid, CachedSchedule>,
    /// User-set account tags for the taskbar badge (1–3 chars, as typed); missing = automatic.
    pub account_tags: HashMap<Uuid, String>,
    pub steering: SteeringSettings,
    /// Terminal and CLI paths for sign-in.
    pub tools: ToolsSettings,
    /// Optional monthly renewal / access end date per account.
    pub billing: HashMap<Uuid, MonthlyBilling>,
    /// Last billing reminder shown per account (cycle date + threshold), so none repeats.
    pub billing_reminders: HashMap<Uuid, ReminderMark>,
    /// Saved local folders shared by all accounts, in the user's manual order (Launch menu).
    pub projects: Vec<Project>,
    /// Update mode, last check and the version whose patch notes were shown.
    pub updates: UpdateSettings,
    /// Statistics window: breakdown view and chart metric.
    pub stats: StatsSettings,
    /// Overview tips the user hid with "Not for me" (stable tip ids, `insights::Tip::id`).
    pub dismissed_tips: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            accounts: vec![],
            groups: vec![],
            display: DisplaySettings::default(),
            hub: HubSettings::default(),
            refresh_minutes: 5,
            readings: HashMap::new(),
            schedules: HashMap::new(),
            account_tags: HashMap::new(),
            steering: SteeringSettings::default(),
            tools: ToolsSettings::default(),
            billing: HashMap::new(),
            billing_reminders: HashMap::new(),
            projects: vec![],
            updates: UpdateSettings::default(),
            stats: StatsSettings::default(),
            dismissed_tips: vec![],
        }
    }
}

pub fn default_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("AI SubHub").join("settings.json")
}

/// Loads settings; a missing file gives defaults. A corrupt file is kept as `settings.json.bad`
/// and defaults are returned (never silently overwritten).
pub fn load(path: &Path) -> anyhow::Result<Settings> {
    crate::settings_store::load(path)
}

/// Atomic write (temp file + rename).
pub fn save(path: &Path, settings: &Settings) -> anyhow::Result<()> {
    crate::settings_store::save(path, settings)
}

#[cfg(test)]
mod compat_tests {
    use super::*;

    #[test]
    fn empty_and_pre_billing_files_still_load() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.tools, ToolsSettings::default());
        assert!(s.billing.is_empty() && s.billing_reminders.is_empty());

        // The shape written before tools/billing existed.
        let mut v = serde_json::to_value(Settings::default()).unwrap();
        for k in ["tools", "billing", "billing_reminders"] {
            assert!(v.as_object_mut().unwrap().remove(k).is_some(), "{k}");
        }
        let s: Settings = serde_json::from_value(v).unwrap();
        assert_eq!(s, Settings::default());
    }

    /// Settings files written while user prices existed (removed 2026-11-07) still load.
    #[test]
    fn old_files_with_user_prices_still_load() {
        let s: Settings = serde_json::from_str(
            r#"{"prices": {"x": {"input": 1.0, "output": 2.0, "cache_write": 1.0, "cache_read": 0.1}}, "refresh_minutes": 7}"#,
        )
        .unwrap();
        assert_eq!(s.refresh_minutes, 7);
    }

    /// Files written before projects existed load with an empty list; projects roundtrip.
    #[test]
    fn older_files_without_projects_still_load() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert!(s.projects.is_empty());

        let mut v = serde_json::to_value(Settings::default()).unwrap();
        assert!(v.as_object_mut().unwrap().remove("projects").is_some());
        let s: Settings = serde_json::from_value(v).unwrap();
        assert_eq!(s, Settings::default());

        let mut s = Settings::default();
        crate::projects::add(&mut s.projects, "App", Path::new("C:\\work\\app")).unwrap();
        s.projects[0].hidden = true;
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    /// Files written before updates existed load with the defaults; a partial block fills in.
    #[test]
    fn older_files_without_updates_still_load() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.updates, UpdateSettings::default());
        assert_eq!(s.updates.mode, UpdateMode::Manual);

        let mut v = serde_json::to_value(Settings::default()).unwrap();
        assert!(v.as_object_mut().unwrap().remove("updates").is_some());
        let s: Settings = serde_json::from_value(v).unwrap();
        assert_eq!(s, Settings::default());

        let s: Settings = serde_json::from_str(r#"{"updates": {"mode": "Auto"}}"#).unwrap();
        assert_eq!(s.updates.mode, UpdateMode::Auto);
        assert!(s.updates.last_check.is_none() && s.updates.last_seen_version.is_empty());

        let mut s = Settings::default();
        s.updates.mode = UpdateMode::Auto;
        s.updates.last_check = Some(Utc::now());
        s.updates.last_seen_version = "0.1.0".into();
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn older_files_without_stats_choices_still_load() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.stats.breakdown, StatsBreakdown::Models);
        assert_eq!(s.stats.metric, StatsMetric::Tokens);

        let mut v = serde_json::to_value(Settings::default()).unwrap();
        assert!(v.as_object_mut().unwrap().remove("stats").is_some());
        let s: Settings = serde_json::from_value(v).unwrap();
        assert_eq!(s, Settings::default());

        let mut s = Settings::default();
        s.stats.breakdown = StatsBreakdown::Days;
        s.stats.metric = StatsMetric::Cost;
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""breakdown":"days""#) && json.contains(r#""metric":"cost""#));
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    /// Files written before the Overview's "Not for me" load with no hidden tips; hidden tips
    /// roundtrip.
    #[test]
    fn older_files_without_dismissed_tips_still_load() {
        let s: Settings = serde_json::from_str(r#"{"refresh_minutes": 7}"#).unwrap();
        assert!(s.dismissed_tips.is_empty());
        assert_eq!(s.refresh_minutes, 7);

        let mut v = serde_json::to_value(Settings::default()).unwrap();
        assert!(
            v.as_object_mut()
                .unwrap()
                .remove("dismissed_tips")
                .is_some()
        );
        let s: Settings = serde_json::from_value(v).unwrap();
        assert_eq!(s, Settings::default());

        let s = Settings {
            dismissed_tips: vec!["warm-up".into(), format!("downgrade:{}", Uuid::new_v4())],
            ..Settings::default()
        };
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn tools_and_billing_roundtrip() {
        let id = Uuid::new_v4();
        let mut s = Settings::default();
        s.tools.terminal = Terminal::PowerShell;
        s.tools.codex_exe = Some(PathBuf::from("C:\\bin\\codex.exe"));
        let date = chrono::NaiveDate::from_ymd_opt(2026, 12, 1).unwrap();
        s.billing.insert(id, MonthlyBilling::new(date));
        s.billing_reminders
            .insert(id, ReminderMark { date, days: 7 });
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        assert_eq!(
            back.tools.exe_for(ProviderKind::Codex),
            s.tools.codex_exe.as_deref()
        );
        assert_eq!(back.tools.exe_for(ProviderKind::Claude), None);
    }
}

#[cfg(test)]
mod taskbar_tests {
    use super::*;

    #[test]
    fn old_taskbar_modes_still_load() {
        for (old, new) in [
            ("Mix", TaskbarMode::Bars),
            ("Blocks", TaskbarMode::Bars),
            ("Table", TaskbarMode::Badge),
            ("Numbers", TaskbarMode::Badge),
            ("Focus", TaskbarMode::Badge),
            ("Off", TaskbarMode::Off),
        ] {
            let s: Settings =
                serde_json::from_str(&format!(r#"{{"display": {{"taskbar_mode": "{old}"}}}}"#))
                    .unwrap();
            assert_eq!(s.display.taskbar_mode, new, "{old}");
        }
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.display.taskbar_accounts, TaskbarAccounts::Favorites);
    }

    /// A file written before the settings were distilled (2026-11-07): density, reset-time
    /// format, running-out threshold, tray metric and the removed taskbar styles all load, the
    /// removed fields are dropped and the style becomes Badge.
    #[test]
    fn files_with_removed_display_settings_still_load() {
        for mode in ["Numbers", "Focus"] {
            let json = format!(
                r#"{{"refresh_minutes": 10, "display": {{
                    "usage_style": "Used",
                    "running_out_threshold": 40,
                    "reset_format": "Exact",
                    "density": "Compact",
                    "tray_metric": {{"Window": {{"account": "{}", "key": "five_hour"}}}},
                    "taskbar_mode": "{mode}",
                    "taskbar_count": 2
                }}}}"#,
                Uuid::new_v4()
            );
            let s: Settings = serde_json::from_str(&json).unwrap();
            assert_eq!(s.refresh_minutes, 10);
            assert_eq!(s.display.usage_style, UsageStyle::Used);
            assert_eq!(s.display.taskbar_mode, TaskbarMode::Badge, "{mode}");
            assert_eq!(s.display.taskbar_count, 2);
            let out = serde_json::to_string(&s).unwrap();
            for gone in [
                "running_out_threshold",
                "reset_format",
                "density",
                "tray_metric",
            ] {
                assert!(!out.contains(gone), "{gone} is written again");
            }
        }
        // The other tray metric shapes and the old defaults load as well.
        for extra in [
            r#""tray_metric": "LowestPinned", "density": "Comfortable""#,
            r#""tray_metric": "LowestAll", "reset_format": "Relative""#,
        ] {
            let s: Settings =
                serde_json::from_str(&format!(r#"{{"display": {{{extra}}}}}"#)).unwrap();
            assert_eq!(s.display, DisplaySettings::default());
        }
    }

    #[test]
    fn monitors_offsets_and_hover_defaults() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.display.taskbar_monitors, TaskbarMonitors::Main);
        assert!(s.display.hover_details);
        let s: Settings =
            serde_json::from_str(r#"{"display": {"taskbar_monitors": {"Chosen": ["a", "b"]}}}"#)
                .unwrap();
        assert_eq!(
            s.display.taskbar_monitors,
            TaskbarMonitors::Chosen(vec!["a".into(), "b".into()])
        );
    }

    #[test]
    fn per_monitor_offsets_fall_back_to_the_legacy_main_offset() {
        let mut d = DisplaySettings {
            taskbar_offset: Some(40.0),
            ..Default::default()
        };
        assert_eq!(d.taskbar_offset_for("main", true), Some(40.0));
        assert_eq!(d.taskbar_offset_for("other", false), None);
        d.set_taskbar_offset("other", false, 12.0);
        assert_eq!(d.taskbar_offset_for("other", false), Some(12.0));
        assert_eq!(d.taskbar_offset, Some(40.0));
        d.set_taskbar_offset("main", true, -5.0);
        assert_eq!(d.taskbar_offset_for("main", true), Some(0.0));
        assert_eq!(d.taskbar_offset, None);
        d.reset_taskbar_offsets();
        assert_eq!(d.taskbar_offset_for("other", false), None);
    }
}
