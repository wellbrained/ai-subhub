//! AI SubHub: tray app. The tray icon is the entry point; the popover is the main surface;
//! settings open in their own window. Closing windows keeps the app in the tray.
#![windows_subsystem = "windows"]

mod assets;
mod format;
mod icons;
mod limits_board;
mod logo;
mod popover;
mod settings_window;
mod startup;
mod state;
mod stats_window;
mod taskbar;
mod theme;
mod tray;
mod updater;
mod whats_new;
mod window_style;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use gpui::{
    App, AppContext, Bounds, Entity, QuitMode, WindowBackgroundAppearance, WindowBounds,
    WindowHandle, WindowKind, WindowOptions, point, px, size,
};
use gpui_component::Root;

use popover::Popover;
use settings_window::SettingsWindow;
use state::AppState;
use theme::Tokens;
use tray::{Tray, TrayEvent};

thread_local! {
    /// The state entity, for element click handlers that only get `&mut App`.
    pub static STATE: RefCell<Option<Entity<AppState>>> = const { RefCell::new(None) };
}

struct Shell {
    tray: Tray,
    popover: Option<WindowHandle<Root>>,
    settings: Option<WindowHandle<Root>>,
    stats: Option<WindowHandle<Root>>,
    /// The "Updated to …" dialog.
    notes: Option<WindowHandle<Root>>,
    /// Always-visible taskbar display and the mode it was opened with.
    taskbar: Option<(
        WindowHandle<taskbar::TaskbarDisplay>,
        subhub_core::settings::TaskbarMode,
    )>,
    closed_at: Option<Instant>,
    state: Entity<AppState>,
}

type ShellRef = Rc<RefCell<Shell>>;

fn main() {
    // Dev helper: `--write-icon <path.ico>` writes the logo as a multi-size .ico (exe resource).
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--write-icon") {
        if let Some(path) = args.get(i + 1) {
            let _ = std::fs::write(path, logo::ico_bytes());
        }
        return;
    }
    // Dev helper: --stats-dump [path] scans the logs once and writes aggregate numbers only.
    if let Some(i) = args.iter().position(|a| a == "--stats-dump") {
        stats_dump(args.get(i + 1));
        return;
    }
    // Started by an update: the old process still owns the tray and the instance mutex, so wait
    // for it to end first (at most 30 s), before either is touched.
    let start = subhub_core::update::parse_start_args(&args);
    if let Some(pid) = start.wait_pid {
        updater::wait_for_pid(pid, Duration::from_secs(30));
    }
    // One instance only: a second start just exits (the first keeps the tray).
    if !single_instance() {
        return;
    }
    updater::clean_leftovers();
    let updated_from = start.updated_from;
    gpui_platform::application()
        .with_assets(assets::Assets)
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            let state = cx.new(|_| AppState::load());
            STATE.with(|c| *c.borrow_mut() = Some(state.clone()));
            let dark = theme::is_dark(state.read(cx).display().theme, cx);
            theme::apply_component_theme(dark, None, cx);

            let (tx, mut rx) = futures::channel::mpsc::unbounded();
            let tray = match Tray::new(tx, "AI SubHub") {
                Ok(t) => t,
                Err(_) => {
                    // Never run without a way back.
                    cx.quit();
                    return;
                }
            };
            let shell: ShellRef = Rc::new(RefCell::new(Shell {
                tray,
                popover: None,
                settings: None,
                stats: None,
                notes: None,
                taskbar: None,
                closed_at: None,
                state: state.clone(),
            }));

            // Keep the tray ring and tooltip in step with the state.
            {
                let shell = shell.clone();
                cx.observe(&state, move |state, cx| {
                    update_tray(&shell, &state, cx);
                    sync_taskbar(&shell, cx);
                    // Billing reminders go out as tray balloons.
                    let notices = state.update(cx, |s, _| std::mem::take(&mut s.pending_notices));
                    for (title, text) in notices {
                        shell.borrow_mut().tray.balloon(&title, &text);
                    }
                })
                .detach();
            }
            {
                let shell = shell.clone();
                cx.spawn(async move |cx| {
                    while let Some(ev) = rx.next().await {
                        let shell = shell.clone();
                        cx.update(|cx| match ev {
                            TrayEvent::Toggle => toggle_popover(&shell, Anchor::Tray, cx),
                            TrayEvent::Open => {
                                if shell.borrow().popover.is_none() {
                                    toggle_popover(&shell, Anchor::Tray, cx)
                                }
                            }
                            TrayEvent::Refresh => {
                                let st = shell.borrow().state.clone();
                                st.update(cx, |s, cx| s.refresh(true, cx));
                            }
                            TrayEvent::Settings => open_settings(&shell, cx),
                            TrayEvent::Statistics => open_stats(&shell, cx),
                            TrayEvent::Quit => cx.quit(),
                        });
                    }
                })
                .detach();
            }

            startup::refresh_if_enabled();
            state.update(cx, |s, cx| {
                s.check_billing_reminders(cx);
                s.refresh(false, cx);
                // Once at startup, with a timeout; the Launch menu's desktop-app items wait for it.
                s.detect_desktop_apps(cx);
            });
            state::spawn_periodic(state.clone(), cx);
            spawn_updater(shell.clone(), cx);
            update_tray(&shell, &state, cx);
            sync_taskbar(&shell, cx);
            show_whats_new(&shell, updated_from.as_deref(), cx);

            // First start without accounts: open Settings on Detect / Import, which lists the
            // CLI profiles found on this PC (and points to the hub setup).
            if state.read(cx).settings.accounts.is_empty() {
                state.update(cx, |s, _| {
                    s.pending_settings_page = Some(state::SettingsTarget::Detect)
                });
                open_settings(&shell, cx);
            }
        });
}

/// Once a minute: lets the state check for updates (first time ~1 min after start, then every
/// 6 h) and, in Auto mode, install when nothing is in use.
fn spawn_updater(shell: ShellRef, cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(Duration::from_secs(60))
                .await;
            let shell = shell.clone();
            cx.update(|cx| {
                let busy = busy_now(&shell, cx);
                let state = shell.borrow().state.clone();
                state.update(cx, |s, cx| s.update_tick(busy, cx));
            });
        }
    })
    .detach();
}

/// Whether a window is still open (a closed window's handle stays in the shell).
fn is_open(handle: Option<WindowHandle<Root>>, cx: &mut App) -> bool {
    handle.is_some_and(|h| h.update(cx, |_, _, _| ()).is_ok())
}

/// What keeps an automatic restart waiting right now.
fn busy_now(shell: &ShellRef, cx: &mut App) -> subhub_core::update::Busy {
    let (popover, settings, stats, notes, state) = {
        let s = shell.borrow();
        (s.popover, s.settings, s.stats, s.notes, s.state.clone())
    };
    let popover_open = is_open(popover, cx);
    let settings_open = is_open(settings, cx);
    let stats_open = is_open(stats, cx);
    let notes_open = is_open(notes, cx);
    let s = state.read(cx);
    subhub_core::update::Busy {
        popover_open,
        settings_open,
        stats_open,
        notes_open,
        signing_in: !s.signing_in.is_empty(),
        popover_dragging: popover_open && s.popover_dragging,
    }
}

/// After an update (or any start on a newer version than the last one seen): the patch notes in
/// a small dialog, then the version is remembered. A fresh install just remembers it.
fn show_whats_new(shell: &ShellRef, updated_from: Option<&str>, cx: &mut App) {
    let state = shell.borrow().state.clone();
    let running = updater::running_version();
    let wn = subhub_core::update::whats_new(
        &state.read(cx).settings.updates.last_seen_version,
        updated_from,
        &running,
        updater::CHANGELOG,
    );
    if wn.mark_seen {
        // Saved before the dialog opens, so it never shows twice.
        state.update(cx, |s, _| {
            s.settings.updates.last_seen_version = running.to_string();
            s.save();
        });
    }
    if wn.sections.is_empty() {
        return;
    }
    let border = border_color(&state, cx);
    let height = whats_new::height_for(&wn.sections);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(
            size(px(whats_new::WIDTH), px(height)),
            cx,
        )),
        titlebar: None,
        kind: WindowKind::PopUp,
        focus: true,
        show: true,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Opaque,
        ..Default::default()
    };
    let sections = wn.sections;
    if let Ok(h) = cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| whats_new::WhatsNew::new(state, running, sections, window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    }) {
        let _ = h.update(cx, |_, window, _| {
            window_style::round(window, border);
            window.activate_window();
        });
        shell.borrow_mut().notes = Some(h);
    }
}

/// --stats-dump: one real scan; prints (or writes to the given path) aggregate numbers only.
fn stats_dump(path: Option<&String>) {
    let text = match subhub_core::stats::scan_default() {
        Ok(scan) => {
            let prices = subhub_core::stats::PriceBook::load(&subhub_core::stats::prices_path());
            subhub_core::stats::dump_report(&scan, chrono::Local::now().naive_local(), &prices)
        }
        Err(e) => format!(
            "scan failed: {e}
"
        ),
    };
    match path {
        Some(p) => {
            let _ = std::fs::write(p, text);
        }
        None => print!("{text}"),
    }
}

/// Opens, closes or reopens the taskbar display to match the Display setting.
fn sync_taskbar(shell: &ShellRef, cx: &mut App) {
    let state = shell.borrow().state.clone();
    let mode = state.read(cx).display().taskbar_mode;
    let current = shell.borrow().taskbar.map(|(_, m)| m);
    if current == Some(mode)
        || (current.is_none() && mode == subhub_core::settings::TaskbarMode::Off)
    {
        return;
    }
    let old = shell.borrow_mut().taskbar.take();
    if let Some((h, _)) = old {
        let _ = h.update(cx, |_, window, _| window.remove_window());
    }
    let toggle_shell = shell.clone();
    let on_click: taskbar::OnClick = Rc::new(move |cx: &mut App, b| {
        toggle_popover(&toggle_shell, Anchor::TaskbarDisplay(b), cx)
    });
    if let Some(h) = taskbar::open(state, on_click, cx) {
        shell.borrow_mut().taskbar = Some((h, mode));
    }
}

fn single_instance() -> bool {
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::w;
    unsafe {
        // The handle is intentionally never closed: held for the process lifetime.
        if CreateMutexW(None, true, w!("Local\\AISubHub.SingleInstance")).is_err() {
            return true;
        }
        GetLastError() != ERROR_ALREADY_EXISTS
    }
}

/// The tray icon is the static logo; only the hover summary follows the state.
fn update_tray(shell: &ShellRef, state: &Entity<AppState>, cx: &mut App) {
    let tip = tray_tip(state.read(cx));
    let _ = shell.borrow_mut().tray.set_tip(&tip);
}

/// Hover summary, within the tooltip's 127-char limit.
fn tray_tip(s: &AppState) -> String {
    let mut lines = vec!["AI SubHub".to_string()];
    for acc in s.settings.accounts.iter().filter(|a| !a.hidden) {
        let Some(r) = s.settings.readings.get(&acc.id) else {
            continue;
        };
        let lowest = r
            .windows
            .iter()
            .filter(|w| !acc.hidden_windows.contains(&w.key))
            .map(|w| w.remaining_percent())
            .fold(None::<f64>, |m, v| Some(m.map_or(v, |m| m.min(v))));
        if let Some(l) = lowest {
            let stale = if s.is_stale(acc.id) { " (stale)" } else { "" };
            lines.push(format!("{}: {:.0}% left{stale}", acc.label, l));
        }
    }
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        if out.len() + l.len() + 1 > 120 {
            out.push_str("\n…");
            break;
        }
        if i > 0 {
            out.push('\n');
        }
        out.push_str(l);
    }
    out
}

/// What the popover opens above: the tray icon, or the taskbar display that was clicked.
#[derive(Clone, Copy)]
enum Anchor {
    Tray,
    /// The taskbar display that was clicked, with its screen bounds (logical px).
    TaskbarDisplay(Bounds<gpui::Pixels>),
}

fn toggle_popover(shell: &ShellRef, anchor: Anchor, cx: &mut App) {
    let mut s = shell.borrow_mut();
    if let Some(handle) = s.popover.take() {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
        s.closed_at = Some(Instant::now());
        return;
    }
    // The tray click that took focus away already closed it; don't reopen at once.
    if s.closed_at
        .is_some_and(|t| t.elapsed() < Duration::from_millis(300))
    {
        return;
    }
    let state = s.state.clone();
    let height = Popover::preferred_height(state.read(cx));
    let scale = s.tray.scale();
    let anchor_rect = match anchor {
        Anchor::Tray => s.tray.rect(),
        // Above the clicked taskbar display, centred on it (physical px for the work-area lookup).
        Anchor::TaskbarDisplay(b) => Some(tray::TrayRect {
            left: (b.origin.x.as_f32() * scale) as i32,
            top: (b.origin.y.as_f32() * scale) as i32,
            right: ((b.origin.x + b.size.width).as_f32() * scale) as i32,
            bottom: ((b.origin.y + b.size.height).as_f32() * scale) as i32,
        }),
    };
    let (x, y) = popover_origin(anchor_rect, scale, height);
    drop(s);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(x), px(y)),
            size: size(px(popover::WIDTH), px(height)),
        })),
        titlebar: None,
        kind: WindowKind::PopUp,
        focus: true,
        show: true,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Opaque,
        ..Default::default()
    };
    let border = border_color(&state, cx);
    let close_shell = shell.clone();
    let on_settings: Rc<dyn Fn(&mut App)> = {
        let shell = shell.clone();
        Rc::new(move |cx: &mut App| {
            close_popover(&shell, cx);
            open_settings(&shell, cx);
        })
    };
    let on_stats: Rc<dyn Fn(&mut App)> = {
        let shell = shell.clone();
        Rc::new(move |cx: &mut App| {
            // The popover stays open next to the Statistics window.
            open_stats(&shell, cx);
        })
    };
    let result = cx.open_window(options, move |window, cx| {
        let on_close = Box::new(move |window: &mut gpui::Window, _cx: &mut App| {
            let mut s = close_shell.borrow_mut();
            if s.popover.take().is_some() {
                s.closed_at = Some(Instant::now());
            }
            drop(s);
            window.remove_window();
        });
        let view = cx.new(|cx| Popover::new(state, on_settings, on_stats, on_close, window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    });
    if let Ok(handle) = result {
        let _ = handle.update(cx, |_, window, _| {
            window_style::round(window, border);
            window.activate_window();
        });
        shell.borrow_mut().popover = Some(handle);
    }
}

fn border_color(state: &Entity<AppState>, cx: &App) -> gpui::Hsla {
    let d = state.read(cx).display();
    Tokens::new(theme::is_dark(d.theme, cx), &d.accent).border
}

/// Gap between the popover and the taskbar / screen edge, like Windows' own flyouts.
const EDGE_GAP: f32 = 12.0;

/// Top-left (logical px) for a popover of `height`: centred on `anchor` (the tray icon or the
/// taskbar display, physical px), kept inside the monitor work area with `EDGE_GAP`, above a
/// bottom taskbar (below a top one).
fn popover_origin(anchor: Option<tray::TrayRect>, scale: f32, height: f32) -> (f32, f32) {
    let Some(r) = anchor else {
        return (100.0, 100.0);
    };
    let Some(work) = tray::work_area(&r) else {
        return (100.0, 100.0);
    };
    let (wl, wt, wr, wb) = (
        work.left as f32 / scale,
        work.top as f32 / scale,
        work.right as f32 / scale,
        work.bottom as f32 / scale,
    );
    let center = (r.left + r.right) as f32 / 2.0 / scale;
    let x = (center - popover::WIDTH / 2.0)
        .min(wr - popover::WIDTH - EDGE_GAP)
        .max(wl + EDGE_GAP);
    let taskbar_on_top = (r.top as f32 / scale) < wt;
    let y = if taskbar_on_top {
        wt + EDGE_GAP
    } else {
        (wb - height - EDGE_GAP).max(wt + EDGE_GAP)
    };
    (x, y)
}

fn close_popover(shell: &ShellRef, cx: &mut App) {
    let handle = shell.borrow_mut().popover.take();
    if let Some(h) = handle {
        let _ = h.update(cx, |_, window, _| window.remove_window());
        shell.borrow_mut().closed_at = Some(Instant::now());
    }
}

/// Opens (or raises) the Statistics window and starts a scan.
fn open_stats(shell: &ShellRef, cx: &mut App) {
    let state = shell.borrow().state.clone();
    let existing = shell.borrow().stats;
    if let Some(h) = existing
        && h.update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        state.update(cx, |s, cx| s.start_scan(cx));
        return;
    }
    let border = border_color(&state, cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(
            size(px(stats_window::WIDTH), px(760.0)),
            cx,
        )),
        titlebar: Some(gpui::TitlebarOptions {
            title: Some("AI SubHub · Statistics".into()),
            ..Default::default()
        }),
        window_min_size: Some(size(px(720.0), px(480.0))),
        icon: logo::window_icon(),
        ..Default::default()
    };
    let view_state = state.clone();
    if let Ok(h) = cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| stats_window::StatsWindow::new(view_state, cx));
        cx.new(|cx| Root::new(view, window, cx))
    }) {
        let _ = h.update(cx, |_, window, _| {
            window_style::round(window, border);
            window.activate_window();
        });
        shell.borrow_mut().stats = Some(h);
        state.update(cx, |s, cx| s.start_scan(cx));
    }
}

fn open_settings(shell: &ShellRef, cx: &mut App) {
    let existing = shell.borrow().settings;
    if let Some(h) = existing
        && h.update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let state = shell.borrow().state.clone();
    let border = border_color(&state, cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(900.0), px(640.0)), cx)),
        titlebar: Some(gpui::TitlebarOptions {
            title: Some("AI SubHub · Settings".into()),
            ..Default::default()
        }),
        window_min_size: Some(size(px(640.0), px(420.0))),
        icon: logo::window_icon(),
        ..Default::default()
    };
    let quit_state = state.clone();
    if let Ok(h) = cx.open_window(options, |window, cx| {
        // The window's close button follows General → Close button: only the window closes
        // (SubHub keeps running in the tray), or SubHub quits completely.
        window.on_window_should_close(cx, move |_, cx| {
            if quit_state.read(cx).display().close_quits {
                cx.quit();
            }
            true
        });
        let view = cx.new(|cx| SettingsWindow::new(state, window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    }) {
        let _ = h.update(cx, |_, window, _| {
            window_style::round(window, border);
            window.activate_window();
        });
        shell.borrow_mut().settings = Some(h);
    }
}
