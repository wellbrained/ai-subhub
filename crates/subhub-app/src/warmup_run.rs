//! Running warm-ups (plan `docs/features/overview.md`, "Warm-up"; rules in
//! `subhub_core::warmup`). Blocking: the app calls [`execute`] on the background executor, the
//! headless `--warm-up` mode (started by the wake task) calls [`headless`] and exits.
//!
//! Whoever runs holds the named mutex `Local\AISubHub.WarmUp` and re-reads the shared run log
//! inside it, so the installed app, a dev build and the wake task never warm the same account
//! twice. While it runs, the PC is kept awake (a PC woken by a timer goes back to sleep after
//! about two idle minutes).
use std::path::Path;
use std::time::Duration as StdDuration;

use chrono::Utc;
use subhub_core::insights::Zone;
use subhub_core::launch;
use subhub_core::settings::{self, Settings};
use subhub_core::warmup::{self, Due, Outcome, Run, RunLog};
use uuid::Uuid;

/// What to run.
#[derive(Debug, Clone)]
pub enum Job {
    /// The accounts due by the schedule (computed inside the lock).
    Scheduled,
    /// "Start a window now" for these accounts.
    Now(Vec<Uuid>),
}

/// The named mutex, released on drop.
struct Lock(windows::Win32::Foundation::HANDLE);

impl Drop for Lock {
    fn drop(&mut self) {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::ReleaseMutex;
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

/// The warm-up lock, or None while another process (or thread) runs a warm-up.
fn lock() -> Option<Lock> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
    use windows::core::w;
    unsafe {
        let h = CreateMutexW(None, false, w!("Local\\AISubHub.WarmUp")).ok()?;
        // WAIT_OBJECT_0 (0) or WAIT_ABANDONED (0x80, a crashed holder): ours now.
        let r = WaitForSingleObject(h, 0).0;
        if r == 0 || r == 0x80 {
            Some(Lock(h))
        } else {
            let _ = CloseHandle(h);
            None
        }
    }
}

/// Keeps the system awake until dropped.
struct Awake;

impl Awake {
    fn new() -> Self {
        use windows::Win32::System::Power::{
            ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
        };
        unsafe {
            SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED);
        }
        Awake
    }
}

impl Drop for Awake {
    fn drop(&mut self) {
        use windows::Win32::System::Power::{ES_CONTINUOUS, SetThreadExecutionState};
        unsafe {
            SetThreadExecutionState(ES_CONTINUOUS);
        }
    }
}

/// The folder the CLI runs in (its sessions are logged under it, recognisably).
pub fn work_dir(settings_path: &Path) -> std::path::PathBuf {
    settings_path.with_file_name("warm-up")
}

/// Runs a job: takes the lock (None = someone else is running, try again later), re-reads the
/// log, runs each account that is due / asked for and not skipped, and writes the log after
/// each. Returns the runs made.
pub fn execute(settings: &Settings, settings_path: &Path, job: &Job) -> Option<Vec<(Uuid, Run)>> {
    let _lock = lock()?;
    let _awake = Awake::new();
    let log_path = warmup::log_path(settings_path);
    let mut log = warmup::load_log(&log_path);
    let todo: Vec<(Uuid, u8, bool)> = match job {
        Job::Scheduled => warmup::due(
            &settings.warmup,
            &settings.accounts,
            &log,
            Utc::now(),
            &Zone::Local,
        )
        .into_iter()
        .map(|Due { account, attempt }| (account, attempt, false))
        .collect(),
        Job::Now(ids) => ids.iter().map(|id| (*id, 1, true)).collect(),
    };
    let cwd = work_dir(settings_path);
    let mut out = vec![];
    for (id, attempt, manual) in todo {
        let Some(account) = settings.accounts.iter().find(|a| a.id == id) else {
            continue;
        };
        let outcome = match warmup::skip(settings, account, Utc::now()) {
            Some(o) => o,
            None => {
                let profile = account.profile_dir.as_deref().unwrap_or(Path::new("."));
                let exe =
                    launch::find_cli(account.provider, settings.tools.exe_for(account.provider));
                match warmup::run(
                    account.provider,
                    exe.as_deref(),
                    profile,
                    &cwd,
                    StdDuration::from_secs(warmup::TIMEOUT_SECS),
                ) {
                    Ok(()) => Outcome::Started,
                    Err(m) => Outcome::Failed(m),
                }
            }
        };
        let run = Run {
            at: Utc::now(),
            outcome,
            manual,
            attempt,
        };
        log.last.insert(id, run.clone());
        let _ = warmup::save_log(&log_path, &log);
        out.push((id, run));
    }
    Some(out)
}

/// Demo: records "nothing was started" for the job, never runs a CLI.
pub fn pretend(settings: &Settings, settings_path: &Path, job: &Job) -> Vec<(Uuid, Run)> {
    let log_path = warmup::log_path(settings_path);
    let mut log: RunLog = warmup::load_log(&log_path);
    let ids: Vec<(Uuid, u8, bool)> = match job {
        Job::Scheduled => warmup::due(
            &settings.warmup,
            &settings.accounts,
            &log,
            Utc::now(),
            &Zone::Local,
        )
        .into_iter()
        .map(|d| (d.account, d.attempt, false))
        .collect(),
        Job::Now(ids) => ids.iter().map(|id| (*id, 1, true)).collect(),
    };
    let mut out = vec![];
    for (id, attempt, manual) in ids {
        let run = Run {
            at: Utc::now(),
            outcome: Outcome::Demo,
            manual,
            attempt,
        };
        log.last.insert(id, run.clone());
        out.push((id, run));
    }
    let _ = warmup::save_log(&log_path, &log);
    out
}

/// `ai-subhub.exe --warm-up` (the wake task): runs what is due and exits. No window, no tray,
/// no single-instance check (the lock keeps it from running twice). A failed first try is
/// retried once after [`warmup::RETRY_MIN`] minutes while the process waits (the PC stays
/// awake), so a wake-only run gets its retry too.
pub fn headless() {
    let path = settings::default_path();
    let Ok(s) = settings::load(&path) else {
        return;
    };
    if !s.warmup.enabled {
        return;
    }
    let _awake = Awake::new();
    let first = execute(&s, &path, &Job::Scheduled).unwrap_or_default();
    if first.iter().any(|(_, r)| r.outcome.failed()) {
        std::thread::sleep(StdDuration::from_secs(warmup::RETRY_MIN as u64 * 60 + 5));
        let _ = execute(&s, &path, &Job::Scheduled);
    }
}
