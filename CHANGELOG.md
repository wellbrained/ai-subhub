# Changelog

Patch notes shown after an update (embedded in the exe). Each release gets `## X.Y.Z – date` and 3-6 short bullets. Work in progress goes under `## Unreleased` and is renamed when a version is released (`scripts/release.ps1` refuses a version without a section).

## 0.1.6 – 2026-11-08

- New font: SubHub now uses Geist, built into the app, so it looks the same on every PC. The taskbar badges keep Windows' own font.
- Hovering the tray icon shows its card in the same font as the rest of the app.
- The README screenshots show the new font.

## 0.1.5 – 2026-11-08

- Settings pages use the full width of the main window, lined up with the Insights pages.
- The taskbar hover card also shows model limits such as Claude's weekly Fable limit, after the 5-hour and weekly rows.
- Fixed Codex accounts showing dashes on the taskbar: their 5-hour and weekly windows are now recognised everywhere (badges, hover card, hub steering, run-out heads-up, charts).
- The README shows the taskbar badges and their hover card.

## 0.1.4 – 2026-11-08

- New logo: a ring of three accounts around one hub. The tray icon follows a dark or light taskbar, and the logo now also sits in the popover header.
- Opening Renewals or Where usage goes from the Overview jumps straight to that part of the page; other pages start at the top.
- Fixed tooltips of threads with the same name, and the README now shows the new main window.

## 0.1.3 – 2026-11-08

- One main window for Statistics and Settings, with a sidebar: Insights (Overview, Plans & renewals, Usage patterns), Data (Limits now, Tokens & cost) and Settings.
- New Overview: how much of your plans you use, key numbers, tips you can hide, and short answers on plans, renewals, 5-hour windows and where your usage goes.
- 5-hour windows explained with your own days, plus an optional Warm-up that starts windows early at a time you choose (off until you set it up; it can wake a sleeping PC if you allow it).
- Optional heads-up before a 5-hour window runs out, when another account still has a fresh one (Settings → General, off by default).
- Choose where new hub threads go from each account card's ⋯ menu; long lists scroll after 5 rows and cut-off names show in full on hover.
- Demo mode (`--demo`) to try SubHub on made-up accounts without touching your own.

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
