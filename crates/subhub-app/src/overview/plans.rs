use super::*;

// ----- Plans -----

pub(super) fn plans_line(o: &Overview, cx: &Ctx) -> (AnyElement, Option<AnyElement>) {
    let t = cx.t;
    let shown: Vec<&AccountInsight> = o
        .accounts
        .iter()
        .filter(|a| !cx.hidden(a.account) && !a.benched)
        .collect();
    // (counts in the totals, words)
    let mut parts: Vec<(bool, Vec<(String, bool)>)> = vec![];
    let mut verdicts: Vec<FitVerdict> = vec![];
    for want in [
        FitVerdict::Downgrade,
        FitVerdict::AtLimit,
        FitVerdict::Unused,
    ] {
        for a in &shown {
            let Some(f) = a.fit.ready().filter(|f| f.verdict == want) else {
                continue;
            };
            if !verdicts.contains(&want) {
                verdicts.push(want);
            }
            let p = match want {
                FitVerdict::Downgrade => {
                    let to = f.suggested.map_or("a smaller plan", |s| s.label());
                    let mut v = vec![b(format!("{} could drop to {to}", label_of(o, a.account)))];
                    if let Some(c) = f.price_change.filter(|c| *c < 0.0) {
                        v.push(m(format!(" and save ~{} a month", usd(c))));
                    }
                    v
                }
                FitVerdict::AtLimit => vec![b(label_of(o, a.account)), m(" is at its limit")],
                _ => vec![b(label_of(o, a.account)), m(" sits unused")],
            };
            parts.push((insights::in_totals(a.provider), p));
        }
    }
    // Claude and Codex first: the line names two plans at most, and the app logins' verdicts
    // belong to their own rows (owner, 2026-11-09).
    parts.sort_by_key(|(totals, _)| !totals);
    let answer = if parts.is_empty() {
        let ready_on = shown
            .iter()
            .filter(|a| !a.plan_conflict)
            .filter_map(|a| a.fit.ready_on())
            .min();
        if shown.iter().any(|a| a.fit.ready().is_some()) {
            plain("Every plan fits how you use it")
        } else if let Some(d) = ready_on {
            plain(collecting_text(d))
        } else {
            plain("No accounts yet")
        }
    } else {
        let mut all = vec![];
        for (i, (_, p)) in parts.into_iter().take(2).enumerate() {
            if i > 0 {
                all.push(m(" · "));
            }
            all.extend(p);
        }
        rich(all, t).into_any_element()
    };
    let right = (!verdicts.is_empty()).then(|| {
        div()
            .flex()
            .gap(px(6.0))
            .children(verdicts.into_iter().map(|v| fit_pill(v, t)))
            .into_any_element()
    });
    (answer, right)
}

pub(super) const C_NAME: f32 = 190.0;
pub(super) const C_NOW: f32 = 118.0;
pub(super) const C_WEEK: f32 = 124.0;
pub(super) const C_HITS: f32 = 62.0;
pub(super) const C_GAP: f32 = 16.0;

/// "Weekly used", "Monthly used", or "Cycle use" when both kinds of account are shown (short:
/// the column is narrow).
pub(super) fn cycle_heading(accounts: &[&AccountInsight]) -> &'static str {
    let monthly = accounts.iter().any(|a| a.cycle == CycleLen::Month);
    let weekly = accounts.iter().any(|a| a.cycle == CycleLen::Week);
    match (weekly, monthly) {
        (true, true) => "Cycle use",
        (false, true) => "Monthly used",
        _ => "Weekly used",
    }
}

/// "week" or "month": what one cycle of the account is.
pub(super) fn cycle_word(a: &AccountInsight) -> &'static str {
    match a.cycle {
        CycleLen::Week => "week",
        CycleLen::Month => "month",
    }
}

pub(super) fn plans_content(o: &Overview, cx: &Ctx) -> AnyElement {
    let t = cx.t;
    let small = |s: String| div().text_size(px(SMALL)).text_color(t.muted).child(s);
    let visible: Vec<&AccountInsight> = o
        .accounts
        .iter()
        .filter(|a| !cx.hidden(a.account))
        .collect();
    let header = div()
        .flex()
        .gap(px(C_GAP))
        .pb(px(6.0))
        .child(div().w(px(C_NAME)).flex_none())
        .child(
            small(
                if cx.s.settings.display.usage_style == UsageStyle::Remaining {
                    "Now · left"
                } else {
                    "Now · used"
                }
                .into(),
            )
            .w(px(C_NOW))
            .flex_none(),
        )
        .child(
            small(format!("{} · {}", cycle_heading(&visible), o.range.label()))
                .w(px(C_WEEK))
                .flex_none(),
        )
        .child(small("5-h hits".into()).w(px(C_HITS)).flex_none())
        .child(small("Plan fit".into()).flex_1());
    let mut rows = div().flex().flex_col();
    let n = visible.len();
    for a in visible {
        rows = rows.child(account_row(a, o, cx));
    }
    div()
        .flex()
        .flex_col()
        .child(header)
        .child(crate::format::capped("plans", n, 100.0, rows))
        .into_any_element()
}

pub(super) fn renewal_of(o: &Overview, id: Uuid) -> Option<&Renewal> {
    o.renewals.iter().find(|r| r.account == id)
}

pub(super) fn account_row(a: &AccountInsight, o: &Overview, cx: &Ctx) -> gpui::Div {
    let t = cx.t;
    let sub = if a.benched {
        "Benched in the hub".to_string()
    } else {
        match renewal_of(o, a.account) {
            Some(r) if r.days >= 0 => format!(
                "{} {} · {} d",
                if r.auto_renew { "Renews" } else { "Ends" },
                date_text(r.date),
                r.days
            ),
            Some(r) => format!("Ended {}", date_text(r.date)),
            None => "No renewal date".into(),
        }
    };
    let name = div()
        .w(px(C_NAME))
        .flex_none()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(7.0))
                .child(dot(cx.color(a.account)))
                .child(
                    div()
                        .min_w_0()
                        .text_size(px(BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .clip("acct", label_of(o, a.account)),
                )
                .children(a.plan.clone().map(|p| {
                    div()
                        .flex_none()
                        .px(px(6.0))
                        .rounded(px(5.0))
                        .bg(t.surface)
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.muted)
                        .child(p)
                })),
        )
        .child(
            div()
                .pl(px(15.0))
                .mt(px(2.0))
                .text_size(px(SMALL))
                .text_color(t.faint)
                .child(sub),
        );

    // Follows "Show usage as" like the popover (% left by default).
    let left = cx.s.settings.display.usage_style == UsageStyle::Remaining;
    let now_bar = |label: &str, w: Option<insights::NowWindow>| {
        let (frac, text, color) = match w {
            Some(w) => {
                let v = if left { 100.0 - w.used } else { w.used };
                (v / 100.0, pct(v), t.level(100.0 - w.used))
            }
            None => (0.0, "—".into(), t.accent),
        };
        div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .text_size(px(TINY))
            .text_color(t.muted)
            .child(div().w(px(22.0)).flex_none().child(label.to_string()))
            .child(mini_bar(frac, color, 5.0, t))
            .child(
                div()
                    .w(px(32.0))
                    .flex_none()
                    .text_right()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text)
                    .child(text),
            )
    };
    let now = div()
        .w(px(C_NOW))
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(5.0))
        .when(a.has_session_window || a.now_session.is_some(), |d| {
            d.child(now_bar("5 h", a.now_session))
        })
        .child(now_bar(
            if a.cycle == CycleLen::Month {
                "Mo"
            } else {
                "Wk"
            },
            a.now_weekly,
        ));

    // Use per cycle (week or month): the last few cycles as columns under a dashed 100 % line.
    let cycles: Vec<&insights::WeeklyCycle> = a.cycles.iter().rev().take(6).rev().collect();
    let n = cycles.len();
    let week = div()
        .w(px(C_WEEK))
        .flex_none()
        .flex()
        .flex_col()
        .child(
            div()
                .relative()
                .h(px(30.0))
                .flex()
                .items_end()
                .gap(px(4.0))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .w(px((n.max(1) as f32) * 19.0 - 4.0))
                        .h(px(1.0))
                        .bg(t.faint.opacity(0.45)),
                )
                .children(cycles.iter().enumerate().map(|(i, c)| {
                    let last = i + 1 == n;
                    div()
                        .w(px(15.0))
                        .h(px((c.peak.clamp(0.0, 100.0) as f32 * 0.3).max(1.0)))
                        .rounded_t(px(2.0))
                        .bg(t.accent.opacity(if last { 0.95 } else { 0.5 }))
                })),
        )
        .child(
            div()
                .mt(px(3.0))
                .text_size(px(TINY))
                .text_color(t.muted)
                .child(match (a.weekly_avg, a.weekly_peak) {
                    (Some(avg), Some(peak)) => format!("avg {} · peak {}", pct(avg), pct(peak)),
                    _ if n > 0 => format!("no full {} yet", cycle_word(a)),
                    _ => "no readings yet".into(),
                }),
        );

    let hits = div()
        .w(px(C_HITS))
        .flex_none()
        .flex()
        .flex_col()
        .child(
            div()
                .text_size(px(BODY))
                .font_weight(FontWeight::SEMIBOLD)
                .when(a.hits_per_week >= AT_LIMIT_HITS && !a.benched, |d| {
                    d.text_color(t.danger)
                })
                .child(if a.has_session_window {
                    num(a.hits_per_week)
                } else {
                    "—".into()
                }),
        )
        .child(
            div()
                .text_size(px(TINY))
                .text_color(t.muted)
                .child(if a.has_session_window {
                    "per week"
                } else {
                    "no 5-h window"
                }),
        );

    let fit_cell = div().flex_1().flex().items_center().child(if a.benched {
        pill("Benched", t.muted, t.surface)
    } else if a.plan_conflict {
        pill(
            "Check plan",
            t.warn,
            t.warn.opacity(if t.dark { 0.16 } else { 0.12 }),
        )
    } else {
        match &a.fit {
            Collect::Ready(f) => fit_pill(f.verdict, t),
            Collect::Collecting { .. } => pill("Collecting", t.muted, t.surface),
        }
    });

    let explain = fit_line(a, o);
    let what_if = a
        .what_if
        .as_ref()
        .map(|w| what_if_text(w, o, Some(a.provider)));
    div()
        .flex()
        .flex_col()
        .pt(px(10.0))
        .pb(px(10.0))
        .border_t_1()
        .border_color(t.border.opacity(0.7))
        .when(a.benched, |d| d.opacity(0.7))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(C_GAP))
                .child(name)
                .child(now)
                .child(week)
                .child(hits)
                .child(fit_cell),
        )
        .child(
            div()
                .pl(px(C_NAME + C_GAP))
                .pt(px(6.0))
                .text_size(px(SMALL))
                .text_color(t.muted)
                .child(explain),
        )
        .children(what_if.map(|w| {
            div()
                .pl(px(C_NAME + C_GAP))
                .pt(px(2.0))
                .text_size(px(SMALL))
                .text_color(t.muted)
                .child(rich(
                    vec![
                        b(match a.what_if.as_ref().map(|x| x.change) {
                            Some(WhatIfChange::Tier(_)) => "What if it downgrades: ",
                            _ => "What if it ends: ",
                        }),
                        m(lower_first(&w)),
                    ],
                    t,
                ))
        }))
}

/// The plan fit in one line.
pub(super) fn fit_line(a: &AccountInsight, o: &Overview) -> String {
    let line = fit_words(a, o);
    if !a.benched {
        return line;
    }
    let stop = if line.ends_with('.') { "" } else { "." };
    format!("{line}{stop} Benched in the hub, so it never counts as capacity.")
}

pub(super) fn fit_words(a: &AccountInsight, o: &Overview) -> String {
    if a.plan_conflict {
        return format!(
            "{} is chosen, but the readings look like the Free plan (no 5-hour limit). Check the plan on the account page.",
            a.plan.as_deref().unwrap_or("A paid plan")
        );
    }
    let f: &PlanFit = match &a.fit {
        Collect::Ready(f) => f,
        Collect::Collecting { ready_on } => return collecting_text(*ready_on),
    };
    let unit = cycle_word(a);
    let unknown = match (f.tier, a.provider) {
        (None, ProviderKind::Antigravity) => " Set its plan on the account page for advice.",
        (None, _) => " Plan unknown, so no price advice.",
        // Google AI plans also carry storage and other perks: no saving is shown.
        (Some(t), _) if t.capacity_only() && f.suggested.is_some() => {
            " By usage only: the plan also covers storage and other perks."
        }
        _ => "",
    };
    let line = match f.verdict {
        FitVerdict::Fits => format!(
            "Well used: avg {}, peak {} of its {unit}.",
            pct(f.avg),
            pct(f.peak)
        ),
        FitVerdict::Downgrade => match (f.suggested, f.peak_on_suggested) {
            (Some(to), Some(p)) => format!(
                "Your busiest {unit} would fit {} ({}).{}",
                to.label(),
                pct(p),
                f.price_change
                    .filter(|c| *c < 0.0)
                    .map(|c| format!(" Saves ~{} a month.", usd(c)))
                    .unwrap_or_default()
            ),
            _ => "A smaller plan would do.".into(),
        },
        FitVerdict::AtLimit => match f.suggested {
            Some(to) => format!(
                "{} {} would give it room{}.",
                if a.has_session_window {
                    format!(
                        "Peak {} and {} 5-hour hits a {unit}.",
                        pct(f.peak),
                        num(f.hits_per_week)
                    )
                } else {
                    format!("Peak {} of its {unit}.", pct(f.peak))
                },
                to.label(),
                f.price_change
                    .filter(|c| *c > 0.0)
                    .map(|c| format!(" (~{} more a month)", usd(c)))
                    .unwrap_or_default()
            ),
            // One login per PC: there is no other account to spread to.
            None if !insights::in_totals(a.provider) => {
                "Already the biggest plan and it reaches its limit.".into()
            }
            None => {
                "Already the biggest plan and it hits its limits: spread work to other accounts."
                    .into()
            }
        },
        FitVerdict::Unused => format!(
            "Used {} on average in {}.{}",
            pct(f.avg),
            o.range.label(),
            f.price_change
                .map(|c| format!(" Not needed? Cancelling saves {} a month.", usd(c)))
                .unwrap_or_default()
        ),
    };
    format!("{line}{unknown}")
}
