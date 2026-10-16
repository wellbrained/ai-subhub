# AI SubHub

Windows-first desktop tray app: C#/.NET 10, Avalonia 12. Product language: `CONTEXT.md`. Before changing behavior, read and update `docs/features/windows-mvp.md`. Scope and rationale: `docs/adr/0001-windows-first.md`.

## Project rules

- `Hub.Core` owns models, parsing, settings and provider/launch adapters; it has no UI dependency. `Hub.App` owns Windows lifecycle, startup integration and Avalonia controls. Startup and terminal launchers are Windows-only for v0.1.
- Secrets stay in provider-owned profiles/keyrings. Persist only metadata and usage snapshots. Provider errors shown or logged by Hub exclude raw responses, tokens and account identity. Tests use synthetic credentials and temporary folders.
- An account is tied to a CLI profile; renaming only changes its display label. Removal only disconnects the Hub record. Profile edits invalidate cached readings.
- Missing data means unavailable. Failed/overdue readings stay visibly stale. Reset countdowns never refill quotas locally. Preserve every recognized returned quota bucket and provider cooldown.
- Claude's usage endpoint is undocumented. Keep its parsing isolated and document compatibility changes. Login/refresh are CLI-managed; no silent CLI updates or token refresh implementation.
- Launch subscription profiles with billing/auth overrides cleared from child environments. Quote paths as data; native processes use `ArgumentList`, PowerShell scripts use escaped literals and `EncodedCommand`.
- Settings use category navigation on the left and a single content column. Keep main usage grouped by provider with one account per row.
- Version is `0.y.0` in `Directory.Build.props`. `build.ps1` checks locally and optionally produces a portable self-contained build. Keep README, manifest and protocol client version aligned when changing versions.

## Feature map

- `docs/features/windows-mvp.md`: profiles, readings, refresh states, launch behavior, settings, tray lifecycle and verification.
- `docs/adr/0001-windows-first.md`: agreed first milestone and tradeoffs.
