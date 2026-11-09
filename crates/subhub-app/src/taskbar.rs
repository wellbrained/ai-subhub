//! Always-visible taskbar display (ADR 0001 refinements). Windows has no API for third-party
//! content in the taskbar, so this is a small borderless, topmost tool window placed over a free
//! part of the taskbar (left by the widgets/weather, or right by the tray), re-raised every second
//! because the taskbar comes to the front when clicked. Click opens the popover.
use std::time::Duration;

use gpui::{
    App, AppContext, Bounds, Context, Entity, Hsla, IntoElement, ParentElement, Render,
    SharedString, Styled, Subscription, Window, WindowBackgroundAppearance, WindowBounds,
    WindowHandle, WindowKind, WindowOptions, div, point, prelude::*, px, size,
};
use subhub_core::model::{Account, AllowanceWindow};
use subhub_core::settings::{RUNNING_OUT_PERCENT, TaskbarAccounts, TaskbarMode, TaskbarPosition};
use uuid::Uuid;

use crate::format;
use crate::state::AppState;
use crate::theme::Tokens;

/// Accounts the display shows, per Appearance → Accounts (the popover always shows all).
fn shown(s: &AppState) -> Vec<&Account> {
    let d = s.display();
    let usable = |a: &&Account| !a.hidden && s.settings.readings.contains_key(&a.id);
    let target = s
        .settings
        .steering
        .preferred
        .or(s.settings.steering.current);
    let by_id = |id: Uuid| {
        s.settings
            .accounts
            .iter()
            .find(|a| a.id == id)
            .filter(usable)
    };
    let focus = || target.and_then(by_id).into_iter().collect::<Vec<_>>();
    let list: Vec<&Account> = match d.taskbar_accounts {
        TaskbarAccounts::Favorites => {
            // Starred accounts always show, with "—" until their first reading.
            let favs: Vec<&Account> = d
                .taskbar_favorites
                .iter()
                .filter_map(|id| {
                    s.settings
                        .accounts
                        .iter()
                        .find(|a| a.id == *id && !a.hidden)
                })
                .take(d.taskbar_count.clamp(1, 5) as usize)
                .collect();
            if favs.is_empty() { focus() } else { favs }
        }
        TaskbarAccounts::Focus => focus(),
        TaskbarAccounts::Auto => {
            let mut v = focus();
            for a in s.settings.accounts.iter().filter(usable) {
                if v.iter().any(|x| x.id == a.id) {
                    continue;
                }
                let low = s.settings.readings[&a.id]
                    .windows
                    .iter()
                    .filter(|w| !a.hidden_windows.contains(&w.key))
                    .any(|w| w.remaining_percent() <= RUNNING_OUT_PERCENT);
                if low {
                    v.push(a);
                }
            }
            v.truncate(d.taskbar_count.clamp(1, 5) as usize);
            v
        }
    };
    if list.is_empty() {
        // Nothing chosen yet: the first account with a reading, so the display is never blank.
        s.settings.accounts.iter().filter(usable).take(1).collect()
    } else {
        list
    }
}

/// Short label for the taskbar: the part after " · " if the label was auto-made from an email
/// ("alex.morgan · gmail" -> "gmail"), else the first 10 chars.
fn short_name(a: &Account) -> String {
    let base = a.label.rsplit(" · ").next().unwrap_or(&a.label);
    base.chars().take(10).collect()
}

/// Two-letter account tag and a stable colour per account (like T3 Code's provider tags).
/// Windows mode (taskbar, Start) is dark unless `SystemUsesLightTheme` = 1. Separate from the
/// app mode, which drives the popover and settings.
pub fn taskbar_is_dark() -> bool {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    let mut value: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            windows::core::w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            windows::core::w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    !(ok.is_ok() && value == 1)
}

/// Provider icon colour (the brand colour, or `neutral`).
fn provider_color(p: subhub_core::model::ProviderKind, neutral: Hsla) -> Hsla {
    crate::icons::provider_color(p, neutral)
}

/// The account's tag: the user's own (Settings → Accounts, up to 3 chars, as typed), else two
/// letters from the short name.
/// Clearly different hues; no two accounts share one while there are enough.
const TAG_PALETTE: [u32; 8] = [
    0x3b82f6, 0x22c55e, 0xef4444, 0xf59e0b, 0xa855f7, 0x14b8a6, 0xec4899, 0x84cc16,
];

/// The preferred palette slot of an account: from its login email (stable when accounts merge
/// and the id changes), else the id.
fn tag_seed(a: &Account) -> usize {
    let seed: Vec<u8> = match &a.email {
        Some(e) => e.to_lowercase().into_bytes(),
        None => a.id.as_bytes().to_vec(),
    };
    seed.iter()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(*b as u32)) as usize
        % TAG_PALETTE.len()
}

/// Palette slot per account in settings order: each takes its preferred slot, or the next free
/// one when an earlier account already has it, so colours never collide (up to 8 accounts).
fn tag_slot(accounts: &[Account], id: uuid::Uuid) -> Option<usize> {
    let mut used = [false; TAG_PALETTE.len()];
    for a in accounts {
        if used.iter().all(|u| *u) {
            used = [false; TAG_PALETTE.len()];
        }
        let mut slot = tag_seed(a);
        while used[slot] {
            slot = (slot + 1) % TAG_PALETTE.len();
        }
        used[slot] = true;
        if a.id == id {
            return Some(slot);
        }
    }
    None
}

pub fn account_tag(s: &AppState, a: &Account) -> (String, Hsla) {
    let tag: String = match s.settings.account_tags.get(&a.id) {
        Some(own) if !own.trim().is_empty() => own.trim().chars().take(3).collect(),
        _ => short_name(a)
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(2)
            .collect::<String>()
            .to_uppercase(),
    };
    let slot = tag_slot(&s.settings.accounts, a.id).unwrap_or_else(|| tag_seed(a));
    (
        if tag.is_empty() { "??".into() } else { tag },
        gpui::rgb(TAG_PALETTE[slot]).into(),
    )
}

pub fn width_for(s: &AppState) -> f32 {
    let n = shown(s).len().max(1) as f32;
    match s.display().taskbar_mode {
        TaskbarMode::Badge => 12.0 + n * 128.0 + (n - 1.0) * 2.0,
        TaskbarMode::Bars => 20.0 + n * 96.0 + (n - 1.0) * 16.0,
        TaskbarMode::Off => 0.0,
    }
}

/// Called when the display is clicked, with its client area (physical screen px).
pub type OnClick = std::rc::Rc<dyn Fn(&mut App, subhub_core::placement::Rect)>;

/// The taskbar this display sits on: its rect (physical px), scale, and whether it is the main
/// one (with the tray icons).
type Bar = (windows::Win32::Foundation::RECT, f32, bool);

pub struct TaskbarDisplay {
    state: Entity<AppState>,
    /// Monitor id (`monitors::Monitor::id`) whose taskbar this display sits on.
    monitor: String,
    on_click: OnClick,
    /// Horizontal drag in progress: (cursor x at press, display left at press, moved yet).
    drag: Option<(i32, f32, bool)>,
    /// The taskbar, looked up when a press starts (a drag moves many times a second).
    drag_bar: Option<Bar>,
    /// Each account's area in the window (logical px), from the last paint; hover cards
    /// anchor to it.
    areas: std::rc::Rc<std::cell::RefCell<Vec<Bounds<gpui::Pixels>>>>,
    _subs: Vec<Subscription>,
}

fn cursor_x() -> i32 {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    p.x
}

impl TaskbarDisplay {
    fn new(
        state: Entity<AppState>,
        monitor: String,
        on_click: OnClick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subs = vec![cx.observe_in(&state, window, |this, state, window, cx| {
            // Re-fit width and position when accounts or the mode change.
            place(window, state.read(cx), &this.monitor);
            cx.notify();
        })];
        Self {
            state,
            monitor,
            on_click,
            drag: None,
            drag_bar: None,
            areas: Default::default(),
            _subs: subs,
        }
    }
}

impl TaskbarDisplay {
    /// Demo showcase (`showcase.rs`): the same view inside an ordinary window. It never places
    /// itself on a taskbar and clicks do nothing.
    pub fn showcase(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let subs = vec![cx.observe(&state, |_, _, cx| cx.notify())];
        Self {
            state,
            monitor: "showcase".into(),
            on_click: std::rc::Rc::new(|_, _| {}),
            drag: None,
            drag_bar: None,
            areas: Default::default(),
            _subs: subs,
        }
    }
}

/// The accounts the display shows, in its order (demo showcase).
pub fn shown_ids(s: &AppState) -> Vec<uuid::Uuid> {
    shown(s).iter().map(|a| a.id).collect()
}

pub(crate) type BadgeRow = (String, Option<AllowanceWindow>);

/// The two rows of an account's badge: "5h" and "W" (first match each). An account with neither
/// (Copilot's monthly credits) shows its own first two windows with a short label ("M"); a
/// missing second one leaves the row out (empty label).
pub(crate) fn badge_windows(s: &AppState, a: &Account) -> (BadgeRow, BadgeRow) {
    let five_w = || (("5h".to_string(), None), ("W".to_string(), None));
    let Some(r) = s.settings.readings.get(&a.id) else {
        return five_w();
    };
    let vis: Vec<&AllowanceWindow> = r
        .windows
        .iter()
        .filter(|w| !a.hidden_windows.contains(&w.key))
        .collect();
    let session = vis.iter().find(|w| w.is_session()).map(|w| (*w).clone());
    let week = vis.iter().find(|w| w.is_weekly()).map(|w| (*w).clone());
    if session.is_some() || week.is_some() || vis.is_empty() {
        return (("5h".into(), session), ("W".into(), week));
    }
    let row = |w: Option<&&AllowanceWindow>| match w {
        Some(w) => (short_window_label(&w.label), Some((*w).clone())),
        None => (String::new(), None),
    };
    (row(vis.first()), row(vis.get(1)))
}

/// A badge label for a window title: "Monthly · AI credits" -> "M", "Weekly" -> "W",
/// "5-hour" -> "5h", "1-day" -> "1d", else its first two letters.
fn short_window_label(label: &str) -> String {
    let base = label.split(" ·").next().unwrap_or(label).trim();
    match base {
        "Monthly" => "M".into(),
        "Weekly" => "W".into(),
        "Daily" => "D".into(),
        _ => match base.split_once('-') {
            Some((n, unit)) if n.chars().all(|c| c.is_ascii_digit()) => {
                format!("{n}{}", unit.chars().next().unwrap_or('?'))
            }
            _ => base.chars().take(2).collect(),
        },
    }
}

impl Render for TaskbarDisplay {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.state.read(cx);
        let d = s.display().clone();
        // Readable on the taskbar: 12 px text, 6 px bars, brighter secondary text.
        // Colours follow the *taskbar's* mode (Windows mode), not the app theme: Windows lets
        // them differ, and the window itself is transparent over the taskbar.
        let t = Tokens::new(taskbar_is_dark(), &d.accent);
        let secondary: Hsla = if t.dark {
            gpui::rgb(0xd4d4d8).into()
        } else {
            gpui::rgb(0x3f3f46).into()
        };
        let label_c: Hsla = if t.dark {
            gpui::rgb(0xa1a1aa).into()
        } else {
            gpui::rgb(0x52525b).into()
        };
        let track: Hsla = if t.dark {
            gpui::rgb(0x3f3f46).into()
        } else {
            gpui::rgb(0xd4d4d8).into()
        };
        let target = s
            .settings
            .steering
            .preferred
            .or(s.settings.steering.current);
        let on_click = self.on_click.clone();
        let accounts = shown(s);

        let value = |w: &Option<AllowanceWindow>, stale: bool| -> (String, Hsla, f32) {
            match w {
                None => ("—".into(), label_c, 0.0),
                Some(w) => {
                    let c = if stale {
                        label_c
                    } else {
                        t.level(w.remaining_percent())
                    };
                    (format::percent(w, &d), c, format::gauge_fraction(w, &d))
                }
            }
        };
        let bar = |frac: f32, c: Hsla, wpx: f32| {
            div().w(px(wpx)).h(px(6.0)).rounded_full().bg(track).child(
                div()
                    .h_full()
                    .rounded_full()
                    .bg(c)
                    .w(gpui::relative(frac.clamp(0.0, 1.0))),
            )
        };
        // Same typography as the taskbar clock: Segoe UI Variable, 12 px, regular, two lines,
        // full-contrast text (owner: "check font settings from the date").
        let clock_text: Hsla = if t.dark {
            gpui::rgb(0xffffff).into()
        } else {
            gpui::rgb(0x000000).into()
        };
        let pct_color = |c: Hsla| {
            if c == t.accent || c == label_c {
                clock_text
            } else {
                c
            }
        };
        // Thin outline, no fill: sits on the taskbar like a tray item.
        let outline: Hsla = if t.dark {
            gpui::white().opacity(0.22)
        } else {
            gpui::black().opacity(0.22)
        };
        let badges = d.taskbar_mode == TaskbarMode::Badge;
        let mut row = div()
            .flex()
            .items_center()
            .gap(px(if badges { 2.0 } else { 16.0 }));
        for (i, a) in accounts.iter().enumerate() {
            let ((top_label, session), (bottom_label, week)) = badge_windows(s, a);
            let stale = s.is_stale(a.id);
            let (sv, sc, sf) = value(&session, stale);
            let (wv, wc, _) = value(&week, stale);
            let dot = target == Some(a.id);
            let col = match d.taskbar_mode {
                // T3-Code-style badge: glyph, coloured account tag, 5h and W mini bars + %.
                TaskbarMode::Badge => {
                    let (tag, tag_bg) = account_tag(s, a);
                    let (_, _, wf) = value(&week, stale);
                    // Two rows like the clock: left block spans both (glyph + tag), right block
                    // is "5h bar %" over "W bar %".
                    let metric = |label: String, v: String, c: Hsla, f: f32| {
                        div()
                            .h(px(16.0))
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .w(px(15.0))
                                    .whitespace_nowrap()
                                    .text_size(px(11.0))
                                    .text_color(label_c)
                                    .child(label),
                            )
                            .child(bar(f, c, 26.0))
                            .child(
                                div()
                                    .w(px(30.0))
                                    .flex()
                                    .justify_end()
                                    .text_color(pct_color(c))
                                    .child(v),
                            )
                    };
                    // Like a tray icon: no box, full height, rounded highlight on hover; the
                    // steering account gets a small accent bar at the bottom (like Windows'
                    // marker under the running app).
                    div()
                        .relative()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .h(px(40.0))
                        .px(px(6.0))
                        .rounded(px(4.0))
                        .hover(|st| st.bg(outline.opacity(0.35)))
                        .when(dot, |d| {
                            d.child(
                                div()
                                    .absolute()
                                    // Under the icon column only (px 6 + icon 26), so it does
                                    // not read as a third row under the W bar.
                                    .bottom(px(0.0))
                                    .left(px(6.0))
                                    .w(px(26.0))
                                    .flex()
                                    .justify_center()
                                    .child(
                                        div().w(px(12.0)).h(px(3.0)).rounded_full().bg(t.accent),
                                    ),
                            )
                        })
                        // T3 Code style: provider icon with the tag centred on its bottom edge.
                        .child(
                            div()
                                .relative()
                                .w(px(26.0))
                                .h(px(30.0))
                                .flex()
                                .justify_center()
                                .child(crate::icons::icon(
                                    crate::icons::Icon::Provider(a.provider),
                                    20.0,
                                    provider_color(a.provider, label_c),
                                ))
                                .child(
                                    div()
                                        .absolute()
                                        .bottom(px(0.0))
                                        .left(px(-6.0))
                                        .right(px(-6.0))
                                        .flex()
                                        .justify_center()
                                        .child(
                                            div()
                                                .px(px(3.0))
                                                .rounded(px(3.0))
                                                .bg(tag_bg)
                                                .text_size(px(9.0))
                                                .line_height(px(11.0))
                                                .font_weight(gpui::FontWeight::BOLD)
                                                .text_color(gpui::white())
                                                .child(tag),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(metric(top_label.clone(), sv.clone(), sc, sf))
                                .when(!bottom_label.is_empty(), |d| {
                                    d.child(metric(bottom_label.clone(), wv.clone(), wc, wf))
                                }),
                        )
                }
                // Line 1: name and 5-hour %, line 2: the 5-hour bar.
                TaskbarMode::Bars | TaskbarMode::Off => div()
                    .flex()
                    .flex_col()
                    .w(px(96.0))
                    .child(
                        div()
                            .h(px(16.0))
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .when(dot, |d| {
                                d.child(div().size(px(5.0)).rounded_full().bg(t.accent))
                            })
                            .child(SharedString::from(short_name(a)))
                            .child(div().flex_1())
                            .child(div().text_color(pct_color(sc)).child(sv.clone())),
                    )
                    .child(
                        div()
                            .h(px(16.0))
                            .flex()
                            .items_center()
                            .child(bar(sf, sc, 96.0)),
                    ),
            };
            // Hover card per account: remember where the account sits (painted bounds), and
            // report hover with that area in physical screen px.
            let areas = self.areas.clone();
            let account = a.id;
            let area = div()
                .id(("tb-acc", i))
                .relative()
                .child(col)
                .child(
                    gpui::canvas(
                        move |b, _, _| {
                            let mut v = areas.borrow_mut();
                            if v.len() <= i {
                                v.resize(i + 1, b);
                            }
                            v[i] = b;
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
                .on_hover({
                    let areas = self.areas.clone();
                    move |hovered, window, cx| {
                        let anchor = areas
                            .borrow()
                            .get(i)
                            .copied()
                            .and_then(|b| physical_rect(window, b));
                        match (hovered, anchor) {
                            (true, Some(r)) => crate::hover_card::badge_enter(account, r, cx),
                            _ => crate::hover_card::badge_leave(cx),
                        }
                    }
                });
            row = row.child(area);
        }
        if accounts.is_empty() {
            row = row.child(
                div()
                    .text_size(px(12.0))
                    .text_color(secondary)
                    .child("AI SubHub"),
            );
        }
        div()
            .id("taskbar-display")
            .size_full()
            .flex()
            .items_center()
            // Clock typography: Segoe UI Variable 12 px, 16 px lines.
            .font_family("Segoe UI Variable Text")
            .text_size(px(12.0))
            .line_height(px(16.0))
            .text_color(if t.dark { gpui::white() } else { gpui::black() })
            .justify_center()
            .cursor_pointer()
            .child(
                // Transparent window, no boxes: a thin divider on each side marks the group as
                // its own taskbar section (badges have per-badge hover like tray icons; other
                // styles hover as a whole).
                div()
                    .h(px(40.0))
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(div().w(px(1.0)).h(px(24.0)).bg(outline))
                    .child(
                        div()
                            .h_full()
                            .flex()
                            .items_center()
                            .px(px(if badges { 0.0 } else { 8.0 }))
                            .rounded(px(4.0))
                            .when(!badges, |d| d.hover(|st| st.bg(outline.opacity(0.35))))
                            .child(row),
                    )
                    .child(div().w(px(1.0)).h(px(24.0)).bg(outline)),
            )
            // Press + move = drag along the taskbar (remembered); press + release = click.
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    crate::hover_card::hide(cx);
                    this.drag = Some((cursor_x(), window.bounds().origin.x.as_f32(), false));
                    this.drag_bar = taskbar_for(&this.monitor).map(|(bar, _)| bar);
                }),
            )
            .on_mouse_move(cx.listener(|this, ev: &gpui::MouseMoveEvent, window, cx| {
                let Some((start, left, moved)) = this.drag else {
                    return;
                };
                if ev.pressed_button != Some(gpui::MouseButton::Left) {
                    return;
                }
                let dx = (cursor_x() - start) as f32 / window.scale_factor();
                if !moved && dx.abs() < 4.0 {
                    return;
                }
                this.drag = Some((start, left, true));
                let b = window.bounds();
                let Some((r, scale, _)) = this.drag_bar else {
                    return;
                };
                let w = b.size.width.as_f32();
                let x = (left + dx).clamp(r.left as f32 / scale, r.right as f32 / scale - w);
                crate::window_style::set_bounds(
                    window,
                    Bounds {
                        origin: point(px(x), b.origin.y),
                        size: b.size,
                    },
                );
                cx.notify();
            }))
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    let bar = this.drag_bar.take();
                    match this.drag.take() {
                        Some((_, _, true)) => {
                            // Save as distance from this taskbar's right edge (per monitor).
                            if let Some((r, scale, is_main)) = bar {
                                let b = window.bounds();
                                let offset = r.right as f32 / scale
                                    - (b.origin.x.as_f32() + b.size.width.as_f32());
                                let monitor = this.monitor.clone();
                                this.state.update(cx, |s, _| {
                                    s.update_display(|d| {
                                        d.set_taskbar_offset(&monitor, is_main, offset)
                                    })
                                });
                            }
                        }
                        _ => {
                            let whole = Bounds {
                                origin: point(px(0.0), px(0.0)),
                                size: window.bounds().size,
                            };
                            if let Some(r) = physical_rect(window, whole) {
                                on_click(cx, r)
                            }
                        }
                    }
                }),
            )
    }
}

/// An area of `window` (logical, window-relative) in physical screen px.
fn physical_rect(window: &Window, b: Bounds<gpui::Pixels>) -> Option<subhub_core::placement::Rect> {
    let (ox, oy) = crate::window_style::client_origin(window)?;
    let s = window.scale_factor();
    Some(subhub_core::placement::Rect {
        left: ox as f32 + b.origin.x.as_f32() * s,
        top: oy as f32 + b.origin.y.as_f32() * s,
        right: ox as f32 + (b.origin.x + b.size.width).as_f32() * s,
        bottom: oy as f32 + (b.origin.y + b.size.height).as_f32() * s,
    })
}

/// The taskbar on this monitor: rect (physical px), scale, main or not, and its measured layout
/// (`taskbar_layout`, None until first read).
fn taskbar_for(monitor: &str) -> Option<(Bar, Option<crate::taskbar_layout::Layout>)> {
    let m = crate::monitors::find(monitor)?;
    let layout = m.taskbar.and_then(crate::taskbar_layout::get);
    Some(((m.taskbar_rect()?, m.scale, m.is_main), layout))
}

fn tray_area_left() -> Option<i32> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW, GetWindowRect};
    unsafe {
        let bar = FindWindowW(windows::core::w!("Shell_TrayWnd"), None).ok()?;
        let notify =
            FindWindowExW(Some(bar), None, windows::core::w!("TrayNotifyWnd"), None).ok()?;
        let mut r = RECT::default();
        GetWindowRect(notify, &mut r).ok()?;
        Some(r.left)
    }
}

/// Logical bounds over this monitor's taskbar for the current settings.
fn bounds_for(s: &AppState, monitor: &str) -> Option<Bounds<gpui::Pixels>> {
    let ((r, scale, is_main), layout) = taskbar_for(monitor)?;
    let layout = layout.unwrap_or_default();
    let height_phys = r.bottom - r.top;
    if height_phys <= 0 || (r.right - r.left) < (r.bottom - r.top) * 3 {
        return None; // vertical or hidden taskbar: not supported
    }
    let w = width_for(s);
    // Start 2 px below the taskbar's top so its top border line stays visible (owner, 2026-11-06),
    // then fill the rest of the height.
    const TOP_GAP: f32 = 2.0;
    let h = (height_phys as f32 / scale - TOP_GAP).clamp(28.0, 64.0);
    let top = r.top as f32 / scale + TOP_GAP;
    let right = r.right as f32 / scale;
    let x = match (
        s.display().taskbar_offset_for(monitor, is_main),
        s.display().taskbar_position,
    ) {
        // Dragged by the user: keep the same distance from the taskbar's right edge.
        (Some(offset), _) => right - offset - w,
        // Right of the widgets button (weather): measured (owner, 2026-11-08: a 44 px icon on
        // his PC, ~170 px with the weather text); without one, right of the taskbar's start.
        (None, TaskbarPosition::Left) => match layout.widgets_right {
            Some(wr) => wr as f32 / scale + 8.0,
            None if layout.tray_left.is_some() => r.left as f32 / scale + 8.0,
            None => r.left as f32 / scale + 176.0,
        },
        // Left of the tray area: TrayNotifyWnd on the main taskbar, else what UI Automation saw
        // (the clock on other monitors); the old guesses only before the first read.
        (None, TaskbarPosition::Right) => {
            let measured = if is_main { tray_area_left() } else { None }.or(layout.tray_left);
            match measured {
                Some(l) => l as f32 / scale - w - 8.0,
                None if is_main => right - 380.0 - w - 8.0,
                None => right - 160.0 - w,
            }
        }
    };
    let x = x.clamp(r.left as f32 / scale, right - w);
    Some(Bounds {
        origin: point(px(x), px(top)),
        size: size(px(w), px(h)),
    })
}

fn place(window: &mut Window, s: &AppState, monitor: &str) {
    if let Some(b) = bounds_for(s, monitor)
        && window.bounds() != b
    {
        crate::window_style::set_bounds(window, b);
    }
}

/// Opens the display on the taskbar of `monitor` (a `monitors::Monitor::id`).
pub fn open(
    state: Entity<AppState>,
    monitor: String,
    on_click: OnClick,
    cx: &mut App,
) -> Option<WindowHandle<TaskbarDisplay>> {
    let s = state.read(cx);
    if !s.display().taskbar_enabled || s.display().taskbar_mode == TaskbarMode::Off {
        return None;
    }
    let bounds = bounds_for(s, &monitor)?;
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        // Opened on its own monitor: moving it there later re-enters GPUI (see monitors.rs).
        display_id: crate::monitors::find(&monitor).map(|m| crate::monitors::display_id(m.handle)),
        titlebar: None,
        kind: WindowKind::PopUp,
        focus: false,
        show: true,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    let handle = cx
        .open_window(options, |window, cx| {
            // No gpui-component Root here: it paints the theme background, and this window
            // must stay transparent (only text, bars and outlines on the taskbar).
            cx.new(|cx| TaskbarDisplay::new(state, monitor, on_click, window, cx))
        })
        .ok()?;
    let raw = handle
        .update(cx, |_, window, _| {
            crate::window_style::no_frame(window);
            crate::window_style::hwnd(window)
        })
        .ok()
        .flatten();
    if let Some(h) = raw {
        crate::foreground::track(h);
    }
    // Keep it above the taskbar, which comes to the front whenever it is clicked.
    cx.spawn(async move |cx| {
        let mut was_dark = taskbar_is_dark();
        // Fullscreen checks in a row: hide only after two (a moment of a fullscreen-looking
        // window, e.g. a game's loading screen or a window being dragged, does not blink it).
        let mut fullscreen_seen = 0u8;
        loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let alive = handle.update(cx, |this, window, cx| {
                // Windows mode switched (light/dark taskbar): redraw with matching colours.
                let dark = taskbar_is_dark();
                if dark != was_dark {
                    was_dark = dark;
                    cx.notify();
                }
                // Follow the taskbar when it moves or the monitor's scale changes (also
                // corrects a first placement made with another monitor's scale).
                if this.drag.is_none() {
                    place(window, this.state.read(cx), &this.monitor);
                }
                // Step aside for fullscreen videos, games and presentations; the taskbar is
                // hidden there too. Otherwise stay above the taskbar.
                // Only for a fullscreen app on this display's own monitor (owner, 2026-11-08).
                if crate::window_style::fullscreen_app_over(window) {
                    fullscreen_seen = fullscreen_seen.saturating_add(1);
                } else {
                    fullscreen_seen = 0;
                }
                if fullscreen_seen >= 2 {
                    crate::window_style::set_visible(window, false);
                } else if fullscreen_seen == 0 {
                    crate::window_style::set_visible(window, true);
                    crate::window_style::raise_topmost(window);
                }
            });
            if alive.is_err() {
                if let Some(h) = raw {
                    crate::foreground::untrack(h);
                }
                break;
            }
        }
    })
    .detach();
    Some(handle)
}

#[cfg(test)]
mod tests {
    use super::short_window_label;

    #[test]
    fn badge_labels_are_short() {
        assert_eq!(short_window_label("Monthly · AI credits"), "M");
        assert_eq!(short_window_label("Weekly · Auto"), "W");
        assert_eq!(short_window_label("5-hour"), "5h");
        assert_eq!(short_window_label("30-day"), "30d");
        assert_eq!(short_window_label("Credits"), "Cr");
    }
}
