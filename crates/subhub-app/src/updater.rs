//! App side of self-update (ADR 0001, "Updates"; docs/features/app.md). The decisions and the file
//! work are in `subhub_core::{update, swap, changelog}`; this module runs them on background
//! threads, keeps the status Settings shows, starts the new process and waits for the old one.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Utc;
use gpui::{Context, WeakEntity};
use subhub_core::settings::UpdateMode;
use subhub_core::swap::{self, Fs};
use subhub_core::update::{self, Busy, Checked, Release, UpdateError, Version};

use crate::state::AppState;

/// Patch notes, embedded at build time.
pub const CHANGELOG: &str = include_str!("../../../CHANGELOG.md");

pub fn running_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("the package version is X.Y.Z")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Checking,
    Downloading,
    Installing,
}

/// Live update status (not saved; the mode, last check and last seen version are settings).
pub struct UpdateState {
    pub phase: Phase,
    /// A newer release found by the last check.
    pub available: Option<Release>,
    /// Its exe is downloaded and verified (Auto mode installs it when idle).
    pub downloaded: bool,
    /// Last problem, in a safe short form; cleared by the next try.
    pub error: Option<String>,
    /// Running from cargo's output folder: checking only, never installing.
    pub dev_build: bool,
    exe: Option<PathBuf>,
    last_try: Option<Instant>,
    last_failed: bool,
    next_download: Option<Instant>,
    /// Version already announced with a tray balloon (Manual mode).
    announced: Option<Version>,
}

impl UpdateState {
    pub fn new() -> Self {
        let exe = std::env::current_exe().ok();
        // Unknown location counts as a development build: better not to install than to guess.
        let dev_build = exe.as_deref().is_none_or(update::is_dev_build);
        Self {
            phase: Phase::Idle,
            available: None,
            downloaded: false,
            error: None,
            dev_build,
            exe,
            last_try: None,
            last_failed: false,
            next_download: None,
            announced: None,
        }
    }
}

impl AppState {
    pub fn set_update_mode(&mut self, mode: UpdateMode) {
        self.settings.updates.mode = mode;
        self.save_user();
    }

    /// Looks for a newer release on a background thread (also what "Check now" does).
    pub fn check_update(&mut self, cx: &mut Context<Self>) {
        if self.update.phase != Phase::Idle {
            return;
        }
        self.update.phase = Phase::Checking;
        self.update.error = None;
        self.update.last_try = Some(Instant::now());
        cx.notify();
        let running = running_version();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { update::check_latest(&running) })
                .await;
            let _ = this.update(cx, |s, cx| s.checked(result, cx));
        })
        .detach();
    }

    fn checked(&mut self, result: Result<Checked, UpdateError>, cx: &mut Context<Self>) {
        self.update.phase = Phase::Idle;
        match result {
            Ok(found) => {
                self.update.last_failed = false;
                self.settings.updates.last_check = Some(Utc::now());
                self.save();
                match found {
                    Checked::UpToDate => {
                        self.update.available = None;
                        self.update.downloaded = false;
                    }
                    Checked::Available(rel) => self.found(rel, cx),
                }
            }
            Err(e) => {
                self.update.last_failed = true;
                self.update.error = Some(format!("Could not check for updates: {e}"));
            }
        }
        cx.notify();
    }

    fn found(&mut self, rel: Release, cx: &mut Context<Self>) {
        let version = rel.version;
        if self
            .update
            .available
            .as_ref()
            .is_none_or(|a| a.version != version)
        {
            self.update.downloaded = false;
        }
        self.update.available = Some(rel);
        if self.update.dev_build {
            return;
        }
        match self.settings.updates.mode {
            UpdateMode::Auto => self.download_update(cx),
            UpdateMode::Manual => {
                if self.update.announced != Some(version) {
                    self.update.announced = Some(version);
                    self.pending_notices.push((
                        format!("Update available: v{version}"),
                        "Open Settings, Updates to install it.".into(),
                    ));
                }
            }
        }
    }

    /// Auto mode: downloads and verifies the update in the background, installs nothing.
    pub fn download_update(&mut self, cx: &mut Context<Self>) {
        let Some(rel) = self.update.available.clone() else {
            return;
        };
        if self.update.phase != Phase::Idle || self.update.dev_build || self.update.downloaded {
            return;
        }
        self.update.phase = Phase::Downloading;
        self.update.error = None;
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { update::download_verified(&rel, &update::updates_dir()) })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.update.phase = Phase::Idle;
                match result {
                    Ok(_) => s.update.downloaded = true,
                    Err(e) => {
                        s.update.error = Some(format!("Download failed: {e}"));
                        s.update.next_download = Some(Instant::now() + update::RETRY_AFTER_FAILURE);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Downloads (when not done yet), verifies, swaps the exe and restarts into the new
    /// version. Manual installs call this straight from the button.
    pub fn install_update(&mut self, cx: &mut Context<Self>) {
        if self.update.phase != Phase::Idle {
            return;
        }
        let Some(rel) = self.update.available.clone() else {
            return;
        };
        if self.update.dev_build {
            self.update.error = Some("Development build: updates are off".into());
            cx.notify();
            return;
        }
        let Some(exe) = self.update.exe.clone() else {
            self.update.error = Some("Cannot update: the program location is unknown".into());
            cx.notify();
            return;
        };
        self.update.phase = Phase::Installing;
        self.update.error = None;
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let target = exe.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let file = update::download_verified(&rel, &update::updates_dir())
                        .map_err(|e| format!("Download failed: {e}"))?;
                    swap::swap_in(&mut Fs, &target, &file).map_err(|e| e.to_string())
                })
                .await;
            let _ = this.update(cx, |s, cx| match result {
                Ok(_old) => s.restart_into(&exe, cx),
                Err(message) => {
                    s.update.phase = Phase::Idle;
                    s.update.error = Some(message);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The swap is done: start the new exe (it waits for this process to end), then quit. If it
    /// cannot start, the swap is undone and this version keeps running.
    fn restart_into(&mut self, exe: &Path, cx: &mut Context<Self>) {
        match start_new(exe, &running_version()) {
            Ok(()) => cx.quit(),
            Err(e) => {
                let undone = swap::revert(&mut Fs, exe);
                self.update.phase = Phase::Idle;
                self.update.downloaded = false;
                self.update.error = Some(match undone {
                    Ok(()) => format!(
                        "Could not start the new version ({}); the current one stays",
                        update::io_reason(&e)
                    ),
                    Err(u) => u.to_string(),
                });
                cx.notify();
            }
        }
    }

    /// Once a minute (from `main`): a due check in either mode; in Auto mode also the
    /// background download and, when nothing is in use, the install.
    pub fn update_tick(&mut self, busy: Busy, cx: &mut Context<Self>) {
        if self.update.phase != Phase::Idle {
            return;
        }
        if update::check_due(
            self.update.last_try.map(|t| t.elapsed()),
            self.update.last_failed,
        ) {
            self.check_update(cx);
            return;
        }
        if self.settings.updates.mode != UpdateMode::Auto
            || self.update.dev_build
            || self.update.available.is_none()
        {
            return;
        }
        if !self.update.downloaded {
            if self
                .update
                .next_download
                .is_none_or(|t| Instant::now() >= t)
            {
                self.download_update(cx);
            }
            return;
        }
        let busy = Busy {
            signing_in: busy.signing_in || !self.signing_in.is_empty(),
            popover_dragging: busy.popover_dragging,
            ..busy
        };
        if update::auto_install_due(
            self.settings.updates.mode,
            self.update.dev_build,
            self.update.downloaded,
            &busy,
        ) {
            self.install_update(cx);
        }
    }
}

/// Starts the freshly installed exe with the hand-over arguments, detached from this process.
fn start_new(exe: &Path, from: &Version) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    std::process::Command::new(exe)
        .arg("--updated-from")
        .arg(from.to_string())
        .arg("--wait-pid")
        .arg(std::process::id().to_string())
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map(|_| ())
}

/// Waits (at most `timeout`) for the process that started us to end, so the two never share the
/// tray or the single-instance mutex. A process that is already gone is not waited for.
pub fn wait_for_pid(pid: u32, timeout: Duration) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };
    if pid == std::process::id() {
        return;
    }
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) else {
            return;
        };
        let _ = WaitForSingleObject(handle, timeout.as_millis().min(u32::MAX as u128) as u32);
        let _ = CloseHandle(handle);
    }
}

/// Deletes what an earlier update left behind: `<exe>.old` and stale files in the updates
/// folder. On a background thread, since the old exe may take a moment to unlock.
pub fn clean_leftovers() {
    let exe = std::env::current_exe().ok();
    std::thread::spawn(move || {
        if let Some(exe) = exe {
            swap::remove_old(&exe);
        }
        update::clean_updates_dir(&update::updates_dir(), &running_version());
    });
}
