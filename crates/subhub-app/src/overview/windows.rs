use super::*;

// ----- 5-hour windows -----

pub(super) fn windows_line(
    o: &Overview,
    warm: &WarmupSettings,
    t: &Tokens,
) -> (AnyElement, Option<AnyElement>) {
    // With the warm-up on, the note says so instead of the waited time.
    let warm_note = warm.enabled.then(|| {
        div()
            .flex()
            .items_center()
            .gap(px(5.0))
            .text_size(px(SMALL))
            .text_color(t.ok)
            .child(icon(Icon::Clock, 12.0, t.ok))
            .child(format!(
                "Warm-up {} {}",
                time_text(warm.time),
                warm.days_text()
            ))
            .into_any_element()
    });
    let w: &WindowsReport = match &o.windows {
        Collect::Ready(w) => w,
        Collect::Collecting { ready_on } => return (plain(collecting_text(*ready_on)), warm_note),
    };
    let answer = if w.hits_with_free_account > 0 {
        let n = w.hits_with_free_account;
        rich(
            vec![
                b(if n == 1 {
                    "Once".to_string()
                } else {
                    format!("{n} times")
                }),
                m(" you waited while another account had a fresh window"),
            ],
            t,
        )
        .into_any_element()
    } else if w.hits > 0 {
        rich(
            vec![
                b(format!("{} hits a week", num(w.hits_per_week))),
                m(", no other account had room then"),
            ],
            t,
        )
        .into_any_element()
    } else {
        plain("No 5-hour limit hits in this range")
    };
    let right = warm_note.or_else(|| {
        (w.waited > Duration::zero()).then(|| {
            div()
                .text_size(px(SMALL))
                .text_color(t.faint)
                .child(format!("{} waited", dur_text(w.waited)))
                .into_any_element()
        })
    });
    (answer, right)
}

pub(super) fn stat(value: String, label: &str, first: bool, t: &Tokens) -> gpui::Div {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .when(!first, |d| {
            d.pl(px(14.0)).border_l_1().border_color(t.border)
        })
        .child(
            div()
                .text_size(px(17.0))
                .font_weight(FontWeight::SEMIBOLD)
                .child(value),
        )
        .child(
            div()
                .text_size(px(SMALL))
                .text_color(t.muted)
                .child(label.to_string()),
        )
}

pub(super) fn windows_content(o: &Overview, cx: &Ctx) -> AnyElement {
    let t = cx.t;
    let w = match &o.windows {
        Collect::Ready(w) => w,
        Collect::Collecting { ready_on } => {
            return div()
                .flex()
                .gap(px(28.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .child(explainer(t))
                        .child(
                            div()
                                .text_size(px(SMALL))
                                .text_color(t.muted)
                                .child(format!(
                                    "The 5-hour analysis needs windows on 5 workdays. {}.",
                                    collecting_text(*ready_on)
                                )),
                        ),
                )
                .child(warmup_column(o, cx))
                .into_any_element();
        }
    };

    // Left: how it works, one day drawn twice, the warm-up result.
    let mut left = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .child(explainer(t));
    if let Some(s) = &w.warmup {
        if let Some(e) = &s.example {
            left = left.child(day_timeline(e, s.at, cx.color(e.account), t).mt(px(14.0)));
        }
        let n = s.simulation.days.len();
        left = left.child(
            div()
                .flex()
                .mt(px(16.0))
                .child(stat(
                    dur_text(s.simulation.waited_now),
                    &format!("waited on the last {n} workdays"),
                    true,
                    t,
                ))
                .child(
                    stat(
                        dur_text(s.simulation.waited_with),
                        &format!("with a {} warm-up, same work", time_text(s.at)),
                        false,
                        t,
                    )
                    .text_color(t.ok),
                )
                .child(stat(
                    format!("+{}", s.simulation.runs),
                    "extra windows, one a day",
                    false,
                    t,
                )),
        );
    }

    // Choosing the start by hand, in words.
    left = left.child(
        div()
            .mt(px(18.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(h4("Set your own start", t))
            .children(
                own_start_lines(w, o, &cx.s.settings.warmup)
                    .into_iter()
                    .map(|parts| {
                        div()
                            .text_size(px(SMALL))
                            .line_height(relative(1.5))
                            .text_color(t.muted)
                            .child(rich(parts, t))
                    }),
            ),
    );
    let right = warmup_column(o, cx);

    let numbers = div()
        .flex()
        .child(stat(w.hits.to_string(), "5-hour limit hits", true, t))
        .child(stat(num(w.hits_per_week), "hits a week", false, t))
        .child(stat(dur_text(w.waited), "waited for resets", false, t))
        .child(stat(
            format!("{}×", w.hits_with_free_account),
            "while another account was free",
            false,
            t,
        ));

    div()
        .flex()
        .flex_col()
        .gap(px(24.0))
        .child(div().flex().gap(px(28.0)).child(left).child(right))
        .child(week_timeline(w, o, cx))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(h4(&format!("In the range · {}", o.range.label()), t))
                .child(numbers),
        )
        .into_any_element()
}

pub(super) fn h4(text: &str, t: &Tokens) -> gpui::Div {
    div()
        .text_size(px(SMALL))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.muted)
        .child(text.to_string())
}

/// How windows and resets work (owner 2026-11-08: explain it shortly).
pub(super) fn explainer(t: &Tokens) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(h4("How the 5-hour window works", t))
        .child(
            div()
                .text_size(px(SMALL))
                .line_height(relative(1.5))
                .text_color(t.muted)
                .child(rich(
                    vec![
                        m("Your "),
                        b("first message starts a window"),
                        m(", and it resets "),
                        b("5 hours later"),
                        m(
                            ", whether you used all of it or not. The next message after a \
                           reset starts the next window. So the time of your first message \
                           decides all resets of the day.",
                        ),
                    ],
                    t,
                )),
        )
}

/// The "Set your own start" text: the user's rhythm and what to do about it.
pub(super) fn own_start_lines(
    w: &WindowsReport,
    o: &Overview,
    warm: &WarmupSettings,
) -> Vec<Vec<(String, bool)>> {
    let r = &w.rhythm;
    let mut out = vec![];
    match (r.usual_start, r.usual_first_hit) {
        (Some(start), Some(hit)) => out.push(vec![
            m("You usually start around "),
            b(time_text(start)),
            m(" and run out of the first window around "),
            b(time_text(start + hit)),
            m("."),
        ]),
        (Some(start), None) => out.push(vec![
            m("You usually start around "),
            b(time_text(start)),
            m(" and your first window rarely runs out."),
        ]),
        _ => {}
    }
    if warm.enabled {
        out.push(vec![
            m("The warm-up starts your windows at "),
            b(time_text(warm.time)),
            m(format!(
                " on {}, so the first reset comes at ",
                warm.days_text()
            )),
            b(time_text(warm.time + Duration::hours(5))),
            m(". To move it, change the time; any message before that starts the window too."),
        ]);
        return out;
    }
    match &w.warmup {
        Some(s) => {
            let who = names(
                &s.accounts
                    .iter()
                    .map(|id| label_of(o, *id))
                    .collect::<Vec<_>>(),
            );
            let mut line = vec![m("Send any short message at "), b(time_text(s.at))];
            if !who.is_empty() {
                line.push(m(" with "));
                line.push(b(who));
            }
            line.push(m(". Its window then resets at "));
            line.push(b(time_text(s.at + Duration::hours(5))));
            line.push(m(", close to when you run out, and the next one is fresh."));
            out.push(line);
            out.push(vec![m(format!(
                "Over your last {} workdays that would have saved about {} of waiting \
                 (an estimate from your readings).",
                s.simulation.days.len(),
                dur_text(s.simulation.saved())
            ))]);
        }
        None if r.recent_first_hits < insights::RECENT_HITS => out.push(vec![m(
            "Your first window seldom runs out, so an earlier start would not save time \
             right now.",
        )]),
        None => out.push(vec![m(
            "An earlier start would not move the first reset closer to when you run out.",
        )]),
    }
    out
}

/// Where `t` lies between `from` and `to`, 0..1.
pub(super) fn frac_of(t: NaiveDateTime, from: NaiveDateTime, to: NaiveDateTime) -> f32 {
    let all = (to - from).num_seconds().max(1) as f32;
    ((t - from).num_seconds() as f32 / all).clamp(0.0, 1.0)
}

/// One lane of windows between `from` and `to` (cut at `now` when given): work in `color`
/// (solid until a hit, lighter in a window without one), the wait after a hit tinted in the
/// danger colour, a warm-up window outlined, an unknown end (PC off) on the track colour.
/// `labels` puts each reset's time above it.
#[allow(clippy::too_many_arguments)]
pub(super) fn lane(
    spans: &[Span],
    from: NaiveDateTime,
    to: NaiveDateTime,
    now: Option<NaiveDateTime>,
    color: Hsla,
    h: f32,
    labels: bool,
    t: &Tokens,
) -> gpui::Div {
    let top = if labels { 15.0 } else { 0.0 };
    let mut d = div().relative().w_full().h(px(h + top));
    let piece = |a: f32, b: f32| {
        div()
            .absolute()
            .top(px(top))
            .h(px(h))
            .left(relative(a))
            .w(relative((b - a).max(0.0)))
    };
    // The wait after a hit: a thin line in the danger colour, so it never blends with a red or
    // pink account colour.
    let wait = |a: f32, b: f32| {
        div()
            .absolute()
            .top(px(top + h / 2.0 - 1.0))
            .h(px(2.0))
            .left(relative(a))
            .w(relative((b - a).max(0.0)))
            .bg(t.danger)
    };
    for s in spans {
        let end = now.map_or(s.reset, |n| s.reset.min(n));
        if end <= from || s.start >= to {
            continue;
        }
        let x0 = frac_of(s.start, from, to);
        let x1 = frac_of(end, from, to);
        let hit = s.hit.filter(|h| *h < end).map(|h| frac_of(h, from, to));
        if s.warm {
            d = d.child(
                piece(x0, x1)
                    .rounded(px(3.0))
                    .border_1()
                    .border_color(t.accent.opacity(0.8))
                    .bg(t.accent.opacity(0.14))
                    .when(labels, |p| {
                        p.overflow_hidden().pl(px(6.0)).flex().items_center().child(
                            div()
                                .text_size(px(10.5))
                                .text_color(t.accent_text())
                                .whitespace_nowrap()
                                .child(format!("warm-up {}", time_text(s.start.time()))),
                        )
                    }),
            );
            if let Some(xh) = hit {
                d = d.child(wait(xh, x1));
            }
        } else if s.unknown {
            d = d.child(piece(x0, x1).rounded(px(3.0)).bg(t.track));
        } else if let Some(xh) = hit {
            d = d
                .child(piece(x0, xh).rounded_l(px(3.0)).bg(color.opacity(0.85)))
                .child(wait(xh, x1));
        } else {
            d = d.child(piece(x0, x1).rounded(px(3.0)).bg(color.opacity(0.35)));
        }
        if labels && s.reset < to {
            let xr = frac_of(s.reset, from, to);
            d = d
                .child(
                    div()
                        .absolute()
                        .top(px(top - 3.0))
                        .h(px(h + 6.0))
                        .left(relative(xr))
                        .w(px(1.0))
                        .bg(t.text.opacity(0.6)),
                )
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left(relative(xr))
                        .ml(px(-18.0))
                        .w(px(36.0))
                        .flex()
                        .justify_center()
                        .text_size(px(10.5))
                        .text_color(t.muted)
                        .child(time_text(s.reset.time())),
                );
        }
    }
    d
}

/// Hour labels every 3 hours under a day timeline.
pub(super) fn hour_axis(from: NaiveDateTime, to: NaiveDateTime, t: &Tokens) -> gpui::Div {
    let mut d = div().relative().w_full().h(px(14.0));
    let mut h = from;
    while h <= to {
        let x = frac_of(h, from, to);
        let (first, last) = (h == from, h == to);
        d = d.child(
            div()
                .absolute()
                .top_0()
                .left(relative(x))
                .ml(px(if last {
                    -36.0
                } else if first {
                    0.0
                } else {
                    -18.0
                }))
                .w(px(36.0))
                .flex()
                .when(last, |d| d.justify_end())
                .when(!last && !first, |d| d.justify_center())
                .text_size(px(10.5))
                .text_color(t.faint)
                .child(if h.time() == NaiveTime::MIN && h > from {
                    "24:00".to_string()
                } else {
                    time_text(h.time())
                }),
        );
        h += Duration::hours(3);
    }
    d
}

pub(super) fn legend(color: Hsla, text: &str) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(5.0))
        .child(div().w(px(12.0)).h(px(8.0)).rounded(px(2.0)).bg(color))
        .child(text.to_string())
}

/// Legend entry for the wait line.
pub(super) fn legend_line(color: Hsla, text: &str) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(5.0))
        .child(div().w(px(12.0)).h(px(2.0)).bg(color))
        .child(text.to_string())
}

/// One past day as it went and with the warm-up.
pub(super) fn day_timeline(e: &ExampleDay, at: NaiveTime, color: Hsla, t: &Tokens) -> gpui::Div {
    const LABEL: f32 = 96.0;
    let first_h = e
        .with
        .iter()
        .chain(&e.real)
        .map(|s| s.start.time().hour())
        .min()
        .unwrap_or(6);
    let from_h = (first_h / 3) * 3;
    let from = e.date.and_hms_opt(from_h, 0, 0).unwrap_or_default();
    let midnight = e.date.and_time(NaiveTime::MIN) + Duration::days(1);
    let last = e
        .real
        .iter()
        .chain(&e.with)
        .map(|s| s.reset)
        .max()
        .unwrap_or(from)
        .min(midnight);
    let mut to = from + Duration::hours(3);
    while to < last {
        to += Duration::hours(3);
    }
    let row = |name: &str, spans: &[Span]| {
        div()
            .flex()
            .items_end()
            .child(
                div()
                    .w(px(LABEL))
                    .flex_none()
                    .pb(px(2.0))
                    .text_size(px(SMALL))
                    .text_color(t.muted)
                    .child(name.to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(lane(spans, from, to, None, color, 18.0, true, t)),
            )
    };
    let start_x = frac_of(e.date.and_time(e.start), from, to);
    let plot = div()
        .relative()
        .flex()
        .flex_col()
        .gap(px(6.0))
        // "You started" marker across both lanes (first, so the lanes paint over it).
        .child(
            div()
                .absolute()
                .top(px(12.0))
                .bottom_0()
                .left(px(LABEL))
                .right_0()
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(relative(start_x))
                        .w(px(1.0))
                        .bg(t.text.opacity(0.4)),
                ),
        )
        .child(row("That day", &e.real))
        .child(row("With warm-up", &e.with));
    let label = format!(
        "{} {}: you started at {}. With a {} warm-up the resets come earlier (estimate).",
        e.date.format("%A"),
        date_text(e.date),
        time_text(e.start),
        time_text(at)
    );
    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(div().text_size(px(TINY)).text_color(t.faint).child(label))
        .child(plot)
        .child(div().pl(px(LABEL)).child(hour_axis(from, to, t)))
        .child(
            div()
                .flex()
                .gap(px(14.0))
                .pl(px(LABEL))
                .pt(px(2.0))
                .text_size(px(TINY))
                .text_color(t.muted)
                .child(legend(color.opacity(0.85), "working"))
                .child(legend_line(t.danger, "waiting for the reset"))
                .child(legend(color.opacity(0.35), "window without a hit")),
        )
}

/// The last 7 days: one lane per account with 5-hour windows.
pub(super) fn week_timeline(w: &WindowsReport, o: &Overview, cx: &Ctx) -> gpui::Div {
    const LABEL: f32 = 130.0;
    let t = cx.t;
    let from = w.week_from;
    let to = from + Duration::days(insights::WEEK_DAYS);
    let lanes: Vec<&insights::WeekLane> = w.week.iter().filter(|l| !cx.hidden(l.account)).collect();
    let day_w = 1.0 / insights::WEEK_DAYS as f32;
    let mut days = div().relative().w_full().h(px(14.0));
    let mut grid = div()
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(LABEL))
        .right_0();
    for i in 0..insights::WEEK_DAYS {
        let d0 = from + Duration::days(i);
        let x = frac_of(d0, from, to);
        let today = d0.date() == o.today;
        days = days.child(
            div()
                .absolute()
                .top_0()
                .left(relative(x))
                .w(relative(day_w))
                .flex()
                .justify_center()
                .text_size(px(10.5))
                .text_color(if today { t.text } else { t.faint })
                .child(if today {
                    "Today".to_string()
                } else {
                    d0.format("%a %-d").to_string()
                }),
        );
        if i > 0 {
            grid = grid.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(x))
                    .w(px(1.0))
                    .bg(t.border),
            );
        }
    }
    let mut list = div().relative().flex().flex_col().gap(px(7.0)).child(grid);
    for l in &lanes {
        list = list.child(
            div()
                .flex()
                .items_center()
                .child(
                    div()
                        .w(px(LABEL))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(7.0))
                        .pr(px(8.0))
                        .child(dot(cx.color(l.account)))
                        .child(
                            div()
                                .min_w_0()
                                .text_size(px(SMALL))
                                .clip("lane", label_of(o, l.account)),
                        ),
                )
                .child(div().flex_1().min_w_0().child(lane(
                    &l.spans,
                    from,
                    to,
                    Some(w.week_to),
                    cx.color(l.account),
                    12.0,
                    false,
                    t,
                ))),
        );
    }
    let hits = lanes
        .iter()
        .flat_map(|l| &l.spans)
        .filter(|s| s.hit.is_some_and(|h| h >= from))
        .count();
    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(10.0))
                .child(h4("Last 7 days, by account", t))
                .child(
                    div()
                        .text_size(px(TINY))
                        .text_color(t.faint)
                        .child(match hits {
                            0 => "no limit hits".to_string(),
                            1 => "1 limit hit".to_string(),
                            n => format!("{n} limit hits"),
                        }),
                ),
        )
        .child(div().pl(px(LABEL)).child(days))
        .child(if lanes.is_empty() {
            div()
                .text_size(px(SMALL))
                .text_color(t.muted)
                .child("No account with a 5-hour window.")
        } else {
            div().child(crate::format::capped("lanes", lanes.len(), 26.0, list))
        })
        .child(
            div()
                .flex()
                .gap(px(14.0))
                .pl(px(LABEL))
                .pt(px(2.0))
                .text_size(px(TINY))
                .text_color(t.muted)
                .child(legend(t.muted.opacity(0.85), "until the limit"))
                .child(legend_line(t.danger, "waiting"))
                .child(legend(t.muted.opacity(0.35), "window without a hit"))
                .child(legend(t.track, "PC off, end unknown")),
        )
}
