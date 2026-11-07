# Cross-platform: macOS and Linux

Research only, 2026-11-07, branch `research/cross-platform`. No app code was changed. The question from the owner: can AI SubHub also ship for macOS and/or Linux, what would it take, and is it worth it?

**Short answer.** macOS is realistic and fits the app well: GPUI's best platform, a native menu-bar item, a Mac to test on, and the same CLIs. It costs about 2 to 3 weeks of work, plus 99 USD a year if the app is to open without Gatekeeper warnings. Linux is possible but expensive to make good. The tray and the anchored popover are unreliable on GNOME and Wayland, so it's a best-effort target at most. The shared core (`subhub-core`) is mostly portable already. Almost all the work is in `subhub-app`'s Windows modules, in `launch.rs`, and in a handful of hard-coded Windows paths.

Unverified claims are marked *(unverified)*.

**Decision (owner, 2026-11-07):** macOS comes only once the Windows app is finished, with all features and the design done and nothing else pending. Linux is not planned. Until then there is no cross-platform work. The open questions in section 7 are decided when the macOS work starts.

---

## 1. GPUI on macOS and Linux

### What the pinned crates contain

- **`gpui-pre-platform 0.3.11`** picks a backend per OS through target-specific dependencies (`gpui-pre-platform-0.3.11/Cargo.toml`, lines 94-130 in the local registry):
  - macOS gets `gpui-pre-macos`.
  - Linux and FreeBSD get `gpui-pre-linux` (Wayland and X11).
  - Windows gets `gpui-pre-windows`.
  - Upstream is Zed: https://github.com/zed-industries/zed
- **Renderers:**
  - macOS uses native Metal (`CAMetalLayer`, `gpui-pre-macos` and `gpui-pre-apple`). Shaders are compiled into a `.metallib` at build time, which needs Apple's Metal toolchain, so macOS builds must run on macOS. The optional `runtime_shaders` feature compiles them at runtime instead.
  - Linux uses `gpui-pre-wgpu` (wgpu with Vulkan and GLES backends). This version no longer uses Blade. Users need a working Vulkan or GLES driver.
- **Default features** of `gpui-pre` are `font-kit`, `wayland`, `x11` and `windows-manifest`. Wayland and X11 are both compiled in and chosen at runtime. Zed's Linux notes: https://github.com/zed-industries/zed/blob/main/docs/src/development/linux.md
- **Linux build packages** (Debian/Ubuntu names) from Zed's `script/linux` (https://github.com/zed-industries/zed/blob/main/script/linux): `libxkbcommon-x11-dev`, `libwayland-dev`, `libx11-xcb-dev`, `libfontconfig-dev`, `libvulkan1`, `libzstd-dev`, `clang`, `cmake`. Add `libasound2-dev` only if audio gets linked.
- **Maturity:**
  - macOS is Zed's first and most polished platform.
  - Linux has been supported since 2024 and is used daily by Zed users, but it is the second-tier platform, with more compositor-specific bugs.
  - `gpui-pre` is a crates.io republish of a Zed snapshot. It is pre-1.0, so the exact pin stays (ADR 0001).

### Features the app relies on

| Need | macOS | Linux X11 | Linux Wayland |
|---|---|---|---|
| Borderless popup above other windows (`WindowKind::PopUp`, `titlebar: None`) | yes | yes | yes, as a toplevel |
| Place the popover at absolute screen coordinates (`window_bounds`) | yes | yes *(unverified, not tested)* | **no**: Wayland clients cannot place toplevel windows ([xdg-shell](https://wayland.app/protocols/xdg-shell)) |
| `WindowKind::LayerShell` (anchor to a screen edge with margins) | n/a | n/a | yes, behind `cfg(feature = "wayland")`, but only on compositors with wlr-layer-shell (KDE, Sway, Hyprland). **GNOME/Mutter has none** *(from general knowledge)* |
| Transparent or blurred background | yes | yes | yes |
| Several displays, `visible_bounds` (work area) | yes | yes | partly (no global coordinates) |
| No Dock icon | `App::set_activation_policy(ActivationPolicy::Accessory)` (`gpui-pre-0.3.11/src/app.rs:2115`, doc: "menu bar utility"), or `LSUIElement` in Info.plist | n/a | n/a |
| System appearance (light/dark) | `cx.window_appearance()`, already used in `theme.rs:124` | yes | yes |
| **Tray / status item** | **none in GPUI** | **none** | **none** |

GPUI has no tray API on any platform. That's already true on Windows, which is why `tray.rs` exists. A grep of `gpui-pre`, `-macos`, `-linux` and `-windows` found no `NSStatusItem` or StatusNotifierItem code.

### gpui-component 0.9.0

- The README says it ships one codebase "to macOS, Windows, and Linux" (https://github.com/longbridge/gpui-component).
- Its Cargo.toml has macOS-only deps (`objc2`, `objc2-app-kit`, …) and Windows-only deps.
- Nothing in it is tray- or OS-specific that the app uses. The components we use (lists, inputs, switches, menus, scroll areas) are plain GPUI, so they should work unchanged.
- I did not audit each component.

---

## 2. Inventory of Windows-only code

The line numbers are from commit `7392e8f`.

### Build and dependencies (blocks every non-Windows build)

| Where | What | macOS / Linux |
|---|---|---|
| `crates/subhub-app/Cargo.toml` (`windows = { … }`) | The `windows` crate is an unconditional dependency, so a cross check fails inside `windows-future` | Move it to `[target.'cfg(windows)'.dependencies]` |
| `Cargo.toml` (`keyring = { features = ["windows-native"] }`) | keyring v3 enables no store by default. **On other OSes it silently falls back to the in-memory mock store**, so the hub key would be "saved" and then lost ([keyring README](https://github.com/hwchen/keyring-rs/blob/v3.6.3/README.md)) | Enable per target: `apple-native` (Keychain) on macOS, and `sync-secret-service` + `crypto-rust` on Linux (needs libdbus at runtime, or add `vendored`). A test should check that the store is not the mock |
| `crates/subhub-app/build.rs` | Embeds `app.rc`/`app.ico` | Already guarded by `CARGO_CFG_TARGET_OS == "windows"`. macOS needs an `.icns` in the bundle; Linux a `.desktop` file plus PNG icon |

### Tray: `crates/subhub-app/src/tray.rs` (417 lines)

- **What it does now:**
  - Hidden window plus `Shell_NotifyIconW` add/modify/delete (`tray.rs:87-113`, `:205`).
  - Icon rect via `Shell_NotifyIconGetRect` to anchor the popover (`:124`).
  - Monitor DPI (`:142`) and work area (`:161`).
  - The `HICON` ring and logo (`:329-345`).
  - Messages arrive through GPUI's own `GetMessageW` loop (`:1-2`), so no second loop is needed.
- **macOS:**
  - Uses an [`NSStatusItem`](https://developer.apple.com/documentation/appkit/nsstatusitem) in the menu bar.
  - Its button takes an image (the ring, as a template or colored `NSImage`) and also a **title**, so text like "62%" can sit next to it.
  - Two ways to build it:
    - **`objc2-app-kit` directly.** It's already in the dependency tree through gpui-component. This is about 200-300 lines, mirroring `tray.rs`.
    - **[`tray-icon`](https://github.com/tauri-apps/tray-icon)** (Tauri). It has `set_title` and `rect()` for anchoring. It must be created on the main thread; inside GPUI's `run` callback that should work *(untested)*.
  - The button's window frame gives the anchor rect, so the popover opens under the item, as macOS menu-bar apps do.
- **Linux:**
  - Uses the [StatusNotifierItem](https://specifications.freedesktop.org/status-notifier-item/latest/) D-Bus spec (still a draft).
  - **[`ksni`](https://docs.rs/ksni/latest/ksni/)** is pure Rust over zbus with its own thread. It needs no GTK, so it does not compete with GPUI's event loop. `tray-icon`'s default Linux backend needs GTK and a GTK loop, which conflicts.
  - SNI gives activate (click) events and a menu, but **no icon position**, so the popover cannot be anchored to the icon.
  - KDE and most panels show SNI natively. **Stock GNOME shows nothing** without the [AppIndicator extension](https://extensions.gnome.org/extension/615/appindicator-support/), though Ubuntu ships it enabled.

### Taskbar display: `crates/subhub-app/src/taskbar.rs` (739 lines)

- **What it does now:** a topmost tool window over the Windows taskbar.
  - It finds the taskbar and tray area with `FindWindowW("Shell_TrayWnd")` / `"TrayNotifyWnd"` (`taskbar.rs:614-640`).
  - It reads the cursor with `GetCursorPos` (`:210`) and the taskbar theme from the registry (`:106-120`).
  - About 600 of the lines are GPUI rendering and would be reusable. Only the placement is Windows-only.
- **macOS:** there is no taskbar to cover; covering the menu bar or Dock is not allowed. The equivalent is the **status item's title text**.
  - A compact string like `C 62 · X 41` uses `NSStatusItem.button.title` or an attributed title.
  - The colored "Bars"/"Blocks" modes could be drawn into the status item image instead, a wider `NSImage`. This replaces the feature rather than porting it.
- **Linux:** no equivalent. SNI has a `Title`, but most hosts don't render it, and the AppIndicator "label" is shown only by some panels. Drop the feature on Linux.

### Window chrome and placement: `crates/subhub-app/src/window_style.rs` (381 lines)

| Function | Windows API | macOS | Linux |
|---|---|---|---|
| `round`, `no_frame` (`:219-280`) | DWM corner preference, border color, NC rendering | `NSWindow` with transparent background plus rounded content, or the content view layer's `cornerRadius`. GPUI's transparent background with our own rounded div is enough, since macOS has no square system border | Draw rounded corners ourselves on a transparent window; compositors differ |
| `set_bounds`, `move_outer`, `resize_*`, `fit_plan`, `outer_bottom` (`:54-130`, `:283-330`) | `SetWindowPos`, `GetWindowRect`, `ClientToScreen` | GPUI's own bounds API should be enough (macOS has no non-client frame to compensate for). Use `NSWindow` directly only if needed | X11: GPUI bounds. Wayland: not possible (see section 1) |
| `cursor_screen_y`, `left_button_down` (`:20-33`) | `GetCursorPos`, `GetAsyncKeyState` (popover drag) | `NSEvent.mouseLocation` / `pressedMouseButtons` | X11 possible; Wayland has no global cursor. Drop the popover drag |
| `work_area_top` (`:35`) | `GetMonitorInfoW` | `NSScreen.visibleFrame`, or GPUI `display.visible_bounds()` | GPUI `visible_bounds()` |
| `fullscreen_app_in_front` (`:135`) | `SHQueryUserNotificationState` plus a foreground-rect check | Only needed for the taskbar display, so not needed | not needed |
| `set_visible`, `raise_topmost` (`:186-215`) | `ShowWindow`, `HWND_TOPMOST` | GPUI `PopUp` level is enough | GPUI `PopUp`; stacking is up to the compositor |
| `copy_client_to_clipboard` (`:336`) | `PrintWindow` + `CF_DIB` (copy as image) | `CGWindowListCreateImage` is deprecated since macOS 14; ScreenCaptureKit needs the Screen Recording permission. Better: render our own content to an image, or drop the feature | Same: render ourselves or drop |

Callers: `window_style::` appears in `main.rs` (4×), `popover.rs` (8×), `stats_window.rs` (3×), `taskbar.rs` (7×), `whats_new.rs` (1×). Each needs a cross-platform version or a no-op.

### Single instance: `crates/subhub-app/src/main.rs:317-330`

- **Now:** a named mutex `Local\AISubHub.SingleInstance` (`CreateMutexW`).
- **macOS:** a bundle launched from Finder already runs once (LaunchServices). For direct launches, use an `flock` on a lock file in the data folder.
- **Linux:** an `flock` on a lock file, or a D-Bus well-known name. Both are about 20 lines.

### Startup at login: `crates/subhub-app/src/startup.rs` (86 lines)

- **Now:** `HKCU\…\Run` value "AI SubHub" (`startup.rs:3-70`), with `startup_command` in `launch.rs:584`.
- **macOS:** [`SMAppService.mainApp.register()`](https://developer.apple.com/documentation/servicemanagement/smappservice) on macOS 13+. It needs a proper `.app` bundle and shows up in System Settings → Login Items. The fallback is a LaunchAgent plist in `~/Library/LaunchAgents`.
- **Linux:** a `.desktop` file in `$XDG_CONFIG_HOME/autostart` ([XDG Autostart](https://specifications.freedesktop.org/autostart/latest/)). For an AppImage, point `Exec=` at `$APPIMAGE`, not the mounted path.
- **Crate option:** [`auto-launch`](https://docs.rs/auto-launch/latest/auto_launch/) covers all three, with a LaunchAgent on macOS. SMAppService is the modern macOS way and is better behaved; it's a small objc2 call.

### Self-updater: `crates/subhub-app/src/updater.rs` + `crates/subhub-core/src/{update,swap}.rs`

- **Platform-neutral already:** version compare, GitHub `releases/latest`, SHA-256 check, idle logic, patch notes.
- **Windows-only parts:**
  - Asset names are fixed: `EXE_ASSET = "ai-subhub.exe"` and `SHA_ASSET` (`update.rs:18-19`), and `download_name` (`update.rs:234`).
  - `updates_dir` uses `LOCALAPPDATA` (`update.rs:227`).
  - `start_new` uses `DETACHED_PROCESS` creation flags (`updater.rs:288-300`).
  - `wait_for_pid` uses `OpenProcess`/`WaitForSingleObject` (`updater.rs:304`).
  - `swap.rs` renames the running exe to `.old`.
- **macOS:**
  - The unit to replace is the whole `.app` folder.
  - Steps: download a zip, verify, unpack with `ditto -x -k`, rename the old `.app` aside, move the new one in, `open -n` the new one, quit.
  - The running binary can be replaced on macOS; the rename works the same way.
  - With signing, check `codesign --verify` and the Team ID before swapping.
  - [Sparkle](https://sparkle-project.org/) is the standard framework, but it's Objective-C with an appcast and EdDSA keys. That's overkill when we already have the GitHub flow.
  - An app in `/Applications` that is not writable by the user needs admin rights. In that case show "Cannot update in this folder", as on Windows.
- **Linux AppImage:** replace the file at `$APPIMAGE` (the env var the runtime sets) by downloading next to it and calling `rename()`. This is atomic on the same filesystem; the running process keeps its mount. Then `exec` the new one.
  - `.deb` installs must not self-update (that's the package manager's job), so update in Manual mode only and link to the release.
  - `wait_for_pid` becomes `kill(pid, 0)` polling (`libc`), and the detach flags become `setsid` or a plain `spawn`.
- **Release assets:** keep `ai-subhub.exe` (old Windows clients look for that exact name) and add per-platform assets:
  - `ai-subhub-macos-arm64.zip` (+ `.sha256`)
  - `ai-subhub-x86_64.AppImage` (+ `.sha256`)
  - `EXE_ASSET` becomes a `cfg`-selected constant.

### Terminal, CLI and desktop-app launching: `crates/subhub-core/src/launch.rs` (1074 lines incl. tests) and `codex.rs`

- **Windows-only parts:**
  - Everything goes through **PowerShell scripts**: `ps_quote` (`:52`), `encode_command` (`-EncodedCommand`, `:82`), `env_script` with `$env:CLAUDE_CONFIG_DIR` (`:88`), `login_script` and `cli_script` (`:104-148`).
  - `terminal_plan` runs `wt new-tab powershell.exe`, or `powershell.exe` alone (`:184-240`).
  - `plan_vscode` uses a hidden PowerShell (`:265`).
  - `CREATE_NO_WINDOW` (`:229`, `:406`).
  - The desktop app opens via `explorer.exe shell:AppsFolder\…` (`:326`).
  - Detection uses `Get-StartApps` / `Get-AppxPackage` (`:360-372`, `:394`).
  - `find_windows_terminal` (`:543`).
- **Already portable:**
  - The `LaunchPlan` struct (program/args/env/cwd) and the menu model (`menu_targets`, `:472`).
  - CLI search: `extensions()` returns `""` off Windows (`:524`), and `login_cwd` falls back to `HOME` (`:514`).
  - The `claude://code/new?folder=…` URI (`:317`).
- **macOS:**
  - **Terminal.app:** `osascript -e 'tell application "Terminal" to do script "<sh command>"'`. A plain `open -a Terminal <dir>` starts no command.
  - **iTerm2:** AppleScript (`create window with default profile command …`).
  - The command is a POSIX `sh` string: `cd '<dir>' && CLAUDE_CONFIG_DIR='<profile>' exec claude`, with single-quote escaping (`'` → `'\''`).
  - VS Code: `code --new-window -- <dir>`, or `open -a "Visual Studio Code" <dir>`.
  - Desktop apps: `open -a Claude` / `open -a Codex`, or `open 'claude://…'`. Detection: look for `/Applications/Claude.app` and `/Applications/Codex.app`, or ask LaunchServices with `mdfind "kMDItemCFBundleIdentifier == …"`.
  - The login exit-code file trick (`:104`) works the same with `; echo $? > file`.
- **Linux:**
  - No standard "open a terminal and run this". Use a list of terminals: `x-terminal-emulator -e` (Debian alternatives), `gnome-terminal -- sh -c …`, `konsole -e`, `kitty`, `alacritty -e`, `wezterm start --`, plus a user-configurable command.
  - `xdg-terminal-exec` is a proposed freedesktop standard *(status unverified)*.
  - Desktop apps: `xdg-open claude://…`. Both Claude Desktop ([Linux beta](https://code.claude.com/docs/en/desktop-linux), Ubuntu 22.04+ / Debian 12+) and the Codex app ([preview](https://developers.openai.com/codex/app)) now exist for Linux.
- **`codex.rs`:**
  - `.cmd` shims go through PowerShell (`codex.rs:316-336`) only on Windows; the `direct` branch already covers Unix.
  - The `taskkill /T` tree-kill (`:406`) is `cfg(windows)`. On Unix, start the child in its own process group (`CommandExt::process_group(0)`) and `kill(-pgid)`.
  - `find_codex_exe` searches `LOCALAPPDATA` for the desktop app's bundled CLI (`:462-475`). On macOS that's `/Applications/Codex.app/Contents/Resources/codex` *(unverified path)*.

### Data and settings paths

`LOCALAPPDATA`, falling back to the **temp dir** (wrong on macOS/Linux; data would land in `/tmp`):
- `settings.rs:328` (settings.json)
- `history.rs:29` (readings history)
- `stats/scan.rs:158` (stats cache)
- `update.rs:227` (updates)

Use `~/Library/Application Support/AI SubHub` on macOS and `$XDG_DATA_HOME/ai-subhub` (or `$XDG_CONFIG_HOME` for settings) on Linux. One `data_dir()` helper, through the `dirs` crate or by hand.

`USERPROFILE` → `HOME` fallback is already in place: `stats/scan.rs:166`, `launch.rs:515`, `:549`, `codex.rs:465`. Good.

Path normalization assumes Windows:
- `profile::same_dir` turns `/` into `\` and compares without case (`profile.rs:38-45`).
- `projects::folder_key` does the same, and strips `\\?\` (`projects.rs:41-52`).
- On Linux, paths are case-sensitive and `\` is a legal file-name character. On macOS (APFS, case-insensitive by default) the case folding is about right, but the separators are not.
- Both need a `cfg` branch. The tests that use `C:\…` literals need gating; some are already `#[cfg(windows)]` (`projects.rs:184`).

### Theme registry read

- `taskbar::taskbar_is_dark` reads `HKCU\…\Themes\Personalize\SystemUsesLightTheme` (`taskbar.rs:106-120`).
- Only the taskbar display uses it. The app theme already uses GPUI's `window_appearance()` (`theme.rs:124`), which works everywhere.
- macOS: the menu bar follows `NSApp.effectiveAppearance`; with a template image the system tints the icon itself.

### Secrets: `crates/subhub-core/src/secrets.rs`

- The code is plain `keyring::Entry`, so it's portable once the features are set (above).
- UI text says "Windows Credential Manager" (`settings_window.rs:2608`, error strings in `secrets.rs:32-74`). Make it per-OS: "Keychain" / "Secret Service (GNOME Keyring / KWallet)".
- A Linux system without a running Secret Service (minimal WMs) must fail visibly. That's already the rule: "missing data means unavailable".

### Other UI text

- `settings_window.rs:143`: placeholder `C:\Users\you\.claude-work`.
- `settings_window.rs:1859`: "(.exe, .cmd or .bat)".
- Terminal choices in Settings → Tools: Windows Terminal / PowerShell.

---

## 3. Data sources on macOS and Linux

| Source | Windows (today) | macOS | Linux |
|---|---|---|---|
| Claude profile folder | `%USERPROFILE%\.claude*`, `CLAUDE_CONFIG_DIR` | `~/.claude*`, same env var | same |
| `.claude.json` (email, `profile.rs:17`) | in profile folder | same | same |
| **Claude OAuth credentials** (`claude.rs:139` reads `<profile>/.credentials.json`) | file | **macOS Keychain** (a generic password item, keyed per config dir when `CLAUDE_CONFIG_DIR` is set). It falls back to the file only if the Keychain write fails ([Claude Code auth docs](https://code.claude.com/docs/en/authentication), [settings](https://code.claude.com/docs/en/settings)) | file `~/.claude/.credentials.json` (mode 0600) |
| Claude session logs (`projects/**/*.jsonl`, stats) | in profile folder | same *(layout not in the docs, but the CLI is the same Node program)* | same |
| Codex home | `%USERPROFILE%\.codex*`, `CODEX_HOME` | `~/.codex*`, same | same |
| Codex auth (`profile.rs:23`, email from `auth.json`) | `auth.json` | `auth.json`, **or the OS keyring** when `cli_auth_credentials_store = "keyring"`/`"auto"` ([Codex auth](https://learn.chatgpt.com/docs/auth)) | same |
| Codex usage | `codex app-server` over stdio (`codex.rs`), no file reading | same, portable | same |
| Codex sessions (`sessions/**/*.jsonl`, `archived_sessions`) | in `CODEX_HOME` | same | same |
| Hub (CLIProxyAPI) | HTTP | HTTP, unchanged | HTTP, unchanged |

So the stats scanner, Codex readings, hub reads and steering work unchanged. The one real gap is the **direct Claude read on macOS**:

- The token lives in the Keychain under Claude Code's item. Reading it from AI SubHub means a `SecItemCopyMatching` (or `security find-generic-password -s "Claude Code-credentials" -w` *(item name unverified)*).
- macOS then shows "AI SubHub wants to use your confidential information stored in 'Claude Code-credentials'", once per item, with "Always Allow".
- **Each update of an unsigned app changes its code identity, so the prompt comes back after every update.** A Developer-ID-signed app keeps the grant.
- This is still within the project rule ("secrets stay in provider-owned profiles/keyrings"): we read the provider's keyring item, we don't copy it.
- Hub-backed accounts don't need it at all. That's the owner's normal case.

Codex's email detection (`profile.rs:23`) fails when Codex keeps its tokens in the keyring. Account matching by email then needs the hub email or a manual label. That's a minor gap.

---

## 4. Packaging and distribution

### macOS

- **Architecture:**
  - Ship **Apple Silicon only** (`aarch64-apple-darwin`) at first.
  - GitHub plans to retire its Intel macOS runners with the macOS 15 image (Fall 2027) ([GitHub changelog](https://github.blog/changelog/2025-09-19-github-actions-macos-13-runner-image-is-closing-down/)). Apple has stopped selling Intel Macs.
  - A universal binary is possible later: build both targets on an arm64 runner (`rustup target add x86_64-apple-darwin`), then merge with `lipo -create` ([Apple: universal binaries](https://developer.apple.com/documentation/apple-silicon/building-a-universal-macos-binary)).
- **Bundle:**
  - `AI SubHub.app/Contents/{MacOS/ai-subhub, Info.plist, Resources/AppIcon.icns}`.
  - `Info.plist` needs `LSUIElement = true` (no Dock icon), `CFBundleIdentifier` (e.g. `com.wellbrained.ai-subhub`), `CFBundleShortVersionString` and `LSMinimumSystemVersion`.
  - This can be hand-rolled in a script (it's a folder plus a plist) or built with [`cargo-packager`](https://github.com/crabnebula-dev/cargo-packager) / `cargo-bundle`.
- **Signing and notarization:**
  - Needs the **Apple Developer Program, 99 USD per year** ([developer.apple.com/programs](https://developer.apple.com/programs/)).
  - Steps: a "Developer ID Application" certificate, then `codesign --options runtime --timestamp`, then `xcrun notarytool submit --wait`, then `xcrun stapler staple` ([Apple: notarizing](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)).
  - In CI: import the .p12 into a temporary keychain, and use an App Store Connect API key for notarytool. That's three or four repo secrets.
- **Unsigned option:**
  - Free, but Gatekeeper blocks the first open. The user must go to System Settings → Privacy & Security → "Open Anyway" ([Apple support](https://support.apple.com/en-us/102445)).
  - macOS 15 also removed the Control-click → Open shortcut *(widely reported; Apple's page doesn't say)*.
  - An ad-hoc signature (`codesign -s -`) is required anyway on Apple Silicon, where binaries must be signed to run. The linker does this automatically.
  - The Keychain re-prompt after each update (section 3) is the practical cost.
  - It's fine for the owner's own Mac and for a few technical users, not for the public.
- **Distribution format:**
  - A `.zip` of the `.app` (made with `ditto -c -k --keepParent`) is enough and is the easiest for the updater.
  - A `.dmg` is nicer for first installs but optional.
- **Homebrew:**
  - The official `homebrew/cask` has required casks to pass Gatekeeper since **2026-09-01**, so unsigned casks are disabled ([Homebrew 5.0.0](https://brew.sh/2025/11/12/homebrew-5.0.0/), [Acceptable Casks](https://docs.brew.sh/Acceptable-Casks)).
  - Without signing, only an own tap is possible, and taps must be trusted explicitly ([Tap trust](https://docs.brew.sh/Tap-Trust)).
  - With signing plus notarization, an official cask is possible once the app is notable enough. Own tap first.

### Linux

- **AppImage (recommended):**
  - One file, self-updatable by replacing `$APPIMAGE`, runs on most distros.
  - Build on the **oldest glibc you want to support**, e.g. `ubuntu-22.04` (glibc 2.35) ([AppImage concepts](https://docs.appimage.org/introduction/concepts.html)).
  - Needs FUSE 2 on the user's system, or `--appimage-extract-and-run`.
  - The Vulkan/GL drivers come from the host, which is correct; don't bundle them.
- **.deb:**
  - Easy with `cargo-deb` or cargo-packager, and good for Ubuntu users.
  - No self-update; the updater runs in Manual mode with a link to the release.
- **Flatpak: poor fit.**
  - The sandbox blocks `~/.claude*` and `~/.codex*` unless `--filesystem=home` (discouraged).
  - Spawning terminals and CLIs on the host needs `flatpak-spawn --host`.
  - The tray needs D-Bus talk-names ([Flatpak sandbox permissions](https://docs.flatpak.org/en/latest/sandbox-permissions.html)).
  - Flathub would likely reject or flag it. Skip.

### `scripts/release.ps1` and the updater

- `release.ps1` stays Windows-only: it builds, tests and writes `dist\ai-subhub.exe` plus `.sha256` locally.
- macOS and Linux artifacts can't be built on the owner's PC (see the compile check below), so they need **GitHub Actions**:
  - A workflow on tag push `v*`.
  - A matrix: `macos-15` (arm64) → `.app` → sign/notarize → zip + sha256, and `ubuntu-22.04` → AppImage (+ `.deb`) + sha256.
  - It uploads to the same release with `gh release upload`.
  - Optionally a `windows-latest` job, if the owner wants Windows built in CI too.
- `release.ps1 -Publish` then creates the tag and release as today, and CI adds the other assets a few minutes later.
- The updater must ignore a release whose asset for its own platform is missing. "No asset for this platform" is not an error; retry later. That avoids a race while CI is still uploading.
- **Runners** ([GitHub-hosted runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)):
  - arm64 macOS: `macos-15`, `macos-26`.
  - Intel: `macos-15-intel`, being retired.
  - Linux: `ubuntu-22.04`, `ubuntu-24.04`, and `ubuntu-24.04-arm` for arm64 Linux.
  - Public repos get these minutes free, which applies here since `wellbrained/ai-subhub` is public.

---

## 5. Compile check from Windows

I ran `rustup target add x86_64-unknown-linux-gnu aarch64-apple-darwin`, then `cargo check -p <crate> --target <triple>`. No C cross toolchain (zig, clang) is installed.

| Crate / target | Result |
|---|---|
| `subhub-core` → linux-gnu | fails in build scripts of `ring` (rustls) and `zstd-sys`: no C cross compiler |
| `subhub-core` → aarch64-apple-darwin | same two |
| `subhub-app` → linux-gnu | the above, plus `windows-future` fails (`IMarshal`/`marshaler` not found), because the `windows` crate is an unconditional dependency |
| `subhub-app` → aarch64-apple-darwin | the above, plus `gpui-pre-apple` can't find `shaders.metallib` (Metal shaders need Apple's toolchain) |

- None of our own code got as far as type-checking. The blockers are C dependencies and the unconditional `windows` dependency, as expected.
- A meaningful check needs a real macOS or Linux machine or CI runner, or `cargo-zigbuild` for Linux.
- Next step if the owner goes ahead: one CI job that runs `cargo check` / `cargo test -p subhub-core` on `ubuntu-24.04` and `macos-15`. The core will likely pass with only small path fixes.

---

## 6. Effort, risks and recommended path

### Shared groundwork (needed for either platform): ~2-3 days

1. **`platform` seam in `subhub-app`:**
   - `src/platform/{mod.rs, windows.rs, macos.rs, linux.rs}`, with one small interface: `Tray` (set icon, set tooltip/title, events, anchor rect: `Option`), window chrome/placement helpers, `single_instance`, `startup::{is_enabled, set_enabled}`, `copy_as_image` (`Option`), `restart_into(new)`.
   - Today's `tray.rs`, `window_style.rs`, `startup.rs` and the `taskbar.rs` placement move under `windows.rs` unchanged.
2. `cfg`-gated dependencies (`windows`, keyring features per target).
3. One `data_dir()` helper replacing the four `LOCALAPPDATA` uses.
4. `cfg` branches in `profile::same_dir` / `projects::folder_key`, and gated Windows-path tests.
5. `launch.rs`: split "what to run" (portable `LaunchPlan`) from "how to wrap it" (PowerShell vs POSIX `sh`). Add a `sh_quote` with tests.
6. A CI job that checks and tests `subhub-core` on macOS and Ubuntu.

**Shared unchanged:** all of `subhub-core` except `launch.rs`, the paths and `claude::read_credentials`. That covers the model, settings, providers' parsing, hub, steering, merge, schedule, priority, billing, stats parse/scan/summary, update logic, and changelog. In `subhub-app`, all GPUI views (popover, settings window, statistics window, limits board, what's new) and `state.rs` are shared unchanged. That's roughly 85-90 % of the ~20k lines.

### macOS: ~10-14 days

| Part | Days |
|---|---|
| Menu-bar item (objc2 `NSStatusItem`): ring image, optional title text as the "taskbar display" replacement, click → popover anchored under it, right-click menu | 3 |
| Accessory activation policy, popover focus/close behaviour, rounded transparent window, multi-display | 1-2 |
| Claude credentials from Keychain (`security-framework`), with a clear "needs Keychain access" state | 1 |
| Launch: Terminal.app / iTerm via `osascript`, VS Code, desktop apps via `open`, detection in `/Applications`, sign-in flow | 2 |
| Startup via `SMAppService` | 0.5 |
| Updater: zip → `.app` swap, relaunch with `open -n` | 1.5 |
| Bundle + CI: Info.plist, icns, signing + notarization (if paid), zip + sha256 upload | 1.5-2 |
| Testing on the owner's MacBook, polish of Settings text per OS | 1-2 |

### Linux: ~8-12 days, plus ongoing support

| Part | Days |
|---|---|
| SNI tray via `ksni`: ring icon as pixmap, tooltip, activate → popover, menu | 1.5 |
| Popover placement: X11 near the cursor/corner; Wayland via layer-shell where available, else a normal small window; GNOME fallback | 2-4 (uncertain) |
| Secret Service keyring, error state when missing | 0.5 |
| Launch: terminal table plus a custom-command setting, `xdg-open` | 1.5 |
| XDG autostart (AppImage-aware) | 0.5 |
| AppImage build on `ubuntu-22.04`, self-update; `.deb` optional | 1.5 |
| Testing on GNOME (Wayland), KDE, one X11 setup | 2 |

### Risks

1. **Linux tray and popover (high).** No icon position from SNI. GNOME shows no tray without an extension. Wayland forbids placing windows and GNOME lacks layer-shell. The result is a popover that opens in a corner, or as a normal window. It works, but it's not the polished Windows experience. Every desktop environment behaves differently, so the support cost is ongoing.
2. **GPUI on Linux (medium).** wgpu/Vulkan driver issues on some machines (VMs, old Intel GPUs). The pinned snapshot may carry Linux bugs fixed upstream later, and bumping the pin is a separate decision (ADR 0001).
3. **macOS Keychain prompts (medium).** These hit direct Claude reads only. Mitigations: hub-first (already the default), Developer ID signing (stable code identity), and clear UI text.
4. **Unsigned macOS app (medium).** Gatekeeper friction on first launch, no official Homebrew cask, and Keychain re-prompts after updates. Fine for the owner, poor for public users.
5. **GPUI pin (low-medium).** `gpui-pre 0.3.11` was only ever used on Windows here. macOS is GPUI's home platform, so the risk is low, but popover/accessory-app behaviour must be checked early (spike, as in ADR 0001).
6. **Two launch backends (low).** The PowerShell quoting is well tested. A POSIX `sh` variant needs the same test coverage (paths with `'`, spaces, `&`).
7. **Taskbar display parity (product, not technical).** It becomes "menu-bar text" on macOS and doesn't exist on Linux.

### Recommended path

1. **Do the shared groundwork first** (2-3 days). It's useful on its own: cleaner seams, CI on three OSes, and no harm to Windows.
2. **macOS next, as a menu-bar app.**
   - Start with a 1-2 day spike on the owner's MacBook: NSStatusItem plus GPUI popover under it, accessory policy, one hub read. This is the same spike approach as ADR 0001, and a bad result stops the work.
   - Then the port. Ship Apple Silicon only, `.zip` via GitHub Releases, self-update.
   - Decide on signing before inviting anyone else to use it.
3. **Linux only if there is demand**, as best-effort: AppImage, SNI tray via `ksni`, popover as a corner window, no taskbar display. Document "GNOME needs the AppIndicator extension". Don't promise feature parity.

### Proof-of-concept sketch: macOS status item (not wired up, objc2 0.6 style)

```rust
// platform/macos.rs: sketch only.
use objc2::MainThreadMarker;
use objc2_app_kit::{NSStatusBar, NSVariableStatusItemLength, NSImage};
use objc2_foundation::NSString;

pub struct Tray { item: objc2::rc::Retained<objc2_app_kit::NSStatusItem> }

impl Tray {
    pub fn new(mtm: MainThreadMarker) -> Self {
        let bar = unsafe { NSStatusBar::systemStatusBar() };
        let item = unsafe { bar.statusItemWithLength(NSVariableStatusItemLength) };
        Self { item }
    }
    /// The "taskbar display" on macOS: text next to the ring.
    pub fn set_title(&self, mtm: MainThreadMarker, text: &str) {
        if let Some(button) = unsafe { self.item.button(mtm) } {
            unsafe { button.setTitle(&NSString::from_str(text)) };
        }
    }
    // set_image(&NSImage) from the same RGBA ring pixels as tray.rs::ring_pixels;
    // click → target/action (a small objc2 `define_class!` handler) → TrayEvent channel;
    // anchor rect = button.window().frame() converted to GPUI display coordinates.
}
```

The seam the app would code against:

```rust
pub trait TrayBackend {
    fn set_icon(&mut self, rgba: &[u8], size: u32);
    fn set_tooltip(&mut self, text: &str);           // Windows/Linux tooltip
    fn set_title(&mut self, text: Option<&str>);     // macOS menu-bar text; no-op elsewhere
    fn anchor(&self) -> Option<Bounds<Pixels>>;      // None on Linux → corner placement
}
```

---

## 7. What the owner needs to decide

1. **Platforms:** macOS only, or macOS plus Linux best-effort? (Recommended: macOS first; Linux only if someone asks.)
2. **Apple signing:** pay **99 USD per year** for the Apple Developer Program (Developer ID + notarization, clean first launch, stable Keychain grant, Homebrew cask possible), or ship unsigned for personal use (free, "Open Anyway" once, Keychain re-prompt after each update)?
3. **Intel Macs:** Apple Silicon only (recommended), or a universal binary (a little more CI work)?
4. **Taskbar display on macOS:** replace it with menu-bar text next to the ring (recommended), or drop it?
5. **CI builds:** OK to add a GitHub Actions release workflow for the macOS/Linux assets? The owner's local `release.ps1` stays for Windows.
6. **Claude direct reads on macOS:** accept the Keychain permission prompt, or rely on the hub there and show profile-only Claude accounts as "needs Keychain access"?
