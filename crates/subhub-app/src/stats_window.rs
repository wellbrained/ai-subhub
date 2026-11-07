//! Statistics window (owner-approved redesign, 2026-11-07): a merged title bar with the
//! `Usage | Limits` tabs, the range switch, Copy as image and Rescan; below it the Usage view
//! (tiles, "By type", then a Models / Providers / Days breakdown with the daily chart) or the
//! live Limits board. Data: `subhub_core::stats` (local session logs).
use std::time::{Duration, Instant};

use chrono::Utc;
use gpui::{
    AnyElement, Context, Entity, FontFeatures, FontWeight, Hsla, IntoElement, ParentElement,
    Render, SharedString, Styled, Subscription, Window, div, prelude::*, px, relative,
};
use gpui_component::scroll::ScrollableElement;
use gpui_component::tooltip::Tooltip;
use subhub_core::model::ProviderKind;
use subhub_core::settings::{StatsBreakdown, StatsMetric, StatsSettings};
use subhub_core::stats::{ModelRow, Point, Range, Summary, summarize};

use crate::icons::{Icon, icon, series_color};
use crate::state::AppState;
use crate::theme::{self, Tokens};
use crate::title_bar::{self, icon_button, seg};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Usage,
    Limits,
}

/// The one-time fit of the window height to the Usage content (Models view).
#[derive(Clone, Copy)]
enum Fit {
    /// No data yet.
    Pending,
    /// Sized from an estimate, now following the measured content height until it is stable.
    Measuring {
        since: Instant,
        last: f32,
        stable: u8,
    },
    /// Never resized by the app again (owner rule: switching tabs or views keeps the height).
    Done,
}

pub struct StatsWindow {
    state: Entity<AppState>,
    range: Range,
    tab: Tab,
    fit: Fit,
    scroll: gpui::ScrollHandle,
    /// The Days table's own scroll area (7 rows visible).
    days_scroll: gpui::ScrollHandle,
    /// Result of the last "Copy as image" and when (true = copied).
    copied: Option<(Instant, bool)>,
    /// The price table under the tiles is shown (ⓘ on "API estimate").
    prices_open: bool,
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
                    .timer(Duration::from_secs(30))
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
            // Opens on Usage: the popover already shows the limits (owner, 2026-11-07).
            tab: Tab::Usage,
            fit: Fit::Pending,
            scroll: gpui::ScrollHandle::new(),
            days_scroll: gpui::ScrollHandle::new(),
            copied: None,
            prices_open: false,
            summary: None,
            _sub: sub,
        }
    }

    /// Re-renders after each delay (ms): the fit follows layouts that happen after a resize.
    fn renotify(&self, cx: &mut Context<Self>, delays: &[u64]) {
        for ms in delays {
            let ms = *ms;
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(ms))
                    .await;
                let _ = this.update(cx, |_, cx| cx.notify());
            })
            .detach();
        }
    }

    /// Sizes the window once to the Usage content: first from an estimate, then to the measured
    /// height of the content column until two layouts agree (at most 2 s). While this runs the
    /// Models view is shown whatever the saved choice, so the height never depends on it.
    fn drive_fit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, summary)) = &self.summary else {
            return;
        };
        if self.tab != Tab::Usage {
            return;
        }
        match self.fit {
            Fit::Done => {}
            Fit::Pending => {
                self.fit = Fit::Measuring {
                    since: Instant::now(),
                    last: 0.0,
                    stable: 0,
                };
                fit_window(window, cx, estimate_height(summary.models.len()));
                self.renotify(cx, &[150, 400, 900, 1600]);
            }
            Fit::Measuring {
                since,
                last,
                stable,
            } => {
                let timed_out = since.elapsed() >= Duration::from_secs(2);
                let h = self
                    .scroll
                    .bounds_for_item(0)
                    .map_or(0.0, |b| f32::from(b.size.height));
                if h <= 0.0 && !timed_out {
                    return;
                }
                let target = title_bar::HEIGHT + h;
                if h > 0.0 && (target - last).abs() > 0.5 && !timed_out {
                    self.fit = Fit::Measuring {
                        since,
                        last: target,
                        stable: 0,
                    };
                    fit_window(window, cx, target);
                    self.renotify(cx, &[150, 400]);
                } else if stable >= 1 || timed_out {
                    self.fit = Fit::Done;
                    self.renotify(cx, &[30]);
                } else {
                    self.fit = Fit::Measuring {
                        since,
                        last,
                        stable: stable + 1,
                    };
                    self.renotify(cx, &[120]);
                }
            }
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
    format!("${}.{frac:02}", thousands(whole))
}

/// Whole dollars from $10,000 up ("$48,705"), otherwise with cents: tile values stay short.
fn money_short(v: f64) -> String {
    if v >= 10_000.0 {
        format!("${}", thousands(v.round() as i64))
    } else {
        money(v)
    }
}

fn thousands(n: i64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "63%", "<1%", "—" for nothing.
fn share_text(frac: f64) -> String {
    if frac <= 0.0 || !frac.is_finite() {
        "—".into()
    } else if frac < 0.01 {
        "<1%".into()
    } else {
        format!("{:.0}%", frac * 100.0)
    }
}

fn provider_label(p: ProviderKind) -> &'static str {
    match p {
        ProviderKind::Claude => "Claude Code",
        ProviderKind::Codex => "Codex",
    }
}

/// Stack order of the series: Claude at the bottom of a bar, Codex on top.
const SERIES: [ProviderKind; 2] = [ProviderKind::Claude, ProviderKind::Codex];

fn slot(p: ProviderKind) -> usize {
    match p {
        ProviderKind::Claude => 0,
        ProviderKind::Codex => 1,
    }
}

/// The metric value of a chart point for one provider slot.
fn point_value(p: &Point, slot: usize, metric: StatsMetric) -> f64 {
    match metric {
        StatsMetric::Tokens => p.work[slot] as f64,
        StatsMetric::Cost => p.cost[slot],
    }
}

fn value_text(v: f64, metric: StatsMetric) -> String {
    match metric {
        StatsMetric::Tokens => tokens_text(v),
        StatsMetric::Cost => money(v),
    }
}

/// An axis step and the top of a 3-step axis for a maximum: 1, 1.5, 2, 2.5, 3, 4, 5, 6, 8 x 10^k.
fn nice_axis(max: f64) -> (f64, f64) {
    if max <= 0.0 || !max.is_finite() {
        return (3.0, 1.0);
    }
    let raw = max / 3.0;
    let mag = 10f64.powf(raw.log10().floor());
    let n = raw / mag;
    let mult = [1.0, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0]
        .into_iter()
        .find(|c| *c >= n - 1e-9)
        .unwrap_or(10.0);
    let step = mult * mag;
    (step * 3.0, step)
}

/// Axis label: "1.5M", "250K", "$15".
fn axis_text(v: f64, metric: StatsMetric) -> String {
    if v <= 0.0 {
        return match metric {
            StatsMetric::Tokens => "0".into(),
            StatsMetric::Cost => "$0".into(),
        };
    }
    match metric {
        StatsMetric::Cost => {
            if v >= 1.0 {
                format!("${}", thousands(v.round() as i64))
            } else {
                format!("${v:.2}")
            }
        }
        StatsMetric::Tokens => {
            let (x, suffix) = if v >= 1e9 {
                (v / 1e9, "B")
            } else if v >= 1e6 {
                (v / 1e6, "M")
            } else if v >= 1e3 {
                (v / 1e3, "K")
            } else {
                (v, "")
            };
            let s = format!("{x:.2}");
            let s = if s.contains('.') {
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            } else {
                s
            };
            format!("{s}{suffix}")
        }
    }
}

/// Tabular figures, so columns of numbers line up.
fn tabular() -> FontFeatures {
    FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)]))
}

// ---------- building blocks ----------

fn card(t: &Tokens) -> gpui::Div {
    div()
        .bg(t.surface)
        .border_1()
        .border_color(t.border)
        .rounded(px(10.0))
}

fn dot(color: Hsla) -> gpui::Div {
    div().size(px(8.0)).flex_none().rounded_full().bg(color)
}

fn heading(text: &str) -> gpui::Div {
    div()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .child(SharedString::from(text.to_string()))
}

/// A small info icon with a tooltip.
fn info(id: &'static str, text: String, t: &Tokens) -> AnyElement {
    div()
        .id(id)
        .flex_none()
        .rounded_full()
        .hover(|s| s.bg(t.accent.opacity(0.18)))
        .child(icon(Icon::Info, 12.0, t.faint))
        .tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx))
        .into_any_element()
}

/// A bar of `frac` (0..1) of the row in `color` on the neutral track.
fn share_bar(frac: f64, color: Hsla, t: &Tokens) -> gpui::Div {
    let frac = if frac > 0.0 {
        frac.clamp(0.015, 1.0) as f32
    } else {
        0.0
    };
    div()
        .relative()
        .flex_1()
        .min_w(px(40.0))
        .h(px(8.0))
        .rounded(px(4.0))
        .bg(t.track)
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .h_full()
                .w(relative(frac))
                .rounded(px(4.0))
                .bg(color),
        )
}

/// A right-aligned table cell of a fixed width.
fn cell(w: f32) -> gpui::Div {
    div().w(px(w)).flex_none().text_right()
}

fn header_row(t: &Tokens) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .h(px(28.0))
        .px(px(4.0))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.muted)
        .border_b_1()
        .border_color(t.border)
}

fn body_row(t: &Tokens, first: bool) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .h(px(32.0))
        .px(px(4.0))
        .text_sm()
        .when(!first, |d| {
            d.border_t_1().border_color(t.border.opacity(0.6))
        })
}

/// What the share bars and the daily chart measure.
fn total_value(s: &Summary, metric: StatsMetric) -> f64 {
    match metric {
        StatsMetric::Tokens => s.work() as f64,
        StatsMetric::Cost => s.cost.total(),
    }
}

fn model_value(m: &ModelRow, metric: StatsMetric) -> f64 {
    match metric {
        StatsMetric::Tokens => m.tokens.work() as f64,
        StatsMetric::Cost => m.cost.unwrap_or(0.0),
    }
}

fn model_cost_cell(m: &ModelRow, t: &Tokens) -> gpui::Div {
    match m.cost {
        Some(c) => cell(100.0)
            .font_weight(FontWeight::SEMIBOLD)
            .child(SharedString::from(money(c))),
        None => cell(100.0).text_color(t.muted).child("Unpriced"),
    }
}

/// Models of one provider by the metric, largest first.
fn models_of(s: &Summary, p: ProviderKind, metric: StatsMetric) -> Vec<&ModelRow> {
    let mut v: Vec<&ModelRow> = s.models.iter().filter(|m| m.provider == p).collect();
    v.sort_by(|a, b| {
        model_value(b, metric)
            .total_cmp(&model_value(a, metric))
            .then(b.tokens.work().cmp(&a.tokens.work()))
            .then(a.model.cmp(&b.model))
    });
    v
}

// ---------- the window ----------

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
        let prefs: StatsSettings = self.state.read(cx).settings.stats.clone();
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

        self.drive_fit(window, cx);
        // While the height is being fitted the Models view is shown whatever was saved.
        let view = if matches!(self.fit, Fit::Done) {
            prefs.breakdown
        } else {
            StatsBreakdown::Models
        };

        // ----- title bar -----
        let tabs = seg(
            "tab",
            &[(Tab::Usage, "Usage"), (Tab::Limits, "Limits")],
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
        let status = |text: &'static str| {
            div()
                .text_xs()
                .text_color(t.muted)
                .child(text)
                .into_any_element()
        };
        let copy_note = self
            .copied
            .filter(|(at, _)| at.elapsed() < Duration::from_secs(2))
            .map(|(_, ok)| {
                if ok {
                    "Copied to clipboard"
                } else {
                    "Could not copy"
                }
            });
        let mut trailing: Vec<AnyElement> = vec![];
        if scanning && has_scan {
            trailing.push(status("Scanning…"));
        }
        if let Some(note) = copy_note {
            trailing.push(status(note));
        }
        if !on_limits {
            let ranges = Range::ALL.map(|r| (r, r.short()));
            trailing.push(seg("range", &ranges, &self.range, &t, {
                let me = me.clone();
                move |r, cx| {
                    me.update(cx, |this, cx| {
                        this.range = r;
                        cx.notify();
                    })
                }
            }));
        }
        trailing.push(
            icon_button("stats-copy", Icon::Copy, &t)
                .tooltip(|window, cx| Tooltip::new("Copy as image").build(window, cx))
                .on_click(cx.listener(|this, _, window, cx| {
                    let ok = crate::window_style::copy_client_to_clipboard(window).is_some();
                    this.copied = Some((Instant::now(), ok));
                    cx.notify();
                    this.renotify(cx, &[2100]);
                }))
                .into_any_element(),
        );
        let state = self.state.clone();
        trailing.push(
            icon_button("stats-refresh", Icon::Refresh, &t)
                .tooltip(move |window, cx| {
                    Tooltip::new(if on_limits {
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
                })
                .into_any_element(),
        );
        let bar = title_bar::bar("stats-bar", "Usage", &t, window, Some(tabs), trailing);

        // ----- body -----
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
        let body: AnyElement = if on_limits {
            crate::limits_board::board(self.state.read(cx), Utc::now(), &t)
        } else if let Some((_, summary)) = self.summary.as_ref().filter(|_| has_scan) {
            let state = self.state.clone();
            // The price table opens under the tiles from the ⓘ on "API estimate"; never while
            // the window height is still being fitted (that height is the Usage content alone).
            let toggle: std::rc::Rc<dyn Fn(&mut gpui::App)> = {
                let me = me.clone();
                std::rc::Rc::new(move |cx: &mut gpui::App| {
                    me.update(cx, |this, cx| {
                        this.prices_open = !this.prices_open;
                        cx.notify();
                    })
                })
            };
            let open = self.prices_open && matches!(self.fit, Fit::Done);
            let prices = PriceInfo {
                note: price_note,
                open,
                card: open
                    .then(|| prices_card(self.state.read(cx), &t, state.clone(), toggle.clone())),
                toggle,
            };
            usage(summary, &prefs, view, &t, state, prices, &self.days_scroll)
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
            .font_features(tabular())
            .child(bar)
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
                                    .pl(px(PAD))
                                    .pr(px(PAD + GUTTER))
                                    .pt(px(16.0))
                                    .pb(px(20.0))
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
/// Side padding of the content (the board is 960 px wide at the default width).
const PAD: f32 = 20.0;
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

/// First guess of the client height for the Models view with `models` rows, before the content
/// has been measured: title bar, padding, tiles, by-type card, breakdown header, chart card,
/// model table with two group rows.
fn estimate_height(models: usize) -> f32 {
    46.0 + 16.0
        + 20.0
        + 112.0
        + 14.0
        + 190.0
        + 14.0
        + 30.0
        + 14.0
        + 262.0
        + 14.0
        + 76.0
        + 28.0
        + 32.0 * (models.clamp(1, 14) as f32 + 2.0)
}

// ---------- Usage view ----------

fn usage(
    s: &Summary,
    prefs: &StatsSettings,
    view: StatsBreakdown,
    t: &Tokens,
    state: Entity<AppState>,
    prices: PriceInfo,
    days_scroll: &gpui::ScrollHandle,
) -> AnyElement {
    let metric = prefs.metric;
    let set_metric = {
        let state = state.clone();
        move |m: StatsMetric, cx: &mut gpui::App| {
            state.update(cx, |st, cx| {
                st.settings.stats.metric = m;
                st.save();
                cx.notify();
            })
        }
    };
    let metric_options = [(StatsMetric::Tokens, "Tokens"), (StatsMetric::Cost, "Cost")];

    let mut header = div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .child(heading("Breakdown"))
        .child(div().flex_1());
    if view == StatsBreakdown::Providers {
        // The daily chart card (with this switch) is not shown in this view.
        header = header.child(seg(
            "metric-b",
            &metric_options,
            &metric,
            t,
            set_metric.clone(),
        ));
    }
    header = header.child(seg(
        "breakdown",
        &[
            (StatsBreakdown::Models, "Models"),
            (StatsBreakdown::Providers, "Providers"),
            (StatsBreakdown::Days, "Days"),
        ],
        &view,
        t,
        {
            let state = state.clone();
            move |v, cx| {
                state.update(cx, |st, cx| {
                    st.settings.stats.breakdown = v;
                    st.save();
                    cx.notify();
                })
            }
        },
    ));

    let chart = || {
        chart_card(
            s,
            metric,
            t,
            seg("metric", &metric_options, &metric, t, set_metric.clone()),
        )
    };
    let breakdown: AnyElement = match view {
        StatsBreakdown::Models => div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(chart())
            .child(model_table(s, metric, t))
            .into_any_element(),
        StatsBreakdown::Providers => providers_view(s, metric, t),
        StatsBreakdown::Days => div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(chart())
            .child(day_table(s, t, days_scroll))
            .into_any_element(),
    };

    div()
        .flex()
        .flex_col()
        .gap(px(14.0))
        .child(tiles(s, t, &prices))
        .children(prices.card)
        .child(by_type(s, t))
        .child(header)
        .child(breakdown)
        .into_any_element()
}

fn tile_label(text: String, extra: Option<AnyElement>, t: &Tokens) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(5.0))
        .text_xs()
        .text_color(t.muted)
        .child(SharedString::from(text))
        .children(extra)
}

fn tile_value(text: String) -> gpui::Div {
    div()
        .text_size(px(20.0))
        .font_weight(FontWeight::SEMIBOLD)
        .child(SharedString::from(text))
}

fn tile_sub(text: String, t: &Tokens) -> gpui::Div {
    div()
        .text_xs()
        .text_color(t.muted)
        .child(SharedString::from(text))
}

fn small_tile(t: &Tokens) -> gpui::Div {
    card(t)
        .w(px(158.0))
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(3.0))
        .px(px(14.0))
        .py(px(14.0))
}

/// The tiles row: work tokens with the provider split, cache, API estimate, cache saved and the
/// month run-rate.
fn tiles(s: &Summary, t: &Tokens, prices: &PriceInfo) -> AnyElement {
    let work = s.work();
    let provider_work = |p: ProviderKind| {
        s.providers
            .iter()
            .find(|x| x.provider == p)
            .map_or(0, |x| x.tokens.work())
    };

    let mut split = div()
        .flex()
        .w_full()
        .h(px(6.0))
        .gap(px(2.0))
        .rounded(px(3.0))
        .overflow_hidden()
        .bg(t.track);
    let mut legend = div().flex().flex_wrap().gap_x_4().gap_y_1().text_xs();
    for p in SERIES {
        let w = provider_work(p);
        let color = series_color(p, t.dark);
        if work > 0 && w > 0 {
            split = split.child(div().h_full().w(relative(w as f32 / work as f32)).bg(color));
        }
        legend = legend.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(dot(color))
                .child(div().text_color(t.muted).child(provider_label(p)))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(SharedString::from(tokens_text(w as f64))),
                ),
        );
    }
    let big = card(t)
        .flex_1()
        .min_w(px(220.0))
        .flex()
        .flex_col()
        .gap(px(3.0))
        .px(px(16.0))
        .py(px(14.0))
        .child(tile_label(
            format!("Work tokens · {}", s.range.label()),
            Some(info(
                "work-info",
                "Work tokens are input + output + cache writes: what the models actually had to process. Cache reads are counted separately.".into(),
                t,
            )),
            t,
        ))
        .child(
            div()
                .text_size(px(30.0))
                .font_weight(FontWeight::BOLD)
                .child(SharedString::from(tokens_text(work as f64))),
        )
        .child(div().my(px(6.0)).child(split))
        .child(legend);

    let hit = s.cache_hit_rate.map_or("—".to_string(), |r| {
        format!("{:.0}% of all tokens read", r * 100.0)
    });
    let cache = small_tile(t)
        .child(tile_label("Served from cache".into(), None, t))
        .child(tile_value(tokens_text(s.tokens.cache_read as f64)))
        .child(tile_sub(hit, t));

    let unpriced = match s.unpriced_models {
        0 => "at API prices".to_string(),
        1 => "1 model unpriced".to_string(),
        n => format!("{n} models unpriced"),
    };
    let api = small_tile(t)
        .child(tile_label(
            "API estimate".into(),
            Some(price_info(prices, t)),
            t,
        ))
        .child(tile_value(money_short(s.cost.total())))
        .child(tile_sub(unpriced, t));

    let saved = small_tile(t)
        .child(tile_label("Cache saved".into(), None, t))
        .child(tile_value(money_short(s.cache_savings)))
        .child(tile_sub("vs. uncached input price".into(), t));

    let month = s.month.month.format("%b").to_string();
    let (rate, rate_sub) = match s.month.projected {
        Some(p) => (
            format!("~{}", money_short(p)),
            format!("{} so far in {month}", money_short(s.month.cost_so_far)),
        ),
        None => ("—".to_string(), "needs a full day of data".to_string()),
    };
    let run_rate = small_tile(t)
        .child(tile_label(
            format!("On track · {month}"),
            Some(info(
                "rate-info",
                "This month's API estimate so far, extrapolated linearly to the end of the month. Independent of the range.".into(),
                t,
            )),
            t,
        ))
        .child(tile_value(rate))
        .child(tile_sub(rate_sub, t));

    div()
        .flex()
        .gap(px(12.0))
        .child(big)
        .child(cache)
        .child(api)
        .child(saved)
        .child(run_rate)
        .into_any_element()
}

/// The ⓘ on "API estimate": the price source in its tooltip; a click shows or hides the price
/// table under the tiles.
fn price_info(p: &PriceInfo, t: &Tokens) -> AnyElement {
    let tip = format!(
        "{} Click to {} the price table.",
        p.note,
        if p.open { "hide" } else { "show" }
    );
    let toggle = p.toggle.clone();
    div()
        .id("price-info")
        .flex_none()
        .rounded_full()
        .cursor_pointer()
        .hover(|s| s.bg(t.accent.opacity(0.18)))
        .active(|s| s.bg(t.accent.opacity(0.3)))
        .when(p.open, |d| d.bg(t.accent.opacity(0.18)))
        .child(icon(
            Icon::Info,
            12.0,
            if p.open { t.accent_text() } else { t.faint },
        ))
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .on_click(move |_, _, cx| toggle(cx))
        .into_any_element()
}

/// What the "API estimate" tile needs for its ⓘ and the price table under the tiles.
struct PriceInfo {
    /// The price source, for the tooltip.
    note: String,
    open: bool,
    toggle: std::rc::Rc<dyn Fn(&mut gpui::App)>,
    /// The table, when open.
    card: Option<AnyElement>,
}

/// "$15", "$0.30", "$3.75": per million tokens, at least two decimals below $10.
fn price_text(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0');
    let s = if s.split('.').nth(1).is_some_and(|d| d.len() >= 2) {
        s.to_string()
    } else {
        format!("{v:.2}")
    };
    format!("${s}")
}

/// Read-only price table (moved here from Settings → Statistics, owner 2026-11-07): source and
/// age, Update now, and per publisher and family the models seen in the logs (plus a family's
/// current model when none was seen) with input / output / cache write / cache read per 1M
/// tokens. The prices themselves stay in `AppState` (`prices`, `update_prices`).
fn prices_card(
    s: &AppState,
    t: &Tokens,
    state: Entity<AppState>,
    close: std::rc::Rc<dyn Fn(&mut gpui::App)>,
) -> AnyElement {
    use std::collections::BTreeSet;
    let book = &s.prices;
    let updating = s.prices_updating;
    let seen: BTreeSet<String> = s
        .stats
        .as_ref()
        .map(|sc| sc.buckets.iter().map(|b| b.model.clone()).collect())
        .unwrap_or_default();
    let now = Utc::now();
    let status = if updating {
        "Updating prices…".to_string()
    } else {
        match (book.fetched_at, &book.error) {
            (Some(at), None) => format!("From LiteLLM · updated {}", crate::format::ago(at, now)),
            (Some(at), Some(e)) => format!(
                "From LiteLLM · updated {} · last update failed: {e}",
                crate::format::ago(at, now)
            ),
            (None, Some(e)) => format!("Built-in prices · LiteLLM not reachable: {e}"),
            (None, None) => "Built-in prices · not updated yet".to_string(),
        }
    };

    // Groups: publisher, then family.
    const CURRENT: [&str; 5] = [
        "claude-opus-5-5",
        "claude-sonnet-5-5",
        "claude-haiku-5-5",
        "claude-fable-5-1",
        "gpt-6.1-sol",
    ];
    let publisher = |m: &str| -> usize {
        if m.starts_with("claude") {
            0
        } else if m.starts_with("gpt")
            || m.contains("codex")
            || m.starts_with("o1")
            || m.starts_with("o3")
            || m.starts_with("o4")
        {
            1
        } else {
            2
        }
    };
    let family = |m: &str| -> usize {
        match publisher(m) {
            0 => ["opus", "sonnet", "haiku", "fable"]
                .iter()
                .position(|f| m.contains(f))
                .unwrap_or(4),
            1 if m.contains("codex") => 1,
            1 if m.starts_with("gpt") => 0,
            1 => 2,
            _ => 0,
        }
    };
    const FAMILIES: [&[&str]; 3] = [
        &["Opus", "Sonnet", "Haiku", "Fable", "Other"],
        &["GPT", "Codex", "Other"],
        &["Models"],
    ];
    const PUBLISHERS: [&str; 3] = ["Anthropic", "OpenAI", "Other"];
    let mut groups: [Vec<BTreeSet<String>>; 3] = [
        vec![Default::default(); 5],
        vec![Default::default(); 3],
        vec![Default::default(); 1],
    ];
    for m in &seen {
        groups[publisher(m)][family(m)].insert(m.clone());
    }
    for m in CURRENT {
        let (p, f) = (publisher(m), family(m));
        if groups[p][f].is_empty() {
            groups[p][f].insert(m.to_string());
        }
    }

    const W: f32 = 84.0;
    const SRC: f32 = 108.0;
    let badge = |text: &'static str, color: Hsla| {
        div()
            .px(px(6.0))
            .py(px(1.0))
            .rounded(px(5.0))
            .bg(t.bg)
            .text_xs()
            .text_color(color)
            .child(text)
    };
    let mut table = div().flex().flex_col();
    for (p, name) in PUBLISHERS.iter().enumerate() {
        if groups[p].iter().all(|g| g.is_empty()) {
            continue;
        }
        table = table.child(
            header_row(t)
                .mt(px(if p == 0 { 0.0 } else { 10.0 }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(SharedString::from(name.to_string())),
                )
                .child(cell(W).child("Input"))
                .child(cell(W).child("Output"))
                .child(cell(W).child("Cache write"))
                .child(cell(W).child("Cache read"))
                .child(cell(SRC).child("Source")),
        );
        for (fi, models) in groups[p].iter().enumerate() {
            if models.is_empty() {
                continue;
            }
            table = table.child(
                div()
                    .pt(px(8.0))
                    .pb(px(2.0))
                    .px(px(4.0))
                    .text_xs()
                    .text_color(t.muted)
                    .child(FAMILIES[p][fi]),
            );
            for (i, m) in models.iter().enumerate() {
                let mut line = body_row(t, i == 0).child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(SharedString::from(m.clone())),
                );
                let (src, color) = match book.lookup(m) {
                    Some((price, src)) => {
                        for v in [
                            price.input,
                            price.output,
                            price.cache_write,
                            price.cache_read,
                        ] {
                            line = line.child(cell(W).child(SharedString::from(price_text(v))));
                        }
                        match src {
                            subhub_core::stats::PriceSource::LiteLlm => {
                                ("LiteLLM", t.accent_text())
                            }
                            subhub_core::stats::PriceSource::BuiltIn => ("built-in", t.muted),
                        }
                    }
                    None => {
                        line = line.child(cell(W * 4.0 + 36.0).text_color(t.muted).child("—"));
                        ("No public price", t.warn)
                    }
                };
                table = table.child(
                    line.child(
                        div()
                            .w(px(SRC))
                            .flex_none()
                            .flex()
                            .justify_end()
                            .child(badge(src, color)),
                    ),
                );
            }
        }
    }

    let update = div()
        .id("prices-update")
        .flex_none()
        .px(px(10.0))
        .py(px(4.0))
        .rounded(px(7.0))
        .text_xs()
        .bg(t.bg)
        .border_1()
        .border_color(t.border)
        .when(!updating, |d| {
            d.cursor_pointer()
                .hover(|s| s.bg(t.surface_hover))
                .on_click(move |_, _, cx| state.update(cx, |s, cx| s.update_prices(true, cx)))
        })
        .when(updating, |d| d.opacity(0.5))
        .child(if updating {
            "Updating…"
        } else {
            "Update now"
        });
    let close_button = div()
        .id("prices-close")
        .size(px(24.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(|s| s.bg(t.surface_hover))
        .child(icon(Icon::Close, 13.0, t.muted))
        .tooltip(|window, cx| Tooltip::new("Hide prices").build(window, cx))
        .on_click(move |_, _, cx| close(cx));

    card(t)
        .flex()
        .flex_col()
        .gap(px(8.0))
        .px(px(16.0))
        .py(px(14.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(heading("Prices"))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(t.muted)
                        .child(SharedString::from(status)),
                )
                .child(update)
                .child(close_button),
        )
        .child(div().text_xs().text_color(t.muted).child(
            "USD per million tokens. Read-only: updated daily from LiteLLM's public price list, the built-in table fills any gap. Shown: the models in your logs.",
        ))
        .child(table)
        .into_any_element()
}

/// "By type": tokens, cost and the share of the cost for the four kinds of token.
fn by_type(s: &Summary, t: &Tokens) -> AnyElement {
    let rows = [
        ("Input", s.tokens.input, s.cost.input),
        ("Output", s.tokens.output, s.cost.output),
        ("Cache write", s.tokens.cache_write, s.cost.cache_write),
        ("Cache read", s.tokens.cache_read, s.cost.cache_read),
    ];
    let total = s.cost.total();
    let mut table = div().flex().flex_col().child(
        header_row(t)
            .child(div().w(px(150.0)).flex_none().child("Type"))
            .child(cell(110.0).child("Tokens"))
            .child(cell(110.0).child("Cost"))
            .child(div().flex_1().pl(px(16.0)).child("Share of cost")),
    );
    for (i, (name, tokens, cost)) in rows.iter().enumerate() {
        let frac = if total > 0.0 { cost / total } else { 0.0 };
        table = table.child(
            body_row(t, i == 0)
                .child(div().w(px(150.0)).flex_none().child(*name))
                .child(cell(110.0).child(SharedString::from(tokens_text(*tokens as f64))))
                .child(
                    cell(110.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(SharedString::from(money(*cost))),
                )
                .child(
                    div()
                        .flex_1()
                        .pl(px(16.0))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .child(share_bar(frac, t.faint, t))
                        .child(
                            div()
                                .w(px(40.0))
                                .flex_none()
                                .text_right()
                                .text_xs()
                                .text_color(t.muted)
                                .child(SharedString::from(share_text(frac))),
                        ),
                ),
        );
    }
    card(t)
        .px(px(16.0))
        .py(px(12.0))
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(heading("By type"))
        .child(table)
        .into_any_element()
}

// ---------- daily chart ----------

/// Tooltip content of one chart column.
#[derive(Clone)]
struct DayTip {
    title: String,
    total: String,
    rows: [(Hsla, &'static str, String); 2],
}

impl DayTip {
    fn new(p: &Point, metric: StatsMetric, dark: bool) -> Self {
        let v = |slot: usize| point_value(p, slot, metric);
        let row = |pr: ProviderKind| {
            (
                series_color(pr, dark),
                provider_label(pr),
                value_text(v(slot(pr)), metric),
            )
        };
        Self {
            title: p.title.clone(),
            total: value_text(v(0) + v(1), metric),
            rows: [row(ProviderKind::Claude), row(ProviderKind::Codex)],
        }
    }

    fn view(&self) -> gpui::Div {
        let mut d = div().flex().flex_col().gap(px(3.0)).child(
            div()
                .flex()
                .gap(px(10.0))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(SharedString::from(self.title.clone())),
                )
                .child(div().child(SharedString::from(self.total.clone()))),
        );
        for (color, name, value) in &self.rows {
            d = d.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(dot(*color))
                    .child(div().child(*name))
                    .child(div().flex_1())
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(SharedString::from(value.clone())),
                    ),
            );
        }
        d
    }
}

/// The card around the daily chart: title, what it measures (and since when), the Tokens / Cost
/// switch, the chart and its legend.
fn chart_card(s: &Summary, metric: StatsMetric, t: &Tokens, switch: AnyElement) -> AnyElement {
    let unit = if s.range == Range::Day { "hour" } else { "day" };
    let (title, what) = match metric {
        StatsMetric::Tokens => (format!("Work per {unit}"), "input + output + cache writes"),
        StatsMetric::Cost => (format!("Cost per {unit}"), "API estimate"),
    };
    let since = match (s.trimmed, s.first_day) {
        (true, Some(d)) => format!(" · since first use {}", d.format("%b %-d")),
        _ => String::new(),
    };
    let mut legend = div()
        .flex()
        .gap_x_4()
        .mt(px(8.0))
        .text_xs()
        .text_color(t.muted);
    for p in SERIES {
        legend = legend.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(dot(series_color(p, t.dark)))
                .child(provider_label(p)),
        );
    }
    card(t)
        .px(px(16.0))
        .py(px(14.0))
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .mb(px(10.0))
                .child(heading(&title))
                .child(
                    div()
                        .text_xs()
                        .text_color(t.muted)
                        .child(SharedString::from(format!("{what}{since}"))),
                )
                .child(div().flex_1())
                .child(switch),
        )
        .child(daily_chart(&s.points, metric, t))
        .child(legend)
        .into_any_element()
}

const PLOT_H: f32 = 160.0;
const AXIS_W: f32 = 44.0;

/// Stacked bars per day (Claude at the bottom, Codex on top, a 2 px gap between the segments),
/// three recessive gridlines, short date labels and a tooltip per day. Plain elements: the
/// hover highlight and tooltip come for free.
fn daily_chart(points: &[Point], metric: StatsMetric, t: &Tokens) -> AnyElement {
    let totals: Vec<f64> = points
        .iter()
        .map(|p| point_value(p, 0, metric) + point_value(p, 1, metric))
        .collect();
    let max = totals.iter().copied().fold(0.0, f64::max);
    if max <= 0.0 {
        return div()
            .h(px(PLOT_H))
            .flex()
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(t.muted)
            .child("No usage in this range")
            .into_any_element();
    }
    let (top, step) = nice_axis(max);

    let mut axis = div().relative().w(px(AXIS_W)).h(px(PLOT_H)).flex_none();
    let mut plot = div().relative().flex_1().h(px(PLOT_H));
    for g in 0..=3 {
        let y = PLOT_H * (1.0 - g as f32 / 3.0);
        axis = axis.child(
            div()
                .absolute()
                .top(px(y - 7.0))
                .right_0()
                .text_size(px(10.5))
                .text_color(t.muted)
                .child(SharedString::from(axis_text(step * g as f64, metric))),
        );
        plot = plot.child(
            div()
                .absolute()
                .left_0()
                .w_full()
                .top(px((y - 0.5).clamp(0.0, PLOT_H - 1.0)))
                .h(px(1.0))
                .bg(if g == 0 {
                    t.border
                } else {
                    t.border.opacity(0.55)
                }),
        );
    }

    let hover_bg = t.text.opacity(0.06);
    let mut bars = div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .items_end();
    let h_of = |v: f64| -> f32 {
        if v <= 0.0 {
            0.0
        } else {
            ((v / top) as f32 * PLOT_H).max(1.5)
        }
    };
    for (i, p) in points.iter().enumerate() {
        let (claude, codex) = (
            h_of(point_value(p, 0, metric)),
            h_of(point_value(p, 1, metric)),
        );
        let tip = DayTip::new(p, metric, t.dark);
        let mut stack = div().flex().flex_col().w_full().max_w(px(30.0));
        if codex > 0.0 {
            stack = stack.child(
                div()
                    .w_full()
                    .h(px(codex))
                    .flex_none()
                    .rounded(px(1.5))
                    .bg(series_color(ProviderKind::Codex, t.dark)),
            );
        }
        if codex > 0.0 && claude > 0.0 {
            stack = stack.child(div().w_full().h(px(2.0)).flex_none());
        }
        if claude > 0.0 {
            stack = stack.child(
                div()
                    .w_full()
                    .h(px(claude))
                    .flex_none()
                    .rounded(px(1.5))
                    .bg(series_color(ProviderKind::Claude, t.dark)),
            );
        }
        bars = bars.child(
            div()
                .id(("day-col", i))
                .flex_1()
                .min_w_0()
                .h_full()
                .px(px(1.0))
                .flex()
                .flex_col()
                .justify_end()
                .items_center()
                .hover(move |s| s.bg(hover_bg))
                .tooltip(move |window, cx| {
                    let tip = tip.clone();
                    Tooltip::element(move |_, _| tip.view()).build(window, cx)
                })
                .child(stack),
        );
    }
    plot = plot.child(bars);

    // Date labels under the plot, every n-th column, centred on it.
    let n = points.len();
    let every = if n <= 8 { 1 } else { n.div_ceil(7) };
    let mut labels = div().flex().ml(px(AXIS_W + 8.0)).mt(px(4.0)).h(px(16.0));
    for (i, p) in points.iter().enumerate() {
        let mut c = div().relative().flex_1().min_w_0().h_full();
        if i % every == 0 {
            c = c.child(
                div()
                    .absolute()
                    .top_0()
                    .left(relative(0.5))
                    .ml(px(-30.0))
                    .w(px(60.0))
                    .flex()
                    .justify_center()
                    .whitespace_nowrap()
                    .text_size(px(10.5))
                    .text_color(t.muted)
                    .child(SharedString::from(p.label.clone())),
            );
        }
        labels = labels.child(c);
    }

    div()
        .flex()
        .flex_col()
        .child(div().flex().gap(px(8.0)).child(axis).child(plot))
        .child(labels)
        .into_any_element()
}

// ---------- Models ----------

/// Model table grouped by provider: provider rows carry the subtotals; each model has its
/// provider dot, a share bar (of work tokens or cost, following the chart switch), work tokens,
/// tokens including cache and cost.
fn model_table(s: &Summary, metric: StatsMetric, t: &Tokens) -> AnyElement {
    let total = total_value(s, metric);
    let share_title = match metric {
        StatsMetric::Tokens => "Share of work",
        StatsMetric::Cost => "Share of cost",
    };
    let mut groups: Vec<(ProviderKind, Vec<&ModelRow>)> = SERIES
        .into_iter()
        .map(|p| (p, models_of(s, p, metric)))
        .filter(|(_, rows)| !rows.is_empty())
        .collect();
    let group_value = |rows: &[&ModelRow]| rows.iter().map(|m| model_value(m, metric)).sum::<f64>();
    groups.sort_by(|a, b| group_value(&b.1).total_cmp(&group_value(&a.1)));

    let mut table = div().flex().flex_col().child(
        header_row(t)
            .child(div().flex_1().min_w_0().child("Model"))
            .child(div().w(px(220.0)).flex_none().child(share_title))
            .child(cell(90.0).child("Work"))
            .child(cell(100.0).child("Incl. cache"))
            .child(cell(100.0).child("Cost")),
    );
    if groups.is_empty() {
        table = table.child(
            div()
                .py(px(20.0))
                .text_sm()
                .text_color(t.muted)
                .child("No usage in this range"),
        );
    }
    for (gi, (p, rows)) in groups.iter().enumerate() {
        let color = series_color(*p, t.dark);
        let work: u64 = rows.iter().map(|m| m.tokens.work()).sum();
        let incl: u64 = rows.iter().map(|m| m.tokens.processed()).sum();
        let cost: f64 = rows.iter().filter_map(|m| m.cost).sum();
        let group_share = if total > 0.0 {
            group_value(rows) / total
        } else {
            0.0
        };
        table = table.child(
            body_row(t, gi == 0)
                .mt(px(if gi == 0 { 0.0 } else { 6.0 }))
                .font_weight(FontWeight::SEMIBOLD)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(dot(color))
                        .child(provider_label(*p)),
                )
                .child(
                    div()
                        .w(px(220.0))
                        .flex_none()
                        .text_xs()
                        .font_weight(FontWeight::NORMAL)
                        .text_color(t.muted)
                        .child(SharedString::from(share_text(group_share))),
                )
                .child(cell(90.0).child(SharedString::from(tokens_text(work as f64))))
                .child(
                    cell(100.0)
                        .font_weight(FontWeight::NORMAL)
                        .text_color(t.muted)
                        .child(SharedString::from(tokens_text(incl as f64))),
                )
                .child(cell(100.0).child(SharedString::from(money(cost)))),
        );
        for m in rows {
            let frac = if total > 0.0 {
                model_value(m, metric) / total
            } else {
                0.0
            };
            table = table.child(
                body_row(t, false)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .pl(px(2.0))
                            .child(dot(color))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .child(SharedString::from(m.model.clone())),
                            ),
                    )
                    .child(
                        div()
                            .w(px(220.0))
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .child(share_bar(frac, color, t))
                            .child(
                                div()
                                    .w(px(36.0))
                                    .flex_none()
                                    .text_right()
                                    .text_xs()
                                    .text_color(t.muted)
                                    .child(SharedString::from(share_text(frac))),
                            ),
                    )
                    .child(
                        cell(90.0).child(SharedString::from(tokens_text(m.tokens.work() as f64))),
                    )
                    .child(
                        cell(100.0)
                            .text_color(t.muted)
                            .child(SharedString::from(tokens_text(m.tokens.processed() as f64))),
                    )
                    .child(model_cost_cell(m, t)),
            );
        }
    }
    card(t)
        .px(px(16.0))
        .py(px(12.0))
        .child(table)
        .into_any_element()
}

// ---------- Providers ----------

/// One column per provider: sessions and cost, KPIs, a mini daily chart in the provider colour
/// and the provider's models.
fn providers_view(s: &Summary, metric: StatsMetric, t: &Tokens) -> AnyElement {
    let total = total_value(s, metric);
    let mut cols = div().flex().gap(px(14.0)).items_start();
    for p in SERIES {
        let color = series_color(p, t.dark);
        let ps = s.providers.iter().find(|x| x.provider == p);
        let (work, incl, cost, sessions) = ps.map_or((0, 0, 0.0, 0), |x| {
            (x.tokens.work(), x.tokens.processed(), x.cost, x.sessions)
        });
        let own = match metric {
            StatsMetric::Tokens => work as f64,
            StatsMetric::Cost => cost,
        };
        let share = if total > 0.0 { own / total } else { 0.0 };
        let share_label = match metric {
            StatsMetric::Tokens => "Share of work",
            StatsMetric::Cost => "Share of cost",
        };
        let kpi = |label: &str, value: String| {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .text_xs()
                        .text_color(t.muted)
                        .child(SharedString::from(label.to_string())),
                )
                .child(
                    div()
                        .text_size(px(17.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(SharedString::from(value)),
                )
        };
        let models = models_of(s, p, metric);
        let own_total: f64 = models.iter().map(|m| model_value(m, metric)).sum();
        let mut list = div().flex().flex_col().mt(px(12.0));
        if models.is_empty() {
            list = list.child(
                div()
                    .py(px(10.0))
                    .text_sm()
                    .text_color(t.muted)
                    .child("No usage in this range"),
            );
        }
        for (i, m) in models.iter().enumerate() {
            let frac = if own_total > 0.0 {
                model_value(m, metric) / own_total
            } else {
                0.0
            };
            list = list.child(
                body_row(t, i == 0)
                    .text_size(px(12.5))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(SharedString::from(m.model.clone())),
                    )
                    .child(
                        div()
                            .w(px(110.0))
                            .flex_none()
                            .flex()
                            .child(share_bar(frac, color, t)),
                    )
                    .child(
                        cell(58.0).child(SharedString::from(tokens_text(m.tokens.work() as f64))),
                    )
                    .child(match m.cost {
                        Some(c) => cell(78.0).child(SharedString::from(money(c))),
                        None => cell(78.0).text_color(t.muted).child("Unpriced"),
                    }),
            );
        }
        cols = cols.child(
            card(t)
                .flex_1()
                .min_w_0()
                .px(px(16.0))
                .py(px(14.0))
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .mb(px(12.0))
                        .child(dot(color))
                        .child(
                            div()
                                .text_base()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(provider_label(p)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(t.muted)
                                .child(SharedString::from(format!("{sessions} sessions"))),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(SharedString::from(money(cost))),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .mb(px(12.0))
                        .child(kpi("Work", tokens_text(work as f64)))
                        .child(kpi("Incl. cache", tokens_text(incl as f64)))
                        .child(kpi(share_label, share_text(share))),
                )
                .child(mini_chart(&s.points, p, metric, color, t))
                .child(list),
        );
    }
    cols.into_any_element()
}

/// A daily bar chart of one provider, scaled to its own maximum.
fn mini_chart(
    points: &[Point],
    p: ProviderKind,
    metric: StatsMetric,
    color: Hsla,
    t: &Tokens,
) -> AnyElement {
    const H: f32 = 70.0;
    let sl = slot(p);
    let max = points
        .iter()
        .map(|pt| point_value(pt, sl, metric))
        .fold(0.0, f64::max);
    if max <= 0.0 {
        return div()
            .h(px(H))
            .flex()
            .items_center()
            .text_xs()
            .text_color(t.muted)
            .child("No usage in this range")
            .into_any_element();
    }
    let hover_bg = t.text.opacity(0.06);
    let mut bars = div()
        .flex()
        .items_end()
        .h(px(H))
        .border_b_1()
        .border_color(t.border);
    for (i, pt) in points.iter().enumerate() {
        let v = point_value(pt, sl, metric);
        let h = if v > 0.0 {
            ((v / max) as f32 * H).max(1.5)
        } else {
            0.0
        };
        let (title, text) = (pt.title.clone(), value_text(v, metric));
        bars = bars.child(
            div()
                .id((if sl == 0 { "mini-claude" } else { "mini-codex" }, i))
                .flex_1()
                .min_w_0()
                .h_full()
                .px(px(0.5))
                .flex()
                .flex_col()
                .justify_end()
                .hover(move |s| s.bg(hover_bg))
                .tooltip(move |window, cx| {
                    let (title, text) = (title.clone(), text.clone());
                    Tooltip::element(move |_, _| {
                        div()
                            .flex()
                            .gap(px(10.0))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(SharedString::from(title.clone())),
                            )
                            .child(div().child(SharedString::from(text.clone())))
                    })
                    .build(window, cx)
                })
                .child(
                    div()
                        .w_full()
                        .h(px(h))
                        .flex_none()
                        .rounded(px(1.0))
                        .bg(color),
                ),
        );
    }
    let (first, last) = (
        points.first().map(|p| p.label.clone()).unwrap_or_default(),
        points.last().map(|p| p.label.clone()).unwrap_or_default(),
    );
    div()
        .flex()
        .flex_col()
        .child(bars)
        .child(
            div()
                .flex()
                .justify_between()
                .mt(px(3.0))
                .text_size(px(10.5))
                .text_color(t.muted)
                .child(SharedString::from(first))
                .child(SharedString::from(last)),
        )
        .into_any_element()
}

// ---------- Days ----------

/// Rows of the Days table visible at once; older days scroll inside the table (owner,
/// 2026-11-07: a full 90-day list made the window far too long).
const DAY_ROWS_VISIBLE: usize = 7;
const DAY_ROW_H: f32 = 32.0;

/// Day table, newest first: the last 7 days visible, the rest scrolls inside the card.
fn day_table(s: &Summary, t: &Tokens, scroll: &gpui::ScrollHandle) -> AnyElement {
    let header = header_row(t)
        .child(div().flex_1().min_w_0().child("Date"))
        .child(cell(120.0).child("Work tokens"))
        .child(cell(120.0).child("Incl. cache"))
        .child(cell(110.0).child("Cost"))
        .child(cell(90.0).child("Sessions"));
    let mut rows = div().flex().flex_col();
    if s.days.is_empty() {
        rows = rows.child(
            div()
                .py(px(20.0))
                .text_sm()
                .text_color(t.muted)
                .child("No usage in this range"),
        );
    }
    for (i, d) in s.days.iter().enumerate() {
        rows = rows.child(
            body_row(t, i == 0)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(SharedString::from(d.day.format("%a, %b %-d").to_string())),
                )
                .child(cell(120.0).child(SharedString::from(tokens_text(d.work as f64))))
                .child(
                    cell(120.0)
                        .text_color(t.muted)
                        .child(SharedString::from(tokens_text(d.tokens as f64))),
                )
                .child(
                    cell(110.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(SharedString::from(money(d.cost))),
                )
                .child(
                    cell(90.0)
                        .text_color(t.muted)
                        .child(SharedString::from(d.sessions.to_string())),
                ),
        );
    }
    let visible = s.days.len().clamp(1, DAY_ROWS_VISIBLE) as f32 * DAY_ROW_H;
    let body = if s.days.len() > DAY_ROWS_VISIBLE {
        div()
            .relative()
            .h(px(visible))
            .child(
                div()
                    .id("days-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(scroll)
                    // Room for the overlay scrollbar so it never covers the Sessions column.
                    .pr(px(GUTTER))
                    .child(rows),
            )
            .vertical_scrollbar(scroll)
            .into_any_element()
    } else {
        rows.into_any_element()
    };
    card(t)
        .px(px(16.0))
        .py(px(12.0))
        .child(
            div()
                .flex()
                .flex_col()
                .child(header.pr(px(if s.days.len() > DAY_ROWS_VISIBLE {
                    GUTTER
                } else {
                    0.0
                })))
                .child(body),
        )
        .into_any_element()
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
        assert_eq!(money_short(4473.88), "$4,473.88");
        assert_eq!(money_short(48705.4), "$48,705");
        assert_eq!(share_text(0.63), "63%");
        assert_eq!(share_text(0.004), "<1%");
        assert_eq!(share_text(0.0), "—");
    }

    #[test]
    fn axis_steps_are_round_and_cover_the_maximum() {
        for max in [0.7, 3.0, 12.0, 99.0, 1234.0, 4.2e6, 297e6, 1.6e9] {
            let (top, step) = nice_axis(max);
            assert!(top >= max, "{max}: top {top}");
            assert!((top - step * 3.0).abs() < 1e-6 * top);
            // The step is 1, 1.5, 2, 2.5, 3, 4, 5, 6, 8 or 10 times a power of ten.
            let mag = 10f64.powf(step.log10().floor());
            let m = step / mag;
            assert!(
                [1.0, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0]
                    .iter()
                    .any(|c| (c - m).abs() < 1e-6),
                "{max}: step {step}"
            );
        }
        assert_eq!(nice_axis(300.0), (300.0, 100.0));
        assert_eq!(axis_text(1.5e6, StatsMetric::Tokens), "1.5M");
        assert_eq!(axis_text(250e3, StatsMetric::Tokens), "250K");
        assert_eq!(axis_text(0.0, StatsMetric::Tokens), "0");
        assert_eq!(axis_text(1500.0, StatsMetric::Cost), "$1,500");
        assert_eq!(axis_text(0.5, StatsMetric::Cost), "$0.50");
    }
}
