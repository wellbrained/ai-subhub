//! Demo mode (`ai-subhub --demo`, owner 2026-11-07): the real app on made-up data that touches
//! nothing real, for README screenshots and UI tests. [`enter`] wipes and reseeds the sandbox
//! `%TEMP%\AI SubHub Demo` (fixtures: `subhub_core::demo`) and points the user folder and
//! `LOCALAPPDATA` at it before anything reads a path. The flag then makes refreshes use the
//! fixtures instead of providers and the hub, steering write nothing, and keeps the hub key,
//! Start with Windows, updates, sign-in and Launch from doing anything outside the process.
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use gpui::{IntoElement, ParentElement, Styled, div, px};
use subhub_core::launch::{DesktopApps, LaunchEnv};
use subhub_core::model::ProviderKind;

use crate::theme::Tokens;

/// The sandbox root once demo mode is on.
static SANDBOX: OnceLock<PathBuf> = OnceLock::new();

/// Note for Sign in and Launch items in the demo.
pub const NOTHING_STARTED: &str = "Demo: nothing was started";
/// Hint for settings that would change something outside the demo.
pub const NOT_AVAILABLE: &str = "Not available in the demo.";

pub fn on() -> bool {
    SANDBOX.get().is_some()
}

/// Screenshot helpers (`--demo-open-row`, `--demo-height`), set once at start; ignored without
/// `--demo`.
static OPEN_ROW: OnceLock<String> = OnceLock::new();
static STATS_HEIGHT: OnceLock<f32> = OnceLock::new();
static HIDDEN: OnceLock<bool> = OnceLock::new();
static FONT: OnceLock<String> = OnceLock::new();
static FONT_FILE: OnceLock<String> = OnceLock::new();

/// `--demo-font NAME`: font family for the popover and hover cards (font comparison shots).
pub fn font() -> Option<&'static str> {
    FONT.get().map(String::as_str)
}

/// `--demo-font-file PATH`: a font file to load first (fonts not installed on the PC).
pub fn font_file() -> Option<&'static str> {
    FONT_FILE.get().map(String::as_str)
}

/// `--demo-hidden`: screenshots without touching the desktop (owner, 2026-11-07: demo runs on
/// the PC he works on). No tray icon, balloons, taskbar display or hover cards; windows open far
/// off-screen, never activated and without a taskbar button (`PrintWindow` still captures them).
pub fn hidden() -> bool {
    HIDDEN.get().copied().unwrap_or(false)
}

/// Where hidden windows are placed (physical px), far outside any monitor.
pub const OFF_SCREEN: i32 = -20000;

/// Remembers the screenshot flags from the start arguments (only in demo mode).
pub fn read_flags(args: &[String]) {
    if !on() {
        return;
    }
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1).cloned())
    };
    let _ = HIDDEN.set(args.iter().any(|a| a == "--demo-hidden"));
    if let Some(f) = value("--demo-font") {
        let _ = FONT.set(f);
    }
    if let Some(f) = value("--demo-font-file") {
        let _ = FONT_FILE.set(f);
    }
    if let Some(row) = value("--demo-open-row") {
        let _ = OPEN_ROW.set(row);
    }
    if let Some(h) = value("--demo-height").and_then(|h| h.parse::<f32>().ok()) {
        let _ = STATS_HEIGHT.set(h.clamp(480.0, 2000.0));
    }
}

/// The Overview row (`plans`, `windows`, `renewals`, `usage`) or `tips` the Statistics window
/// opens with.
pub fn open_row() -> Option<&'static str> {
    OPEN_ROW.get().map(String::as_str)
}

/// The Statistics window's opening height.
pub fn stats_height() -> Option<f32> {
    STATS_HEIGHT.get().copied()
}

/// `--demo-theme light|dark`: the theme the demo starts with (None: as seeded, System).
pub fn theme(args: &[String]) -> Option<subhub_core::settings::ThemeMode> {
    use subhub_core::settings::ThemeMode;
    if !on() {
        return None;
    }
    let i = args.iter().position(|a| a == "--demo-theme")?;
    match args.get(i + 1)?.as_str() {
        "light" => Some(ThemeMode::Light),
        "dark" => Some(ThemeMode::Dark),
        _ => None,
    }
}

/// Sets demo mode up. Must run first in `main`, while the process has a single thread (it
/// changes environment variables). An error means no demo: the caller must exit rather than
/// run on the real data.
pub fn enter() -> anyhow::Result<()> {
    let temp = std::env::temp_dir();
    let root = subhub_core::demo::sandbox_dir(&temp);
    subhub_core::demo::reset(&root, &temp)?;
    let home = root.join("home");
    let local = root.join("local");
    // SAFETY: called at the very start of `main`, before GPUI or any other thread exists.
    unsafe {
        std::env::set_var("USERPROFILE", &home);
        std::env::set_var("HOME", &home);
        std::env::set_var("LOCALAPPDATA", &local);
        // These would point profile detection, the statistics scan and launches at real folders.
        for var in ["CLAUDE_CONFIG_DIR", "CODEX_HOME", "XDG_CONFIG_HOME"] {
            std::env::remove_var(var);
        }
    }
    subhub_core::demo::seed(&root, chrono::Utc::now(), env!("CARGO_PKG_VERSION"))?;
    let _ = SANDBOX.set(root);
    Ok(())
}

/// "AI SubHub", or "AI SubHub (demo)" for the tray tip.
pub fn app_name() -> &'static str {
    if on() {
        "AI SubHub (demo)"
    } else {
        "AI SubHub"
    }
}

/// Tray balloon titles in the demo start with "AI SubHub (demo) · ", so a made-up billing
/// reminder never reads as a real notice (owner, 2026-11-07).
pub fn balloon_title(title: &str) -> String {
    if on() {
        format!("AI SubHub (demo) · {title}")
    } else {
        title.to_string()
    }
}

/// A path for display: sandbox paths show as `C:\Users\demo\…`, so screenshots never carry the
/// real temp folder (and with it the user name).
pub fn show_path(p: &Path) -> String {
    SANDBOX
        .get()
        .and_then(|root| subhub_core::demo::display_path(p, root))
        .unwrap_or_else(|| p.display().to_string())
}

/// The CLI the demo pretends to have (never started).
pub fn fake_cli(provider: ProviderKind) -> Option<PathBuf> {
    let root = SANDBOX.get()?;
    let name = match provider {
        ProviderKind::Claude => "claude.exe",
        ProviderKind::Codex => "codex.exe",
    };
    Some(root.join("home").join(".local").join("bin").join(name))
}

/// Everything "installed", so every Launch item is enabled (and does nothing).
pub fn launch_env(provider: ProviderKind) -> LaunchEnv {
    LaunchEnv {
        cli: fake_cli(provider),
        windows_terminal: Some(PathBuf::from("wt.exe")),
        vscode: Some(PathBuf::from("code.cmd")),
        desktop_app: Some(
            desktop_apps()
                .for_provider(provider)
                .unwrap_or("demo")
                .to_string(),
        ),
    }
}

pub fn desktop_apps() -> DesktopApps {
    DesktopApps {
        claude: Some("Claude".into()),
        codex: Some("OpenAI.Codex".into()),
    }
}

/// A small, quiet "Demo" pill next to a window title (popover header, title bars), so a
/// screenshot is never mistaken for real data.
pub fn pill(t: &Tokens) -> impl IntoElement {
    div()
        .flex_none()
        .px(px(6.0))
        .py(px(1.0))
        .rounded_full()
        .border_1()
        .border_color(t.border)
        .text_xs()
        .text_color(t.muted)
        .child("Demo")
}
