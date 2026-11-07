//! The Limits tab of the Statistics window: a live board of the latest readings, modelled on
//! T3 Code's limits board. One section per provider, one card per window kind (5-hour, Weekly,
//! model-scoped weekly), one bar per account in the settings order. The math is in
//! `subhub_core::limits`; this file only reads `AppState` and draws.
use chrono::{DateTime, Utc};
use gpui::{
    AnyElement, FontWeight, Hsla, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*, px, relative,
};
use subhub_core::limits::{self, Slot};
use subhub_core::model::{Account, ProviderKind, UsageState};

use crate::icons::{Icon, icon, provider_color};
use crate::state::{AppState, provider_name};
use crate::taskbar::account_tag;
use crate::theme::Tokens;

const BAR_H: f32 = 30.0;
const BAR_RADIUS: f32 = 7.0;
/// Horizontal distance between two hatch stripes.
const HATCH_GAP: f32 = 7.0;

/// One account's fresh or stale reading of a window kind.
struct Cell {
    left: f64,
    resets_at: Option<DateTime<Utc>>,
    stale: bool,
}

/// One account's column: its name, colour and readings by card title.
struct Col {
    name: String,
    color: Hsla,
    cells: Vec<(String, Cell)>,
}

/// The whole board for the current state, `now` for the countdowns.
pub fn board(s: &AppState, now: DateTime<Utc>, t: &Tokens) -> AnyElement {
    let mut out = div().flex().flex_col().gap(px(14.0));
    let mut any = false;
    for provider in [ProviderKind::Claude, ProviderKind::Codex] {
        let accounts: Vec<&Account> = s
            .settings
            .accounts
            .iter()
            .filter(|a| a.provider == provider && !a.hidden)
            .collect();
        if accounts.is_empty() {
            continue;
        }
        any = true;
        out = out.child(section(s, provider, &accounts, now, t));
    }
    if !any {
        return div()
            .py_10()
            .text_sm()
            .text_color(t.muted)
            .child("No accounts yet")
            .into_any_element();
    }
    if let Some(banner) = errors(s, t) {
        out = out.child(banner);
    }
    out.into_any_element()
}

fn section(
    s: &AppState,
    provider: ProviderKind,
    accounts: &[&Account],
    now: DateTime<Utc>,
    t: &Tokens,
) -> impl IntoElement {
    let cols: Vec<Col> = accounts
        .iter()
        .map(|a| {
            let stale = s.is_stale(a.id) || a.hub.as_ref().is_some_and(|h| h.benched);
            let cells = s
                .settings
                .readings
                .get(&a.id)
                .map(|r| {
                    let mut seen: Vec<String> = Vec::new();
                    let mut cells = Vec::new();
                    for w in r
                        .windows
                        .iter()
                        .filter(|w| !a.hidden_windows.contains(&w.key))
                    {
                        let title = limits::window_title(&w.label);
                        if seen.contains(&title) {
                            continue;
                        }
                        seen.push(title.clone());
                        cells.push((
                            title,
                            Cell {
                                left: w.remaining_percent(),
                                resets_at: w.resets_at,
                                stale,
                            },
                        ));
                    }
                    cells
                })
                .unwrap_or_default();
            Col {
                name: a.label.clone(),
                color: account_tag(s, a).1,
                cells,
            }
        })
        .collect();

    // Card titles: 5-hour, Weekly, then the rest in order of first appearance.
    let mut titles: Vec<String> = Vec::new();
    for col in &cols {
        for (title, _) in &col.cells {
            if !titles.contains(title) {
                titles.push(title.clone());
            }
        }
    }
    titles.sort_by_key(|title| limits::title_rank(title));

    let color = provider_color(provider, t.text);
    let mut sec = div().flex().flex_col().gap(px(10.0)).child(
        div()
            .flex()
            .items_center()
            .gap_2()
            .px(px(2.0))
            .child(icon(Icon::Provider(provider), 16.0, color))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(provider_name(provider)),
            ),
    );
    if titles.is_empty() {
        sec = sec.child(div().text_sm().text_color(t.muted).child("No readings yet"));
    }
    for title in &titles {
        sec = sec.child(card(title, &cols, now, t));
    }
    sec
}

fn card(title: &str, cols: &[Col], now: DateTime<Utc>, t: &Tokens) -> impl IntoElement {
    fn cell_of<'a>(col: &'a Col, title: &str) -> Option<&'a Cell> {
        col.cells.iter().find(|(c, _)| c == title).map(|(_, c)| c)
    }
    let slots: Vec<Slot> = cols
        .iter()
        .filter_map(|c| cell_of(c, title))
        .filter(|c| !c.stale)
        .map(|c| Slot {
            left: c.left,
            resets_at: c.resets_at,
        })
        .collect();
    let agg = limits::aggregate(&slots, now);
    let total = limits::window_duration(title);

    let mut left = div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .w(px(200.0))
        .flex_none()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .truncate()
                .child(SharedString::from(title.to_string())),
        )
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(6.0))
                .child(
                    div()
                        .text_size(px(28.0))
                        .line_height(px(34.0))
                        .font_weight(FontWeight::BOLD)
                        .child(match agg {
                            Some(a) => format!("{}%", a.left.round() as i64),
                            None => "—".to_string(),
                        }),
                )
                .child(div().text_xs().text_color(t.muted).child("left")),
        );
    if let Some(a) = agg
        && a.regain.round() >= 1.0
    {
        left = left.child(
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .child(icon(Icon::Refresh, 11.0, t.text))
                .child(format!("+{}%", a.regain.round() as i64)),
        );
    }

    let mut bars = div().flex().flex_1().min_w_0().gap(px(16.0));
    for col in cols {
        let slot = div().flex_1().min_w_0().h(px(BAR_H));
        bars = bars.child(match cell_of(col, title) {
            Some(cell) => {
                let pace = match (cell.resets_at, total) {
                    (Some(at), Some(total)) => Some(limits::pace_fraction(at - now, total)),
                    _ => None,
                };
                slot.child(bar(col, cell, pace, now, t))
            }
            None => slot,
        });
    }

    div()
        .flex()
        .items_center()
        .gap(px(16.0))
        .px(px(16.0))
        .py(px(14.0))
        .rounded(px(12.0))
        .bg(t.surface)
        .border_1()
        .border_color(t.border)
        .child(left)
        .child(bars)
}

/// The fill colour and the dim colour of the unfilled part, from the account's colour.
fn shades(color: Hsla, dark: bool) -> (Hsla, Hsla, Hsla) {
    if dark {
        let fill = gpui::hsla(color.h, color.s.min(0.75), 0.34, 1.0);
        let dim = gpui::hsla(color.h, color.s * 0.45, 0.15, 1.0);
        let hatch = gpui::hsla(color.h, color.s * 0.5, 0.27, 1.0);
        (fill, dim, hatch)
    } else {
        (color, color.opacity(0.14), color.opacity(0.3))
    }
}

fn bar(
    col: &Col,
    cell: &Cell,
    pace: Option<f32>,
    now: DateTime<Utc>,
    t: &Tokens,
) -> impl IntoElement {
    let (fill, dim, hatch) = shades(col.color, t.dark);
    let frac = (cell.left / 100.0).clamp(0.0, 1.0) as f32;
    let radius = px(BAR_RADIUS);

    let pill_text = if cell.stale {
        Some("stale".to_string())
    } else {
        cell.resets_at.map(|at| {
            if at <= now {
                "due".to_string()
            } else {
                limits::compact_duration(at - now)
            }
        })
    };

    let stripes = gpui::canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            let x0 = w * frac;
            if x0 >= w {
                return;
            }
            for ((ax, ay), (bx, by)) in hatch_segments(x0, w, h, HATCH_GAP, BAR_RADIUS) {
                let mut b = gpui::PathBuilder::stroke(px(1.0));
                b.move_to(gpui::point(
                    bounds.origin.x + px(ax),
                    bounds.origin.y + px(ay),
                ));
                b.line_to(gpui::point(
                    bounds.origin.x + px(bx),
                    bounds.origin.y + px(by),
                ));
                if let Ok(p) = b.build() {
                    window.paint_path(p, hatch);
                }
            }
        },
    )
    .size_full();

    let mut el = div()
        .relative()
        .size_full()
        .rounded(radius)
        .overflow_hidden()
        .bg(dim)
        .when(cell.stale, |d| d.opacity(0.55))
        .child(div().absolute().top_0().left_0().size_full().child(stripes))
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .h_full()
                .w(relative(frac))
                .rounded_l(radius)
                // A fill reaching the right corners is rounded there too (clipping is
                // rectangular, so a square end would stick out of the rounded bar).
                .when(frac >= 0.97, |d| d.rounded_r(radius))
                .bg(fill),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .h_full()
                .flex()
                .items_center()
                .gap(px(5.0))
                .pl(px(9.0))
                .pr(px(if pill_text.is_some() { 82.0 } else { 9.0 }))
                .max_w_full()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(gpui::white())
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(SharedString::from(col.name.clone())),
                )
                .child(
                    div()
                        .flex_none()
                        .child(format!("{}%", cell.left.round() as i64)),
                ),
        );
    if let Some(p) = pace {
        el = el.child(
            div()
                .absolute()
                .top_0()
                .left(relative(p))
                .h_full()
                .w(px(2.0))
                .bg(gpui::white().opacity(0.95)),
        );
    }
    if let Some(text) = pill_text {
        el = el.child(
            div()
                .absolute()
                .top(px((BAR_H - 20.0) / 2.0))
                .right(px(6.0))
                .h(px(20.0))
                .px(px(6.0))
                .rounded(px(5.0))
                .flex()
                .items_center()
                .gap(px(3.0))
                .bg(gpui::black().opacity(0.6))
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(gpui::white())
                .child(icon(Icon::Refresh, 10.0, gpui::white()))
                .child(SharedString::from(text)),
        );
    }
    el
}

/// Diagonal stripes (bottom-left to top-right) across `x0..x1` of a box `x1` wide and `h` tall
/// with corner radius `r`, clipped to its rounded outline (gpui clips children to rectangles), as
/// ((x, y), (x, y)) segments relative to its top-left corner. Stripe positions are fixed
/// multiples of `gap`, so they do not shift when the fill changes.
fn hatch_segments(x0: f32, x1: f32, h: f32, gap: f32, r: f32) -> Vec<((f32, f32), (f32, f32))> {
    let mut out = Vec::new();
    if h <= 0.0 || gap <= 0.0 || x1 <= x0 {
        return out;
    }
    let r = r.min(h / 2.0).min(x1 / 2.0).max(0.0);
    let inside = |(x, y): (f32, f32)| {
        let cx = x.clamp(r, x1 - r);
        let cy = y.clamp(r, h - r);
        (x - cx).powi(2) + (y - cy).powi(2) <= r * r + 1e-3
    };
    let mut sx = ((x0 - h) / gap).floor() * gap;
    while sx < x1 {
        let t0 = ((x0 - sx) / h).max(0.0);
        let t1 = ((x1 - sx) / h).min(1.0);
        if t1 > t0 {
            // Walk the stripe in half-pixel steps and keep the runs inside the rounded outline.
            let at = |t: f32| (sx + t * h, h - t * h);
            let steps = ((t1 - t0) * h * 2.0).ceil().max(1.0) as usize;
            let t_of = |i: usize| t0 + (t1 - t0) * i as f32 / steps as f32;
            let mut run: Option<usize> = None;
            for i in 0..=steps {
                match (inside(at(t_of(i))), run) {
                    (true, None) => run = Some(i),
                    (false, Some(start)) => {
                        if i - 1 > start {
                            out.push((at(t_of(start)), at(t_of(i - 1))));
                        }
                        run = None;
                    }
                    _ => {}
                }
            }
            if let Some(start) = run
                && steps > start
            {
                out.push((at(t_of(start)), at(t1)));
            }
        }
        sx += gap;
    }
    out
}

/// Amber banner with one line per shown account whose last check did not give a reading:
/// "<account> · <status message>" (the message the popover shows).
fn errors(s: &AppState, t: &Tokens) -> Option<AnyElement> {
    let lines: Vec<String> = s
        .settings
        .accounts
        .iter()
        .filter(|a| !a.hidden)
        .filter_map(|a| {
            let st = s.status.get(&a.id)?;
            (st.state != UsageState::Ready && !st.message.trim().is_empty())
                .then(|| format!("{} · {}", a.label, st.message.trim()))
        })
        .collect();
    if lines.is_empty() {
        return None;
    }
    let mut col = div().flex().flex_col().gap(px(3.0)).min_w_0();
    for line in lines {
        col = col.child(div().min_w_0().child(SharedString::from(line)));
    }
    Some(
        div()
            .flex()
            .items_start()
            .gap(px(10.0))
            .px(px(16.0))
            .py(px(12.0))
            .rounded(px(12.0))
            .bg(t.warn.opacity(0.12))
            .border_1()
            .border_color(t.warn.opacity(0.45))
            .text_sm()
            .font_weight(FontWeight::MEDIUM)
            .text_color(t.warn)
            .child(div().pt(px(2.0)).child(icon(Icon::Warning, 14.0, t.warn)))
            .child(col)
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stripes_stay_inside_the_box() {
        let segs = hatch_segments(40.0, 100.0, 30.0, 7.0, 0.0);
        assert!(!segs.is_empty());
        for ((ax, ay), (bx, by)) in segs {
            for (x, y) in [(ax, ay), (bx, by)] {
                assert!((40.0 - 1e-3..=100.0 + 1e-3).contains(&x), "x {x}");
                assert!((-1e-3..=30.0 + 1e-3).contains(&y), "y {y}");
            }
        }
        assert!(hatch_segments(50.0, 50.0, 30.0, 7.0, 0.0).is_empty());
    }

    #[test]
    fn stripes_stay_out_of_rounded_corners() {
        let r: f32 = 7.0;
        let segs = hatch_segments(0.0, 100.0, 30.0, 7.0, r);
        assert!(!segs.is_empty());
        for ((ax, ay), (bx, by)) in segs {
            for (x, y) in [(ax, ay), (bx, by)] {
                let cx = x.clamp(r, 100.0 - r);
                let cy = y.clamp(r, 30.0 - r);
                assert!(
                    (x - cx).powi(2) + (y - cy).powi(2) <= r * r + 0.5,
                    "({x}, {y})"
                );
            }
        }
    }
}
