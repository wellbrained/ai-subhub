//! Statistics window, modelled on T3 Code's usage board: range and Cost / Tokens / Limits at the
//! top, then the total with the provider split next to a daily chart, totals, cost (or tokens) by
//! type and a breakdown by model or day. Data: `subhub_core::stats` (local session logs).
use std::rc::Rc;

use chrono::{Datelike, Utc};
use gpui::{
    Context, Entity, FontWeight, Hsla, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Window, div, prelude::*, px, relative,
};
use gpui_component::chart::AreaChart;
use gpui_component::scroll::ScrollableElement;
use subhub_core::model::ProviderKind;
use subhub_core::stats::{ModelRow, Point, Range, Summary, summarize};

use crate::icons::{Icon, icon, provider_color};
use crate::state::AppState;
use crate::theme::{self, Tokens};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Cost,
    Tokens,
    Limits,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Breakdown {
    Model,
    Day,
}

pub struct StatsWindow {
    state: Entity<AppState>,
    range: Range,
    tab: Tab,
    by: Breakdown,
    /// Sized to its content once, when the first data arrived.
    sized: bool,
    /// When the window was first sized; for a short while after, overflow found by the first
    /// layout grows the window to fit exactly.
    fitted_at: Option<std::time::Instant>,
    scroll: gpui::ScrollHandle,
    /// Last client height asked for by the exact fit.
    fit_target: f32,
    /// Result of the last "Copy as image" and when (true = copied).
    copied: Option<(std::time::Instant, bool)>,
    /// The summary for (range, state.stats_rev); recomputed only when either changes.
    summary: Option<((Range, u64), Summary)>,
    _sub: Subscription,
}

impl StatsWindow {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe(&state, |_, _, cx| cx.notify());
        // The Limits countdowns tick: re-render twice a minute while that tab is open. Ends with
        // the window.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(30))
                    .await;
                let alive = this.update(cx, |this, cx| {
                    if this.tab == Tab::Limits {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            state,
            range: Range::Quarter,
            // Tokens first: the popover already shows the limits (owner, 2026-11-07).
            tab: Tab::Tokens,
            by: Breakdown::Model,
            sized: false,
            fitted_at: None,
            scroll: gpui::ScrollHandle::new(),
            fit_target: 0.0,
            copied: None,
            summary: None,
            _sub: sub,
        }
    }
}

// ---------- formatting ----------

/// 3 significant digits with K / M / B, like the usage board ("15.9B", "4.30B", "912M", "10B").
fn tokens_text(n: f64) -> String {
    let (v, suffix) = if n >= 1e9 {
        (n / 1e9, "B")
    } else if n >= 1e6 {
        (n / 1e6, "M")
    } else if n >= 1e3 {
        (n / 1e3, "K")
    } else {
        return format!("{}", n.round() as u64);
    };
    let s = if v >= 100.0 {
        format!("{v:.0}")
    } else if v >= 10.0 {
        format!("{v:.1}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    } else {
        format!("{v:.2}")
    };
    format!("{s}{suffix}")
}

/// "$4,475.42".
fn money(v: f64) -> String {
    let cents = (v * 100.0).round() as i64;
    let (whole, frac) = (cents / 100, cents % 100);
    let digits = whole.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("${out}.{frac:02}")
}

fn percent(part: f64, whole: f64) -> String {
    if whole <= 0.0 {
        "—".into()
    } else {
        format!("{:.1}%", part / whole * 100.0)
    }
}

fn provider_label(p: ProviderKind) -> &'static str {
    match p {
        ProviderKind::Claude => "Claude Code",
        ProviderKind::Codex => "Codex",
    }
}

// ---------- building blocks ----------

fn seg<T: PartialEq + Clone + 'static>(
    id: &'static str,
    options: &[(T, &'static str)],
    current: &T,
    t: &Tokens,
    on_pick: impl Fn(T, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let on_pick = Rc::new(on_pick);
    let mut bar = div()
        .flex()
        .flex_none()
        .h(px(28.0))
        .gap(px(2.0))
        .p(px(2.0))
        .rounded(px(8.0))
        .bg(t.surface);
    for (i, (value, label)) in options.iter().enumerate() {
        let active = value == current;
        let value = value.clone();
        let on_pick = on_pick.clone();
        bar = bar.child(
            div()
                .id((id, i))
                .h(px(24.0))
                .flex()
                .items_center()
                .px(px(10.0))
                .rounded(px(6.0))
                .text_xs()
                .cursor_pointer()
                .when(active, |d| {
                    d.bg(t.accent.opacity(if t.dark { 0.2 } else { 0.14 }))
                        .text_color(t.accent_text())
                        .font_weight(FontWeight::MEDIUM)
                })
                .when(!active, |d| {
                    d.text_color(t.muted).hover(|s| s.text_color(t.text))
                })
                .child(*label)
                .on_click(move |_, _, cx| on_pick(value.clone(), cx)),
        );
    }
    bar
}

fn icon_btn(id: &'static str, glyph: Icon, t: &Tokens) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(28.0))
        .rounded(px(8.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.bg(t.accent.opacity(0.16)))
        .child(icon(glyph, 15.0, t.muted))
}

fn heading(text: &str) -> gpui::Div {
    div()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .child(SharedString::from(text.to_string()))
}

/// Colours of the four token kinds in the "by type" bar: input, cache read, cache write, output.
fn kind_colors(t: &Tokens) -> [Hsla; 4] {
    let c = |h: u32| -> Hsla { gpui::rgb(h).into() };
    if t.dark {
        [c(0x8a8a8f), c(0x55555a), c(0xb8b5b0), c(0xf4efe6)]
    } else {
        [c(0x9ca3af), c(0xd4d4d8), c(0x6b7280), c(0x18181b)]
    }
}

impl Render for StatsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let d = self.state.read(cx).display().clone();
        let t = Tokens::new(theme::is_dark(d.theme, cx), &d.accent);
        let me = cx.entity();

        let (rev, scanning, has_scan, error) = {
            let s = self.state.read(cx);
            (
                s.stats_rev,
                s.stats_scanning,
                s.stats.is_some(),
                s.stats_error.clone(),
            )
        };
        let on_limits = self.tab == Tab::Limits;
        if has_scan {
            let key = (self.range, rev);
            if self.summary.as_ref().map(|(k, _)| *k) != Some(key) {
                let s = self.state.read(cx);
                let scan = s.stats.clone().unwrap_or_default();
                let summary = summarize(
                    &scan,
                    self.range,
                    chrono::Local::now().naive_local(),
                    &s.prices,
                );
                self.summary = Some((key, summary));
            }
        }

        // The scrollbar (always visible, see theme::apply_component_theme) overlays the
        // reserved right gutter.
        // Re-assert the mode in case something reset it (read first: setting refreshes windows).
        if gpui_component::Theme::global(cx).scrollbar_mode
            != gpui_component::scroll::ScrollbarMode::Always
        {
            gpui_component::Theme::set_scrollbar_mode(
                gpui_component::scroll::ScrollbarMode::Always,
                cx,
            );
        }
        if !self.sized
            && let Some((_, summary)) = &self.summary
        {
            self.sized = true;
            self.fitted_at = Some(std::time::Instant::now());
            fit_window(window, cx, content_height(summary.models.len()));
            // Re-check after the first layouts: the estimate may be a few px off.
            for ms in [250u64, 700, 1400] {
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(ms))
                        .await;
                    let _ = this.update(cx, |_, cx| cx.notify());
                })
                .detach();
            }
        } else if self.sized
            && !on_limits
            && self
                .fitted_at
                .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(2))
        {
            // The content column's own height (first child of the scroll area) is the height the
            // window needs, whether or not it overflows now.
            if let Some(item) = self.scroll.bounds_for_item(0) {
                let target = TOP_BAR + f32::from(item.size.height);
                if item.size.height > gpui::px(0.0) && (self.fit_target - target).abs() > 0.5 {
                    self.fit_target = target;
                    fit_window(window, cx, target);
                }
            }
        }

        let range_options = Range::ALL.map(|r| (r, r.label()));
        let range_bar = seg("range", &range_options, &self.range, &t, {
            let me = me.clone();
            move |r, cx| {
                me.update(cx, |this, cx| {
                    this.range = r;
                    cx.notify();
                })
            }
        });
        let tab_bar = seg(
            "tab",
            &[
                (Tab::Cost, "Cost"),
                (Tab::Tokens, "Tokens"),
                (Tab::Limits, "Limits"),
            ],
            &self.tab,
            &t,
            {
                let me = me.clone();
                move |v, cx| {
                    me.update(cx, |this, cx| {
                        this.tab = v;
                        cx.notify();
                    })
                }
            },
        );
        let state = self.state.clone();
        let copy_note = self
            .copied
            .filter(|(at, _)| at.elapsed() < std::time::Duration::from_secs(2))
            .map(|(_, ok)| {
                if ok {
                    "Copied to clipboard"
                } else {
                    "Could not copy"
                }
            });
        let top = div()
            .flex()
            .items_center()
            .gap_3()
            .h(px(52.0))
            .flex_none()
            .pt(px(12.0))
            .pb(px(8.0))
            .px(px(16.0))
            .border_b_1()
            .border_color(t.border)
            .child(heading("Usage").text_base())
            .when(scanning && has_scan, |d| {
                d.child(div().text_xs().text_color(t.muted).child("Scanning…"))
            })
            .when_some(copy_note, |d, note| {
                d.child(div().text_xs().text_color(t.muted).child(note))
            })
            .child(div().flex_1())
            .when(!on_limits, |d| d.child(range_bar))
            .child(tab_bar)
            .child(
                icon_btn("stats-copy", Icon::Copy, &t)
                    .tooltip(|window, cx| {
                        gpui_component::tooltip::Tooltip::new("Copy as image").build(window, cx)
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        let ok = crate::window_style::copy_client_to_clipboard(window).is_some();
                        this.copied = Some((std::time::Instant::now(), ok));
                        cx.notify();
                        cx.spawn(async move |this, cx| {
                            cx.background_executor()
                                .timer(std::time::Duration::from_millis(2100))
                                .await;
                            let _ = this.update(cx, |_, cx| cx.notify());
                        })
                        .detach();
                    })),
            )
            .child(
                icon_btn("stats-refresh", Icon::Refresh, &t)
                    .tooltip(move |window, cx| {
                        gpui_component::tooltip::Tooltip::new(if on_limits {
                            "Refresh accounts"
                        } else {
                            "Rescan logs"
                        })
                        .build(window, cx)
                    })
                    .on_click(move |_, _, cx| {
                        state.update(cx, |s, cx| {
                            if on_limits {
                                s.refresh(true, cx)
                            } else {
                                s.start_scan(cx)
                            }
                        })
                    }),
            );

        let price_note = {
            let book = &self.state.read(cx).prices;
            match book.fetched_at {
                Some(at) => format!(
                    "API estimate: tokens priced with LiteLLM's public price list, updated {} ({}). Models without a public price are Unpriced.",
                    at.with_timezone(&chrono::Local).format("%b %-d, %H:%M"),
                    book.source
                ),
                None => "API estimate: tokens priced with the built-in price table (LiteLLM list not fetched yet). Models without a public price are Unpriced.".to_string(),
            }
        };
        let body: gpui::AnyElement = if on_limits {
            crate::limits_board::board(self.state.read(cx), Utc::now(), &t)
        } else if let Some((_, summary)) = self.summary.as_ref().filter(|_| has_scan) {
            self.board(summary, &t, me, price_note).into_any_element()
        } else {
            let text = match (&error, scanning) {
                (Some(e), false) => e.clone(),
                _ => "Scanning…".to_string(),
            };
            div()
                .py_10()
                .text_sm()
                .text_color(t.muted)
                .child(text)
                .into_any_element()
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .text_color(t.text)
            .child(top)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("stats-scroll")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .child(
                                // Right padding = scrollbar gutter (the bar overlays it), so
                                // the layout never shifts when the bar appears.
                                div()
                                    .w_full()
                                    .max_w(px(WIDTH))
                                    .mx_auto()
                                    .pl(px(16.0))
                                    .pr(px(16.0 + GUTTER))
                                    .pt(px(14.0))
                                    .pb(px(16.0))
                                    .flex()
                                    .flex_col()
                                    .gap(px(14.0))
                                    .child(body),
                            ),
                    )
                    .vertical_scrollbar(&self.scroll),
            )
    }
}

/// Window client width: the 1000 px board plus the scrollbar gutter.
pub const WIDTH: f32 = 1000.0 + GUTTER;
/// Height of the top bar including its border (the scroll area sits below it).
const TOP_BAR: f32 = 53.0;
/// Right gutter reserved for the overlay scrollbar.
const GUTTER: f32 = 12.0;

/// Sizes the window to a client height (clamped to the work area, centred): GPUI resizes first so
/// its viewport follows, then the window is moved.
fn fit_window(window: &mut Window, cx: &mut gpui::App, height: f32) {
    window.defer(cx, move |window, cx| {
        let Some(((w, h), (x, y))) = crate::window_style::fit_plan(window, WIDTH, height) else {
            return;
        };
        window.resize(gpui::size(px(w), px(h)));
        window.defer(cx, move |window, _| {
            crate::window_style::move_outer(window, x, y);
            window.refresh();
        });
    });
}

/// Client height that shows every section and `rows` breakdown rows (at most 12; more scroll):
/// top bar, padding, chart block, totals, by-type, breakdown header and rows, section gaps.
fn content_height(rows: usize) -> f32 {
    558.0 + 34.0 * rows.clamp(1, 12) as f32
}

impl StatsWindow {
    fn board(
        &self,
        s: &Summary,
        t: &Tokens,
        me: Entity<Self>,
        price_note: String,
    ) -> impl IntoElement {
        let cost_tab = self.tab == Tab::Cost;
        let total_cost = s.cost.total();
        let total_tokens = s.tokens.processed() as f64;

        // Left: big total, sessions, provider rows.
        let big = if cost_tab {
            money(total_cost)
        } else {
            tokens_text(total_tokens)
        };
        let sessions = format!("{} sessions", s.sessions);
        let mut left = div()
            .flex()
            .flex_col()
            .gap_1()
            .w(px(280.0))
            .flex_none()
            .child(
                div()
                    .text_size(px(28.0))
                    .font_weight(FontWeight::BOLD)
                    .child(SharedString::from(big)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .text_color(t.muted)
                    .child(SharedString::from(sessions))
                    .when(cost_tab, |d| {
                        d.child("· API estimate").child(
                            div()
                                .id("price-info")
                                .rounded_full()
                                .hover(|s| s.bg(t.accent.opacity(0.18)))
                                .child(icon(Icon::Info, 12.0, t.faint))
                                .tooltip(move |window, cx| {
                                    gpui_component::tooltip::Tooltip::new(price_note.clone())
                                        .build(window, cx)
                                }),
                        )
                    }),
            );
        for p in &s.providers {
            let color = provider_color(p.provider, t.text);
            let (value, detail) = if cost_tab {
                (
                    money(p.cost),
                    format!(
                        "{} of cost · {} tokens",
                        percent(p.cost, total_cost),
                        tokens_text(p.tokens.processed() as f64)
                    ),
                )
            } else {
                (
                    tokens_text(p.tokens.processed() as f64),
                    format!(
                        "{} of tokens · {}",
                        percent(p.tokens.processed() as f64, total_tokens),
                        money(p.cost)
                    ),
                )
            };
            left = left.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .pt(px(8.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().size(px(8.0)).rounded_full().bg(color))
                            .child(icon(Icon::Provider(p.provider), 15.0, color))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(provider_label(p.provider)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(t.muted)
                                    .child(SharedString::from(format!("{} sessions", p.sessions))),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(SharedString::from(value)),
                            ),
                    )
                    .child(
                        div()
                            .pl(px(16.0))
                            .text_xs()
                            .text_color(t.muted)
                            .child(SharedString::from(detail)),
                    ),
            );
        }

        let top = div().flex().gap(px(24.0)).child(left).child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_2()
                .child(heading(if cost_tab {
                    "Daily cost"
                } else {
                    "Daily processed tokens"
                }))
                .child(
                    div()
                        .h(px(190.0))
                        .w_full()
                        .child(chart(&s.points, cost_tab, s.range, t)),
                ),
        );

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(top)
            .child(totals(s, t))
            .child(by_type(s, cost_tab, t))
            .child(self.breakdown(s, cost_tab, t, me))
    }

    fn breakdown(
        &self,
        s: &Summary,
        cost_tab: bool,
        t: &Tokens,
        me: Entity<Self>,
    ) -> impl IntoElement {
        let total_cost = s.cost.total();
        let total_tokens = s.tokens.processed() as f64;
        let by = self.by;
        let toggle = seg(
            "by",
            &[(Breakdown::Model, "Model"), (Breakdown::Day, "Day")],
            &by,
            t,
            move |v, cx| {
                me.update(cx, |this, cx| {
                    this.by = v;
                    cx.notify();
                })
            },
        );

        let cell = |w: f32| div().w(px(w)).flex_none().text_right();
        let header = div()
            .flex()
            .items_center()
            .px_2()
            .h(px(26.0))
            .text_xs()
            .text_color(t.muted)
            .border_b_1()
            .border_color(t.border)
            .child(div().w(px(32.0)).flex_none().child("#"))
            .child(div().flex_1().child(match by {
                Breakdown::Model => "Model",
                Breakdown::Day => "Day",
            }))
            .child(cell(110.0).child("Cost"))
            .child(cell(80.0).child("Share"))
            .child(cell(90.0).child("Tokens"));

        let mut rows = div().flex().flex_col();
        match by {
            Breakdown::Model => {
                let mut models: Vec<&ModelRow> = s.models.iter().collect();
                if !cost_tab {
                    models.sort_by_key(|m| std::cmp::Reverse(m.tokens.processed()));
                }
                let value = |m: &ModelRow| {
                    if cost_tab {
                        m.cost.unwrap_or(0.0)
                    } else {
                        m.tokens.processed() as f64
                    }
                };
                let max = models.iter().map(|m| value(m)).fold(0.0, f64::max);
                for (i, m) in models.iter().enumerate() {
                    let color = provider_color(m.provider, t.text);
                    let share = if cost_tab {
                        m.cost.map_or("—".to_string(), |c| percent(c, total_cost))
                    } else {
                        percent(m.tokens.processed() as f64, total_tokens)
                    };
                    let frac = if max > 0.0 { value(m) / max } else { 0.0 };
                    rows = rows.child(
                        div()
                            .flex()
                            .items_center()
                            .px_2()
                            .h(px(34.0))
                            .border_b_1()
                            .border_color(t.border)
                            .text_sm()
                            .child(
                                div()
                                    .w(px(32.0))
                                    .flex_none()
                                    .text_xs()
                                    .text_color(t.muted)
                                    .child(SharedString::from((i + 1).to_string())),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.0))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(icon(Icon::Provider(m.provider), 14.0, color))
                                            .child(SharedString::from(m.model.clone())),
                                    )
                                    .child(
                                        div()
                                            .h(px(2.0))
                                            .w(px((190.0 * frac).max(4.0) as f32))
                                            .rounded_full()
                                            .bg(color),
                                    ),
                            )
                            .child(match m.cost {
                                Some(c) => cell(110.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(SharedString::from(money(c))),
                                None => cell(110.0).text_color(t.muted).child("Unpriced"),
                            })
                            .child(
                                cell(80.0)
                                    .text_color(t.muted)
                                    .child(SharedString::from(share)),
                            )
                            .child(cell(90.0).child(SharedString::from(tokens_text(
                                m.tokens.processed() as f64,
                            )))),
                    );
                }
            }
            Breakdown::Day => {
                for (i, d) in s.days.iter().enumerate() {
                    let share = if cost_tab {
                        percent(d.cost, total_cost)
                    } else {
                        percent(d.tokens as f64, total_tokens)
                    };
                    let label = format!(
                        "{}, {} {}",
                        d.day.format("%a"),
                        d.day.format("%b"),
                        d.day.day()
                    );
                    rows = rows.child(
                        div()
                            .flex()
                            .items_center()
                            .px_2()
                            .h(px(34.0))
                            .border_b_1()
                            .border_color(t.border)
                            .text_sm()
                            .child(
                                div()
                                    .w(px(32.0))
                                    .flex_none()
                                    .text_xs()
                                    .text_color(t.muted)
                                    .child(SharedString::from((i + 1).to_string())),
                            )
                            .child(div().flex_1().child(SharedString::from(label)))
                            .child(
                                cell(110.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(SharedString::from(money(d.cost))),
                            )
                            .child(
                                cell(80.0)
                                    .text_color(t.muted)
                                    .child(SharedString::from(share)),
                            )
                            .child(
                                cell(90.0).child(SharedString::from(tokens_text(d.tokens as f64))),
                            ),
                    );
                }
            }
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(heading("Breakdown"))
                    .child(div().flex_1())
                    .child(toggle),
            )
            .child(header)
            .child(rows)
    }
}

/// Daily (or hourly) area chart, one line per provider: Claude orange, Codex in the text colour.
fn chart(points: &[Point], cost_tab: bool, range: Range, t: &Tokens) -> impl IntoElement {
    let claude = provider_color(ProviderKind::Claude, t.text);
    let codex = provider_color(ProviderKind::Codex, t.text);
    let value = move |p: &Point, slot: usize| -> f64 {
        if cost_tab {
            p.cost[slot]
        } else {
            p.tokens[slot] as f64
        }
    };
    AreaChart::<Point, String, f64>::new(points.to_vec())
        .id(("stats-chart", range.days() as usize))
        .x(|p| p.label.clone())
        .y(move |p| value(p, 0))
        .stroke(claude)
        .fill(claude.opacity(0.14))
        .name("Claude Code")
        .y(move |p| value(p, 1))
        .stroke(codex)
        .fill(codex.opacity(0.08))
        .name("Codex")
        .natural()
        .natural()
        .y_axis(true)
        .y_tick_count(5)
        .y_tick_format(move |v| {
            if cost_tab {
                format!("${v:.0}")
            } else if v <= 0.0 {
                "0".to_string()
            } else {
                tokens_text(v)
            }
        })
        .tooltip_value(move |_, _, v| {
            if cost_tab {
                money(v).into()
            } else {
                tokens_text(v).into()
            }
        })
        .x_tick_count(3)
}

fn totals(s: &Summary, t: &Tokens) -> impl IntoElement {
    let item = |label: &str, value: String| {
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .min_w(px(150.0))
            .child(
                div()
                    .text_xs()
                    .text_color(t.muted)
                    .child(SharedString::from(label.to_string())),
            )
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(SharedString::from(value)),
            )
    };
    let n = |v: u64| tokens_text(v as f64);
    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(heading("Totals"))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_x_8()
                .gap_y_3()
                .child(item("Processed tokens", n(s.tokens.processed())))
                .child(item("Cached input", n(s.tokens.cache_read)))
                .child(item("Uncached input", n(s.tokens.input)))
                .child(item("Output", n(s.tokens.output)))
                .child(item("Cache savings", money(s.cache_savings))),
        )
}

/// "Cost by type" / "Tokens by type": a stacked bar with a legend.
fn by_type(s: &Summary, cost_tab: bool, t: &Tokens) -> impl IntoElement {
    let parts: [(&str, f64, String); 4] = if cost_tab {
        [
            ("Input", s.cost.input, money(s.cost.input)),
            ("Cache read", s.cost.cache_read, money(s.cost.cache_read)),
            ("Cache write", s.cost.cache_write, money(s.cost.cache_write)),
            ("Output", s.cost.output, money(s.cost.output)),
        ]
    } else {
        let n = |v: u64| (v as f64, tokens_text(v as f64));
        let (a, b, c, d) = (
            n(s.tokens.input),
            n(s.tokens.cache_read),
            n(s.tokens.cache_write),
            n(s.tokens.output),
        );
        [
            ("Input", a.0, a.1),
            ("Cache read", b.0, b.1),
            ("Cache write", c.0, c.1),
            ("Output", d.0, d.1),
        ]
    };
    let colors = kind_colors(t);
    let total: f64 = parts.iter().map(|p| p.1).sum();
    let mut bar = div()
        .flex()
        .gap(px(1.0))
        .h(px(8.0))
        .w_full()
        .max_w(px(480.0))
        .rounded_full()
        .overflow_hidden()
        .bg(t.track);
    let mut legend = div().flex().flex_wrap().gap_x_4().gap_y_1();
    for (i, (label, v, text)) in parts.iter().enumerate() {
        if total > 0.0 && *v > 0.0 {
            bar = bar.child(div().h_full().w(relative((v / total) as f32)).bg(colors[i]));
        }
        legend = legend.child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .text_xs()
                .child(div().size(px(8.0)).rounded(px(2.0)).bg(colors[i]))
                .child(div().text_color(t.muted).child(*label))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(SharedString::from(text.clone())),
                ),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(heading(if cost_tab {
            "Cost by type"
        } else {
            "Tokens by type"
        }))
        .child(bar)
        .child(legend)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_formats_match_the_usage_board() {
        assert_eq!(tokens_text(15.9e9), "15.9B");
        assert_eq!(tokens_text(4.30e9), "4.30B");
        assert_eq!(tokens_text(912e6), "912M");
        assert_eq!(tokens_text(49.3e6), "49.3M");
        assert_eq!(tokens_text(10.0e9), "10B");
        assert_eq!(tokens_text(1.46e9), "1.46B");
        assert_eq!(tokens_text(0.0), "0");
        assert_eq!(money(4475.42), "$4,475.42");
        assert_eq!(money(48206.47), "$48,206.47");
        assert_eq!(money(0.5), "$0.50");
        assert_eq!(percent(1.0, 4.0), "25.0%");
        assert_eq!(percent(1.0, 0.0), "—");
    }
}
