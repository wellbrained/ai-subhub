# macOS test version (0.11)

## Scope

The compact shared Avalonia UI, usage/plan/email/reset metadata, refresh/backoff, account order and project order/visibility match Windows. Mac launch menus offer Terminal.app, VS Code and detected Claude/Codex desktop apps. Desktop/IDE logins remain separate from the selected CLI profile. Claude chat Projects integration stays deferred. This milestone provides self-contained Apple Silicon and Intel test packages for macOS 15+. Public Developer ID signing/notarization is deferred; no CI service or signing account is configured.

## Profiles and credentials

Settings live in `~/Library/Application Support/AI SubHub/settings.json`. The existing JSON schema is shared; `StartWithWindows` remains its compatibility field for the platform's login-startup preference. No Windows settings or credential files are imported automatically. Profiles default to `~/.claude` and `~/.codex`; custom folders are supported.

Claude first queries `/usr/bin/security find-generic-password` with separate argument-list values. The ordinary production OAuth service is `Claude Code-credentials` for the default profile. For custom profiles it is that name plus `-` and the first eight lowercase SHA-256 hex characters of the UTF-8, Unicode-NFC config directory. The account selector is USER (fallback Environment.UserName), restricted to letters, digits, dot, underscore and hyphen; invalid values use `claude-code-user`. Query output stays in memory and diagnostics are drained without logging. An eight-second deadline terminates a blocked process. If the selected Keychain item is absent/unavailable, only that profile's `.credentials.json` is the fallback. No other account's Keychain item is tried.

The default `~/.claude` profile is deliberately launched with `CLAUDE_CONFIG_DIR` unset. Claude distinguishes unset from explicitly setting that same path when naming its Keychain item. Custom profiles set the saved directory; credential reads and launches use the same rule. Inherited `CLAUDE_SECURESTORAGE_CONFIG_DIR` is cleared along with API-key/cloud billing overrides. An unusual existing login created by explicitly setting the default directory may need signing in again using Hub's default-profile launch. Nonproduction/custom Claude OAuth namespaces are outside this adapter.

The service algorithm was verified against Anthropic's public darwin-arm64 CLI 2.1.301 on 2026-10-16. It is a compatibility detail isolated in `MacPlatform.ClaudeKeychainService`, covered by synthetic tests. Keychain access can require native permission approval. Subscription plan labels come from the same in-memory credential metadata. Codex stays delegated to official app-server authentication, including any provider-owned keyring storage; planType can supply its known plan label. Missing metadata remains unknown.

## Launching, menu bar and login startup

CLI discovery includes extensionless executables, `~/.local/bin`, Homebrew paths and inherited PATH. Tools accepts explicit CLI executable paths. CLI subprocesses execute directly on Mac, while Windows cmd/bat shims retain PowerShell handling. CLI child PATH includes common Node/tool locations needed by shell scripts.

Terminal.app receives an AppleScript through `/usr/bin/osascript` with a separately escaped string containing a POSIX-quoted `/bin/zsh -lc` command. That command clears auth overrides, sets the selected profile, changes to the saved folder and invokes the CLI. A shell remains open afterwards. Sign-in uses the same adapter. macOS may ask to allow Terminal automation the first time. VS Code opens the selected folder through its app bundle or CLI; desktop discovery checks `/Applications` and `~/Applications`. Claude uses the official folder deep link. Codex desktop folder selection remains manual.

The app bundle uses LSUIElement so it lives in the menu bar. Close hides the window; menu-bar Open and a second launch restore it. Avalonia's Reopen activation also restores/unhides it. Exit shuts down explicitly. The current-user named-pipe activation signal carries a single byte, with no network endpoint or account data, and pending activation is retained until the UI subscribes.

Start at login writes only `~/Library/LaunchAgents/org.aisubhub.app.plist`, using the current executable and a separate `--tray` argument, RunAtLoad and no KeepAlive. Disable removes that file. Changes affect the next login; Hub does not log out, restart, bootstrap a second app or terminate the current process. Move the bundle to Applications first; after moving it later, run it from its new location to refresh enabled startup.

## Build and validation

`build.ps1 -Task PublishMac -PythonExecutable python` builds/tests then publishes osx-arm64 and osx-x64. Python 3 standard-library packaging creates `.app` bundles, Info.plist, the existing icon converted to ICNS and ZIP entries with Unix creator type and executable 0755 permissions. Outputs use unique timestamps to preserve earlier test packages. These packages have no Developer ID signature/notarization; .NET's apphost retains its generated local ad-hoc signature.

On a Mac with .NET 10 SDK, `bash scripts/build-macos.sh` builds/tests for the host architecture, creates the same bundle, applies a local ad-hoc signature and verifies it with codesign before ZIP creation. This remains a local test build. No Gatekeeper settings are disabled. For a downloaded test bundle macOS may require the owner's normal explicit Privacy & Security approval.

Verified on Windows: shared build/tests, synthetic Keychain namespace and quoting tests, activation reconnect/disposal, both cross-publishes, bundle properties, Mach-O CPU architecture, ZIP CRCs and executable modes, generated apphost ad-hoc code hashes. Windows demo layout/lifecycle is checked independently. These checks do not establish native Mac runtime behavior.

Pending on an actual Mac:

1. Check macOS version/architecture in About This Mac and launch its matching test app.
2. Add default and custom Claude profiles and Codex. Confirm identity, plan tags and independent usage; approve Keychain access if requested. Test CLI sign-in for a new profile.
3. Launch projects containing spaces/apostrophes in Terminal and VS Code. Confirm selected profile identity; check detected desktop targets and their separate logins.
4. Verify menu-bar icon, Open/Refresh/Exit, close-to-hide, second-launch/reopen, compact main/settings windows and both usage styles.
5. Enable Start at login from Applications; after the owner's next normal login confirm one hidden app. Disable and confirm no app starts at the following login. Inspect only the app's own LaunchAgent.

Primary references: [Avalonia packaging](https://docs.avaloniaui.net/docs/deployment/macos), [.NET 10 supported operating systems](https://github.com/dotnet/core/blob/main/release-notes/10.0/supported-os.md), [Claude authentication](https://code.claude.com/docs/en/authentication), [official CLI package](https://registry.npmjs.org/@anthropic-ai/claude-code-darwin-arm64/2.1.301).

## GitHub prerelease (0.12)

On 2026-10-16 the owner requested both Mac packages on GitHub with a new version. Version0.12.0 rebuilds the same test milestone, publishes its source and attaches Apple Silicon/Intel ZIPs plus SHA256SUMS.txt to the private repository's v0.12.0 prerelease. Native Mac validation and Developer ID signing/notarization remain pending.

## Login completion refresh (0.13)

The shared sign-in controller now waits for a per-launch numeric exit-code file before refreshing the selected account. Terminal's zsh command writes that code immediately after the CLI finishes, then leaves its interactive shell open. POSIX/AppleScript quoting and per-profile environment rules still apply. The file carries no credentials, is cleaned up and times out after ten minutes. Native Mac validation of this path is still pending.
