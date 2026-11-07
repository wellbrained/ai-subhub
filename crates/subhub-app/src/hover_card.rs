//! Hover card: a small window styled like the popover that shows usage while the pointer rests
//! on the tray icon (all visible accounts) or on a taskbar badge (that account). One card at a
//! time, never focused, never over a fullscreen app or the open popover. Content comes from
//! `subhub_core::hover`, placement from `subhub_core::placement`.
//!
//! Showing: the tray icon reports `NIN_POPUPOPEN` after Windows' own hover delay; badges wait
//! `BADGE_DELAY`. Hiding: `NIN_POPUPCLOSE`, a click, a drag, the popover opening, or the
//! watchdog (the pointer left the anchor), since Windows does not always send the close.
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use gpui::{
    App, AppContext, Bounds, Context, Entity, Hsla, IntoElement, ParentElement, Render,
    SharedString, Styled, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle,
    WindowKind, WindowOptions, div, point, prelude::*, px, size,
};
use subhub_core::hover::{self, AccountView, Card, CardAccount, Health, Steering};
use subhub_core::model::AllowanceWindow;
use subhub_core::placement::{self, Rect};
use uuid::Uuid;

use crate::format;
use crate::state::AppState;
use crate::theme::Tokens;

/// Pause before a badge's card shows (about Windows' tooltip delay).
const BADGE_DELAY: Duration = Duration::from_millis(400);
/// How often the watchdog checks the pointer.
const WATCH: Duration = Duration::from_millis(100);
/// Most accounts on the tray card; the rest are "+N more".
pub const TRAY_MAX: usize = 6;
const TRAY_WIDTH: f32 = 300.0;
const BADGE_WIDTH: f32 = 264.0;
// Fixed row heights (logical px) so the window height is known before drawing.
const PAD: f32 = 10.0;
const HEADER_H: f32 = 22.0;
const NAME_H: f32 = 20.0;
const METRIC_H: f32 = 18.0;
const BLOCK_PAD: f32 = 6.0;
const FOOTER_H: f32 = 22.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Tray,
    Badge(Uuid),
}

type Blocked = Rc<dyn Fn(&App) -> bool>;

#[derive(Default)]
struct Ctl {
    state: Option<Entity<AppState>>,
    /// True while hover cards must not show (the popover is open).
    blocked: Option<Blocked>,
    handle: Option<WindowHandle<HoverCard>>,
    /// The monitor the card window was opened on.
    display: Option<gpui::DisplayId>,
    /// What the card shows now (None = hidden).
    shown: Option<Source>,
    /// Physical rect of what the card belongs to; the watchdog hides the card outside it.
    anchor: Option<Rect>,
    /// Bumped on every enter/leave/hide; pending delays and watchdogs from older ones stop.
    generation: u64,
}

thread_local! {
    static CTL: RefCell<Ctl> = RefCell::new(Ctl::default());
}

/// Called once at start.
pub fn init(state: Entity<AppState>, blocked: Blocked) {
    CTL.with(|c| {
        let mut c = c.borrow_mut();
        c.state = Some(state);
        c.blocked = Some(blocked);
    });
}

fn bump() -> u64 {
    CTL.with(|c| {
        let mut c = c.borrow_mut();
        c.generation += 1;
        c.generation
    })
}

fn generation() -> u64 {
    CTL.with(|c| c.borrow().generation)
}

/// The pointer entered a badge (`anchor` in physical px). Moving between badges switches the
/// card at once; otherwise it shows after `BADGE_DELAY` if the pointer is still there.
pub fn badge_enter(account: Uuid, anchor: Rect, cx: &mut App) {
    let g = bump();
    // A card is up (another badge or the tray): switch at once, so the old one never lingers
    // without its watchdog.
    if CTL.with(|c| c.borrow().shown).is_some() {
        show(Source::Badge(account), anchor, g, cx);
        return;
    }
    cx.spawn(async move |cx| {
        cx.background_executor().timer(BADGE_DELAY).await;
        cx.update(|cx| {
            if generation() == g && cursor_in(&anchor) {
                show(Source::Badge(account), anchor, g, cx);
            }
        });
    })
    .detach();
}

/// The pointer left a badge: cancel a pending show. A shown card is left to the watchdog, so
/// moving onto the next badge does not flicker.
pub fn badge_leave(_cx: &mut App) {
    if CTL.with(|c| c.borrow().shown.is_none()) {
        bump();
    }
}

/// Hides the card now (click, drag, popover opening, setting switched off).
pub fn hide(cx: &mut App) {
    bump();
    let handle = CTL.with(|c| {
        let mut c = c.borrow_mut();
        c.shown = None;
        c.anchor = None;
        c.handle
    });
    if let Some(h) = handle {
        let _ = h.update(cx, |_, window, _| {
            crate::window_style::set_visible(window, false)
        });
    }
}

fn cursor() -> Option<(f32, f32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p).ok()? };
    Some((p.x as f32, p.y as f32))
}

/// The pointer is over `r` (with a little slack for the edges).
fn cursor_in(r: &Rect) -> bool {
    const SLACK: f32 = 3.0;
    cursor().is_some_and(|(x, y)| {
        x >= r.left - SLACK && x <= r.right + SLACK && y >= r.top - SLACK && y <= r.bottom + SLACK
    })
}

/// Everything the card knows about the accounts, in settings order.
pub fn account_views(s: &AppState) -> Vec<AccountView<'_>> {
    let steer = &s.settings.steering;
    s.settings
        .accounts
        .iter()
        .map(|a| AccountView {
            account: a,
            reading: s.settings.readings.get(&a.id),
            last_state: s.status.get(&a.id).map(|st| st.state),
            stale: s.is_stale(a.id),
            plan: s.plans.get(&a.id).cloned(),
            steering: if steer.preferred == Some(a.id) {
                Steering::Preferred
            } else if steer.enabled && steer.current == Some(a.id) {
                Steering::Next
            } else {
                Steering::None
            },
        })
        .collect()
}

/// The card for `source`; None when the account is gone.
fn card_for(s: &AppState, source: Source) -> Option<Card> {
    let views = account_views(s);
    match source {
        Source::Tray => Some(hover::tray_card(&views, TRAY_MAX)),
        Source::Badge(id) => {
            let v = views.iter().find(|v| v.account.id == id)?;
            Some(Card {
                accounts: vec![hover::card_account(v)],
                more: 0,
            })
        }
    }
}

/// Height of the card (logical px) for these rows.
fn card_height(card: &Card, source: Source) -> f32 {
    let mut h = PAD * 2.0;
    if source == Source::Tray {
        h += HEADER_H;
    }
    for (i, a) in card.accounts.iter().enumerate() {
        h += BLOCK_PAD * 2.0 + NAME_H + METRIC_H * a.windows.len().max(1) as f32;
        if i > 0 {
            h += 1.0; // divider
        }
    }
    if card.more > 0 {
        h += METRIC_H + 2.0;
    }
    if matches!(source, Source::Badge(_)) {
        h += FOOTER_H;
    }
    h
}

fn width_of(source: Source) -> f32 {
    match source {
        Source::Tray => TRAY_WIDTH,
        Source::Badge(_) => BADGE_WIDTH,
    }
}

fn show(source: Source, anchor: Rect, g: u64, cx: &mut App) {
    let (state, blocked, handle) = CTL.with(|c| {
        let c = c.borrow();
        (c.state.clone(), c.blocked.clone(), c.handle)
    });
    let Some(state) = state else { return };
    if !state.read(cx).display().hover_details
        || blocked.is_some_and(|b| b(cx))
        || crate::window_style::fullscreen_app_in_front()
    {
        hide(cx); // a card from before must not stay up without its watchdog
        return;
    }
    let Some(card) = card_for(state.read(cx), source) else {
        hide(cx);
        return;
    };
    let (w, h) = (width_of(source), card_height(&card, source));
    let cxp = ((anchor.left + anchor.right) / 2.0) as i32;
    let cyp = ((anchor.top + anchor.bottom) / 2.0) as i32;
    let Some((display, scale, work)) = crate::monitors::at_point(cxp, cyp) else {
        hide(cx);
        return;
    };
    let (x, y) = placement::card_origin(
        anchor,
        w * scale,
        h * scale,
        crate::monitors::rect(work),
        12.0 * scale,
        8.0 * scale,
    );
    let (pw, ph) = ((w * scale).round() as i32, (h * scale).round() as i32);

    // The card window lives on one monitor; for another monitor it is opened anew there
    // (moving it across monitors inside a GPUI update panics, see monitors.rs).
    let same = CTL.with(|c| c.borrow().display) == Some(display);
    if !same && let Some(old) = handle {
        let _ = old.update(cx, |_, window, _| window.remove_window());
    }
    let handle = match handle.filter(|h| same && h.update(cx, |_, _, _| ()).is_ok()) {
        Some(h) => h,
        None => {
            let options = WindowOptions {
                display_id: Some(display),
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(x / scale), px(y / scale)),
                    size: size(px(w), px(h)),
                })),
                titlebar: None,
                kind: WindowKind::PopUp,
                focus: false,
                // Shown at once: a hidden window stays on the default monitor until shown, and
                // moving it to its own monitor later would re-enter GPUI.
                show: true,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                window_background: WindowBackgroundAppearance::Opaque,
                ..Default::default()
            };
            let st = state.clone();
            let Ok(h) =
                cx.open_window(options, |_, cx| cx.new(|cx| HoverCard::new(st, source, cx)))
            else {
                return;
            };
            CTL.with(|c| {
                let mut c = c.borrow_mut();
                c.handle = Some(h);
                c.display = Some(display);
            });
            h
        }
    };
    let dark = crate::theme::is_dark(state.read(cx).display().theme, cx);
    let border = Tokens::new(dark, &state.read(cx).display().accent).border;
    let _ = handle.update(cx, |view, window, cx| {
        view.source = source;
        cx.notify();
        crate::window_style::round(window, border);
        crate::window_style::place_client_physical(window, x as i32, y as i32, pw, ph);
        crate::window_style::set_visible(window, true);
    });
    CTL.with(|c| {
        let mut c = c.borrow_mut();
        c.shown = Some(source);
        c.anchor = Some(anchor);
    });
    watch(g, cx);
}

/// Hides the card once the pointer has been off its anchor for two checks in a row, the
/// popover opened, or a fullscreen app came to the front.
fn watch(g: u64, cx: &mut App) {
    cx.spawn(async move |cx| {
        let mut outside = 0;
        loop {
            cx.background_executor().timer(WATCH).await;
            let done = cx.update(|cx| {
                if generation() != g {
                    return true;
                }
                let (anchor, blocked) =
                    CTL.with(|c| (c.borrow().anchor, c.borrow().blocked.clone()));
                let Some(anchor) = anchor else { return true };
                outside = if cursor_in(&anchor) { 0 } else { outside + 1 };
                if outside >= 2
                    || blocked.is_some_and(|b| b(cx))
                    || crate::window_style::fullscreen_app_in_front()
                {
                    hide(cx);
                    return true;
                }
                false
            });
            if done {
                break;
            }
        }
    })
    .detach();
}

pub struct HoverCard {
    state: Entity<AppState>,
    source: Source,
}

impl HoverCard {
    fn new(state: Entity<AppState>, source: Source, cx: &mut Context<Self>) -> Self {
        // Live while shown: a refresh during the hover updates the numbers.
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        Self { state, source }
    }
}

fn pill(text: impl Into<SharedString>, bg: Hsla, fg: Hsla) -> gpui::Div {
    div()
        .flex_none()
        .whitespace_nowrap()
        .px(px(6.0))
        .rounded_full()
        .bg(bg)
        .text_size(px(10.0))
        .line_height(px(15.0))
        .text_color(fg)
        .child(text.into())
}

impl Render for HoverCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.state.read(cx);
        let d = s.display().clone();
        let t = Tokens::new(crate::theme::is_dark(d.theme, cx), &d.accent);
        let now = Utc::now();
        let card = card_for(s, self.source).unwrap_or(Card {
            accounts: vec![],
            more: 0,
        });
        // Label column: narrow for "5h"/"W", wider when a provider label shows (e.g. "30-day").
        let label_w = if card
            .accounts
            .iter()
            .flat_map(|a| &a.windows)
            .all(|w| w.label.chars().count() <= 2)
        {
            18.0
        } else {
            40.0
        };
        let account_block = |a: &CardAccount, first: bool| {
            let acc = s.account(a.id);
            let mut name = div()
                .h(px(NAME_H))
                .flex()
                .items_center()
                .gap(px(6.0))
                .min_w_0()
                // Provider mark in the validated series colour (icons::series_color).
                .child(crate::icons::icon(
                    crate::icons::Icon::Provider(a.provider),
                    13.0,
                    crate::icons::series_color(a.provider, t.dark),
                ));
            if let Some(acc) = acc {
                let (tag, tag_bg) = crate::taskbar::account_tag(s, acc);
                name = name.child(
                    div()
                        .flex_none()
                        .px(px(3.0))
                        .rounded(px(3.0))
                        .bg(tag_bg)
                        .text_size(px(9.0))
                        .line_height(px(12.0))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(gpui::white())
                        .child(tag),
                );
            }
            let title = match (&a.plan, self.source) {
                (Some(p), Source::Badge(_)) if d.show_plan => format!("{} · {p}", a.name),
                _ => a.name.clone(),
            };
            name = name.child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(title),
            );
            match a.steering {
                Steering::Preferred => {
                    name = name.child(pill("★ Hub · preferred", t.surface_hover, t.accent))
                }
                Steering::Next => name = name.child(pill("Hub · next", t.surface_hover, t.accent)),
                Steering::None => {}
            }
            if let Some(label) = a.health.label().filter(|_| a.health != Health::NoReading) {
                let text = match (a.health, a.fetched_at) {
                    (Health::Stale, Some(at)) => format!("Stale · {}", format::ago(at, now)),
                    _ => label.to_string(),
                };
                name = name.child(pill(text, t.surface_hover, t.warn));
            }
            let dim = a.health.dimmed();
            // Fixed columns: label, % 64, reset 62, three 6 px gaps, 12 px padding each side.
            let bar_w = width_of(self.source) - 24.0 - label_w - 64.0 - 62.0 - 18.0;
            let mut block = div()
                .flex()
                .flex_col()
                .py(px(BLOCK_PAD))
                .when(!first, |b| b.border_t_1().border_color(t.border))
                .child(name);
            if a.windows.is_empty() {
                block = block.child(
                    div()
                        .h(px(METRIC_H))
                        .text_size(px(11.0))
                        .text_color(t.muted)
                        .child(a.health.label().unwrap_or("No reading yet")),
                );
            }
            for w in &a.windows {
                let aw = AllowanceWindow {
                    key: String::new(),
                    label: w.label.clone(),
                    used_percent: w.used_percent,
                    resets_at: w.resets_at,
                };
                let color = if dim {
                    t.faint
                } else {
                    t.level(w.remaining_percent, d.running_out_threshold)
                };
                let reset = format::reset(&aw, &d, now).unwrap_or_default();
                block = block.child(
                    div()
                        .h(px(METRIC_H))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(px(11.0))
                        .when(dim, |r| r.opacity(0.6))
                        .child(
                            div()
                                .flex_none()
                                .w(px(label_w))
                                .whitespace_nowrap()
                                .text_color(t.muted)
                                .child(w.label.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .w(px(bar_w))
                                .h(px(5.0))
                                .rounded_full()
                                .bg(t.track)
                                .child(
                                    div()
                                        .h_full()
                                        .rounded_full()
                                        .bg(color)
                                        .w(gpui::relative(format::gauge_fraction(&aw, &d))),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .w(px(64.0))
                                .flex()
                                .justify_end()
                                .text_color(t.text)
                                .child(format::usage(&aw, &d)),
                        )
                        .child(
                            div()
                                .flex_none()
                                .w(px(62.0))
                                .flex()
                                .justify_end()
                                .whitespace_nowrap()
                                .text_color(t.muted)
                                .child(reset),
                        ),
                );
            }
            block
        };

        let mut body = div()
            .size_full()
            .flex()
            .flex_col()
            .px(px(12.0))
            .py(px(PAD))
            .bg(t.bg)
            .text_color(t.text)
            .text_size(px(12.0))
            .line_height(px(16.0))
            .font_family("Segoe UI Variable Text");
        if self.source == Source::Tray {
            let latest = card.accounts.iter().filter_map(|a| a.fetched_at).max();
            body = body.child(
                div()
                    .h(px(HEADER_H))
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(px(11.0))
                    .text_color(t.muted)
                    .child("AI SubHub")
                    .when_some(latest, |d, at| {
                        d.child(format!("Updated {}", format::ago(at, now)))
                    }),
            );
        }
        for (i, a) in card.accounts.iter().enumerate() {
            body = body.child(account_block(a, i == 0));
        }
        if card.accounts.is_empty() {
            body = body.child(div().text_color(t.muted).child("No accounts yet"));
        }
        if card.more > 0 {
            body = body.child(
                div()
                    .h(px(METRIC_H))
                    .mt(px(2.0))
                    .text_size(px(11.0))
                    .text_color(t.muted)
                    .child(format!("+{} more · click to open", card.more)),
            );
        }
        if let Source::Badge(_) = self.source {
            let at = card.accounts.first().and_then(|a| a.fetched_at);
            body = body.child(
                div()
                    .h(px(FOOTER_H))
                    .flex()
                    .items_end()
                    .text_size(px(11.0))
                    .text_color(t.muted)
                    .child(match at {
                        Some(at) => format!("Updated {} · click to open", format::ago(at, now)),
                        None => "Click to open".to_string(),
                    }),
            );
        }
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use subhub_core::hover::CardWindow;
    use subhub_core::model::ProviderKind;

    fn acc(windows: usize) -> CardAccount {
        CardAccount {
            id: Uuid::new_v4(),
            name: "A".into(),
            provider: ProviderKind::Claude,
            plan: None,
            health: Health::Fresh,
            steering: Steering::None,
            fetched_at: None,
            windows: (0..windows)
                .map(|_| CardWindow {
                    label: "5h".into(),
                    remaining_percent: 50.0,
                    used_percent: 50.0,
                    resets_at: None,
                })
                .collect(),
        }
    }

    #[test]
    fn height_counts_rows_dividers_and_extras() {
        let card = Card {
            accounts: vec![acc(2), acc(0)],
            more: 3,
        };
        // pad 20 + header 22 + (12+20+36) + (12+20+18+1) + more 20
        assert_eq!(card_height(&card, Source::Tray), 181.0);
        let one = Card {
            accounts: vec![acc(2)],
            more: 0,
        };
        // pad 20 + (12+20+36) + footer 22
        assert_eq!(card_height(&one, Source::Badge(Uuid::nil())), 110.0);
    }
}
