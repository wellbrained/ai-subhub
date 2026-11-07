# Spike: tray, popover, hub (2026-11-06)

Throwaway crate in `spike/` (release build, Windows 11, 2560x1440 at 100%). It proves the risky parts before any feature work; it is not the app's code base.

## Run

- `subhub-spike.exe`: tray only. Left-click the tray icon toggles the popover; right-click → Quit spike.
- `--auto-open` opens the popover at start; `--stay` disables close-on-focus-loss (screenshots).
- `--store-hub-key <file>` stores the hub management key in Credential Manager (`AI SubHub` / `hub-management-key`); `--check-key` normalizes pasted `name = value` lines to the bare key and logs only its shape.
- `--dump-rings <file.bmp>` renders sample tray rings. Log: `%TEMP%\subhub-spike.log` (no key, tokens or raw bodies).

## Results

| Goal | Result |
|---|---|
| Tray icon with live-drawn ring | ✅ Own `Shell_NotifyIconW` module; GPUI's `GetMessageW(None)` loop dispatches its hidden window, so no second loop. Anti-aliased ring redrawn via `NIM_MODIFY`; blue → orange (≤25% left) → red (≤10%). |
| Popover above the tray | ✅ `WindowKind::PopUp` + `titlebar: None` + transparent background gives a borderless, topmost, rounded panel; opens in 25–37 ms. Anchored with `Shell_NotifyIconGetRect`. |
| Close on focus loss | ✅ `observe_window_activation` → `remove_window()`; 300 ms guard so the tray click that closed it does not reopen it. Not yet checked by a real tray click (see open points). |
| Alive with no window | ✅ `QuitMode::Explicit`; if the tray cannot be created the app quits at once, so it never runs without a way back. |
| One real hub read | ✅ `auth-files` listed 4 accounts (3 Claude, 1 Codex, 0 benched); Claude usage via `api-call` + `$TOKEN$` returned five-hour and weekly windows. Header `Authorization: Bearer <management key>`. |
| Idle memory | ✅ No window: 46 MB working set / 58 MB private, 0.02% CPU over 30 s. One popover open: ~65 MB. Avalonia 0.13.3 at the same time: 77 MB / 129 MB private. |
| Size | 21 MB exe, 8.3 MB zipped (no LTO/strip yet) vs 42 MB Avalonia ZIP. Tray ready ~230 ms after process start. |

`gpui-pre` =0.3.11 + `gpui-component` =0.9.0 + `gpui-pre-platform` =0.3.11 (package renamed to `gpui_platform`) built cleanly on Rust 1.99.1 with no extra Windows setup.

## Open points for the real build (handled in crates/subhub-app, see app.md; real-click checks still need the owner)

- Windows 11 places a new tray icon in the overflow ("^") area; then the reported rect is inside the overflow flyout. Anchor to the taskbar's notification area instead when the icon is hidden, and tell the user how to pin it.
- gpui-component `Button` did not render in the popover: its theme was not applied (needs `Theme` setup for light/dark). Set up the theme before using components.
- With `NOTIFYICON_VERSION_4` a left click sends `WM_LBUTTONUP` and `NIN_SELECT` (right click: `WM_RBUTTONUP` and `WM_CONTEXTMENU`). Handle only `NIN_SELECT`/`NIN_KEYSELECT` and `WM_CONTEXTMENU`; handling both made the popover open and close at once (found by the owner, fixed in the spike).
- Verify real tray clicks: toggle, focus-loss close, right-click menu (spike tested with `--auto-open` only).
- Per-monitor DPI: the spike uses the system DPI; use the tray's monitor DPI.
- Field names of `auth-files` beyond `provider`/`type`, `email`, `disabled`, `auth_index` are unverified; parse defensively.
