# Windows MVP

## Profiles and configuration

One account record contains an ID, provider, editable label and absolute CLI profile path. A profile cannot be added twice for the same provider using the same normalized path. Labels are not verified identity. Edit supports renaming or changing the profile folder; changing folders clears its old reading. Remove disconnects the record and cached usage while retaining provider files. New profiles are created on request and signed in using the CLI in a visible terminal.

The JSON store is `%LOCALAPPDATA%\AI SubHub\settings.json`. On first use, existing legacy settings are copied if the new settings file is absent; the old file is retained and new settings are never overwritten. It contains metadata, project order/visibility, preferences and usage cache. Writes replace via a temporary file. Invalid settings are preserved in a timestamped unreadable backup. Profile credentials are never serialized into it. The optional `IsHidden` project field defaults to false for older settings; list order is the user's manual order. Legacy selected-project metadata remains readable, but no longer drives launching.

## Usage providers

`ClaudeUsageProvider` reads the selected profile's `.credentials.json` access token into memory and sends it only to `https://api.anthropic.com/api/oauth/usage`. Prefer self-describing `limits[]` including model scope; preserve inactive windows because the flag describes the binding limit, not its existence. Fall back to flat 5-hour, weekly and model-specific legacy windows. No refresh-token handling or CLI updates. A failed authorization asks for CLI sign-in.

`CodexUsageProvider` starts a short-lived app-server over private redirected stdio in the selected `CODEX_HOME`, initializes the protocol, reads account status and requests rate limits. No thread, inference turn or model request is created. All returned rate-limit buckets are parsed with their own durations; a legacy single bucket is the fallback. Authentication is delegated to Codex, including profile-derived keyring namespaces. API-key profiles show subscription usage unavailable.

Errors exclude raw provider responses and credentials. Ready readings replace the cache; failures preserve the previous snapshot and mark it stale. Overdue readings are also visibly stale. Missing/null/unrecognized quotas are unavailable, never zero. Percent remaining is the complement of provider percent used. Expired reset countdowns say a reset is due and require refresh to confirm capacity.

## Refresh and recovery

Default cadence is five minutes, adjustable 1–60. A 30-second scheduler checks per-account deadlines. Requests run sequentially and have a 35-second cancellation deadline. Concurrent refresh is suppressed. Failures exponentially increase retry delay; computed backoff is capped at 24 hours, while longer provider Retry-After values remain authoritative. Manual refresh skips ordinary cadence but respects known provider rate limits. Sign-in is an explicit terminal action. After its CLI exits successfully, Hub automatically refreshes that account; manual refresh remains available.

## Launching and settings

Each account's Launch menu first lists visible projects in manual order; each project opens a target submenu with Windows Terminal, PowerShell, VS Code and the matching desktop app. There is no global project selector. Manage projects opens the Projects settings category, even if the project list is empty or all projects are hidden. Hidden projects remain editable and keep their place in the full list. Up/down controls reorder the full list and therefore the filtered launch list. Showing a hidden project restores it at that position. Removing a shortcut leaves the folder/files untouched.

Explicit terminal choices override the saved sign-in terminal preference. CLI launches set `CLAUDE_CONFIG_DIR` or `CODEX_HOME` for the child session and clear inherited API-key/cloud-auth overrides to avoid accidentally preferring token billing. Profile config remains unchanged. Paths are encoded as escaped PowerShell literals. VS Code opens the selected folder without changing its extension login. CLI executables are detected on PATH/common local directories or set explicitly in Tools. Missing folders/applications have disabled launch items with explanatory hints.

Desktop apps are detected from Windows Start app/package registration, once at startup with a timeout. Claude's documented `claude://code/new?folder=...` link opens the chosen local folder in Desktop's Code view and asks the user to confirm it; the CLI profile does not switch Desktop's account. See [official Desktop links](https://support.claude.com/en/articles/14729294-open-claude-desktop-with-a-link). Codex can be registered under the ChatGPT application package. Windows folder navigation is not verified: the [CLI reference](https://learn.chatgpt.com/docs/cli/reference#codex-app) says Windows prints the path rather than opening it. Its menu explicitly says to choose the folder inside Desktop; the app displays that reminder after opening it.

The main window defaults to 620 × 404 logical pixels and keeps overflow in its account scroll area. Each compact account row has its name, last-refresh age, nested Launch menu and one line per allowance (label, progress, remaining percentage, relative reset). Full labels and absolute reset times are tooltips; errors and stale states stay explicit. Small text-only Add account buttons always sit beside Claude and Codex.

Settings defaults to 600 × 320, with a 100-pixel navigation column and one content column. Accounts/Projects are flat rows with separators, Edit buttons and an overflow menu, rather than cards. Account paths are in edit dialogs/tooltips; sign-in and removal are in the row menu. Projects additionally have up/down ordering and Show controls. General uses compact label/editor rows; Tools and About share the same sizing. Account/project editors use 460-pixel-wide dialogs with inline field labels. Errors appear only when needed; large lists scroll inside the window. General controls theme, opt-in per-user Windows Run startup, refresh interval and sign-in terminal. Startup records the current portable executable path and `--tray`; an already-enabled entry is refreshed when the executable runs.

Claude chat Projects are separate from local folders. On 2026-10-15 the owner deferred integration after research: consumer project-list sync has no documented interface; the project data API requires Enterprise compliance access. Official project deep links could support manual linked projects later. See [access requirements](https://platform.claude.com/docs/en/manage-claude/compliance-faq). No project URLs, app session cookies, remote project data or synchronization logic are added in v0.3.

## Lifecycle and verification

One Windows instance at a time. Main-window close hides it; tray click opens it; tray menu refreshes or exits. Exit cancels requests and disposes the tray and HTTP resources. The main window remains the tray fallback if Explorer hides the icon.

Core tests cover null/missing windows, multiple quota buckets and duration labels, new and legacy Claude schemas, retry/backoff, account rename/duplicate handling, child-environment auth isolation, settings roundtrip/recovery and safe authentication errors. `--demo` supplies synthetic readings without real settings/provider calls. `--demo --ui-smoke --snapshot <absolute-path>` renders the window and exits. `--smoke-test --provider Claude|Codex --profile <folder> --report <file>` performs a real read-only check but reports only provider state, safe message and number of windows.

Full checks/publishing run through `build.ps1`. Artifacts stay under ignored `artifacts/`; the ZIP includes a self-contained Windows x64 executable, README and MIT license. First delivery checks are recorded in `docs/validation.md`.

## Tray summary and account order (0.4)

Settings → Accounts has ↑/↓ controls within each provider group. The persisted Accounts list is the manual order; main rows, settings and tray summaries all use Claude then Codex and preserve each group's relative order. Reordering never changes IDs, labels, profile directories or readings. No schema migration is needed.

The tray tooltip updates on controller changes and its 30-second scheduler. Each line shows provider, compact account label, lowest remaining allowance and that same allowance's relative reset. Old/failed cached readings say stale; missing readings say waiting or their safe status. An expired reset says due without refilling the allowance. Labels have control characters removed and are shortened without splitting a UTF-16 surrogate pair. Windows tooltip text is limited to 127 UTF-16 characters; complete lines are retained with a +N more suffix when necessary. See [Microsoft NOTIFYICONDATAW](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ns-shellapi-notifyicondataw). The full account list remains in the main window.

Demo-only --tray-report writes the actual tooltip text and length during UI smoke verification. Real account labels are never written by that report.

## Subscription plan tags (0.5)

A compact tag follows each account name in the main window and Accounts settings. Plans are read from the selected profile's saved login metadata at startup, refresh and settings save. Claude uses subscriptionType and recognized Max rate-limit tiers; Codex decodes only the plan claim from its local ID token. This is display metadata, not token verification or a live billing check. The tooltip explains that plan changes may require a new login. Missing, malformed or unrecognized metadata displays Plan unknown; arbitrary provider strings are never shown. No additional network calls, stored credentials or settings schema changes. Plan tags are kept in memory and demo tags are synthetic. Existing window/row sizes remain unchanged.

## CoreHub production startup (0.6, 2026-10-16)

On this PC, CoreHub is the startup owner: AI SubHub is Live, with its production port assigned by CoreHub and no dev port. CoreHub's existing logon task starts Live production projects through scripts/start-server.ps1. The adapter reads artifacts/current-production.json (updated only after successful Publish), starts that self-contained production executable with --tray --health-port <CoreHub port>, and waits for it. No SDK, dev watcher or separate AI SubHub scheduled task is needed at logon. CoreHub starts the tree outside its own scheduler job, so CoreHub updates do not kill it; its normal Stop/Restart owns the adapter and app tree.

The app binds its health port only to IPv4 loopback and exposes only GET /api/health with app/version/status. It has no account data or control API and closes with the app process. CoreHub gets real process liveness rather than a detached dummy listener. The startup checkbox shows Managed by CoreHub for this launch mode, preventing a second Windows Run entry. The old direct startup preference/Run entry is disabled on this PC. For a standalone portable launch without --health-port, direct startup remains available.

Diagnosis: after reboot the project was not Live, had no production port or start-server adapter, so CoreHub excluded it. The independent startup preference was enabled but did not provide CoreHub management. Verification uses CoreHub's actual production Start/Stop/Restart API plus the app health endpoint and current-user session/process checks; a full PC reboot is left for the owner's next normal restart.

### Native settings recovery — 2026-10-16

Codex's packaged Windows process redirects AppData access to its LocalCache. Consequently the pre-CoreHub app and our shell checks saw three accounts/four projects there, while the WMI/CoreHub native launch saw an empty native AppData settings.json. The earlier preservation check only compared the redirected copy and did not verify native storage. Recovery stopped AI SubHub via CoreHub, preserved the empty native file, copied the verified metadata from the Codex LocalCache settings to native AppData (direct startup=false), and retained the source copy. No profiles or credentials were moved. A native WMI-launched production UI smoke read all three accounts and rendered successful live usage for each. Production then started again through CoreHub. Future storage verification must execute outside packaged AppData redirection; ordinary Codex shell reads of absolute AppData paths can still be redirected. CoreHub production uses the native user settings from now on.

## Complete naming and compact startup (0.7, 2026-10-16)

AI SubHub is the product name; ai-subhub is the repository/folder slug. Solution: AI.SubHub.slnx. Manifest identity, instance signals, startup registry value and protocol client identity use the new name. Old-name literals remain only in compatibility migration code/tests so existing settings are recoverable. The local project and matching vault folder move to ai-subhub, with CoreHub folderPath updated. Settings copy to AI SubHub on first native startup, preserving accounts, usage cache, project order/visibility and preferences; legacy settings remain as a backup. Profile folders and credentials do not move.

The main window starts at 620×404 logical client pixels, matching the owner's resized window (622×436 including Windows borders/title bar at the inspected scale). The account area still scrolls for longer lists. CoreHub remains production-only on port 4200 and starts in the tray.

Standard usage labels match across providers: 5-hour and Weekly. Claude's Session reading and Codex's default named 1-week reading are normalized only for display; stored provider labels remain available on hover. Model-specific and nonstandard windows retain their distinctions.

### Startup ownership revised — 2026-10-16 (supersedes CoreHub startup above)

The owner requested the ordinary Start with Windows checkbox for everyone, including this PC. General always offers it outside demo mode. It writes the current portable executable with --tray into HKCU Run under AISubHub, updates the entry when an enabled app runs from a new version/path, and removes it when unchecked. The old startup identifier is removed during this operation. No administrator rights or CoreHub dependency. This PC uses direct Windows startup=true; CoreHub project #14 is no longer Live and has neither a prod nor dev port, avoiding duplicate startup. The optional script/health integration remains available for external management but is not this PC's startup mechanism.

## Signed-in email and manual reset availability (0.8)

Hover an account name in the main window or Accounts settings for its provider-reported signed-in email. Claude reads auth status in the selected CLI profile with a five-second limit; Codex uses account/read. No editable label is treated as identity. These APIs report a signed-in email, not a separate email-ownership verification flag, so no verification checkmark is invented. Email stays in memory, is not added to settings/cache, and is excluded from smoke report contents. Missing identity says unavailable.

Codex's official app-server rate-limit response can include rateLimitResetCredits with availableCount and optional credit expiry details. A compact N resets tag appears for fresh known data, including explicit zero. Hover shows the earliest supplied expiry. Missing/unsupported/failed metadata means unknown, never zero; a capped credit detail list never overrides the provider count. Claude's current CLI usage integration does not establish manual reset availability; its tooltip directs users to the provider Usage page. Claude's official reset guide says manual resets can cover five-hour or weekly usage and may expire. This is separate from ordinary quota reset countdowns. AI SubHub only reads availability; there is no consume/reset/purchase request.

Sources: https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/src/protocol/v2/account.rs and https://support.claude.com/en/articles/17007452-what-is-a-limit-reset.

## Usage style and tighter rows (0.9)

Settings → General → Usage display selects % remaining (default for existing/new settings) or % used. The preference persists; the main percentage and bar fill switch immediately using complementary values, while warnings still indicate low remaining capacity. The tray displays the matching style for the same most-constrained window and corresponding reset time. Provider readings stay unchanged. Unknown style values normalize to Remaining.

Unknown manual-reset metadata is omitted entirely from account tooltips as well as tags. Known zero remains visible. Usage rows use a 56px label column, 4px gaps, content-sized percentage/reset columns and smaller side padding; reset text aligns against the right edge with no oversized reserved column. General spacing tightens to fit the new field in 600×320. Demo-only --used exercises the alternate style without changing real settings.

### Column alignment refinement (0.10)

All usage rows now share the same fixed label/percentage/reset columns (56/92/108px) and flexible bar column. Percentage/reset text aligns to the left edge of its column, so starts stay identical even when text lengths differ. A 12px gap follows each bar (8px grid gap + 4px bar right margin); ordinary gaps are 8px. Window size remains unchanged.

## macOS version (0.11)

The owner agreed to the same compact features/design, menu-bar integration and native Mac launch targets, with unsigned test .app packages first. Both Apple Silicon and Intel packages are built because test hardware is not yet confirmed. Shared UI, quotas, ordering/visibility, styles and caching remain common; platform details and pending native validation are in [macOS](macos.md).

## Login completion refresh (0.13, 2026-10-16)

Sign in / Sign in again now pass a unique temporary completion-file path into the terminal command. After the CLI exits, PowerShell writes only its numeric exit code; the interactive terminal can remain open. Hub watches the local signal for up to ten minutes and refreshes only the selected account on exit0, bypassing ordinary failure backoff while preserving known provider cooldown. If another refresh is active, the login refresh waits for it. Failed/unconfirmed login shows a safe notice without claiming success; app exit cancels the wait. A removed account or changed profile is not refreshed.

The temporary signal contains no token/email/provider response and is removed after completion/cancellation. Login stays CLI-managed. Main-window and Settings sign-in actions share the same controller path. Synthetic tests execute the actual generated Windows command and verify the completion-to-usage handoff, including paths with spaces/apostrophes, delayed active reads, failure backoff, failed login and provider cooldown.