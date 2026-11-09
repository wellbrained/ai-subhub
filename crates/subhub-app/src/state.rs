//! App state entity: settings, live statuses and the refresh engine. All provider and hub I/O
//! runs on background threads; results are applied here on the main thread.
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use chrono::{DateTime, Duration, Local, NaiveDate, Utc};
use gpui::{Context, WeakEntity};
use subhub_core::billing;
use subhub_core::detect::{self, Attach, DetectedProfile, ImportReport};
use subhub_core::history::{self, History};
use subhub_core::hub::{HubAccount, HubClient, HubError};
use subhub_core::launch::{self, DesktopApps, LaunchEnv, LoginCompletion, Target};
use subhub_core::model::{Account, ProviderKind, ReadSource, UsageResult, UsageState};
use subhub_core::profile::ProfileEnv;
use subhub_core::projects;
use subhub_core::schedule::Schedule;
use subhub_core::settings::{self, DisplaySettings, Settings, Terminal};
use subhub_core::stats::{self, PriceBook, Scan};
use subhub_core::{merge, refresh, secrets};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub enum HubStatus {
    Off,
    NoKey,
    Unknown,
    Ok(usize),
    Unreachable,
    Unauthorized,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct AccountStatus {
    pub state: UsageState,
    pub message: String,
    pub checked_at: DateTime<Utc>,
    pub source: ReadSource,
    pub next_attempt: DateTime<Utc>,
}

/// A settings page another window asks the Settings window to open ("Manage projects…").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTarget {
    General,
    Appearance,
    Accounts,
    Hub,
    About,
    Projects,
    /// Accounts → Add accounts (first start without accounts).
    AddAccounts,
    /// One account's page (a card's ⋯ menu → Account settings…).
    Account(Uuid),
}

/// The floating menus on a popover account card (one open at a time).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardMenu {
    /// The rocket: Launch in a project.
    Launch,
    /// The ⋯ menu: hub routing ("Use for new hub threads" / "Back to Auto"), account settings.
    More,
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

pub struct AppState {
    pub settings: Settings,
    path: PathBuf,
    pub status: HashMap<Uuid, AccountStatus>,
    pub hub: HubStatus,
    pub refreshing: bool,
    pub last_refresh: Option<DateTime<Utc>>,
    /// Shown once in Settings after a load problem (e.g. corrupt file kept as .bad).
    pub notice: Option<String>,
    /// A priority write is in flight.
    steering_busy: bool,
    /// A hub sign-in waiting for the browser (Settings → Hub → Add): provider, the hub's state.
    pub hub_login: Option<(ProviderKind, String)>,
    /// Hub logins (file names) whose removal is running.
    pub hub_removing: HashSet<String>,
    /// Compact popover: accounts opened to their full card (this session only).
    pub popover_expanded: HashSet<Uuid>,
    /// Last steering failure, shown in Settings → Hub.
    pub steering_error: Option<String>,
    /// The hub's login for an account answered 429: read its CLI profile (a different login)
    /// instead until then. Not persisted.
    pub hub_cooldown: HashMap<Uuid, DateTime<Utc>>,
    /// Bumped on every save caused by the user (not by refreshes); Settings shows "Saved".
    pub user_saves: u64,
    /// One-off message for the Settings footer from a state action (e.g. a profile was linked).
    pub message_for_user: Option<String>,
    /// Short per-account feedback for sign-in / retry (shown on the popover card and the
    /// account page); cleared by the next successful reading.
    pub account_notes: HashMap<Uuid, String>,
    /// The card menu open in the popover, and whose card (UI only, not saved).
    pub card_menu: Option<(Uuid, CardMenu)>,
    /// When a press outside closed a card menu (so that press's click on its button does not
    /// reopen it).
    pub card_menu_closed: Option<(Uuid, CardMenu, std::time::Instant)>,
    /// Accounts whose sign-in terminal was started and is being watched.
    pub signing_in: HashSet<Uuid>,
    /// Retries asked for while a refresh was running; run right after it.
    queued_retry: HashSet<Uuid>,
    /// Billing reminders to show as tray balloons (title, text); drained by main.
    pub pending_notices: Vec<(String, String)>,
    /// Recent 5-hour readings per account (run-out nudge pace). Not persisted.
    nudge_trails: HashMap<Uuid, Vec<subhub_core::nudge::Point>>,
    /// Latest usage-statistics scan (shared with the Statistics window) and its bookkeeping.
    pub stats: Option<std::sync::Arc<Scan>>,
    pub stats_scanning: bool,
    /// Bumped when a scan finishes or prices change, so views know to recompute their summary.
    pub stats_rev: u64,
    pub stats_error: Option<String>,
    /// Every successful reading is appended here for the Limits chart (later).
    pub history: History,
    /// Fetched LiteLLM prices (read-only; layered over the built-in table) and update state.
    pub prices: PriceBook,
    pub prices_updating: bool,
    prices_tried: Option<std::time::Instant>,
    /// Desktop apps found once at startup (Launch menu); empty until the check ends.
    pub desktop: DesktopApps,
    /// A page the Settings window should open as soon as it can (taken by it).
    pub pending_settings_page: Option<SettingsTarget>,
    /// Self-update status (see `updater.rs`).
    pub update: crate::updater::UpdateState,
    /// The popover's top edge is being dragged (an automatic restart waits for it).
    pub popover_dragging: bool,
    /// Patch notes waiting for the popover's next opening (shown as a dialog over it until the
    /// user closes it). Not saved; recomputed at every start.
    pub pending_notes: Option<PendingNotes>,
    /// The Overview row the Statistics window should open (Settings -> General "Set up…").
    pub pending_overview_row: Option<subhub_core::insights::TipRow>,
    /// Warm-up: the shared run log (re-read before each tick), what is running now, and the
    /// last wake-task error.
    pub warmup_log: subhub_core::warmup::RunLog,
    pub warmup_running: HashSet<Uuid>,
    pub warmup_task_error: Option<String>,
}

/// The "Updated to vX.Y.Z" dialog's content.
#[derive(Debug, Clone)]
pub struct PendingNotes {
    pub version: subhub_core::update::Version,
    /// Newest first.
    pub sections: Vec<subhub_core::changelog::Section>,
    /// Closing it saves `last_seen_version` (false for the `--show-whats-new` test flag).
    pub remember: bool,
}

impl AppState {
    pub fn load() -> Self {
        let path = settings::default_path();
        let (settings, notice) = match settings::load(&path) {
            Ok(s) => (s, None),
            Err(e) => (
                Settings::default(),
                Some(format!("Settings could not be read: {e}")),
            ),
        };
        let hub = if settings.hub.enabled {
            HubStatus::Unknown
        } else {
            HubStatus::Off
        };
        Self {
            settings,
            path,
            status: HashMap::new(),
            hub,
            refreshing: false,
            last_refresh: None,
            notice,
            steering_busy: false,
            hub_login: None,
            hub_removing: HashSet::new(),
            popover_expanded: HashSet::new(),
            steering_error: None,
            hub_cooldown: HashMap::new(),
            user_saves: 0,
            message_for_user: None,
            account_notes: HashMap::new(),
            card_menu: None,
            card_menu_closed: None,
            signing_in: HashSet::new(),
            queued_retry: HashSet::new(),
            pending_notices: vec![],
            nudge_trails: HashMap::new(),
            stats: None,
            stats_scanning: false,
            stats_rev: 0,
            stats_error: None,
            history: History::open(history::default_path()),
            prices: PriceBook::load(&stats::prices_path()),
            prices_updating: false,
            prices_tried: None,
            desktop: DesktopApps::default(),
            pending_settings_page: None,
            update: crate::updater::UpdateState::new(),
            popover_dragging: false,
            pending_notes: None,
            pending_overview_row: None,
            warmup_log: subhub_core::warmup::load_log(&subhub_core::warmup::log_path(
                &settings::default_path(),
            )),
            warmup_running: HashSet::new(),
            warmup_task_error: None,
        }
    }

    // ---- warm-up (docs/features/overview.md, "Warm-up") ----

    /// Once a minute: runs the scheduled warm-ups that are due (catch-up included).
    pub fn warmup_tick(&mut self, cx: &mut Context<Self>) {
        if !self.warmup_running.is_empty() || !self.settings.warmup.enabled {
            return;
        }
        let log_path = subhub_core::warmup::log_path(&self.path);
        self.warmup_log = subhub_core::warmup::load_log(&log_path);
        let due = subhub_core::warmup::due(
            &self.settings.warmup,
            &self.settings.accounts,
            &self.warmup_log,
            Utc::now(),
            &subhub_core::insights::Zone::Local,
        );
        if due.is_empty() {
            return;
        }
        let ids = due.iter().map(|d| d.account).collect();
        self.start_warmup(crate::warmup_run::Job::Scheduled, ids, cx);
    }

    /// "Start a window now" for the chosen accounts.
    pub fn warmup_now(&mut self, ids: Vec<Uuid>, cx: &mut Context<Self>) {
        if !self.warmup_running.is_empty() || ids.is_empty() {
            return;
        }
        let set = ids.iter().copied().collect();
        self.start_warmup(crate::warmup_run::Job::Now(ids), set, cx);
    }

    fn start_warmup(
        &mut self,
        job: crate::warmup_run::Job,
        ids: HashSet<Uuid>,
        cx: &mut Context<Self>,
    ) {
        self.warmup_running = ids;
        cx.notify();
        let settings = self.settings.clone();
        let path = self.path.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let runs = cx
                .background_executor()
                .spawn(async move {
                    if crate::demo::on() {
                        Some(crate::warmup_run::pretend(&settings, &path, &job))
                    } else {
                        crate::warmup_run::execute(&settings, &path, &job)
                    }
                })
                .await;
            let started: Vec<Uuid> = runs
                .unwrap_or_default()
                .into_iter()
                .filter(|(_, r)| r.outcome == subhub_core::warmup::Outcome::Started)
                .map(|(id, _)| id)
                .collect();
            let _ = this.update(cx, |s, cx| {
                s.warmup_running.clear();
                s.warmup_log =
                    subhub_core::warmup::load_log(&subhub_core::warmup::log_path(&s.path));
                cx.notify();
            });
            if started.is_empty() {
                return;
            }
            // Read the accounts again once the provider has counted the prompt.
            cx.background_executor()
                .timer(std::time::Duration::from_secs(
                    subhub_core::warmup::REFRESH_AFTER_SECS,
                ))
                .await;
            for id in started {
                let _ = this.update(cx, |s, cx| s.refresh_account(id, cx));
            }
        })
        .detach();
    }

    /// Changes the warm-up settings, saves, and brings the wake task in line.
    pub fn set_warmup(
        &mut self,
        f: impl FnOnce(&mut subhub_core::warmup::WarmupSettings),
        cx: &mut Context<Self>,
    ) {
        f(&mut self.settings.warmup);
        self.save_user();
        self.sync_wake_task(cx);
        cx.notify();
    }

    /// Creates, updates or removes the wake task to match the settings (in the background).
    /// Never in demo mode, and never from a dev build (the task would point at the build's exe):
    /// the installed app owns it.
    pub fn sync_wake_task(&mut self, cx: &mut Context<Self>) {
        if crate::demo::on() || self.dev_build() {
            return;
        }
        let w = self.settings.warmup.clone();
        let want = w.enabled && w.wake_pc && !w.days.is_empty();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if want {
                        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                        let today = chrono::Local::now().date_naive();
                        subhub_core::warmup::install_task(&exe, &w, today)
                    } else {
                        subhub_core::warmup::remove_task()
                    }
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.warmup_task_error = result
                    .err()
                    .map(|e| format!("The wake task could not be changed: {e}"));
                cx.notify();
            });
        })
        .detach();
    }

    /// Running from cargo's target folder.
    pub fn dev_build(&self) -> bool {
        std::env::current_exe()
            .map(|p| subhub_core::update::is_dev_build(&p))
            .unwrap_or(false)
    }

    /// At start: decides whether patch notes are waiting (nothing opens by itself; the popover
    /// shows them next time it opens). A fresh install, a downgrade or no notes for the versions
    /// in between only remember the version, silently. `force` (the `--show-whats-new` start
    /// flag) pretends the previous version was 0.1.0 and saves nothing.
    pub fn prepare_whats_new(&mut self, updated_from: Option<&str>, force: bool) {
        let running = crate::updater::running_version();
        let last_seen = if force {
            "0.1.0"
        } else {
            self.settings.updates.last_seen_version.as_str()
        };
        let wn = subhub_core::update::whats_new(
            last_seen,
            updated_from,
            &running,
            crate::updater::CHANGELOG,
        );
        if !wn.sections.is_empty() {
            self.pending_notes = Some(PendingNotes {
                version: running,
                sections: wn.sections,
                remember: !force,
            });
        } else if wn.mark_seen && !force {
            self.settings.updates.last_seen_version = running.to_string();
            self.save();
        }
    }

    /// The dialog was closed (Got it, scrim, Esc): only now the notes count as seen.
    pub fn dismiss_whats_new(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.pending_notes.take() {
            if p.remember {
                self.settings.updates.last_seen_version = p.version.to_string();
                self.save();
            }
            cx.notify();
        }
    }

    /// Detects the installed desktop apps once, on a background thread with a timeout.
    pub fn detect_desktop_apps(&mut self, cx: &mut Context<Self>) {
        if crate::demo::on() {
            self.desktop = crate::demo::desktop_apps();
            return;
        }
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let found = cx
                .background_executor()
                .spawn(async { launch::detect_desktop_apps(std::time::Duration::from_secs(20)) })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.desktop = found;
                cx.notify();
            });
        })
        .detach();
    }

    // ---- usage statistics ----

    /// Scans the session logs on a background thread (incremental, cached on disk). The
    /// Statistics window shows "Scanning…" meanwhile; nothing else waits for it.
    pub fn start_scan(&mut self, cx: &mut Context<Self>) {
        if self.stats_scanning {
            return;
        }
        self.stats_scanning = true;
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let result = cx
                .background_executor()
                .spawn(async { stats::scan_default() })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.stats_scanning = false;
                match result {
                    Ok(scan) => {
                        s.stats = Some(std::sync::Arc::new(scan));
                        s.stats_error = None;
                    }
                    Err(e) => s.stats_error = Some(format!("Scan failed: {e}")),
                }
                s.stats_rev += 1;
                // A model the price list does not know yet: ask again (at most hourly).
                let missing = s.seen_models().iter().any(|m| s.prices.price(m).is_none());
                if missing {
                    s.update_prices(false, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Model ids seen in the last scan.
    fn seen_models(&self) -> std::collections::HashSet<String> {
        self.stats
            .as_ref()
            .map(|s| s.buckets.iter().map(|b| b.model.clone()).collect())
            .unwrap_or_default()
    }

    /// Updates the LiteLLM price list on a background thread when it is older than a day
    /// (or now when forced). Automatic tries wait an hour after a failed one. Prices are never typed in.
    pub fn update_prices(&mut self, force: bool, cx: &mut Context<Self>) {
        let now = Utc::now();
        if self.prices_updating
            || (!force
                && (!self.prices.is_stale(now)
                    || self
                        .prices_tried
                        .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(3600))))
        {
            return;
        }
        self.prices_updating = true;
        self.prices_tried = Some(std::time::Instant::now());
        let wanted = self.seen_models();
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let book = cx
                .background_executor()
                .spawn(async move {
                    stats::refresh_prices(&stats::prices_path(), &wanted, force, Utc::now())
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.prices = book;
                s.prices_updating = false;
                s.stats_rev += 1;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn save(&mut self) {
        if let Err(e) = settings::save(&self.path, &self.settings) {
            self.notice = Some(format!("Settings could not be saved: {e}"));
        }
    }

    /// Save after a change the user made (Settings footer confirms it).
    pub fn save_user(&mut self) {
        self.save();
        self.user_saves += 1;
    }

    pub fn display(&self) -> &DisplaySettings {
        &self.settings.display
    }

    pub fn account(&self, id: Uuid) -> Option<&Account> {
        self.settings.accounts.iter().find(|a| a.id == id)
    }

    /// Last check failed, or the reading is older than two refresh intervals.
    pub fn is_stale(&self, id: Uuid) -> bool {
        let interval = Duration::minutes(self.settings.refresh_minutes.max(1) as i64);
        let failed = self
            .status
            .get(&id)
            .is_some_and(|s| s.state != UsageState::Ready);
        let old = self
            .settings
            .readings
            .get(&id)
            .is_some_and(|r| Utc::now() - r.fetched_at > interval * 2);
        failed || old
    }

    pub fn next_update(&self) -> Option<DateTime<Utc>> {
        self.status.values().map(|s| s.next_attempt).min()
    }

    // ---- refresh engine ----

    /// Lists the hub (when on), merges its logins, then reads every due account.
    /// `force` = manual refresh: skips failure backoff, never a provider cooldown.
    pub fn refresh(&mut self, force: bool, cx: &mut Context<Self>) {
        self.refresh_filtered(force, None, cx);
    }

    /// Retry for one account: skips failure backoff, never a provider cooldown (then it only
    /// says when the cooldown ends). Runs right after a refresh that is already going.
    pub fn refresh_account(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let now = Utc::now();
        if let Some(cached) = self.settings.schedules.get(&id).cloned() {
            let sched = Schedule::from_cached(cached);
            if sched.cooling_down(now) {
                let msg = format!(
                    "Usage checks are cooling down (provider limit) · next try in {}.",
                    crate::format::duration_until(sched.0.next_attempt, now)
                );
                self.account_notes.insert(id, msg);
                cx.notify();
                return;
            }
        }
        self.account_notes.remove(&id);
        if self.refreshing {
            self.queued_retry.insert(id);
            cx.notify();
            return;
        }
        self.refresh_filtered(true, Some(HashSet::from([id])), cx);
    }

    fn refresh_filtered(
        &mut self,
        force: bool,
        only: Option<HashSet<Uuid>>,
        cx: &mut Context<Self>,
    ) {
        if self.refreshing {
            return;
        }
        if crate::demo::on() {
            self.demo_refresh(force, only, cx);
            return;
        }
        self.refreshing = true;
        cx.notify();
        let hub_cfg = self.settings.hub.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            // 1. Hub listing (local HTTP, cheap).
            let listing = cx
                .background_executor()
                .spawn(async move {
                    if !hub_cfg.enabled {
                        return (HubStatus::Off, None, None);
                    }
                    let key = match secrets::hub_key() {
                        Ok(Some(k)) => k,
                        _ => return (HubStatus::NoKey, None, None),
                    };
                    let client = HubClient::new(hub_cfg.base_url.clone(), key.clone());
                    match client.list_accounts() {
                        Ok(list) => (HubStatus::Ok(list.len()), Some(list), Some(key)),
                        Err(HubError::Unreachable) => (HubStatus::Unreachable, None, Some(key)),
                        Err(HubError::Unauthorized) => (HubStatus::Unauthorized, None, None),
                        Err(HubError::Other(m)) => (HubStatus::Error(m), None, Some(key)),
                    }
                })
                .await;
            let (hub_status, list, key) = listing;
            let Ok((due, base_url, codex_exe, cooled)) = this.update(cx, |s, cx| {
                s.apply_listing(hub_status.clone(), list.as_deref());
                cx.notify();
                let now = Utc::now();
                s.hub_cooldown.retain(|_, until| *until > now);
                let cooled: std::collections::HashSet<Uuid> =
                    s.hub_cooldown.keys().copied().collect();
                (
                    s.due_accounts(force, only.as_ref()),
                    s.settings.hub.base_url.clone(),
                    s.settings.tools.codex_exe.clone(),
                    cooled,
                )
            }) else {
                return;
            };

            // 2. Read due accounts in parallel threads.
            let hub_usable = matches!(hub_status, HubStatus::Ok(_));
            let results = cx
                .background_executor()
                .spawn(async move {
                    let client = key
                        .filter(|_| hub_usable)
                        .map(|k| HubClient::new(base_url, k));
                    std::thread::scope(|scope| {
                        let handles: Vec<_> = due
                            .iter()
                            .map(|acc| {
                                let exe = codex_exe.as_deref();
                                let has_profile = acc.profile_dir.is_some();
                                // The hub's login is cooling down: use the account's own CLI
                                // profile (a different login) until then.
                                let client = client
                                    .as_ref()
                                    .filter(|_| !(has_profile && cooled.contains(&acc.id)));
                                scope.spawn(move || {
                                    let r = refresh::read_account(acc, client, exe);
                                    if r.state == UsageState::RateLimited
                                        && r.source == ReadSource::Hub
                                        && has_profile
                                    {
                                        // Only the hub's login is throttled (seen 2026-11-06:
                                        // the profile login of the same account still read
                                        // fine). Read the profile now, and skip the hub for
                                        // this account until its cooldown ends.
                                        let until = Utc::now()
                                            + Duration::seconds(
                                                r.retry_after_secs.unwrap_or(600) as i64
                                            );
                                        let p = refresh::read_account(acc, None, exe);
                                        return (acc.id, p, Some(until));
                                    }
                                    (acc.id, r, None)
                                })
                            })
                            .collect();
                        handles
                            .into_iter()
                            .filter_map(|h| h.join().ok())
                            .collect::<Vec<_>>()
                    })
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                let mut plain = Vec::with_capacity(results.len());
                for (id, r, cooled_until) in results {
                    if let Some(until) = cooled_until {
                        s.hub_cooldown.insert(id, until);
                    }
                    plain.push((id, r));
                }
                s.apply_results(plain);
                s.refreshing = false;
                s.last_refresh = Some(Utc::now());
                s.steer(cx);
                cx.notify();
                // Retries asked for during this round run now.
                let queued: HashSet<Uuid> = std::mem::take(&mut s.queued_retry);
                if !queued.is_empty() {
                    s.refresh_filtered(true, Some(queued), cx);
                }
            });
        })
        .detach();
    }

    /// Demo mode's refresh: no provider, hub or Credential Manager. Due accounts get the
    /// fixture result (`subhub_core::demo::refresh_result`), the hub counts as connected, then
    /// the normal apply and steering (which writes nothing in the demo) run.
    fn demo_refresh(&mut self, force: bool, only: Option<HashSet<Uuid>>, cx: &mut Context<Self>) {
        let now = Utc::now();
        self.hub = if self.settings.hub.enabled {
            HubStatus::Ok(
                self.settings
                    .accounts
                    .iter()
                    .filter(|a| a.hub.is_some())
                    .count(),
            )
        } else {
            HubStatus::Off
        };
        let results = self
            .due_accounts(force, only.as_ref())
            .iter()
            .map(|a| {
                let r =
                    subhub_core::demo::refresh_result(a, self.settings.readings.get(&a.id), now);
                (a.id, r)
            })
            .collect();
        self.apply_results(results);
        self.queued_retry.clear();
        self.last_refresh = Some(now);
        self.steer(cx);
        cx.notify();
    }

    // ---- hub steering (ADR 0001 refinements: credential priority; logins below are the only
    // other hub writes, each started by the owner) ----

    /// Puts the steering target on top in the hub and restores every other login SubHub changed.
    /// Writes run in the background; originals are persisted only after all writes succeeded.
    pub fn steer(&mut self, cx: &mut Context<Self>) {
        if self.steering_busy || !matches!(self.hub, HubStatus::Ok(_)) {
            return;
        }
        let now = Utc::now();
        let target = subhub_core::steer::target(
            &self.settings,
            &self.settings.accounts,
            &self.settings.readings,
            &|id| self.is_stale(id),
            now,
        );
        let (changes, originals) =
            subhub_core::steer::plan(&self.settings.accounts, &self.settings.steering, target);
        if changes.is_empty() {
            if self.settings.steering.current != target {
                self.settings.steering.current = target;
                self.save();
            }
            return;
        }
        if crate::demo::on() {
            // No PATCH: the plan is applied to the local copy as if the hub had taken it.
            self.steering_applied(&changes, originals, target);
            self.save();
            cx.notify();
            return;
        }
        self.steering_busy = true;
        let base_url = self.settings.hub.base_url.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let to_apply = changes.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let key = secrets::hub_key()
                        .ok()
                        .flatten()
                        .ok_or("No hub key stored.")?;
                    let client = HubClient::new(base_url, key);
                    for c in &to_apply {
                        client
                            .set_priority(&c.file_name, c.priority)
                            .map_err(|e| match e {
                                HubError::Unauthorized => "The hub rejected the management key.",
                                HubError::Unreachable => "The hub is not reachable.",
                                HubError::Other(_) => "The hub did not accept the priority change.",
                            })?;
                    }
                    Ok::<(), &'static str>(())
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.steering_busy = false;
                match result {
                    Ok(()) => s.steering_applied(&changes, originals, target),
                    Err(e) => s.steering_error = Some(e.to_string()),
                }
                s.save();
                cx.notify();
            });
        })
        .detach();
    }

    // ---- hub logins (owner, 2026-11-09: add and remove CLI accounts in the hub) ----

    /// Removes the account's login from the hub (deletes its auth file there). The account stays
    /// in SubHub: the next listing drops its hub link, so it reads through its CLI profile, or
    /// shows Not connected without one.
    pub fn remove_hub_login(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(acc) = self.account(id).cloned() else {
            return;
        };
        let Some(file) = acc.hub.as_ref().map(|h| h.file_name.clone()) else {
            return;
        };
        if crate::demo::on() {
            self.message_for_user = Some(crate::demo::NOT_AVAILABLE.into());
            cx.notify();
            return;
        }
        if !self.hub_removing.insert(file.clone()) {
            return;
        }
        cx.notify();
        let base_url = self.settings.hub.base_url.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let f = file.clone();
            let result = cx
                .background_executor()
                .spawn(async move { hub_client(base_url)?.remove_login(&f).map_err(hub_text) })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.hub_removing.remove(&file);
                s.message_for_user = Some(match result {
                    Ok(()) => {
                        let stays = if acc.profile_dir.is_some() {
                            "as a CLI account"
                        } else {
                            "as Not connected (no CLI profile)"
                        };
                        format!(
                            "Removed {} from the hub. It stays in SubHub {stays}.",
                            acc.label
                        )
                    }
                    Err(e) => format!("Could not remove {} from the hub: {e}", acc.label),
                });
                s.refresh(true, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Adds a login to the hub: the hub starts its sign-in, the browser opens it, and SubHub
    /// waits (up to 10 min) until the hub reports it done, then lists the hub again.
    pub fn start_hub_login(&mut self, provider: ProviderKind, cx: &mut Context<Self>) {
        if crate::demo::on() {
            self.message_for_user = Some(crate::demo::NOT_AVAILABLE.into());
            cx.notify();
            return;
        }
        if self.hub_login.is_some() {
            return;
        }
        self.hub_login = Some((provider, String::new()));
        cx.notify();
        let base_url = self.settings.hub.base_url.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let b = base_url.clone();
            let started = cx
                .background_executor()
                .spawn(async move { hub_client(b)?.start_login(provider).map_err(hub_text) })
                .await;
            let start = match started {
                Ok(s) => s,
                Err(e) => {
                    let _ = this.update(cx, |s, cx| {
                        s.hub_login = None;
                        s.message_for_user = Some(format!("Could not start the sign-in: {e}"));
                        cx.notify();
                    });
                    return;
                }
            };
            let state = start.state.clone();
            // Cancelled while the hub was starting: drop it there too.
            let go = this
                .update(cx, |s, cx| {
                    if s.hub_login.is_none() {
                        return false;
                    }
                    s.hub_login = Some((provider, state.clone()));
                    cx.open_url(&start.url);
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !go {
                let (b, st) = (base_url.clone(), state.clone());
                cx.background_executor()
                    .spawn(async move {
                        if let Ok(c) = hub_client(b) {
                            c.cancel_login(&st)
                        }
                    })
                    .await;
                return;
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
            let outcome = loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;
                // Cancelled (or replaced) meanwhile.
                let still = this
                    .update(cx, |s, _| {
                        s.hub_login.as_ref().is_some_and(|(_, st)| *st == state)
                    })
                    .unwrap_or(false);
                if !still {
                    return;
                }
                if std::time::Instant::now() > deadline {
                    let (b, st) = (base_url.clone(), state.clone());
                    cx.background_executor()
                        .spawn(async move {
                            if let Ok(c) = hub_client(b) {
                                c.cancel_login(&st)
                            }
                        })
                        .await;
                    break Err("No sign-in within 10 minutes.".to_string());
                }
                let (b, st) = (base_url.clone(), state.clone());
                let status = cx
                    .background_executor()
                    .spawn(async move { hub_client(b)?.login_status(&st).map_err(hub_text) })
                    .await;
                match status {
                    Ok(subhub_core::hub::LoginStatus::Waiting) => continue,
                    Ok(subhub_core::hub::LoginStatus::Done) => break Ok(()),
                    Ok(subhub_core::hub::LoginStatus::Failed(e)) => break Err(e),
                    // A hiccup while waiting: keep trying until the deadline.
                    Err(_) => continue,
                }
            };
            let _ = this.update(cx, |s, cx| {
                s.hub_login = None;
                s.message_for_user = Some(match outcome {
                    Ok(()) => format!(
                        "Signed in. The new {} login shows up in a moment.",
                        provider_name(provider)
                    ),
                    Err(e) => format!("The sign-in did not finish: {e}"),
                });
                s.refresh(true, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Stops waiting for a hub sign-in and tells the hub to drop it.
    pub fn cancel_hub_login(&mut self, cx: &mut Context<Self>) {
        let Some((_, state)) = self.hub_login.take() else {
            return;
        };
        cx.notify();
        if state.is_empty() {
            return;
        }
        let base_url = self.settings.hub.base_url.clone();
        cx.background_executor()
            .spawn(async move {
                if let Ok(c) = hub_client(base_url) {
                    c.cancel_login(&state)
                }
            })
            .detach();
    }

    /// The hub took the priority writes: mirror them locally so the next plan sees them.
    fn steering_applied(
        &mut self,
        changes: &[subhub_core::steer::PriorityChange],
        originals: HashMap<String, Option<i64>>,
        target: Option<Uuid>,
    ) {
        for c in changes {
            for a in self.settings.accounts.iter_mut() {
                if let Some(h) = a.hub.as_mut().filter(|h| h.file_name == c.file_name) {
                    h.priority = c.priority;
                }
            }
        }
        self.settings.steering.original_priorities = originals;
        self.settings.steering.current = target;
        self.steering_error = None;
    }

    /// Clicked Hub tag: prefer this account for new hub threads; clicking it again returns to Auto.
    pub fn toggle_preferred(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let st = &mut self.settings.steering;
        st.preferred = if st.preferred == Some(id) {
            None
        } else {
            Some(id)
        };
        if st.preferred.is_some() {
            st.enabled = true;
        }
        self.save_user();
        self.steer(cx);
        cx.notify();
    }

    pub fn set_steering_enabled(&mut self, on: bool, cx: &mut Context<Self>) {
        self.settings.steering.enabled = on;
        if !on {
            self.settings.steering.preferred = None;
        }
        self.save_user();
        self.steer(cx);
        cx.notify();
    }

    fn apply_listing(&mut self, status: HubStatus, list: Option<&[HubAccount]>) {
        self.hub = status;
        if let Some(list) = list {
            let report = merge::apply_hub_accounts(&mut self.settings.accounts, list);
            if report != merge::MergeReport::default() {
                self.save();
            }
        }
    }

    fn due_accounts(&mut self, force: bool, only: Option<&HashSet<Uuid>>) -> Vec<Account> {
        let now = Utc::now();
        self.settings
            .accounts
            .iter()
            .filter(|a| only.is_none_or(|set| set.contains(&a.id)))
            .filter(|a| {
                let sched = self
                    .settings
                    .schedules
                    .get(&a.id)
                    .cloned()
                    .map(Schedule::from_cached)
                    .unwrap_or_else(|| Schedule::new(now));
                // Antigravity runs its CLI: once signed out, only a manual retry asks it again.
                let parked = a.provider == ProviderKind::Antigravity
                    && sched.0.state == UsageState::SignInRequired;
                sched.can_attempt(force, now) && (force || !parked)
            })
            .cloned()
            .collect()
    }

    fn apply_results(&mut self, results: Vec<(Uuid, UsageResult)>) {
        let now = Utc::now();
        let interval = self.settings.refresh_minutes;
        for (id, result) in results {
            let mut sched = self
                .settings
                .schedules
                .get(&id)
                .cloned()
                .map(Schedule::from_cached)
                .unwrap_or_else(|| Schedule::new(now));
            sched.complete(&result, now, interval);
            let next_attempt = sched.0.next_attempt;
            self.settings.schedules.insert(id, sched.0);
            if result.state == UsageState::Ready
                && let Some(reading) = result.reading.clone()
            {
                self.history.record(id, &reading);
                self.trail_point(id, &reading);
                self.settings.readings.insert(id, reading);
            }
            if let Some(acc) = self.settings.accounts.iter_mut().find(|a| a.id == id)
                && acc.email.is_none()
            {
                acc.email = result.email.clone();
            }
            if let Some(plan) = &result.plan {
                self.settings.plans.insert(id, plan.clone());
            }
            if result.state == UsageState::Ready {
                self.account_notes.remove(&id);
            }
            self.status.insert(
                id,
                AccountStatus {
                    state: result.state,
                    message: result.message.clone(),
                    checked_at: now,
                    source: result.source,
                    next_attempt,
                },
            );
        }
        self.fold_duplicates();
        self.check_nudges();
        self.save();
    }

    /// Keeps the last hour of 5-hour readings per account for the run-out nudge's pace.
    fn trail_point(&mut self, id: Uuid, reading: &subhub_core::model::Reading) {
        let Some(w) = reading.windows.iter().find(|w| w.is_session()) else {
            return;
        };
        let trail = self.nudge_trails.entry(id).or_default();
        if trail.last().is_some_and(|p| p.at == reading.fetched_at) {
            return;
        }
        trail.push(subhub_core::nudge::Point {
            at: reading.fetched_at,
            used: w.used_percent,
            resets_at: w.resets_at,
        });
        let cutoff = reading.fetched_at - Duration::minutes(60);
        trail.retain(|p| p.at >= cutoff);
    }

    /// Run-out nudge (owner, 2026-11-08): a tray balloon about 15 minutes before an account's
    /// 5-hour window runs out, when another account of the same provider has a fresh one. Off
    /// by default; quiet for hub accounts while steering moves them; one per window.
    fn check_nudges(&mut self) {
        use subhub_core::nudge;
        let now = Utc::now();
        nudge::prune(&mut self.settings.nudge, now);
        if !self.settings.nudge.enabled {
            return;
        }
        let steering = self.settings.steering.enabled && matches!(self.hub, HubStatus::Ok(_));
        let due = nudge::due(&nudge::Input {
            now,
            accounts: &self.settings.accounts,
            readings: &self.settings.readings,
            trails: &self.nudge_trails,
            stale: &|id| self.is_stale(id),
            settings: &self.settings.nudge,
            steering,
            refresh_minutes: self.settings.refresh_minutes,
        });
        for n in due {
            let name = |id| {
                self.account(id)
                    .map(|a| format!("{} ({})", a.label, provider_name(a.provider)))
                    .unwrap_or_default()
            };
            let reset = n
                .resets_at
                .with_timezone(&chrono::Local)
                .format("%-H:%M")
                .to_string();
            self.pending_notices.push((
                format!("{} runs out in {}", name(n.account), nudge::about(n.minutes)),
                format!(
                    "{} has a fresh 5-hour window ({:.0}% left); this one resets at {reset}. Click to see your accounts.",
                    name(n.other),
                    n.other_left
                ),
            ));
            self.settings.nudge.shown.insert(n.account, n.resets_at);
        }
    }

    pub fn set_nudge(&mut self, on: bool) {
        self.settings.nudge.enabled = on;
        self.user_changed();
    }

    /// A profile account that just learned its email absorbs the hub-only account for the same
    /// login (merge::fold_duplicates); its reading, schedule, status and plan move with it when
    /// the kept account has none yet.
    fn fold_duplicates(&mut self) {
        for (removed, kept) in merge::fold_duplicates(&mut self.settings.accounts) {
            if let Some(r) = self.settings.readings.remove(&removed) {
                self.settings.readings.entry(kept).or_insert(r);
            }
            if let Some(sc) = self.settings.schedules.remove(&removed) {
                self.settings.schedules.entry(kept).or_insert(sc);
            }
            if let Some(st) = self.status.remove(&removed) {
                self.status.entry(kept).or_insert(st);
            }
            if let Some(p) = self.settings.plans.remove(&removed) {
                self.settings.plans.entry(kept).or_insert(p);
            }
            if let Some(p) = self.settings.manual_plans.remove(&removed) {
                self.settings.manual_plans.entry(kept).or_insert(p);
            }
            if let Some(until) = self.hub_cooldown.remove(&removed) {
                self.hub_cooldown.entry(kept).or_insert(until);
            }
            if let Some(b) = self.settings.billing.remove(&removed) {
                self.settings.billing.entry(kept).or_insert(b);
            }
            if let Some(m) = self.settings.billing_reminders.remove(&removed) {
                self.settings.billing_reminders.entry(kept).or_insert(m);
            }
            self.account_notes.remove(&removed);
            // Keep what the user set on the hub-only account: tag, star, steering pick.
            if let Some(tag) = self.settings.account_tags.remove(&removed) {
                self.settings.account_tags.entry(kept).or_insert(tag);
            }
            let favs = &mut self.settings.display.taskbar_favorites;
            if let Some(i) = favs.iter().position(|f| *f == removed) {
                if favs.contains(&kept) {
                    favs.remove(i);
                } else {
                    favs[i] = kept;
                }
            }
            let steer = &mut self.settings.steering;
            if steer.preferred == Some(removed) {
                steer.preferred = Some(kept);
            }
            if steer.current == Some(removed) {
                steer.current = Some(kept);
            }
        }
    }

    // ---- account management (all persist immediately) ----

    pub fn rename(&mut self, id: Uuid, label: &str) {
        let label = label.trim();
        if label.is_empty() || label.chars().count() > 60 {
            return;
        }
        if let Some(a) = self.settings.accounts.iter_mut().find(|a| a.id == id) {
            a.label = label.to_string();
            self.save_user();
        }
    }

    /// Moves within the same provider (views group by provider).
    pub fn move_account(&mut self, id: Uuid, up: bool) {
        let accs = &mut self.settings.accounts;
        let Some(i) = accs.iter().position(|a| a.id == id) else {
            return;
        };
        let provider = accs[i].provider;
        let target = if up {
            (0..i).rev().find(|&j| accs[j].provider == provider)
        } else {
            (i + 1..accs.len()).find(|&j| accs[j].provider == provider)
        };
        if let Some(j) = target {
            accs.swap(i, j);
            self.save_user();
        }
    }

    pub fn set_group(&mut self, id: Uuid, group: Option<String>) {
        let group = group
            .map(|g| g.trim().to_string())
            .filter(|g| !g.is_empty());
        if let Some(g) = &group
            && !self.settings.groups.contains(g)
        {
            self.settings.groups.push(g.clone());
        }
        if let Some(a) = self.settings.accounts.iter_mut().find(|a| a.id == id) {
            a.group = group;
        }
        self.save_user();
    }

    pub fn toggle_hidden(&mut self, id: Uuid) {
        if let Some(a) = self.settings.accounts.iter_mut().find(|a| a.id == id) {
            a.hidden = !a.hidden;
            self.save_user();
        }
    }

    pub fn toggle_window_hidden(&mut self, id: Uuid, key: &str) {
        if let Some(a) = self.settings.accounts.iter_mut().find(|a| a.id == id) {
            toggle(&mut a.hidden_windows, key);
            a.pinned_windows.retain(|k| k != key);
            self.save_user();
        }
    }

    /// ☆/★ on a card: show this account on the taskbar (Favorites mode, first 3 in star order).
    /// Own badge tag (1–3 characters, kept as typed); empty returns to the automatic tag.
    pub fn set_account_tag(&mut self, id: Uuid, tag: &str) {
        let tag: String = tag.trim().chars().take(3).collect();
        if tag.is_empty() {
            self.settings.account_tags.remove(&id);
        } else {
            self.settings.account_tags.insert(id, tag);
        }
        self.save_user();
    }

    /// The popover height dragged by the user (None = fit the content); saved quietly.
    pub fn set_popover_height(&mut self, height: Option<f32>) {
        if self.settings.display.popover_height != height {
            self.settings.display.popover_height = height;
            self.save();
        }
    }

    pub fn toggle_taskbar_favorite(&mut self, id: Uuid) {
        let favs = &mut self.settings.display.taskbar_favorites;
        if let Some(i) = favs.iter().position(|f| *f == id) {
            favs.remove(i);
        } else {
            favs.push(id);
        }
        self.save_user();
    }

    /// Disconnects the record only; profile and hub login stay untouched.
    pub fn remove(&mut self, id: Uuid) {
        self.settings.accounts.retain(|a| a.id != id);
        self.settings.readings.remove(&id);
        self.settings.schedules.remove(&id);
        self.settings.billing.remove(&id);
        self.settings.billing_reminders.remove(&id);
        self.settings.plans.remove(&id);
        self.settings.manual_plans.remove(&id);
        self.account_notes.remove(&id);
        self.status.remove(&id);
        self.save_user();
    }

    /// The plan chosen by hand on the account page (Antigravity); None clears it.
    pub fn set_manual_plan(&mut self, id: Uuid, plan: Option<&str>) {
        match plan {
            Some(p) => {
                self.settings.manual_plans.insert(id, p.to_string());
            }
            None => {
                self.settings.manual_plans.remove(&id);
            }
        }
        self.save_user();
    }

    pub fn add_profile(
        &mut self,
        provider: ProviderKind,
        label: &str,
        dir: &str,
    ) -> Result<(), String> {
        let label = label.trim();
        let dir = dir.trim();
        if label.is_empty() || label.chars().count() > 60 {
            return Err("Use a name from 1 to 60 characters.".into());
        }
        let path = PathBuf::from(dir);
        if !path.is_dir() {
            return Err("That folder does not exist.".into());
        }
        let account = Account {
            id: Uuid::new_v4(),
            provider,
            label: label.into(),
            email: None,
            profile_dir: Some(path),
            hub: None,
            group: None,
            hidden: false,
            hidden_windows: vec![],
            pinned_windows: vec![],
        };
        // No duplicates (owner rule): see `detect::attach_profile`.
        match detect::attach_profile(&mut self.settings.accounts, account) {
            Attach::Duplicate => Err("This profile is already connected.".into()),
            Attach::Linked { label, .. } => {
                self.save_user();
                self.message_for_user = Some(format!(
                    "Linked the profile to {label}, which already uses this login."
                ));
                Ok(())
            }
            Attach::Added(_) => {
                self.save_user();
                Ok(())
            }
        }
    }

    /// Profile detection against the current accounts (reads only folder names and the email
    /// metadata; cheap enough for the UI thread: a handful of small files).
    pub fn detect_profiles(&self) -> Vec<DetectedProfile> {
        let Some(home) = subhub_core::profile::home_dir() else {
            return vec![];
        };
        detect::detect(&home, &ProfileEnv::from_process(), &self.settings.accounts)
    }

    /// Imports the picked profiles, saves, starts a refresh and returns the report plus a
    /// summary for the toast.
    pub fn import_profiles(
        &mut self,
        picks: &[DetectedProfile],
        cx: &mut Context<Self>,
    ) -> (ImportReport, String) {
        let report = detect::import(&mut self.settings.accounts, picks);
        // New account or linked to the one with this login: the same to the user (owner).
        let n = report.added.len() + report.linked.len();
        let msg = match n {
            0 => "Nothing to import: these profiles are already added.".to_string(),
            n => format!("Imported {n} profile{}.", plural(n)),
        };
        if n > 0 {
            self.save_user();
            self.refresh(false, cx);
        }
        (report, msg)
    }

    // ---- projects ----

    /// Saves a folder as a project (label = folder name); the same folder never twice.
    pub fn add_project(&mut self, dir: PathBuf) -> Result<(), String> {
        projects::add(&mut self.settings.projects, "", &dir)
            .map_err(|e| e.message().to_string())?;
        self.save_user();
        Ok(())
    }

    pub fn rename_project(&mut self, id: Uuid, label: &str) {
        let label = label.trim();
        if label.is_empty() || label.chars().count() > projects::MAX_LABEL {
            return;
        }
        if let Some(p) = self.settings.projects.iter_mut().find(|p| p.id == id)
            && p.label != label
        {
            p.label = label.to_string();
            self.save_user();
        }
    }

    pub fn move_project(&mut self, id: Uuid, up: bool) {
        if projects::move_project(&mut self.settings.projects, id, up) {
            self.save_user();
        }
    }

    pub fn set_project_hidden(&mut self, id: Uuid, hidden: bool) {
        if projects::set_hidden(&mut self.settings.projects, id, hidden) {
            self.save_user();
        }
    }

    /// Removes the shortcut only; the folder is never touched.
    pub fn remove_project(&mut self, id: Uuid) {
        if projects::remove(&mut self.settings.projects, id) {
            self.save_user();
        }
    }

    // ---- Launch menu ----

    /// What is installed, looked up when a Launch menu opens (cheap file checks).
    pub fn launch_env(&self, provider: ProviderKind) -> LaunchEnv {
        if crate::demo::on() {
            return crate::demo::launch_env(provider);
        }
        LaunchEnv {
            cli: self.cli_path(provider),
            windows_terminal: launch::find_windows_terminal(),
            vscode: launch::find_vscode(),
            desktop_app: self.desktop.for_provider(provider).map(str::to_string),
        }
    }

    /// Short feedback on the account's card; removed again after `secs` if still the same.
    fn note_for(&mut self, id: Uuid, text: String, secs: u64, cx: &mut Context<Self>) {
        self.account_notes.insert(id, text.clone());
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(secs))
                .await;
            let _ = this.update(cx, |s, cx| {
                if s.account_notes.get(&id) == Some(&text) {
                    s.account_notes.remove(&id);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Starts one Launch-menu target for an account and a project. Nothing here reads or passes
    /// a token; CLI launches use the same environment isolation as sign-in. Problems and the
    /// Codex Desktop reminder appear as a note on the account's card.
    pub fn launch_target(
        &mut self,
        account_id: Uuid,
        project_id: Uuid,
        target: Target,
        cx: &mut Context<Self>,
    ) {
        let Some(acc) = self.account(account_id).cloned() else {
            return;
        };
        let Some(project) = self
            .settings
            .projects
            .iter()
            .find(|p| p.id == project_id)
            .cloned()
        else {
            return;
        };
        if !project.directory.is_dir() {
            self.note_for(
                account_id,
                "The project folder no longer exists. Edit it in Settings → Projects.".into(),
                10,
                cx,
            );
            return;
        }
        if crate::demo::on() {
            self.note_for(account_id, crate::demo::NOTHING_STARTED.into(), 10, cx);
            return;
        }
        let name = provider_name(acc.provider);
        let result: Result<Option<String>, String> = match target {
            Target::WindowsTerminal | Target::PowerShell => {
                match (acc.profile_dir.as_ref(), self.cli_path(acc.provider)) {
                    (None, _) => Err(launch::HINT_NO_PROFILE.to_string()),
                    (_, None) => Err(format!(
                        "{name} CLI not found. Set its path in Settings → General → Advanced."
                    )),
                    (Some(profile), Some(exe)) => {
                        // An explicit terminal choice overrides the saved sign-in terminal.
                        let terminal = if target == Target::WindowsTerminal {
                            Terminal::WindowsTerminal
                        } else {
                            Terminal::PowerShell
                        };
                        let _ = std::fs::create_dir_all(profile);
                        launch::plan_project_cli(
                            acc.provider,
                            profile,
                            &exe,
                            &project.directory,
                            terminal,
                            launch::find_windows_terminal().as_deref(),
                        )
                        .spawn()
                        .map(|()| None)
                        .map_err(|e| format!("The terminal could not be started: {e}"))
                    }
                }
            }
            Target::VsCode => match launch::find_vscode() {
                None => Err(launch::HINT_NO_CODE.to_string()),
                Some(code) => launch::plan_vscode(&code, &project.directory)
                    .spawn()
                    .map(|()| None)
                    .map_err(|e| format!("VS Code could not be started: {e}")),
            },
            Target::Desktop => match acc.provider {
                ProviderKind::Claude => {
                    // Claude's documented link; Claude Desktop asks to confirm the folder.
                    cx.open_url(&launch::claude_desktop_uri(&project.directory));
                    Ok(None)
                }
                ProviderKind::Codex => match self.desktop.codex.clone() {
                    None => Err("Codex Desktop is not installed.".to_string()),
                    Some(id) => launch::plan_desktop_app(&id)
                        .spawn()
                        .map(|()| {
                            Some(format!(
                                "Codex Desktop opened. Choose {} there.",
                                project.label
                            ))
                        })
                        .map_err(|e| format!("Codex Desktop could not be started: {e}")),
                },
                _ => Err("Launch is not available for this provider.".to_string()),
            },
        };
        match result {
            Ok(Some(note)) => self.note_for(account_id, note, 15, cx),
            Ok(None) => {}
            Err(e) => self.note_for(account_id, e, 10, cx),
        }
    }

    /// A change made outside the settings file (e.g. the Run key): show the "Saved" toast.
    pub fn user_changed(&mut self) {
        self.user_saves += 1;
    }

    // ---- sign in / tools ----

    pub fn set_terminal(&mut self, terminal: Terminal) {
        self.settings.tools.terminal = terminal;
        self.save_user();
    }

    /// Sets (or with empty text clears) a provider's CLI path. Returns the user-facing problem.
    pub fn set_cli_path(&mut self, provider: ProviderKind, text: &str) -> Result<(), String> {
        let path = launch::validate_cli_path(text).map_err(str::to_string)?;
        match provider {
            ProviderKind::Claude => self.settings.tools.claude_exe = path,
            ProviderKind::Codex => self.settings.tools.codex_exe = path,
            _ => return Err("This provider has no CLI path setting.".into()),
        }
        // A new path may fix a "CLI missing" account: try again at the next round.
        self.save_user();
        Ok(())
    }

    /// The CLI that would be used, configured or detected.
    pub fn cli_path(&self, provider: ProviderKind) -> Option<PathBuf> {
        if crate::demo::on() {
            return crate::demo::fake_cli(provider);
        }
        launch::find_cli(provider, self.settings.tools.exe_for(provider))
    }

    /// Opens the provider CLI's login for the account's profile in the chosen terminal, then
    /// watches for the CLI to end (exit-code file) and refreshes that account. Hub-only accounts
    /// have no profile: their login belongs to the hub.
    pub fn sign_in(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(acc) = self.account(id).cloned() else {
            return;
        };
        let Some(dir) = acc.profile_dir.clone() else {
            self.account_notes.insert(
                id,
                "Sign in through the hub: this login belongs to it.".into(),
            );
            cx.notify();
            return;
        };
        if !acc.provider.is_cli_profile() {
            self.account_notes
                .insert(id, launch::app_sign_in_hint(acc.provider).into());
            cx.notify();
            return;
        }
        if self.signing_in.contains(&id) {
            return;
        }
        if crate::demo::on() {
            self.note_for(id, crate::demo::NOTHING_STARTED.into(), 10, cx);
            return;
        }
        let name = provider_name(acc.provider);
        let Some(exe) = self.cli_path(acc.provider) else {
            self.account_notes.insert(
                id,
                format!("{name} CLI not found. Set its path in Settings → General → Advanced."),
            );
            cx.notify();
            return;
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.account_notes
                .insert(id, format!("The profile folder could not be created: {e}"));
            cx.notify();
            return;
        }
        let completion = LoginCompletion::new().ok();
        let plan = launch::plan_login(
            acc.provider,
            &dir,
            &exe,
            &launch::login_cwd(&dir),
            self.settings.tools.terminal,
            launch::find_windows_terminal().as_deref(),
            completion.as_ref().map(LoginCompletion::path),
        );
        self.signing_in.insert(id);
        self.account_notes.insert(
            id,
            "Sign-in opened in a terminal. Finish it there; usage refreshes afterwards.".into(),
        );
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let started = cx
                .background_executor()
                .spawn(async move { plan.spawn() })
                .await;
            let outcome: Result<Option<i32>, String> = match (started, completion) {
                (Err(e), _) => Err(format!("The terminal could not be started: {e}")),
                (Ok(()), None) => Ok(None),
                (Ok(()), Some(done)) => loop {
                    if let Some(code) = done.poll() {
                        break Ok(Some(code));
                    }
                    if done.timed_out() {
                        break Ok(None);
                    }
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(250))
                        .await;
                },
            };
            let _ = this.update(cx, |s, cx| {
                s.signing_in.remove(&id);
                match outcome {
                    Ok(Some(0)) => {
                        s.account_notes
                            .insert(id, "Signed in. Checking usage…".into());
                        s.refresh_account(id, cx);
                    }
                    Ok(Some(_)) => {
                        s.account_notes
                            .insert(id, "Sign-in did not complete. Try again.".into());
                    }
                    Ok(None) => {
                        s.account_notes.insert(
                            id,
                            "Sign-in was not confirmed. Use Retry once you are signed in.".into(),
                        );
                    }
                    Err(e) => {
                        s.account_notes.insert(id, e);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ---- monthly billing ----

    pub fn today() -> NaiveDate {
        Local::now().date_naive()
    }

    /// Sets the account's date. `auto_renew` true = Monthly renewal, false = Access end date.
    pub fn set_billing(
        &mut self,
        id: Uuid,
        date: NaiveDate,
        auto_renew: bool,
    ) -> Result<(), String> {
        let edit = billing::apply_edit(
            self.settings.billing.get(&id),
            date,
            auto_renew,
            Self::today(),
        )
        .map_err(str::to_string)?;
        if !edit.keep_history {
            self.settings.billing_reminders.remove(&id);
        }
        self.settings.billing.insert(id, edit.billing);
        self.save_user();
        Ok(())
    }

    /// Switches Renewal / Access ends for an existing date.
    pub fn set_billing_kind(&mut self, id: Uuid, auto_renew: bool) -> Result<(), String> {
        let Some(b) = self.settings.billing.get(&id) else {
            return Ok(());
        };
        let shown = billing::next_date(b, Self::today());
        self.set_billing(id, shown, auto_renew)
    }

    pub fn set_billing_reminders(&mut self, id: Uuid, on: bool) {
        if let Some(b) = self.settings.billing.get_mut(&id) {
            b.reminders = on;
            self.save_user();
        }
    }

    pub fn clear_billing(&mut self, id: Uuid) {
        let had = self.settings.billing.remove(&id).is_some();
        self.settings.billing_reminders.remove(&id);
        if had {
            self.save_user();
        }
    }

    /// Reminders due today (7 and 3 days before), at most one per account. The delivered mark is
    /// saved before the notice goes out (at most once; a failed save retries next round).
    /// Called at startup and every 30 s; days roll over without a restart.
    pub fn check_billing_reminders(&mut self, cx: &mut Context<Self>) {
        let today = Self::today();
        let mut due = vec![];
        for acc in &self.settings.accounts {
            let Some(b) = self.settings.billing.get(&acc.id) else {
                continue;
            };
            if let Some(r) =
                billing::reminder(b, self.settings.billing_reminders.get(&acc.id), today)
            {
                due.push((acc.id, acc.provider, acc.label.clone(), b.clone(), r));
            }
        }
        if due.is_empty() {
            return;
        }
        for (id, _, _, _, r) in &due {
            self.settings.billing_reminders.insert(
                *id,
                billing::ReminderMark {
                    date: r.date,
                    days: r.threshold,
                },
            );
        }
        if let Err(e) = settings::save(&self.path, &self.settings) {
            // Not saved: it would repeat every round. Undo the marks and try again later.
            for (id, ..) in &due {
                self.settings.billing_reminders.remove(id);
            }
            self.notice = Some(format!("Settings could not be saved: {e}"));
            return;
        }
        for (_, provider, label, b, r) in due {
            let kind = if b.auto_renew {
                "Renewal"
            } else {
                "Access end"
            };
            let when = match r.days_remaining {
                0 => "today".to_string(),
                1 => "tomorrow".to_string(),
                d => format!("in {d} days"),
            };
            let source = if r.date == b.date {
                "manual"
            } else {
                "estimated"
            };
            self.pending_notices.push((
                format!("{kind} {when} · {label}"),
                format!(
                    "{} {}: {} ({source}). Open billing from the account page to decide.",
                    provider_name(provider),
                    kind.to_lowercase(),
                    billing::format_date(r.date),
                ),
            ));
        }
        cx.notify();
    }

    pub fn update_display(&mut self, f: impl FnOnce(&mut DisplaySettings)) {
        f(&mut self.settings.display);
        self.save_user();
    }

    pub fn set_refresh_minutes(&mut self, m: u32) {
        self.settings.refresh_minutes = m.clamp(1, 60);
        self.save_user();
    }

    pub fn set_hub_enabled(&mut self, on: bool) {
        self.settings.hub.enabled = on;
        self.hub = if on {
            HubStatus::Unknown
        } else {
            HubStatus::Off
        };
        self.save_user();
    }

    pub fn set_hub_url(&mut self, url: &str) {
        let url = url.trim().trim_end_matches('/');
        if url.starts_with("http://") || url.starts_with("https://") {
            self.settings.hub.base_url = url.to_string();
            self.save_user();
        }
    }
}

fn toggle(list: &mut Vec<String>, key: &str) {
    if let Some(i) = list.iter().position(|k| k == key) {
        list.remove(i);
    } else {
        list.push(key.to_string());
    }
}

pub fn provider_name(p: ProviderKind) -> &'static str {
    p.name()
}

/// Checks every 30 s which accounts are due; each account's schedule decides (interval, backoff,
/// cooldown). Holds the state weakly so it ends with the app.
pub fn spawn_periodic(state: gpui::Entity<AppState>, cx: &mut gpui::App) {
    let weak = state.downgrade();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(30))
                .await;
            if weak
                .update(cx, |s, cx| {
                    s.check_billing_reminders(cx);
                    s.update_prices(false, cx);
                    s.refresh(false, cx)
                })
                .is_err()
            {
                break;
            }
        }
    })
    .detach();
}

/// A hub client with the stored key (background threads only: reads Credential Manager).
fn hub_client(base_url: String) -> Result<HubClient, String> {
    let key = secrets::hub_key()
        .ok()
        .flatten()
        .ok_or_else(|| "No hub key stored.".to_string())?;
    Ok(HubClient::new(base_url, key))
}

fn hub_text(e: HubError) -> String {
    e.to_string()
}
