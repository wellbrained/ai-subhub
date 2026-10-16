# 0002: Shared UI with native macOS adapters

Date: 2026-10-16. Status: accepted by owner for the test-build milestone.

The owner wants the Windows features and compact design on macOS, menu-bar operation and native launch targets. Test hardware is not yet confirmed. Deliver self-contained Apple Silicon and Intel .app ZIPs, with native validation explicitly pending. Public signing/notarization is a later milestone.

Reuse the Avalonia controls and shared domain/settings/usage logic. Isolate Mac credentials, process/terminal/app launch, login startup and bundle packaging. Keep Codex authentication delegated to app-server. Read Claude's selected Keychain namespace without copying credentials to Hub storage; align the default-profile unset environment with the CLI's normal Mac login. Preserve Windows adapters and production installation.

Use a per-user LaunchAgent for opt-in login startup in the portable test app. It records the installed executable and --tray, and changes only the next-login behavior. Native signing/distribution work may replace this adapter later. No CI service, signing credentials or automatic public release is needed for this milestone.

Minimum supported macOS is 15 per .NET 10's current OS matrix. Build/packaging checks on Windows do not replace validation of Keychain, Terminal automation, menu-bar lifecycle and login startup on a real Mac.
