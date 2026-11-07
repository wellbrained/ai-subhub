# AI SubHub

AI SubHub, a Windows tray app in Rust + GPUI. It started as a rewrite of the earlier C#/Avalonia app (now `ai-subhub-old`) and replaced it on 2026-11-07. Windows first; macOS later behind a platform seam. Decisions: `docs/adr/0001-gpui-rewrite.md`. Product language: `CONTEXT.md`. Comparison with the old app: `docs/comparison.md`.

## Stack

- `gpui-pre` and `gpui-component` pinned exactly (`=`) to the pair gpui-component itself uses; bump both together and note it in the ADR.
- Own tray module on the `windows` crate (`Shell_NotifyIconW`); `gpui-tray` is a reference, not a dependency.
- `reqwest` + `serde` for providers and the hub; secrets only in Windows Credential Manager.

## Project rules

- Behavior rules match the C# app: secrets stay in provider-owned profiles/keyrings or Credential Manager, never in `settings.json`, logs or errors; missing data means unavailable; failed/overdue readings stay visibly stale; reset countdowns never refill quotas locally; provider cooldowns are respected and persisted; Claude's undocumented usage endpoint is parsed in one isolated module.
- The CLIProxyAPI hub: list accounts and read usage through its management API (`api-call` with `$TOKEN$`), never its auth files. The only write is hub steering: each account's credential priority (ADR 0001, refinements); remember and restore original priorities. Never bench, add, sign in or change config.
- Own settings file `%LOCALAPPDATA%\AI SubHub\settings.json`; standalone: it never reads the C# app's files (the import was removed, owner 2026-11-07).
- Settings window: category menu on the left, one content column. Main surface is the tray popover.
- Version starts at 0.1.0 in `Cargo.toml`; patch bump per deployed change.

## Feature map

- Workspace: `crates/subhub-core` (no UI: model, settings, providers, hub, merge, schedule, priority, projects, launch, stats, hover, placement) and `crates/subhub-app` (GPUI). `spike/` is the throwaway spike.
- `docs/features/app.md`: tray, popover, settings window, refresh flow.
- `docs/features/spike.md`: spike goals, results and open points for the real build.

## Releasing

- Update source is GitHub Releases of `wellbrained/ai-subhub` (public repo needed); details in `docs/features/app.md`, "Updates".
- Patch bump per release; the minor version only when the owner says a feature is complete; ask before a major bump.
- Steps: put the notes under `## X.Y.Z – date` in `CHANGELOG.md` (3-6 short bullets; `## Unreleased` while working), run `scripts/release.ps1 -Version X.Y.Z` (checks the section, sets `[workspace.package]` version, tests, builds release, writes `dist\ai-subhub.exe` + `.sha256`), commit the bump, then `scripts/release.ps1 -Version X.Y.Z -Publish` (tag, push tag, `gh release create`). Asset names must stay `ai-subhub.exe` and `ai-subhub.exe.sha256`.
- Agents do not run the release script or `cargo build --release`; the owner does.
