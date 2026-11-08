# Changelog

Patch notes shown after an update (embedded in the exe). Each release gets `## X.Y.Z – date` and 3-6 short bullets. Work in progress goes under `## Unreleased` and is renamed when a version is released (`scripts/release.ps1` refuses a version without a section).

## Unreleased

- Statistics and Settings are now one window with a sidebar: Insights (Overview and a page per topic), Data (Limits now, Tokens & cost) and Settings. Statistics opens it on the Overview, Settings on its pages.
- Statistics opens on a new Overview: one sentence on how much of your plans you use, four key numbers, the most useful tip, and short answers for Plans, 5-hour windows, Renewals and Where usage goes, each with its own page.
- The 5-hour windows section explains how windows and resets work, draws one of your days as it went and with an earlier first window, and shows the last 7 days of windows per account.
- Warm-up: SubHub can start your 5-hour windows at a set time on chosen days with a one-word prompt, so the first reset comes when you usually run out. Off until you turn it on: "Set it up…" opens a dialog filled in with the suggested time and accounts, which you confirm or change. It skips accounts whose window is already running, can wake a sleeping PC for it (your choice), and "Start a window now" does it right away.
- Optional heads-up (Settings → General, off by default): a notification about 15 minutes before an account's 5-hour window runs out, when another account of the same provider still has a fresh one. Clicking it opens SubHub. It stays quiet for hub accounts while hub steering is on.
- Plans are remembered across restarts, and the Overview's "Now" bars follow "Show usage as".
- Tips you don't want can be hidden with "Not for me" (and brought back with "Show hidden tips"). The former tabs are now "Limits now" and "Tokens & cost" in the sidebar.
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
