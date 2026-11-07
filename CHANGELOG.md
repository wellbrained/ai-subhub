# Changelog

Patch notes shown after an update (embedded in the exe). Each release gets `## X.Y.Z – date` and 3-6 short bullets. Work in progress goes under `## Unreleased` and is renamed when a version is released (`scripts/release.ps1` refuses a version without a section).

## Unreleased

- Demo mode: start with `--demo` to try SubHub (or take screenshots) on made-up accounts and usage; it runs next to your real SubHub and never touches your accounts, hub, settings or startup.
- Choose where new hub threads go from the ⋯ menu on each account card (Use for new hub threads / Back to Auto); the Hub badge now only shows the state. The menu also opens the account's settings.

## 0.1.2 – 2026-11-07

- After an update, the patch notes no longer pop up at startup; they appear as a small dialog the next time you open the popover.
- Simpler Settings: six fixed sections (General, Appearance, Accounts, Hub, Projects, About & updates), one Accounts list with Add accounts, and rarely used options tucked under Advanced / More options.
- Removed options that had no real choice: tray icon metric, density, reset time format, running-out level (now 25%) and the Numbers / Focus taskbar styles; hover a reset time for the exact date. The price table is now behind the ⓘ on "API estimate" in Statistics.

## 0.1.1 – 2026-11-07

- Statistics redesigned: work tokens, cache hit rate, month run-rate, cost by type and a Models / Providers / Days breakdown with a daily chart per provider.
- Statistics and Settings get their own title bar in the app's colours.
- Detect / Import finds your Claude and Codex CLI profiles and imports the ones you pick, without duplicates.
- Taskbar display: a clear on/off switch, Badge style by default, per-monitor placement and hover cards on the badges.
- New app icon; simpler tray icon whose tooltip is just the app name.
- Codex is blue in the statistics; the Days table shows the last week and scrolls for older days.

## 0.1.0 – 2026-11-07

- Tray popover with every Claude and Codex account: remaining allowance per window, reset countdowns, stale and cooldown states.
- Taskbar display (badge, bars, numbers or focus) and a live tray tooltip.
- CLIProxyAPI hub support: pooled logins, and steering new threads to the account that resets first.
- Statistics window with local token usage and an API cost estimate, plus a live limits board.
- Launch menu per account: open saved project folders in Windows Terminal, PowerShell, VS Code or the desktop app; sign-in and retry from the popover.
- Monthly billing reminders, themes and accent colour, Start with Windows.
