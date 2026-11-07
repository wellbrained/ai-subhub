//! Throwaway spike: tray ring icon, popover above the tray, app alive with no window,
//! one hub read, idle memory. See docs/features/spike.md.
#![windows_subsystem = "windows"]

mod hub;
mod tray;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use gpui::{
    App, AppContext, Bounds, Context, Entity, IntoElement, ParentElement, QuitMode, Render,
    Styled, Subscription, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle,
    WindowKind, WindowOptions, div, point, prelude::*, px, rgb, size,
};
use gpui_component::{Root, button::Button, progress::ProgressCircle};

use hub::HubAccount;
use tray::{Tray, TrayEvent};

const POPOVER_W: f32 = 360.0;
const POPOVER_H: f32 = 440.0;

#[derive(Default)]
struct SpikeState {
    hub: Option<Result<Vec<HubAccount>, String>>,
    mem_mb: f64,
    started: Option<Instant>,
}

struct Shell {
    tray: Tray,
    popover: Option<WindowHandle<Root>>,
    closed_at: Option<Instant>,
    state: Entity<SpikeState>,
}

fn log(line: &str) {
    use std::io::Write;
    let path = std::env::temp_dir().join("subhub-spike.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{} {line}", chrono_now());
    }
}

fn chrono_now() -> String {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    format!("{}.{:03}", d.as_secs(), d.subsec_millis())
}

fn working_set_mb() -> f64 {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut c = PROCESS_MEMORY_COUNTERS::default();
    unsafe {
        let _ = GetProcessMemoryInfo(GetCurrentProcess(), &mut c, size_of::<PROCESS_MEMORY_COUNTERS>() as u32);
    }
    c.WorkingSetSize as f64 / 1024.0 / 1024.0
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--store-hub-key") {
        let path = args.get(i + 1).expect("--store-hub-key <file>");
        let key = std::fs::read_to_string(path).expect("read key file");
        hub::key_entry().unwrap().set_password(key.trim()).expect("store key");
        log("hub key stored in Credential Manager");
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--dump-rings") {
        let rings = [(0.82, color_for(82.0)), (0.44, color_for(44.0)), (0.2, color_for(20.0)), (0.07, color_for(7.0))];
        tray::dump_rings(&args[i + 1], &rings).expect("dump");
        return;
    }
    if args.iter().any(|a| a == "--check-key") {
        // Shape only, never the value.
        let entry = hub::key_entry().unwrap();
        let raw = entry.get_password().unwrap_or_default();
        let k = hub::normalize_key(&raw);
        if k != raw {
            entry.set_password(&k).expect("store normalized key");
            log("key normalized and re-stored");
        }
        log(&format!(
            "key shape: len {}, whitespace {}, colon {}, equals {}, non-ascii {}",
            k.len(),
            k.contains(char::is_whitespace),
            k.contains(':'),
            k.contains('='),
            !k.is_ascii()
        ));
        return;
    }
    let auto_open = args.iter().any(|a| a == "--auto-open");
    let started = Instant::now();
    log("start");

    gpui_platform::application()
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            let (tx, mut rx) = futures::channel::mpsc::unbounded();
            let tray = match Tray::new(tx.clone(), "AI SubHub spike") {
                Ok(t) => t,
                Err(e) => {
                    log(&format!("tray failed: {e}"));
                    cx.quit();
                    return;
                }
            };
            log(&format!("tray ready after {} ms", started.elapsed().as_millis()));
            let state = cx.new(|_| SpikeState { started: Some(started), ..Default::default() });
            let shell = Rc::new(RefCell::new(Shell { tray, popover: None, closed_at: None, state: state.clone() }));

            // Tray events -> toggle popover / quit.
            {
                let shell = shell.clone();
                cx.spawn(async move |cx| {
                    while let Some(ev) = rx.next().await {
                        let shell = shell.clone();
                        cx.update(|cx| match ev {
                            TrayEvent::Toggle => toggle_popover(&shell, cx),
                            TrayEvent::Quit => cx.quit(),
                        });
                    }
                })
                .detach();
            }

            // One hub read on a background thread.
            {
                let state = state.clone();
                let shell = shell.clone();
                cx.spawn(async move |cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async { hub::read().map_err(|e| e.to_string()) })
                        .await;
                    match &result {
                        Ok(a) => log(&format!(
                            "hub ok: {} accounts ({} benched); usage windows read: {}; usage errors: {:?}",
                            a.len(),
                            a.iter().filter(|x| x.disabled).count(),
                            a.iter().map(|x| x.usage.len()).sum::<usize>(),
                            a.iter().filter_map(|x| x.usage_error.clone()).collect::<Vec<_>>()
                        )),
                        Err(e) => log(&format!("hub error: {e}")),
                    }
                    cx.update(|cx| {
                        let (fraction, color, tip) = ring_from(&result);
                        let _ = shell.borrow_mut().tray.set_ring(fraction, color, &tip);
                        state.update(cx, |s, cx| {
                            s.hub = Some(result);
                            cx.notify();
                        });
                    });
                })
                .detach();
            }

            // Memory sampling; also animates the ring until the hub answers (live redraw proof).
            {
                let state = state.clone();
                let shell = shell.clone();
                cx.spawn(async move |cx| {
                    let mut step = 0u32;
                    loop {
                        cx.background_executor().timer(Duration::from_secs(2)).await;
                        step += 1;
                        let mem = working_set_mb();
                        if step % 15 == 0 {
                            log(&format!("mem {mem:.1} MB, popover open: {}", shell.borrow().popover.is_some()));
                        }
                        cx.update(|cx| {
                            let waiting = state.read(cx).hub.is_none();
                            if waiting {
                                let f = (step % 10) as f32 / 10.0;
                                let _ = shell.borrow_mut().tray.set_ring(f, [0x3b, 0x82, 0xf6], "AI SubHub spike: reading hub");
                            }
                            state.update(cx, |s, cx| {
                                s.mem_mb = mem;
                                cx.notify();
                            });
                        });
                    }
                })
                .detach();
            }

            if auto_open {
                toggle_popover(&shell, cx);
            }
        });
}

/// Lowest "% left" across read windows drives the ring and its color.
fn ring_from(result: &Result<Vec<HubAccount>, String>) -> (f32, [u8; 3], String) {
    match result {
        Err(e) => (0.0, [0x9c, 0xa3, 0xaf], format!("Hub unavailable: {e}")),
        Ok(accounts) => {
            let lowest = accounts
                .iter()
                .flat_map(|a| a.usage.iter().map(|(_, used)| 100.0 - used))
                .fold(None::<f64>, |m, l| Some(m.map_or(l, |m| m.min(l))));
            match lowest {
                None => (0.0, [0x9c, 0xa3, 0xaf], format!("{} hub accounts, no usage read", accounts.len())),
                Some(left) => (
                    (left / 100.0) as f32,
                    color_for(left),
                    format!("Lowest: {left:.0}% left ({} hub accounts)", accounts.len()),
                ),
            }
        }
    }
}

fn color_for(left: f64) -> [u8; 3] {
    if left <= 10.0 {
        [0xef, 0x44, 0x44]
    } else if left <= 25.0 {
        [0xf5, 0x9e, 0x0b]
    } else {
        [0x3b, 0x82, 0xf6]
    }
}

fn toggle_popover(shell: &Rc<RefCell<Shell>>, cx: &mut App) {
    let mut s = shell.borrow_mut();
    if let Some(handle) = s.popover.take() {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
        s.closed_at = Some(Instant::now());
        log("popover closed by tray click");
        return;
    }
    // A tray click first deactivates (and closes) the popover; don't reopen it right away.
    if s.closed_at.is_some_and(|t| t.elapsed() < Duration::from_millis(300)) {
        return;
    }
    let scale = unsafe { windows::Win32::UI::HiDpi::GetDpiForSystem() } as f32 / 96.0;
    let (x, y) = match s.tray.rect() {
        Some(r) => {
            let right = r.right as f32 / scale;
            let top = r.top as f32 / scale;
            let center = (r.left + r.right) as f32 / 2.0 / scale;
            let x = (center - POPOVER_W / 2.0).min(right + 40.0 - POPOVER_W);
            (x.max(8.0), (top - POPOVER_H - 12.0).max(8.0))
        }
        None => (100.0, 100.0),
    };
    let state = s.state.clone();
    let opened = Instant::now();
    let shell_for_view = shell.clone();
    drop(s);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(x), px(y)),
            size: size(px(POPOVER_W), px(POPOVER_H)),
        })),
        titlebar: None,
        kind: WindowKind::PopUp,
        focus: true,
        show: true,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    match cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| Popover::new(state, shell_for_view, window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    }) {
        Ok(handle) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            log(&format!("popover opened in {} ms at ({x:.0},{y:.0}) scale {scale}", opened.elapsed().as_millis()));
            shell.borrow_mut().popover = Some(handle);
        }
        Err(e) => log(&format!("popover failed: {e}")),
    }
}

struct Popover {
    state: Entity<SpikeState>,
    _subs: Vec<Subscription>,
}

impl Popover {
    fn new(state: Entity<SpikeState>, shell: Rc<RefCell<Shell>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subs = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            // Close on focus loss.
            cx.observe_window_activation(window, move |_, window, _| {
                if !window.is_window_active() && !std::env::args().any(|a| a == "--stay") {
                    let mut s = shell.borrow_mut();
                    if s.popover.take().is_some() {
                        s.closed_at = Some(Instant::now());
                        log("popover closed on focus loss");
                    }
                    drop(s);
                    window.remove_window();
                }
            }),
        ];
        Self { state, _subs: subs }
    }
}

impl Render for Popover {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.state.read(cx);
        let mut body = div().flex().flex_col().gap_3();
        match &s.hub {
            None => body = body.child(div().text_color(rgb(0x9ca3af)).child("Reading hub…")),
            Some(Err(e)) => body = body.child(div().text_color(rgb(0xf87171)).child(format!("Hub: {e}"))),
            Some(Ok(accounts)) => {
                for (i, a) in accounts.iter().enumerate() {
                    let left = a.usage.first().map(|(_, used)| 100.0 - used);
                    let detail = if a.disabled {
                        "Benched".to_string()
                    } else if let Some(e) = &a.usage_error {
                        format!("Usage: {e}")
                    } else if a.usage.is_empty() {
                        "Not read in spike".to_string()
                    } else {
                        a.usage.iter().map(|(n, u)| format!("{n} {:.0}% left", 100.0 - u)).collect::<Vec<_>>().join(" · ")
                    };
                    body = body.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                ProgressCircle::new(("ring", i))
                                    .value(left.unwrap_or(0.0) as f32)
                                    .color(gpui::Rgba::from(rgb(u32::from_be_bytes([0, color_for(left.unwrap_or(100.0))[0], color_for(left.unwrap_or(100.0))[1], color_for(left.unwrap_or(100.0))[2]]))))
                                    .size_8(),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(div().text_sm().text_color(rgb(0xf3f4f6)).child(format!("{} · {}", a.provider, a.email)))
                                    .child(div().text_xs().text_color(rgb(0x9ca3af)).child(detail)),
                            ),
                    );
                }
            }
        }
        let uptime = s.started.map(|t| t.elapsed().as_secs()).unwrap_or(0);
        div()
            .size_full()
            .p_4()
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x18181b))
            .rounded_xl()
            .border_1()
            .border_color(rgb(0x2f2f35))
            .text_color(rgb(0xf3f4f6))
            .child(div().text_lg().child("AI SubHub · spike"))
            .child(body)
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(div().text_xs().text_color(rgb(0x9ca3af)).child(format!("{:.1} MB · up {uptime}s", s.mem_mb)))
                    .child(Button::new("quit").label("Quit").on_click(|_, _, cx| cx.quit())),
            )
    }
}
