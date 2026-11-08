use super::*;

// ----- warm-up controls (build step 4; mockup `warmup.html`) -----

/// A line of the controls column: label (and a small hint) left, the control right.
pub(super) fn control_line(
    label: &str,
    hint: Option<String>,
    control: AnyElement,
    t: &Tokens,
) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(10.0))
        .py(px(8.0))
        .border_t_1()
        .border_color(t.border)
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(div().text_size(px(BODY)).child(label.to_string()))
                .children(hint.map(|h| {
                    div()
                        .mt(px(1.0))
                        .text_size(px(TINY))
                        .text_color(t.faint)
                        .child(h)
                })),
        )
        .child(control)
}

/// A small toggle chip.
pub(super) fn toggle_chip(
    id: impl Into<gpui::ElementId>,
    text: impl Into<SharedString>,
    on: bool,
    enabled: bool,
    t: &Tokens,
    on_click: impl Fn(&mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id.into())
        .flex_none()
        .px(px(7.0))
        .py(px(2.0))
        .rounded(px(6.0))
        .text_size(px(TINY))
        .border_1()
        .when(on, |d| {
            d.bg(t.accent.opacity(if t.dark { 0.2 } else { 0.14 }))
                .border_color(gpui::transparent_black())
                .text_color(t.text)
        })
        .when(!on, |d| d.border_color(t.border).text_color(t.muted))
        .when(enabled, |d| {
            d.cursor_pointer()
                .hover(|s| s.text_color(t.text))
                .on_click(move |_, _, cx| on_click(cx))
        })
        .when(!enabled, |d| d.opacity(0.5))
        .child(text.into())
}

/// A small square button with one character ("−", "+").
pub(super) fn step_button(
    id: &'static str,
    text: &'static str,
    t: &Tokens,
    on_click: impl Fn(&mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .border_1()
        .border_color(t.border)
        .text_size(px(BODY))
        .text_color(t.muted)
        .cursor_pointer()
        .hover(|s| s.text_color(t.text).bg(t.surface_hover))
        .child(text)
        .on_click(move |_, _, cx| on_click(cx))
}

/// "today 7:00", "yesterday 7:02", "Nov 6, 7:00".
pub(super) fn when_text(at: chrono::DateTime<Utc>, today: NaiveDate) -> String {
    let local = at.with_timezone(&chrono::Local).naive_local();
    let time = time_text(local.time());
    match (today - local.date()).num_days() {
        0 => format!("today {time}"),
        1 => format!("yesterday {time}"),
        _ => format!("{}, {time}", date_text(local.date())),
    }
}

/// The accounts the warm-up can be chosen for: visible ones with a 5-hour window.
pub(super) fn warm_candidates<'a>(o: &'a Overview, s: &AppState) -> Vec<&'a AccountInsight> {
    o.accounts
        .iter()
        .filter(|a| a.has_session_window && !s.account(a.account).is_some_and(|x| x.hidden))
        .collect()
}

pub(super) fn can_warm_id(s: &AppState, id: Uuid) -> bool {
    s.account(id).is_some_and(subhub_core::warmup::can_warm)
}

/// What the setup dialog opens with: the current settings when the warm-up is on; otherwise
/// the suggestion (time and accounts) over the last choices, with Wake the PC off. The user
/// confirms or changes it; nothing runs before (owner, 2026-11-08).
pub fn suggested_draft(o: &Overview, s: &AppState) -> WarmupSettings {
    let cur = &s.settings.warmup;
    if cur.enabled {
        return cur.clone();
    }
    let mut d = cur.clone();
    d.wake_pc = false;
    if let Some(x) = o.windows.ready().and_then(|r| r.warmup.as_ref()) {
        d.time = x.at;
        let picked: Vec<Uuid> = x
            .accounts
            .iter()
            .copied()
            .filter(|id| can_warm_id(s, *id))
            .collect();
        if !picked.is_empty() {
            d.accounts = picked;
        }
    }
    d.accounts.retain(|id| can_warm_id(s, *id));
    if d.days.is_empty() {
        d.days = subhub_core::warmup::WORKDAYS.to_vec();
    }
    d
}

/// Opens the setup dialog (and the 5-hour row) with [`suggested_draft`].
pub(super) fn open_dialog(
    me: Entity<StatsWindow>,
    state: Entity<AppState>,
) -> impl Fn(&mut App) + 'static {
    move |app: &mut App| {
        let draft = {
            let o = me.read(app).overview_ref().latest();
            o.map(|o| suggested_draft(&o, state.read(app)))
        };
        me.update(app, |w, cx| {
            let ov = w.overview_mut();
            ov.open = Some(TipRow::Windows);
            ov.warm_dialog = Some(draft.unwrap_or_default());
            cx.notify();
        });
    }
}

/// "Personal (Claude), Work (Codex)".
pub(super) fn account_names(o: &Overview, ids: &[Uuid]) -> String {
    names(&ids.iter().map(|id| label_of(o, *id)).collect::<Vec<_>>())
}

pub(super) fn small_text(text: impl Into<SharedString>, t: &Tokens) -> gpui::Div {
    div()
        .text_size(px(TINY))
        .line_height(relative(1.5))
        .text_color(t.faint)
        .child(text.into())
}

/// A text button (secondary, or the accent for the main action).
pub(super) fn text_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    primary: bool,
    enabled: bool,
    t: &Tokens,
    on_click: impl Fn(&mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id.into())
        .flex_none()
        .px(px(12.0))
        .py(px(5.0))
        .rounded(px(8.0))
        .text_size(px(12.5))
        .font_weight(FontWeight::SEMIBOLD)
        .when(primary, |d| d.bg(t.accent).text_color(gpui::white()))
        .when(!primary, |d| {
            d.border_1().border_color(t.border).bg(t.surface)
        })
        .when(!enabled, |d| d.opacity(0.5))
        .when(enabled, |d| {
            d.cursor_pointer()
                .hover(move |s| {
                    if primary {
                        s.opacity(0.9)
                    } else {
                        s.bg(t.surface_hover)
                    }
                })
                .on_click(move |_, _, cx| on_click(cx))
        })
        .child(label.into())
}

/// The Warm-up column of the 5-hour row. Off: what is suggested and "Set up warm-up…". On:
/// the settings in words, the last runs, "Change…", "Turn off" and "Start a window now".
pub(super) fn warmup_column(o: &Overview, cx: &Ctx) -> gpui::Div {
    let t = cx.t;
    let s = cx.s;
    let w = &s.settings.warmup;
    let col = div()
        .w(px(300.0))
        .flex_none()
        .pl(px(28.0))
        .border_l_1()
        .border_color(t.border)
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(h4("Warm-up", t));
    let setup = open_dialog(cx.me.clone(), cx.state.clone());
    let para = |parts: Vec<(String, bool)>| {
        div()
            .text_size(px(SMALL))
            .line_height(relative(1.5))
            .text_color(t.muted)
            .child(rich(parts, t))
    };

    if !w.enabled {
        let suggestion = o.windows.ready().and_then(|r| r.warmup.as_ref());
        let mut parts = vec![m(
            "SubHub can send a one-word prompt at a time you choose, so the first window starts \
             early. It only runs once you turn it on.",
        )];
        if let Some(x) = suggestion {
            parts = vec![m("Suggested: "), b(time_text(x.at)), m(" on weekdays")];
            let who: Vec<Uuid> = x
                .accounts
                .iter()
                .copied()
                .filter(|id| can_warm_id(s, *id))
                .collect();
            if !who.is_empty() {
                parts.push(m(" for "));
                parts.push(b(account_names(o, &who)));
            }
            parts.push(m(". It only runs once you turn it on."));
        }
        return col
            .child(para(parts))
            .child(div().mt(px(4.0)).flex().child(text_button(
                "warmup-setup",
                "Set up warm-up…",
                true,
                true,
                t,
                setup,
            )));
    }

    // On.
    let chosen: Vec<Uuid> = w
        .accounts
        .iter()
        .copied()
        .filter(|id| can_warm_id(s, *id))
        .collect();
    let mut summary = vec![
        m("On at "),
        b(time_text(w.time)),
        m(format!(" {}", w.days_text())),
    ];
    if chosen.is_empty() {
        summary.push(m(", no account chosen"));
    } else {
        summary.push(m(" for "));
        summary.push(b(account_names(o, &chosen)));
    }
    summary.push(m(if w.wake_pc {
        ". Wakes the PC for it."
    } else {
        ". Runs while SubHub is running."
    }));
    let runs: Vec<String> = chosen
        .iter()
        .filter_map(|id| {
            let r = s.warmup_log.last.get(id)?;
            Some(format!(
                "{} · {} · {}",
                label_of(o, *id),
                when_text(r.at, o.today),
                r.outcome.text()
            ))
        })
        .collect();
    let running = !s.warmup_running.is_empty();
    let start_now = {
        let state = cx.state.clone();
        let ids = chosen.clone();
        text_button(
            "warmup-now",
            if running {
                "Starting…"
            } else if chosen.len() > 1 {
                "Start windows now"
            } else {
                "Start a window now"
            },
            false,
            !chosen.is_empty() && !running,
            t,
            move |app| {
                let ids = ids.clone();
                state.update(app, |s, cx| s.warmup_now(ids, cx));
            },
        )
    };
    let turn_off = {
        let state = cx.state.clone();
        link("warmup-off", "Turn off", true, t, move |app| {
            state.update(app, |s, cx| s.set_warmup(|w| w.enabled = false, cx));
        })
    };
    col.child(para(summary))
        .children(
            s.warmup_task_error
                .clone()
                .map(|e| div().text_size(px(TINY)).text_color(t.danger).child(e)),
        )
        .when(w.wake_pc && s.dev_build(), |d| {
            d.child(small_text(
                "Dev build: the installed app sets up the wake task.",
                t,
            ))
        })
        .when(!runs.is_empty(), |d| {
            d.child(
                div()
                    .mt(px(4.0))
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(h4("Last runs", t))
                    .children(
                        runs.into_iter()
                            .map(|r| div().text_size(px(TINY)).text_color(t.muted).child(r)),
                    ),
            )
        })
        .child(
            div()
                .mt(px(6.0))
                .flex()
                .items_center()
                .gap(px(12.0))
                .child(start_now)
                .child(link("warmup-change", "Change…", false, t, setup))
                .child(turn_off),
        )
}

/// The setup dialog over the Statistics window, editing a draft; "Turn on" / "Save" applies it,
/// Cancel drops it.
pub fn dialog(ov: &OverviewState, cx: &Ctx) -> Option<AnyElement> {
    let draft = ov.warm_dialog.as_ref()?;
    let o = ov.latest()?;
    let o = o.as_ref();
    let t = cx.t;
    let s = cx.s;
    let editing = s.settings.warmup.enabled;
    let suggestion = o.windows.ready().and_then(|r| r.warmup.as_ref());
    let candidates = warm_candidates(o, s);
    // Every control edits the draft.
    let edit = |f: Rc<dyn Fn(&mut WarmupSettings)>| {
        cx.upd(move |ov| {
            if let Some(d) = ov.warm_dialog.as_mut() {
                f(d);
            }
        })
    };

    let step = |mins: i64| -> Rc<dyn Fn(&mut WarmupSettings)> {
        Rc::new(move |w: &mut WarmupSettings| {
            let m = (w.time.hour() as i64 * 60 + w.time.minute() as i64 + mins).rem_euclid(24 * 60);
            let m = m - m % 15;
            w.time = NaiveTime::from_hms_opt((m / 60) as u32, (m % 60) as u32, 0).unwrap_or(w.time);
        })
    };
    let usual = o.windows.ready().and_then(|r| r.rhythm.usual_start);
    let time_hint = match (suggestion, usual) {
        (Some(x), Some(start)) if x.at == draft.time => Some(format!(
            "Suggested from your usual start, {}",
            time_text(start)
        )),
        (Some(x), Some(start)) => Some(format!(
            "Suggested {} from your usual start, {}",
            time_text(x.at),
            time_text(start)
        )),
        _ => None,
    };
    let use_suggested = suggestion.filter(|x| x.at != draft.time).map(|x| {
        let at = x.at;
        link(
            "warmup-use-suggested",
            format!("Use {}", time_text(at)),
            false,
            t,
            edit(Rc::new(move |w: &mut WarmupSettings| w.time = at)),
        )
    });
    let time = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .children(use_suggested)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(step_button("warmup-earlier", "−", t, edit(step(-15))))
                .child(
                    div()
                        .w(px(48.0))
                        .flex()
                        .justify_center()
                        .py(px(2.0))
                        .rounded(px(6.0))
                        .bg(t.surface)
                        .text_size(px(BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(time_text(draft.time)),
                )
                .child(step_button("warmup-later", "+", t, edit(step(15)))),
        );

    let mut days = div().flex().gap(px(3.0));
    for (i, d) in subhub_core::warmup::WEEK.iter().enumerate() {
        let d = *d;
        days = days.child(toggle_chip(
            ("warmup-day", i),
            subhub_core::warmup::short_day(d),
            draft.runs_on(d),
            true,
            t,
            edit(Rc::new(move |w: &mut WarmupSettings| {
                if w.runs_on(d) {
                    w.days.retain(|x| *x != d);
                } else {
                    w.days.push(d);
                }
            })),
        ));
    }

    let mut accounts = div().flex().flex_wrap().gap(px(4.0));
    let mut hub_only: Vec<String> = vec![];
    for (i, a) in candidates.iter().enumerate() {
        let id = a.account;
        let ok = can_warm_id(s, id);
        if !ok {
            hub_only.push(label_of(o, id));
        }
        accounts = accounts.child(toggle_chip(
            ("warmup-acc", i),
            label_of(o, id),
            ok && draft.accounts.contains(&id),
            ok,
            t,
            edit(Rc::new(move |w: &mut WarmupSettings| {
                if w.accounts.contains(&id) {
                    w.accounts.retain(|x| *x != id);
                } else {
                    w.accounts.push(id);
                }
            })),
        ));
    }

    let wake = {
        let toggle = edit(Rc::new(|w: &mut WarmupSettings| w.wake_pc = !w.wake_pc));
        Switch::new("warmup-wake")
            .checked(draft.wake_pc)
            .disabled(crate::demo::on())
            .on_click(move |_, _, app| toggle(app))
    };
    let wake_hint = if crate::demo::on() {
        "Not in demo mode".to_string()
    } else {
        "A Windows task wakes the PC from sleep or hibernation (not when shut down)".to_string()
    };

    let valid = !draft.days.is_empty() && draft.accounts.iter().any(|id| can_warm_id(s, *id));
    let apply = {
        let state = cx.state.clone();
        let d = draft.clone();
        let close = cx.upd(|ov| ov.warm_dialog = None);
        move |app: &mut App| {
            let d = d.clone();
            state.update(app, |s, cx| {
                s.set_warmup(
                    |w| {
                        *w = d;
                        w.enabled = true;
                    },
                    cx,
                )
            });
            close(app);
        }
    };
    let cancel = cx.upd(|ov| ov.warm_dialog = None);

    let reset = draft.time + Duration::hours(5);
    let panel = div()
        .id("warmup-dialog")
        .w(px(460.0))
        .flex()
        .flex_col()
        .p(px(22.0))
        .rounded(px(12.0))
        .border_1()
        .border_color(t.border)
        .bg(t.bg)
        .shadow_lg()
        // Clicks inside never reach the backdrop.
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .text_size(px(16.0))
                .font_weight(FontWeight::SEMIBOLD)
                .child(if editing {
                    "Warm-up"
                } else {
                    "Set up warm-up"
                }),
        )
        .child(
            div()
                .mt(px(6.0))
                .mb(px(8.0))
                .text_size(px(SMALL))
                .line_height(relative(1.5))
                .text_color(t.muted)
                .child(rich(
                    vec![
                        m("At "),
                        b(time_text(draft.time)),
                        m(" SubHub sends the one-word prompt \"ok\" with the smallest model to each \
                           chosen account. Its 5-hour window then resets at "),
                        b(time_text(reset)),
                        m(". Accounts whose window is already running are skipped."),
                    ],
                    t,
                )),
        )
        .child(control_line("Time", time_hint, time.into_any_element(), t))
        .child(control_line("Days", None, days.into_any_element(), t))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .py(px(8.0))
                .border_t_1()
                .border_color(t.border)
                .child(div().text_size(px(BODY)).child("Accounts"))
                .child(if candidates.is_empty() {
                    small_text("No account with a 5-hour window.", t)
                } else {
                    accounts
                })
                .when(!hub_only.is_empty(), |d| {
                    d.child(small_text(
                        format!(
                            "{} {} a CLI profile on this PC.",
                            names(&hub_only),
                            if hub_only.len() == 1 { "needs" } else { "need" }
                        ),
                        t,
                    ))
                }),
        )
        .child(control_line(
            "Wake the PC",
            Some(wake_hint),
            wake.into_any_element(),
            t,
        ))
        .child(
            div()
                .pt(px(8.0))
                .border_t_1()
                .border_color(t.border)
                .child(small_text(
                    "Costs well under 1% of a window. Runs while SubHub is running; with Wake \
                     the PC also from sleep (wake timers must be allowed in the power options). \
                     A missed time still runs within an hour. You can turn it off any time.",
                    t,
                )),
        )
        .child(
            div()
                .mt(px(16.0))
                .flex()
                .justify_end()
                .gap(px(8.0))
                .child(text_button("warmup-cancel", "Cancel", false, true, t, cancel))
                .child(text_button(
                    "warmup-apply",
                    if editing { "Save" } else { "Turn on" },
                    true,
                    valid,
                    t,
                    apply,
                )),
        );
    let backdrop_close = cx.upd(|ov| ov.warm_dialog = None);
    Some(
        div()
            .id("warmup-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::black().opacity(if t.dark { 0.45 } else { 0.25 }))
            .on_click(move |_, _, cx| backdrop_close(cx))
            .child(panel)
            .into_any_element(),
    )
}
