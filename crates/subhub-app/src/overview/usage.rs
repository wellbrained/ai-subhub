use super::*;

// ----- Where usage goes -----

pub(super) fn usage_line(o: &Overview, cx: &Ctx) -> AnyElement {
    let u = &o.usage;
    if u.work == 0 {
        return plain(if cx.s.stats.is_none() {
            "Reading the session logs…"
        } else {
            "No session logs in this range"
        });
    }
    let mut parts = vec![m("Mostly ")];
    let kinds: Vec<&insights::Share<TaskKind>> = u.kinds.iter().take(2).collect();
    for (i, k) in kinds.iter().enumerate() {
        if i > 0 {
            parts.push(m(" and "));
        }
        parts.push(b(kind_lower(k.key)));
    }
    let projects: Vec<String> = u
        .projects
        .iter()
        .filter(|p| p.key != ProjectKey::Unknown && !p.label.is_empty())
        .take(2)
        .map(|p| p.label.clone())
        .collect();
    if !projects.is_empty() {
        parts.push(m(", in "));
        for (i, p) in projects.iter().enumerate() {
            if i > 0 {
                parts.push(m(" and "));
            }
            parts.push(b(p.clone()));
        }
    }
    rich(parts, cx.t).into_any_element()
}

/// One bar row: label (+ subline), main part solid and subagent part light, share.
pub(super) fn share_row(
    label_w: f32,
    label: String,
    sub: Option<String>,
    share: f64,
    sub_part: f64,
    max: f64,
    t: &Tokens,
) -> gpui::Div {
    let scale = if max > 0.0 { share / max } else { 0.0 };
    let main_w = (scale * (1.0 - sub_part)).clamp(0.0, 1.0) as f32;
    let sub_w = (scale * sub_part).clamp(0.0, 1.0) as f32;
    div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .py(px(5.0))
        .child(
            div()
                .w(px(label_w))
                .flex_none()
                .flex()
                .flex_col()
                .child(div().text_size(px(BODY)).clip("bar", label))
                .children(sub.map(|s| {
                    div()
                        .text_size(px(TINY))
                        .text_color(t.faint)
                        .clip("bar-sub", s)
                })),
        )
        .child(
            div()
                .flex_1()
                .h(px(10.0))
                .rounded(px(3.0))
                .bg(t.track)
                .flex()
                .overflow_hidden()
                .child(div().h_full().w(relative(main_w)).bg(t.accent))
                .child(div().h_full().w(relative(sub_w)).bg(t.accent.opacity(0.4))),
        )
        .child(
            div()
                .w(px(44.0))
                .flex_none()
                .text_right()
                .text_size(px(BODY))
                .text_color(t.muted)
                .child(share_text(share)),
        )
}

pub(super) fn share_text(frac: f64) -> String {
    if frac <= 0.0 {
        "—".into()
    } else if frac < 0.01 {
        "<1%".into()
    } else {
        pct(frac * 100.0)
    }
}

pub(super) fn sub_part(by_tokens: bool, usd: f64, sub_usd: f64, work: u64, sub_work: u64) -> f64 {
    let (a, b) = if by_tokens {
        (sub_work as f64, work as f64)
    } else {
        (sub_usd, usd)
    };
    if b > 0.0 {
        (a / b).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

pub(super) fn thread_title(th: &insights::ThreadShare) -> String {
    th.title
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "Untitled thread".into())
}

/// "writing code · 3 days" (Biggest threads); with the project in the Thread view, which has
/// the room.
pub(super) fn thread_sub(th: &insights::ThreadShare, o: &Overview, with_project: bool) -> String {
    let mut s = vec![];
    if with_project {
        s.push(
            o.usage
                .projects
                .iter()
                .find(|p| p.key == th.project)
                .map(|p| project_label(&p.key, &p.label))
                .unwrap_or_else(|| project_label(&th.project, "")),
        );
    }
    if let Some(k) = th.kind {
        s.push(kind_lower(k));
    }
    s.push(format!(
        "{} day{}",
        th.days,
        if th.days == 1 { "" } else { "s" }
    ));
    s.join(" · ")
}

pub(super) fn usage_content(ov: &OverviewState, o: &Overview, cx: &Ctx) -> AnyElement {
    let t = cx.t;
    let u = &o.usage;
    if u.work == 0 {
        return div()
            .text_size(px(SMALL))
            .text_color(t.muted)
            .child(if cx.s.stats.is_none() {
                "The session logs are being read."
            } else {
                "No Claude Code or Codex session logs in this range."
            })
            .into_any_element();
    }
    let me = cx.me.clone();
    let switch = seg(
        "ov-dim",
        &[
            (Dim::Task, "Task"),
            (Dim::Project, "Project"),
            (Dim::Thread, "Thread"),
            (Dim::Model, "Model"),
        ],
        &ov.dim,
        t,
        move |d, cx| {
            me.update(cx, |w, cx| {
                w.overview_mut().dim = d;
                cx.notify();
            })
        },
    );
    let swatch = |opacity: f32| {
        div()
            .w(px(12.0))
            .h(px(10.0))
            .rounded(px(2.0))
            .bg(t.accent.opacity(opacity))
    };
    let legend = div()
        .flex()
        .gap(px(14.0))
        .ml_auto()
        .text_size(px(SMALL))
        .text_color(t.muted)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(swatch(1.0))
                .child("main threads"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(swatch(0.4))
                .child("subagents"),
        );

    let bt = u.by_tokens;
    let bars: Vec<gpui::Div> = match ov.dim {
        Dim::Task => {
            let max = u.kinds.first().map_or(0.0, |k| k.share);
            u.kinds
                .iter()
                .map(|k| {
                    share_row(
                        LABEL_W,
                        k.label.clone(),
                        Some(kind_covers(k.key).to_string()),
                        k.share,
                        sub_part(bt, k.usd, k.sub_usd, k.work, k.sub_work),
                        max,
                        t,
                    )
                })
                .collect()
        }
        Dim::Project => {
            let max = u.projects.first().map_or(0.0, |k| k.share);
            u.projects
                .iter()
                .take(10)
                .map(|p| {
                    share_row(
                        LABEL_W,
                        project_label(&p.key, &p.label),
                        None,
                        p.share,
                        sub_part(bt, p.usd, p.sub_usd, p.work, p.sub_work),
                        max,
                        t,
                    )
                })
                .collect()
        }
        Dim::Model => {
            let max = u.models.first().map_or(0.0, |k| k.share);
            u.models
                .iter()
                .take(10)
                .map(|p| {
                    share_row(
                        LABEL_W,
                        p.label.clone(),
                        None,
                        p.share,
                        sub_part(bt, p.usd, p.sub_usd, p.work, p.sub_work),
                        max,
                        t,
                    )
                })
                .collect()
        }
        Dim::Thread => {
            let max = u.threads.first().map_or(0.0, |k| k.share);
            u.threads
                .iter()
                .take(10)
                .map(|th| {
                    share_row(
                        THREAD_LABEL_W,
                        thread_title(th),
                        Some(thread_sub(th, o, true)),
                        th.share,
                        sub_part(bt, th.usd, th.sub_usd, th.work, 0),
                        max,
                        t,
                    )
                })
                .collect()
        }
    };

    // Biggest threads on the right (not in the Thread view, which lists them already).
    let side = (ov.dim != Dim::Thread).then(|| {
        let top: Vec<&insights::ThreadShare> = u.threads.iter().take(4).collect();
        let shown: f64 = top.iter().map(|x| x.share).sum();
        let others = u.thread_count.saturating_sub(top.len());
        let entry = |key: String,
                     title: String,
                     sub: Option<String>,
                     share: f64,
                     first: bool,
                     faint: bool| {
            div()
                .flex()
                .gap(px(8.0))
                .py(px(6.0))
                .when(!first, |d| {
                    d.border_t_1().border_color(t.border.opacity(0.7))
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(px(12.5))
                                .when(faint, |d| d.text_color(t.faint))
                                .clip(format!("entry-{key}"), title),
                        )
                        .children(sub.map(|s| {
                            div()
                                .text_size(px(TINY))
                                .text_color(t.faint)
                                .clip(format!("entry-sub-{key}"), s)
                        })),
                )
                .child(
                    div()
                        .w(px(40.0))
                        .flex_none()
                        .text_right()
                        .text_size(px(12.5))
                        .text_color(t.muted)
                        .child(share_text(share)),
                )
        };
        let mut col = div().w(px(290.0)).flex_none().flex().flex_col().child(
            div()
                .mb(px(4.0))
                .text_size(px(SMALL))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.muted)
                .child("Biggest threads"),
        );
        for (i, th) in top.iter().enumerate() {
            let sub = thread_sub(th, o, false);
            col = col.child(entry(
                i.to_string(),
                thread_title(th),
                Some(sub),
                th.share,
                i == 0,
                false,
            ));
        }
        if others > 0 {
            col = col.child(entry(
                "others".into(),
                format!(
                    "{others} other thread{}",
                    if others == 1 { "" } else { "s" }
                ),
                None,
                (1.0 - shown).max(0.0),
                top.is_empty(),
                true,
            ));
        }
        col
    });

    // One insight line.
    let builds = u.kind_share(TaskKind::BuildsTests);
    let sub_share = u.sub_share();
    let (title, rest) = if builds > insights::TASK_INSIGHT_SHARE {
        (
            format!("Builds & tests take {}.", pct(builds * 100.0)),
            "Mostly re-reading long build output. Quieter runs (only the failing test, less output) would cut a good part of it.".to_string(),
        )
    } else if sub_share >= 0.25 {
        (
            format!("Subagents do {} of the work.", pct(sub_share * 100.0)),
            "Cheaper models for searches and reading would lower that share.".to_string(),
        )
    } else if let Some(k) = u.kinds.first() {
        (
            format!("{} leads with {}.", k.label, pct(k.share * 100.0)),
            format!("Subagents do {} of the work.", share_text(sub_share)),
        )
    } else {
        (String::new(), String::new())
    };
    let basis = if bt {
        "Shares of work tokens (nothing in this range has a price)."
    } else {
        "Shares of the API estimate, so models compare."
    };

    div()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .mb(px(14.0))
                .child(switch)
                .child(legend),
        )
        .child(
            div()
                .flex()
                .gap(px(36.0))
                .child(div().flex_1().min_w_0().flex().flex_col().children(bars))
                .children(side),
        )
        .when(!title.is_empty(), |d| {
            d.child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.0))
                    .mt(px(16.0))
                    .child(chip(Msg::Info, 24.0, t))
                    .child(
                        div()
                            .flex_1()
                            .pt(px(3.0))
                            .text_size(px(12.5))
                            .line_height(relative(1.45))
                            .text_color(t.muted)
                            .child(rich(vec![b(title), m(format!(" {rest}"))], t)),
                    ),
            )
        })
        .child(
            div()
                .mt(px(10.0))
                .text_size(px(TINY))
                .text_color(t.faint)
                .child(basis),
        )
        .into_any_element()
}
