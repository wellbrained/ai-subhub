# Delivery checks

## 0.13.2 local Windows delivery (2026-10-24)

- Release solution build: zero warnings/errors. All 102 tests pass (87 core, 15 app).
- Red/green regression: synthetic Claude usage plus live profile Max 5× initially displayed the saved Pro label; now the live profile wins. Optional profile authorization/rate-limit/malformed/oversized failures retain valid usage. Codex rate-limit plan beats cached identity metadata.
- Controller checks: plan visible before Changed, same-known-identity fallback, sign-out/account-switch clearing, billing edits during in-flight usage reads.
- Billing checks: optional legacy-compatible settings, monthly anchor/no drift, leap years, fixed access end, seven-/three-day catch-up and persistent deduplication across restart/removal. The date editor preserves the original monthly anchor when saving an unchanged projected date.
- Isolated native Windows test: long allowance and unavailable-reset labels wrap at the unchanged 620px width, account rename retains billing, date picker loads correctly, and a reminder window remains visible with the main window hidden.
- Demo-only dark main window, dark/light billing editor and reminder captures inspected under artifacts/subscription-checks. No real credentials, provider responses or financial dates were used in tests/captures.
- Self-contained Windows 0.13.2 portable build/ZIP produced; packaged reminder smoke passed and the running local 0.13.1 process was replaced with 0.13.2. Production pointer and startup script accept patch releases. Public GitHub downloads remain 0.13.0; no commit/push/release. Native Mac behavior remains pending.


## v0.3.0 delivery checks

Date: 2026-10-15. Target: this Windows x64 PC. Development SDK: .NET 10.0.402, installed locally under `.tools/dotnet`; UI packages: Avalonia 12.1.4.

- Release solution build: zero warnings/errors.
- Core test suite: 15 passed. Existing quota/auth/config checks plus manual project order, hiding/showing without dropping metadata, persistence/reload, old settings without a visibility field, and safely encoded Claude local-folder links.
- Live read-only Codex profile check: Ready, two allowance windows. Uses app-server without creating threads or model requests.
- Live read-only Claude profile check: Ready, two allowance windows. Uses selected existing subscription credentials in memory; no credentials copied to Hub settings or shown in reports.
- Packaged self-contained executable: rendered dark usage window and light Accounts settings window, exited successfully in isolated demo mode. Visual review confirms visible remaining percentages, reset times, last refresh state, scrolling and left-category/single-column settings.
- Lifecycle checks passed: tray startup hides the window, closing keeps the app alive, reopening restores the window, and launching a second instance brings the existing instance forward. A startup/activation race caught by this check was corrected before delivery.
- Portable output: executable, README and MIT license, plus ZIP. Native debug symbols excluded. The app runs from the self-contained output without a globally installed .NET runtime.
- v0.2 refinement: renamed AI SubHub; compact rows measured at 82 logical pixels for two quota windows, down from roughly 250. Three demo accounts fit inside the default 740 × 460 window. Dark/light render inspection confirms persistent Add account buttons and inline quota rows. Each account has four launch choices: Windows Terminal, PowerShell, VS Code project folder and its matching desktop app. Installed Claude/Codex desktop apps were detected through Windows registration. Desktop options retain their own login and do not apply the selected CLI profile.

Interactive provider login, launches into real AI sessions/VS Code, Windows logon startup and operation through an actual quota exhaustion/reset have not been exercised end-to-end. These are implemented and require user review in their own workflow; no real account was signed out, no API-key billing was used, and Windows startup remains opt-in. macOS/Linux are outside this delivery.

## v0.3 redesign

- Baseline reproduced the oversized settings screen: 800 × 650. New settings is 600 × 320, with flat 39-pixel account rows, narrow navigation and compact outline buttons. All three demo accounts fit together; no cards or repeated path paragraphs.
- Accounts, Projects, General, Tools and About were rendered and checked for row visibility and content fit at the default settings size. Dark and light account screens were visually inspected.
- Account edit dialog rendered at 460 × 149. Names/profile paths remain editable; sign-in/removal actions are available through the account row menu.
- Main window reduced to 740 × 420 by removing the global project picker. Three usage accounts still fit. Launch menus contain visible projects in stored order, each with four launch targets; a synthetic hidden project is excluded but remains in Settings.
- Project preferences survive save/reload. Existing settings default all old projects to visible. Hiding does not remove files or shortcuts, and showing restores the stored position.
- Claude chat Projects integration was explicitly deferred. No private app/session storage is accessed. Claude Desktop's local Code-folder launch is a separate documented deep link. Codex Desktop folder selection remains a labelled manual step.

Ignored `artifacts/checks` holds sanitized local smoke reports and demo PNGs, not account identities or credentials. Source remains uncommitted on `codex/windows-mvp`; there is no remote or public release.

## 0.4.0 — 2026-10-15

- Release build: zero warnings/errors; all 21 tests passed. Added tests for within-provider movement across interleaved records, boundary controls, unchanged profile metadata after settings reload, lowest allowance/reset pairing, stale/failed/expired readings, safe unavailable states and bounded tooltip overflow.
- Published self-contained Windows x64 EXE and portable ZIP.
- Isolated actual TrayIcon tooltip report: three demo accounts, 121 UTF-16 characters; shows lowest remaining percentages and corresponding countdowns.
- Accounts settings visual/report: 600×320, three 39px rows; all rows/content fit with up/down controls. Boundary arrows disabled per provider. Main window remains 420px high with all three 82px demo account rows fitting.
- Synthetic smoke reports do not read real credentials or alter real settings.

## 0.5.0 — 2026-10-15

- Release build: zero warnings/errors, all 27 tests passed. Plan tests cover Claude Pro/Max tiers, unknown values, Codex base64url plan claims, malformed tokens and missing profiles.
- Main and Accounts settings screenshots verified compact plan tags immediately after names. Main remains 740×420 with three 82px rows; Settings remains 600×320 with three 39px rows, all rows/content fit.
- No extra provider requests or token persistence introduced. Plan metadata is a saved-login label, not a live billing assertion.

## 0.6.0 — 2026-10-16

- Root configuration check failed before the fix: AI SubHub Live=false, prodPort=null, devPort=null, and no scripts/start-server.ps1. No AI.SubHub process after the owner's reboot. CoreHub's logon task was enabled/running.
- Added a production-only adapter and in-process loopback health endpoint. Build has zero warnings/errors; all 28 tests pass, including read-only health routing and endpoint shutdown with process lifetime.
- Authoritative CoreHub project #14 configured Live=true, prodPort=4200 (next available CoreHub production port), devPort=null. No CoreHub source changes or new scheduled tasks.
- CoreHub authenticated companion prod-start launched the production 0.6.0 EXE with --tray --health-port 4200 in the user's interactive session (session 1). GET /api/health returned app/version/status.
- CoreHub prod-stop removed both the tray process and listening port; subsequent prod-start/prod-restart left exactly one healthy production process. The local direct /servers/prod/start route is intentionally blocked by CoreHub's production-data middleware; used its supported authenticated companion operations without exposing credentials.
- Standalone Windows Run registration removed, StartWithWindows=false; startup owner shown as CoreHub. Existing account/project metadata preserved.
- Checked CoreHub's enabled logon trigger and autostart configuration. No forced reboot; actual reboot confirmation remains for the next normal PC restart.

### Correction: native AppData verification and recovery

The earlier metadata-preservation statement compared files through Codex package redirection and missed the native empty settings. A detached native WMI check reproduced the precise issue: native AppData had 0 accounts/0 projects; Codex LocalCache had 3/4. Stopped only AI SubHub through CoreHub, backed up the empty native file, restored all metadata and disabled standalone startup, leaving the original source intact. A native production UI smoke rendered all 3 accounts (82px rows, all fit) with successful Claude/Codex usage; it retained four projects, including two hidden entries, with original order. Restarted through CoreHub, health reports 0.6.0, and opened its window for review. Configuration recovery only; no new application build/version.

## 0.7.0 — 2026-10-16

- Completed AI SubHub naming across GitHub repository/local folder, solution, manifest identity, mutex/events, startup key, protocol client, demo paths and current documentation. Old-name literals only remain in legacy migration/cleanup and their test fixture.
- Build: zero warnings/errors, 36 tests passed. Migration test checks account/project identity, hidden state, preserving the old settings file and never replacing existing new settings. Quota label tests preserve custom/model-specific buckets.
- Native Windows production verification: settings copied to the new AI SubHub AppData directory; 3 accounts and 4 projects preserved. Compact main layout is 620×404 client pixels with three 82px rows and all accounts fitting. Visual check confirms identical 5-hour/Weekly labels for Claude and Codex.
- General screenshot confirms the enabled Start with Windows checkbox and content fitting 600×320. The native HKCU AISubHub startup command points to the renamed production 0.7.0 EXE with --tray; legacy startup value is absent. App running outside Codex package virtualization with native saved settings.
- Per owner's revised request, CoreHub only keeps project record #14 at the renamed path: Live=false and both ports unset; it no longer owns app startup. No dev instance.
- GitHub repository is private at https://github.com/wellbrained/ai-subhub. No forced Windows reboot.

## 0.8.0 — 2026-10-16

- Build: zero warnings/errors; all 47 tests pass. Added coverage for missing/invalid reset metadata vs explicit zero, provider count vs capped detail rows, expiry selection, and safe plain email handling.
- Live read-only checks: both Claude profiles and Codex return signed-in email (report records booleans only). Codex currently returns available reset count=0. Claude manual reset availability stays unknown through this connection. No reset redemption endpoint is called.
- Demo and native UI: 620×404 default, three 82px account rows all fit; reset badge adds no row/window height. Native settings/account/project metadata preserved (3 accounts, 4 projects), email/reset runtime metadata absent from persisted quota cache.
- Native restart updates the existing Windows startup entry to 0.8.0. No CoreHub startup dependency. Changes remain local pending a new push request.

## 0.9.0 — 2026-10-16

- Build: zero warnings/errors; all 50 tests pass. Added checks for complementary displayed percentage/bar values, matching tray style using the same constrained quota bucket/reset, and persisted preference with old/unknown settings defaulting to Remaining.
- Remaining/Used demo screenshots visually verified at 620×404, three 82px rows fit. Progress gaps reduced to 4px, label column 56px, content-sized reset column right aligned with smaller row side padding. No larger window.
- General screenshot/report: five preference rows, tallest 40px, all content fits 600×320. Changing Usage display saves immediately and the controller notifies main/tray views.
- Unknown manual-reset information removed from account tooltips entirely; known counts including zero remain displayed.
- Native upgrade preserves 3 accounts/4 projects, defaults to Remaining and refreshes the existing Windows startup path to 0.9.0. No commit/push requested; changes remain local.

## 0.10.0 — 2026-10-16

Fixed 56px label, flexible bar, 92px percentage and 108px reset columns, with left-aligned text and 12px gap after the bar. Remaining/Used visual checks confirm identical column starts and bar endpoints across all accounts and quota rows. Both fit 620×404 with unchanged 82px account rows. Build has zero warnings/errors; 50 tests pass. Native restart preserves account/project metadata and usage style, and updates Windows startup to 0.10.0. Changes remain local.

## 0.11.0 macOS test-build milestone — 2026-10-16

Windows v0.10 was pushed to main (22b48d3) before Mac work. Mac implementation is local on codex/macos; the installed Windows v0.10 executable and startup entry were not replaced.

- Shared Release build: zero warnings/errors; 56 tests pass. Added coverage for exact Keychain hash namespace, default/custom profile environment alignment, Unicode handling, POSIX/AppleScript quoting, escaped login-agent arguments, and reconnectable current-user activation.
- Cross-published self-contained osx-arm64 and osx-x64, packaged as unsigned .app ZIPs. Inspected Mach-O architectures, matching executable/version/minimum-OS/LSUIElement bundle metadata, archive CRCs and Unix creator/executable0755 modes. Both generated apphosts retain valid ad-hoc SHA-256 code pages; no Developer ID signature or notarization was performed. No credential/settings files in packages.
- Isolated Windows demo regression: Remaining/Used both fit 620×404 with 3×82px accounts and existing nested launch targets; General content fits600×320. Tray configured, second-instance activation received, close hides and reopen succeeds. Visual PNGs inspected.
- No real Mac is available yet. Keychain permission/scoping, live provider reads onMac, Terminal automation, desktop targets, native menu-bar/reopen, and login startup remain pending against docs/features/macos.md. Cross-build checks do not claim native runtime success.
- Mac scope/decisions and docs mirrored to the existing vault project via disk; Obsidian MCP unavailable. No signing account, GitHub workflow or public release created.

## 0.12.0 — GitHub Mac test release, 2026-10-16

Release-only minor bump requested by the owner: rebuild both Mac architectures, run local Release checks and package validation, then publish source and both ZIPs with checksums as a GitHub prerelease. The 0.11 native Mac testing limitations continue to apply. Installed Windows v0.10 and its startup configuration are preserved.
Local v0.12 checks completed: 56 tests pass, zero warnings/errors; both rebuilt archives pass version/architecture/bundle/CRC/executable-mode/ad-hoc-code-hash checks. SHA-256 checksums generated for the release assets.

## 0.13.0 — Sign-in refresh fix, 2026-10-16

Reproduced Claude-on-Windows symptom with a synthetic CLI completing successfully while the original terminal command produced no completion signal (red regression test). Root cause: sign-in launched a detached/remaining-open terminal without a success-to-refresh handoff; ordinary authentication-failure backoff could then delay new usage, and an active-refresh guard could drop an immediate retry.

Fix adds a credential-free per-login exit-code handoff and targeted controller refresh after success; active reads are awaited, ordinary backoff bypassed and known provider cooldown retained. Full terminal/controller regression uses synthetic CLI data and no real login. Installed Windows build is updated with native storage preservation checks; Mac native validation remains pending. The owner subsequently requested publication because the missing refresh handoff affects other users too.
Verification complete: 62 tests pass (57 core +5 controller), zero warnings/errors. Native v0.13 is visible/running; all4 accounts and4 projects plus preferences match the pre-upgrade metadata hash, startup points to0.13, and the newly added account has a fresh successful cached reading. Demo layout remains620×404 with82px rows. A native settings backup was retained. Source publication requested after local verification.
Download publication requested: v0.13.0 includes Windows x64 plus Apple Silicon/Intel Mac ZIPs, rebuilt from the same fix with SHA-256 checksums. Windows verification and the existing Mac native-validation limitations remain applicable.
Release packaging verified: 62 tests pass with zero warnings; Windows ZIP CRC/version/x64 executable and demo layout/tray lifecycle pass. Both Mac ZIPs pass CRC, version/architecture/plist, Unix0755 and generated apphost ad-hoc-code-hash checks. Release SHA256SUMS covers all three packages.

## Window placement — 2026-10-19 (local)

Native Windows regression reproduced the fixed-default startup: a resized 760×470 window reopened at 620px wide. After the fix, the same test passes normal position/size restoration, maximized restart, tray hide/reopen, normal/maximized minimization and retention of normal bounds. Native UI tests run in an isolated process because Avalonia's UI dispatcher is process-wide. Synthetic settings tests verify roundtrip/backward compatibility and preservation of existing account/preferences; monitor/scaling tests verify fallback into an available working area.

`build.ps1 -Task Check`: 65 tests pass (58 core, 7 app), zero build warnings/errors. No deployment, version bump, commit or publication. Native Mac placement/full-screen validation remains pending.
Demo lifecycle smoke also passes: tray configured, close hides, and Open restores visibility.

## 0.13.1 — Local Windows replacement, 2026-10-19

Built and installed the window-placement patch locally on request. Full Release build: zero warnings/errors; all 65 tests pass. Packaged the self-contained Windows x64 executable and ZIP. Version identifiers in build properties, manifest, UI, health response and provider clients match 0.13.1; README distinguishes current source from the published 0.13.0 downloads.

Native storage backup retained before replacement. Exactly one responsive 0.13.1 instance is running with its main window visible. All four accounts, four projects and preferences pass the pre/post metadata hash comparison. Login startup points to the new executable, and native settings contain saved main-window placement. No commit, push or public release; native Mac validation remains pending.
