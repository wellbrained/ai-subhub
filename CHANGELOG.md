# Changelog

Patch notes shown after an update (embedded in the exe). Each release gets `## X.Y.Z – date` and 3-6 short bullets. Work in progress goes under `## Unreleased` and is renamed when a version is released (`scripts/release.ps1` refuses a version without a section).

## Unreleased

- Statistics redesigned: work tokens, cache hit rate, month run-rate, a Models / Providers / Days breakdown and a daily chart per provider.
- Statistics and Settings have their own title bar in the app's colours instead of the Windows caption bar.
- New app icon ("Night glow") and a simpler, sharper tray icon.
- The Days table shows the last week and scrolls for older days; Codex is blue in the statistics.
- Detect / Import: finds your Claude and Codex CLI profiles and imports the ones you pick, without duplicates. Offered on first start and in Settings.
- Settings → Taskbar: a clear "Show usage next to the tray" switch; off leaves just the tray icon and keeps your style.
- New installs start with the Badge style on the taskbar.
- Hover cards: resting on the tray icon or a taskbar badge shows a small card with both windows, % left, reset times and status (Settings → Taskbar → "Show details on hover").
- The taskbar display can show on the main taskbar, all taskbars or chosen monitors, with its own position per monitor.

## 0.1.0 – 2026-11-07

- Tray popover with every Claude and Codex account: remaining allowance per window, reset countdowns, stale and cooldown states.
- Taskbar display (badge, bars, numbers or focus) and a live tray tooltip.
- CLIProxyAPI hub support: pooled logins, and steering new threads to the account that resets first.
- Statistics window with local token usage and an API cost estimate, plus a live limits board.
- Launch menu per account: open saved project folders in Windows Terminal, PowerShell, VS Code or the desktop app; sign-in and retry from the popover.
- Monthly billing reminders, themes and accent colour, Start with Windows.
