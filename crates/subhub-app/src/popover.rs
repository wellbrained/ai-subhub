//! The tray popover, layout "A · Classic, refined" (ADR 0001 refinements): header with status and
//! actions, provider headings, one card per account with source tags and one bar row per window.
use chrono::Utc;
use gpui::{
    App, Context, Entity, FontWeight, Hsla, IntoElement, ParentElement, Render, SharedString,
    Styled, Subscription, Window, div, prelude::*, px,
};
use subhub_core::billing;
use subhub_core::launch;
use subhub_core::model::{Account, AllowanceWindow, ProviderKind, UsageState};
use subhub_core::projects;
use subhub_core::schedule::Schedule;

use crate::format;
use crate::icons::{Icon, icon, provider_color};
use crate::state::{AppState, HubStatus, PendingNotes, SettingsTarget, provider_name};
use gpui_component::scroll::ScrollableElement;
use crate::theme::{self, Tokens};

pub const WIDTH: f32 = 460.0;

/// Closes the popover (tray click, Esc, ✕, or focus loss when enabled).
pub type CloseHandler = Box<dyn Fn(&mut Window, &mut App)>;

pub struct Popover {
    state: Entity<AppState>,
    on_settings: std::rc::Rc<dyn Fn(&mut App)>,
    on_stats: std::rc::Rc<dyn Fn(&mut App)>,
    on_close: std::rc::Rc<CloseHandler>,
    focus: gpui::FocusHandle,
    last_height: f32,
    /// The top edge is being dragged (the height follows the cursor, not the content).
    dragging: bool,
    /// Scroll position of the patch-notes dialog's body.
    notes_scroll: gpui::ScrollHandle,
    _subs: Vec<Subscription>,
}

impl Popover {
    pub fn new(
        state: Entity<AppState>,
        on_settings: std::rc::Rc<dyn Fn(&mut App)>,
        on_stats: std::rc::Rc<dyn Fn(&mut App)>,
        on_close: CloseHandler,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let on_close = std::rc::Rc::new(on_close);
        // A drag cut short by closing the popover must not block automatic updates.
        // A Launch dropdown left open last time would float above the notes dialog.
        state.update(cx, |s, _| {
            s.popover_dragging = false;
            if s.pending_notes.is_some() {
                s.launch_open = None;
            }
        });
        let blur_close = on_close.clone();
        let subs = vec![
            // Content changes (new readings, hidden windows) re-fit the height, bottom anchored.
            cx.observe_in(&state, window, |this, state, window, cx| {
                let h = Self::preferred_height(state.read(cx));
                if this.dragging {
                    cx.notify();
                    return;
                }
                if (h - this.last_height).abs() > 1.0 {
                    this.last_height = h;
                    // Resize on the next tick, outside the event that changed the state: resizing
                    // while a click in this window is still being handled left the new area
                    // unpainted (white strip, the content below cut off; launch panel, 2026-11-07).
                    cx.spawn_in(window, async move |this, cx| {
                        let _ = this.update_in(cx, |_, window, cx| {
                            // Through GPUI, so its drawing surface follows (a raw SetWindowPos
                            // that grew the window left the new area white, like the Statistics
                            // window once); the bottom edge stays put.
                            crate::window_style::resize_keep_bottom(window, cx, h);
                            cx.notify();
                        });
                    })
                    .detach();
                }
                cx.notify();
            }),
            // Only when the user opted in; by default the popover stays open.
            cx.observe_window_activation(window, move |this, window, cx| {
                let enabled = this.state.read(cx).display().close_popover_on_blur;
                if enabled && !window.is_window_active() {
                    blur_close(window, cx);
                }
            }),
        ];
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let last_height = Self::preferred_height(state.read(cx));
        Self {
            state,
            on_settings,
            on_stats,
            on_close,
            focus,
            last_height,
            dragging: false,
            notes_scroll: gpui::ScrollHandle::new(),
            _subs: subs,
        }
    }

    /// Drag on the top edge: follows the cursor (polled, so it keeps working above the window,
    /// where this window gets no mouse events) until the button is released, keeps the bottom
    /// edge, then saves the height. Double-click goes back to fitting the content.
    fn start_top_drag(&mut self, click_count: usize, window: &mut Window, cx: &mut Context<Self>) {
        if click_count >= 2 {
            self.state.update(cx, |s, cx| {
                s.set_popover_height(None);
                cx.notify();
            });
            return;
        }
        let (Some(start_y), Some(top)) = (
            crate::window_style::cursor_screen_y(),
            crate::window_style::work_area_top(window),
        ) else {
            return;
        };
        let scale = window.scale_factor();
        let start_h = f32::from(window.viewport_size().height);
        let bottom = crate::window_style::outer_bottom(window).unwrap_or(start_y);
        // Up to the work area's top edge (a little margin), at least the usual minimum.
        let max_h = ((bottom - top) as f32 / scale - 12.0).max(MIN_HEIGHT);
        self.dragging = true;
        // An automatic update restart waits while the edge is held.
        self.state.update(cx, |s, _| s.popover_dragging = true);
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(16))
                    .await;
                let done = !crate::window_style::left_button_down();
                let y = crate::window_style::cursor_screen_y().unwrap_or(start_y);
                let h = (start_h + (start_y - y) as f32 / scale)
                    .clamp(MIN_HEIGHT, max_h)
                    .round();
                let ok = this.update_in(cx, |this, window, cx| {
                    if (h - this.last_height).abs() >= 1.0 {
                        this.last_height = h;
                        crate::window_style::resize_to_bottom(window, cx, h, bottom);
                    }
                    if done {
                        this.dragging = false;
                        this.state.update(cx, |s, cx| {
                            s.popover_dragging = false;
                            s.set_popover_height(Some(h));
                            cx.notify();
                        });
                    }
                });
                if ok.is_err() || done {
                    break;
                }
            }
        })
        .detach();
    }

    /// The height the user dragged the top edge to, else the height that fits the content
    /// (240–720 px); anything taller scrolls.
    pub fn preferred_height(s: &AppState) -> f32 {
        if let Some(h) = s.display().popover_height {
            return h.clamp(MIN_HEIGHT, 2000.0);
        }
        let row = 26.0;
        let visible: Vec<&Account> = s.settings.accounts.iter().filter(|a| !a.hidden).collect();
        if visible.is_empty() {
            return 240.0;
        }
        let providers = [ProviderKind::Claude, ProviderKind::Codex]
            .iter()
            .filter(|p| visible.iter().any(|a| a.provider == **p))
            .count();
        let cards: f32 = visible
            .iter()
            .map(|a| {
                let rows = s.settings.readings.get(&a.id).map_or(1, |r| {
                    r.windows
                        .iter()
                        .filter(|w| !a.hidden_windows.contains(&w.key))
                        .count()
                        .max(1)
                });
                let note = if s.is_stale(a.id) { 20.0 } else { 0.0 }
                    + if s.display().show_email && a.email.is_some() {
                        16.0
                    } else {
                        0.0
                    }
                    + card_extras(a, s);
                62.0 + rows as f32 * row + note
            })
            .sum();
        // Header, provider headings, cards, plus room under the last card (it touched the edge).
        (78.0 + providers as f32 * 40.0 + cards).clamp(MIN_HEIGHT, 720.0)
    }
}

const MIN_HEIGHT: f32 = 240.0;

impl Render for Popover {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.state.read(cx);
        let d = s.display().clone();
        let t = Tokens::new(theme::is_dark(d.theme, cx), &d.accent);
        let esc_close = self.on_close.clone();
        let esc_state = self.state.clone();
        let notes_dialog = s
            .pending_notes
            .as_ref()
            .map(|p| self.notes_dialog(p, &t, f32::from(window.viewport_size().height), cx));
        // "Manage projects…" in a Launch menu: open Settings on the Projects page.
        let manage: std::rc::Rc<dyn Fn(&mut App)> = {
            let state = self.state.clone();
            let on_settings = self.on_settings.clone();
            std::rc::Rc::new(move |cx: &mut App| {
                state.update(cx, |s, cx| {
                    s.pending_settings_page = Some(SettingsTarget::Projects);
                    cx.notify();
                });
                on_settings(cx);
            })
        };

        let mut body = div().flex().flex_col();
        if s.settings.accounts.iter().all(|a| a.hidden) {
            body = body.child(empty_state(s, &t));
        }
        for provider in [ProviderKind::Claude, ProviderKind::Codex] {
            let accs: Vec<&Account> = s
                .settings
                .accounts
                .iter()
                .filter(|a| a.provider == provider && !a.hidden)
                .collect();
            if accs.is_empty() {
                continue;
            }
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_1()
                    .pt_3()
                    .pb_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(provider_name(provider)),
                    )
                    .child(icon(
                        Icon::Provider(provider),
                        14.0,
                        provider_color(provider, t.muted),
                    )),
            );
            for acc in accs {
                body = body.child(account_card(acc, s, &t, self.state.clone(), manage.clone()));
            }
        }

        div()
            .track_focus(&self.focus)
            .on_key_down(move |ev: &gpui::KeyDownEvent, window, cx| {
                if ev.keystroke.key == "escape" {
                    // The patch-notes dialog closes first, not the popover.
                    if esc_state.read(cx).pending_notes.is_some() {
                        esc_state.update(cx, |s, cx| s.dismiss_whats_new(cx));
                    } else {
                        esc_close(window, cx);
                    }
                }
            })
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .text_color(t.text)
            .relative()
            // Corners, outline and shadow come from DWM (window_style::round).
            .child(self.header(s, &t))
            .child(
                div()
                    .id("body")
                    .flex_1()
                    .overflow_y_scroll()
                    .px_4()
                    .pb_3()
                    .child(body),
            )
            .child(
                // Top edge: drag up or down to resize (saved); double-click fits the content.
                div()
                    .id("resize-top")
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .h(px(6.0))
                    .cursor_ns_resize()
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, ev: &gpui::MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.start_top_drag(ev.click_count, window, cx);
                        }),
                    ),
            )
            .children(notes_dialog)
    }
}

impl Popover {
    /// "Updated to vX.Y.Z" over the popover content: a dim scrim (click closes) and a rounded
    /// card near the top with the notes of every version since the last one seen, scrollable,
    /// and a Got it button. Absolutely positioned, so the popover's height never changes.
    fn notes_dialog(
        &self,
        p: &PendingNotes,
        t: &Tokens,
        viewport_h: f32,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        const TOP: f32 = 40.0;
        const BOTTOM_GAP: f32 = 12.0;
        // Title, button and padding around the scrolling body.
        const CHROME: f32 = 126.0;
        const GUTTER: f32 = 12.0;
        let body_max = (viewport_h * 0.6)
            .min(viewport_h - TOP - BOTTOM_GAP - CHROME)
            .max(60.0);

        let mut notes = div().flex().flex_col().gap_3();
        for s in &p.sections {
            let mut head = div().flex().items_baseline().gap_2().child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(SharedString::from(format!("v{}", s.version))),
            );
            if let Some(date) = &s.date {
                head = head.child(
                    div()
                        .text_xs()
                        .text_color(t.faint)
                        .child(SharedString::from(date.clone())),
                );
            }
            let mut block = div().flex().flex_col().gap(px(6.0)).child(head);
            for b in &s.bullets {
                block = block.child(
                    div()
                        .flex()
                        .gap_2()
                        .text_sm()
                        .child(
                            div()
                                .flex_none()
                                .mt(px(7.0))
                                .size(px(4.0))
                                .rounded_full()
                                .bg(t.faint),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(SharedString::from(b.clone())),
                        ),
                );
            }
            notes = notes.child(block);
        }

        let scrim = if t.dark { 0.55 } else { 0.32 };
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                div()
                    .id("notes-scrim")
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .occlude()
                    .bg(gpui::hsla(0.0, 0.0, 0.0, scrim))
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _: &gpui::MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.state.update(cx, |s, cx| s.dismiss_whats_new(cx));
                        }),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .top(px(TOP))
                    .left_0()
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .id("notes-card")
                            .occlude()
                            .w(px(380.0))
                            .max_w_full()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .p_4()
                            .rounded(px(12.0))
                            .bg(t.surface)
                            .border_1()
                            .border_color(t.border)
                            .shadow(vec![gpui::BoxShadow {
                                color: gpui::hsla(0.0, 0.0, 0.0, if t.dark { 0.45 } else { 0.16 }),
                                offset: gpui::point(px(0.0), px(8.0)),
                                blur_radius: px(24.0),
                                spread_radius: px(-4.0),
                                inset: false,
                            }])
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.0))
                                    .child(
                                        div()
                                            .text_base()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(SharedString::from(format!(
                                                "Updated to v{}",
                                                p.version
                                            ))),
                                    )
                                    .child(
                                        div().text_xs().text_color(t.muted).child("What's new"),
                                    ),
                            )
                            .child(
                                div()
                                    .relative()
                                    .child(
                                        div()
                                            .id("notes-scroll")
                                            .max_h(px(body_max))
                                            .overflow_y_scroll()
                                            .track_scroll(&self.notes_scroll)
                                            // Room for the always-visible scrollbar.
                                            .pr(px(GUTTER))
                                            .child(notes),
                                    )
                                    .vertical_scrollbar(&self.notes_scroll),
                            )
                            .child(
                                div().flex().justify_end().child(
                                    div()
                                        .id("notes-ok")
                                        .px_4()
                                        .py(px(5.0))
                                        .rounded(px(8.0))
                                        .text_xs()
                                        .cursor_pointer()
                                        .bg(t.accent)
                                        .text_color(gpui::white())
                                        .hover(|s| s.opacity(0.9))
                                        .child("Got it")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.state.update(cx, |s, cx| s.dismiss_whats_new(cx));
                                        })),
                                ),
                            ),
                    ),
            )
    }

    fn header(&self, s: &AppState, t: &Tokens) -> impl IntoElement {
        let now = Utc::now();
        let status = if s.refreshing {
            "Updating…".to_string()
        } else {
            let updated = s
                .last_refresh
                .map(|at| format!("Updated {}", format::ago(at, now)));
            let next = s
                .next_update()
                .filter(|n| *n > now)
                .map(|n| format!("next in {}", format::duration_until(n, now)));
            match (&s.hub, updated, next) {
                (HubStatus::Unauthorized, ..) => "Hub key rejected · check Settings".into(),
                (_, Some(u), Some(n)) => format!("{u} · {n}"),
                (_, Some(u), None) => u,
                (_, None, _) => "Waiting for first update".into(),
            }
        };
        let state = self.state.clone();
        let on_settings = self.on_settings.clone();
        let on_stats = self.on_stats.clone();
        let close = self.on_close.clone();
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .pt_3()
            .pb_1()
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("AI SubHub"),
            )
            .child(div().flex_1())
            .child(
                // When the data is from, next to the refresh button (quiet).
                div()
                    .text_xs()
                    .text_color(t.faint)
                    .whitespace_nowrap()
                    .pr_1()
                    .child(status),
            )
            .child(
                icon_button("refresh", Icon::Refresh, t).on_click(move |_, _, cx| {
                    state.update(cx, |s, cx| s.refresh(true, cx));
                }),
            )
            .child(icon_button("statistics", Icon::Chart, t).on_click(move |_, _, cx| on_stats(cx)))
            .child(
                icon_button("settings", Icon::Settings, t)
                    .on_click(move |_, _, cx| on_settings(cx)),
            )
            .child({
                // ✕ follows General → Close button: keep running in the tray, or quit.
                let quits = s.display().close_quits;
                icon_button("close", Icon::Close, t).on_click(move |_, window, cx| {
                    if quits {
                        cx.quit();
                    } else {
                        close(window, cx);
                    }
                })
            })
    }
}

fn icon_button(id: &'static str, glyph: Icon, t: &Tokens) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(28.0))
        .rounded(px(8.0))
        .flex()
        .items_center()
        .justify_center()
        .bg(t.surface)
        .border_1()
        .border_color(t.border)
        .text_sm()
        .text_color(t.muted)
        .cursor_pointer()
        .hover(|s| s.bg(t.surface_hover).text_color(t.text))
        .child(icon(glyph, 15.0, t.muted))
}

fn pill(text: impl Into<SharedString>, bg: Hsla, fg: Hsla) -> gpui::Div {
    div()
        .flex_none()
        .whitespace_nowrap()
        .px(px(7.0))
        .py(px(1.0))
        .rounded_full()
        .bg(bg)
        .text_xs()
        .text_color(fg)
        .child(text.into())
}

/// Source tags: Hub (clickable: prefer this account for new hub threads / back to Auto),
/// benched Hub, and "Local only" for a CLI profile outside the hub.
fn source_tags(acc: &Account, s: &AppState, t: &Tokens, state: Entity<AppState>) -> gpui::Div {
    let mut row = div().flex().flex_none().items_center().gap_1();
    let hub_bg = if t.dark {
        gpui::rgb(0x2d2546).into()
    } else {
        gpui::rgb(0xede9fe).into()
    };
    let hub_fg: Hsla = if t.dark {
        gpui::rgb(0xc4b5fd).into()
    } else {
        gpui::rgb(0x6d28d9).into()
    };
    match &acc.hub {
        Some(h) if h.benched => {
            row = row.child(pill("Hub · benched", t.surface_hover, t.faint));
        }
        Some(_) => {
            let steering = &s.settings.steering;
            let preferred = steering.preferred == Some(acc.id);
            let current = steering.enabled && steering.current == Some(acc.id);
            let label = if preferred {
                "★ Hub · preferred"
            } else if current {
                "Hub · next"
            } else {
                "Hub"
            };
            let id = acc.id;
            row = row.child(
                pill(
                    label,
                    if preferred || current {
                        t.accent
                    } else {
                        hub_bg
                    },
                    if preferred || current {
                        gpui::white()
                    } else {
                        hub_fg
                    },
                )
                .id(SharedString::from(format!("hubtag-{id}")))
                .cursor_pointer()
                .hover(|st| st.opacity(0.85))
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(if preferred {
                        "New hub threads go to this account. Click to return to Auto."
                    } else {
                        "In the hub. Click to send new hub threads here first."
                    })
                    .build(window, cx)
                })
                .on_click(move |_, _, cx| state.update(cx, |s, cx| s.toggle_preferred(id, cx))),
            );
        }
        None => {}
    }
    // A CLI profile next to a hub login is the normal case and gets no pill (owner, 2026-11-07);
    // only an account outside the hub is marked.
    if acc.profile_dir.is_some() && acc.hub.is_none() {
        row = row.child(pill("Local only", t.surface_hover, t.muted));
    }
    row
}

fn account_card(
    acc: &Account,
    s: &AppState,
    t: &Tokens,
    state: Entity<AppState>,
    on_manage: std::rc::Rc<dyn Fn(&mut App)>,
) -> impl IntoElement {
    let d = s.display();
    let now = Utc::now();
    let stale = s.is_stale(acc.id);
    let status = s.status.get(&acc.id);
    let reading = s.settings.readings.get(&acc.id);
    let state_for_actions = state.clone();

    // Header row on one line: name, pills, star, freshness. The login email (Appearance → Show
    // emails) gets its own full-width line under the whole row.
    let email = d.show_email.then(|| acc.email.clone()).flatten();
    let mut head = div().flex().items_center().gap_2().child(
        div()
            .min_w_0()
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .truncate()
            .child(acc.label.clone()),
    );
    if d.show_plan
        && let Some(plan) = s.plans.get(&acc.id)
    {
        let (bg, fg): (Hsla, Hsla) = if t.dark {
            (gpui::rgb(0x2b3550).into(), gpui::rgb(0xa9c1ff).into())
        } else {
            (gpui::rgb(0xe0e9ff).into(), gpui::rgb(0x1d4ed8).into())
        };
        head = head.child(pill(plan.clone(), bg, fg));
    }
    if let Some(g) = &acc.group {
        head = head.child(pill(g.clone(), t.surface_hover, t.muted));
    }
    let state_for_star = state.clone();
    head = head.child(source_tags(acc, s, t, state));
    if stale && !acc.hub.as_ref().is_some_and(|h| h.benched) {
        let label = match status.map(|s| s.state) {
            Some(UsageState::RateLimited) => "Cooling down",
            Some(UsageState::SignInRequired) => "Sign in needed",
            Some(UsageState::CliMissing) => "CLI missing",
            _ => "Stale",
        };
        let (bg, fg): (Hsla, Hsla) = if t.dark {
            (gpui::rgb(0x3b2a12).into(), gpui::rgb(0xfbbf24).into())
        } else {
            (gpui::rgb(0xfef3c7).into(), gpui::rgb(0xb45309).into())
        };
        head = head.child(pill(label, bg, fg));
    }
    head = head.child(div().flex_1());
    head = head.child(launch_button(
        acc,
        s,
        t,
        state_for_star.clone(),
        on_manage.clone(),
    ));
    {
        // ☆/★: show this account on the taskbar (Favorites mode).
        let fav = s.display().taskbar_favorites.contains(&acc.id);
        let id = acc.id;
        let st = state_for_star.clone();
        head = head.child(
            div()
                .id(SharedString::from(format!("fav-{id}")))
                .px(px(4.0))
                .text_sm()
                .cursor_pointer()
                .text_color(if fav { t.warn } else { t.faint })
                .hover(|d| d.text_color(t.text))
                .child(icon(Icon::Star, 15.0, if fav { t.warn } else { t.faint }))
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(if fav {
                        "Shown on the taskbar. Click to remove."
                    } else {
                        "Show on the taskbar (Favorites)"
                    })
                    .build(window, cx)
                })
                .on_click(move |_, _, cx| {
                    st.update(cx, |s, cx| {
                        s.toggle_taskbar_favorite(id);
                        cx.notify();
                    })
                }),
        );
    }

    let mut card = div()
        .flex()
        .flex_col()
        .gap(px(5.0))
        .mb_2()
        .px(px(14.0))
        .py(px(12.0))
        .rounded(px(12.0))
        .bg(t.surface)
        .border_1()
        .border_color(t.border)
        .when(stale, |d| d.opacity(0.7))
        .child(div().pb_1().child(head))
        .when_some(email, |d, e| {
            d.child(
                div()
                    .w_full()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(t.faint)
                    .child(e),
            )
        });

    match reading {
        None => {
            let msg = status
                .map(|s| s.message.clone())
                .unwrap_or_else(|| waiting_message(acc, s, now));
            card = card.child(div().text_xs().text_color(t.faint).child(msg));
        }
        Some(r) => {
            for w in r
                .windows
                .iter()
                .filter(|w| !acc.hidden_windows.contains(&w.key))
            {
                card = card.child(window_row(acc.id, w, stale, s, t));
            }
            if stale {
                let msg = status.map(|s| s.message.clone()).filter(|m| !m.is_empty());
                let when = format::ago(r.fetched_at, now);
                card = card.child(div().pt_1().text_xs().text_color(t.faint).child(match msg {
                    Some(m) => format!("Last reading {when} · {m}"),
                    None => format!("Last reading {when}"),
                }));
            }
        }
    }
    if let Some(actions) = recovery_row(acc, s, t, state_for_actions.clone(), now) {
        card = card.child(actions);
    }
    if let Some(footer) = billing_footer(acc, s, t) {
        card = card.child(footer);
    }
    card
}

/// The rocket button in a card header: opens this account's Launch dropdown, a card floating
/// over the popover content (one at a time). Drawn by us, not gpui-component's menu: it keeps the
/// popover's colours and type, has no submenus (one click per target) and is snapped inside the
/// small popover window, so it never opens off-side and never resizes the window (owner,
/// 2026-11-07).
fn launch_button(
    acc: &Account,
    s: &AppState,
    t: &Tokens,
    state: Entity<AppState>,
    on_manage: std::rc::Rc<dyn Fn(&mut App)>,
) -> impl IntoElement {
    let id = acc.id;
    let open = s.launch_open == Some(id);
    let toggle_state = state.clone();
    let button = div()
        .id(SharedString::from(format!("launch-{id}")))
        .flex()
        .items_center()
        .justify_center()
        .size(px(24.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .when(open, |d| d.bg(t.accent.opacity(0.18)))
        .hover(|d| d.bg(t.surface_hover))
        .child(icon(
            Icon::Rocket,
            14.0,
            if open { t.accent } else { t.muted },
        ))
        .when(!open, |d| {
            d.tooltip(|window, cx| {
                gpui_component::tooltip::Tooltip::new("Launch in a project").build(window, cx)
            })
        })
        .on_click(move |_, _, cx| {
            toggle_state.update(cx, |s, cx| {
                // A press outside the open dropdown (this button included) already closed it;
                // that same click must not open it again.
                let just_closed = s
                    .launch_closed
                    .is_some_and(|(cid, at)| cid == id && at.elapsed().as_millis() < 400);
                s.launch_open = if s.launch_open == Some(id) || just_closed {
                    None
                } else {
                    Some(id)
                };
                cx.notify();
            })
        });
    let mut wrap = div().relative().flex_none().child(button);
    if open {
        wrap = wrap.child(
            div().absolute().top(px(28.0)).right_0().child(
                gpui::deferred(
                    gpui::anchored()
                        .anchor(gpui::Anchor::TopRight)
                        .snap_to_window_with_margin(px(8.0))
                        .child(launch_panel(acc, s, t, state, on_manage)),
                )
                .with_priority(1),
            ),
        );
    }
    wrap
}

fn launch_panel(
    acc: &Account,
    s: &AppState,
    t: &Tokens,
    state: Entity<AppState>,
    on_manage: std::rc::Rc<dyn Fn(&mut App)>,
) -> gpui::Stateful<gpui::Div> {
    let id = acc.id;
    let env = s.launch_env(acc.provider);
    let has_profile = acc.profile_dir.is_some();
    let list = projects::visible(&s.settings.projects);
    let close_state = state.clone();
    let mut panel = div()
        .id(SharedString::from(format!("launch-menu-{id}")))
        .occlude()
        .w(px(272.0))
        .flex()
        .flex_col()
        .p(px(6.0))
        .rounded(px(10.0))
        .bg(t.surface)
        .border_1()
        .border_color(t.border)
        .shadow(vec![gpui::BoxShadow {
            color: gpui::hsla(0.0, 0.0, 0.0, if t.dark { 0.45 } else { 0.16 }),
            offset: gpui::point(px(0.0), px(8.0)),
            blur_radius: px(24.0),
            spread_radius: px(-4.0),
            inset: false,
        }])
        .on_mouse_down_out(move |_, _, cx| {
            close_state.update(cx, |s, cx| {
                if s.launch_open == Some(id) {
                    s.launch_open = None;
                    s.launch_closed = Some((id, std::time::Instant::now()));
                    cx.notify();
                }
            })
        });
    if list.is_empty() {
        panel = panel.child(
            div()
                .h(px(34.0))
                .px(px(8.0))
                .flex()
                .items_center()
                .text_xs()
                .text_color(t.muted)
                .child("No visible projects yet"),
        );
    }
    for project in list {
        let targets =
            launch::menu_targets(acc.provider, has_profile, project.directory.is_dir(), &env);
        let dir_text: SharedString = project.directory.to_string_lossy().into_owned().into();
        let mut buttons = div().flex().flex_none().items_center().gap(px(2.0));
        for target in targets {
            let ic = match target.target {
                launch::Target::WindowsTerminal => Icon::Terminal,
                launch::Target::PowerShell => Icon::SquareTerminal,
                launch::Target::VsCode => Icon::Code,
                launch::Target::Desktop => Icon::AppWindow,
            };
            let tip: SharedString = match target.disabled {
                Some(hint) => format!("{} · {hint}", target.label).into(),
                None => target.label.into(),
            };
            let enabled = target.disabled.is_none();
            let which = target.target;
            let project_id = project.id;
            let st = state.clone();
            buttons = buttons.child(
                div()
                    .id(SharedString::from(format!(
                        "launch-{id}-{project_id}-{which:?}"
                    )))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(26.0))
                    .rounded(px(6.0))
                    .child(icon(ic, 14.0, if enabled { t.text } else { t.faint }))
                    .when(enabled, |d| {
                        d.cursor_pointer().hover(|d| d.bg(t.surface_hover))
                    })
                    .when(!enabled, |d| d.opacity(0.45))
                    .tooltip(move |window, cx| {
                        gpui_component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
                    })
                    .on_click(move |_, _, cx| {
                        if enabled {
                            st.update(cx, |s, cx| {
                                s.launch_open = None;
                                s.launch_target(id, project_id, which, cx);
                            });
                        }
                    }),
            );
        }
        let tip = dir_text.clone();
        panel = panel.child(
            div()
                .h(px(34.0))
                .pl(px(8.0))
                .pr(px(2.0))
                .flex()
                .items_center()
                .gap_2()
                .rounded(px(7.0))
                .hover(|d| d.bg(t.surface_hover.opacity(0.5)))
                .child(icon(Icon::Folder, 14.0, t.muted))
                .child(
                    div()
                        .id(SharedString::from(format!(
                            "launch-label-{id}-{}",
                            project.id
                        )))
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .text_color(t.text)
                        .child(SharedString::from(project.label.clone()))
                        .tooltip(move |window, cx| {
                            gpui_component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
                        }),
                )
                .child(buttons),
        );
    }
    let st = state.clone();
    panel.child(
        div()
            .id(SharedString::from(format!("launch-manage-{id}")))
            .mt(px(4.0))
            .pt(px(4.0))
            .border_t_1()
            .border_color(t.border)
            .h(px(32.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap_2()
            .text_xs()
            .text_color(t.muted)
            .cursor_pointer()
            .hover(|d| d.text_color(t.text))
            .child(icon(Icon::Settings, 12.0, t.muted))
            .child("Manage projects…")
            .on_click(move |_, _, cx| {
                st.update(cx, |s, cx| {
                    s.launch_open = None;
                    cx.notify();
                });
                on_manage(cx);
            }),
    )
}

/// The usage state of the last check, this session's or the saved schedule's.
fn last_state(s: &AppState, id: uuid::Uuid) -> Option<UsageState> {
    s.status
        .get(&id)
        .map(|st| st.state)
        .or_else(|| s.settings.schedules.get(&id).map(|sc| sc.state))
}

/// Sign in / Retry applies: the check failed or is overdue, and it is not a benched hub login.
fn needs_attention(acc: &Account, s: &AppState) -> bool {
    if acc.hub.as_ref().is_some_and(|h| h.benched) && acc.profile_dir.is_none() {
        return false;
    }
    s.is_stale(acc.id) || last_state(s, acc.id).is_some_and(|st| st != UsageState::Ready)
}

fn retry_possible(acc: &Account, s: &AppState, now: chrono::DateTime<Utc>) -> bool {
    let cooling = s
        .settings
        .schedules
        .get(&acc.id)
        .cloned()
        .map(Schedule::from_cached)
        .is_some_and(|sc| sc.cooling_down(now));
    last_state(s, acc.id) != Some(UsageState::SignInRequired) && !cooling
}

/// Extra height a card needs for its sign-in/retry row and billing line.
fn card_extras(acc: &Account, s: &AppState) -> f32 {
    let mut h = 0.0;
    if needs_attention(acc, s) {
        h += 30.0;
    }
    if s.account_notes.contains_key(&acc.id) {
        h += 16.0;
    }
    if s.settings.billing.contains_key(&acc.id) {
        h += 24.0;
    }
    h
}

fn small_button(
    id: SharedString,
    label: &str,
    ic: Icon,
    primary: bool,
    t: &Tokens,
) -> gpui::Stateful<gpui::Div> {
    let fg = if primary { gpui::white() } else { t.text };
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .px_2()
        .py(px(3.0))
        .rounded(px(7.0))
        .text_xs()
        .cursor_pointer()
        .when(primary, |d| {
            d.bg(t.accent).text_color(fg).hover(|s| s.opacity(0.9))
        })
        .when(!primary, |d| {
            d.bg(t.surface_hover)
                .text_color(fg)
                .hover(|s| s.bg(t.border))
        })
        .child(icon(ic, 12.0, if primary { fg } else { t.muted }))
        .child(SharedString::from(label.to_string()))
}

/// Sign in (login not accepted, accounts with a profile) and Retry (this account only) with the
/// last action's note. Hub-only accounts cannot sign in from SubHub.
fn recovery_row(
    acc: &Account,
    s: &AppState,
    t: &Tokens,
    state: Entity<AppState>,
    now: chrono::DateTime<Utc>,
) -> Option<gpui::Div> {
    let note = s.account_notes.get(&acc.id).cloned();
    if !needs_attention(acc, s) && note.is_none() {
        return None;
    }
    let id = acc.id;
    let mut row = div().flex().items_center().gap_2().pt_1();
    if needs_attention(acc, s) {
        let sign_in_state = last_state(s, id) == Some(UsageState::SignInRequired);
        if sign_in_state {
            if acc.profile_dir.is_some() {
                let st = state.clone();
                let busy = s.signing_in.contains(&id);
                row = row.child(
                    small_button(
                        SharedString::from(format!("signin-{id}")),
                        if busy { "Signing in…" } else { "Sign in" },
                        Icon::LogIn,
                        true,
                        t,
                    )
                    .on_click(move |_, _, cx| st.update(cx, |s, cx| s.sign_in(id, cx))),
                );
            } else {
                row = row.child(
                    div()
                        .text_xs()
                        .text_color(t.muted)
                        .child("Sign in through the hub"),
                );
            }
        }
        if retry_possible(acc, s, now) {
            let st = state.clone();
            row = row.child(
                small_button(
                    SharedString::from(format!("retry-{id}")),
                    if s.refreshing { "Checking…" } else { "Retry" },
                    Icon::Refresh,
                    false,
                    t,
                )
                .on_click(move |_, _, cx| st.update(cx, |s, cx| s.refresh_account(id, cx))),
            );
        }
    }
    Some(row.when_some(note, |d, n| {
        d.child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(t.faint)
                .child(n),
        )
    }))
}

/// "Renews Dec 1, 2026 · in 25 days · estimated" with an Open billing link; only for accounts
/// that track a date. Accent when it is within the reminder window.
fn billing_footer(acc: &Account, s: &AppState, t: &Tokens) -> Option<gpui::Div> {
    let b = s.settings.billing.get(&acc.id)?;
    let today = AppState::today();
    let days = billing::days_until(b, today);
    let soon = b.reminders && (0..=7).contains(&days);
    let url = billing::billing_url(acc.provider);
    let id = acc.id;
    Some(
        div()
            .flex()
            .items_center()
            .gap_2()
            .pt_1()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(if soon { t.accent } else { t.faint })
                    .child(billing::description(b, today)),
            )
            .child(
                small_button(
                    SharedString::from(format!("billing-{id}")),
                    "Open billing",
                    Icon::ExternalLink,
                    false,
                    t,
                )
                .tooltip(|window, cx| {
                    gpui_component::tooltip::Tooltip::new(BILLING_HINT).build(window, cx)
                })
                .on_click(move |_, _, cx| cx.open_url(url)),
            ),
    )
}

pub const BILLING_HINT: &str = "Opens your browser's signed-in account, which may differ from this CLI profile. Apple/Google purchases are managed in that store; team plans may need an admin.";

fn window_row(
    account: uuid::Uuid,
    w: &AllowanceWindow,
    stale: bool,
    s: &AppState,
    t: &Tokens,
) -> impl IntoElement {
    let d = s.display();
    let remaining = w.remaining_percent();
    let color = if stale { t.faint } else { t.level(remaining) };
    // The countdown ("2d 4h"); the exact local time is in its tooltip.
    let exact = w.resets_at.map(format::reset_exact);
    let value_color = if stale || color == t.accent {
        t.text
    } else {
        color
    };
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .w(px(104.0))
                .text_sm()
                .text_color(t.muted)
                .truncate()
                .child(SharedString::from(w.label.clone())),
        )
        .child(
            div().flex_1().h(px(6.0)).rounded_full().bg(t.track).child(
                div()
                    .h_full()
                    .rounded_full()
                    .bg(color)
                    .w(gpui::relative(format::gauge_fraction(w, d).clamp(0.0, 1.0))),
            ),
        )
        .child(
            div()
                .w(px(76.0))
                .flex()
                .justify_end()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(value_color)
                .child(format::usage(w, d)),
        )
        .child(
            div()
                .id(SharedString::from(format!("reset-{account}-{}", w.key)))
                .w(px(58.0))
                .flex()
                .justify_end()
                .text_xs()
                .text_color(t.faint)
                .child(format::reset(w, Utc::now()).unwrap_or_else(|| "—".into()))
                .when_some(exact, |d, tip| {
                    d.tooltip(move |window, cx| {
                        gpui_component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
                    })
                }),
        )
}

/// No check in this session yet: explain the saved schedule (cooldown, sign-in, backoff).
fn waiting_message(acc: &Account, s: &AppState, now: chrono::DateTime<Utc>) -> String {
    if acc.hub.as_ref().is_some_and(|h| h.benched) {
        return "Benched in the hub".into();
    }
    let Some(sched) = s.settings.schedules.get(&acc.id) else {
        return "Not read yet".into();
    };
    let when = if sched.next_attempt > now {
        format!(
            "next try in {}",
            format::duration_until(sched.next_attempt, now)
        )
    } else {
        "checking soon".into()
    };
    match sched.state {
        UsageState::RateLimited => format!("Usage checks cooling down · {when}"),
        UsageState::SignInRequired => format!("Login not accepted last time · {when}"),
        UsageState::CliMissing => format!("CLI not found last time · {when}"),
        UsageState::Unavailable => format!("Last check failed · {when}"),
        UsageState::Ready => "Not read yet".into(),
    }
}

fn empty_state(s: &AppState, t: &Tokens) -> impl IntoElement {
    let hint = match s.hub {
        HubStatus::NoKey => {
            "Add the hub key in Settings → Hub, or add accounts in Settings → Accounts."
        }
        HubStatus::Unreachable => "The hub is not reachable. Add accounts in Settings → Accounts.",
        _ => "Accounts appear here once the hub lists them or you add them in Settings.",
    };
    div()
        .py_6()
        .px_2()
        .flex()
        .flex_col()
        .items_center()
        .gap_1()
        .child(div().text_sm().child("No accounts yet"))
        .child(
            div()
                .text_xs()
                .text_color(t.muted)
                .text_center()
                .child(hint),
        )
}
