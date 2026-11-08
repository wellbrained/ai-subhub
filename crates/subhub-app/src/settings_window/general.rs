use super::*;

impl SettingsWindow {
    pub(super) fn general(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let minutes = self.state.read(cx).settings.refresh_minutes;
        let state = self.state.clone();
        div()
            .flex()
            .flex_col()
            .child(heading("General"))
            .child(row(
                "Refresh every",
                Some("Failures back off automatically; provider cooldowns are always respected."),
                chips("refresh", &[(1u32, "1m"), (2, "2m"), (5, "5m"), (10, "10m"), (15, "15m"), (30, "30m")], &minutes, t, move |m, cx| {
                    state.update(cx, |s, cx| {
                        s.set_refresh_minutes(m);
                        cx.notify();
                    })
                }),
                t,
            ))
            .child({
                // Warm-up is set up next to its explanation (Statistics → 5-hour windows); here
                // only its state and the way there.
                let s = self.state.read(cx);
                let w = &s.settings.warmup;
                let hint = if w.enabled {
                    let names: Vec<String> = w
                        .accounts
                        .iter()
                        .filter_map(|id| s.account(*id))
                        .map(|a| {
                            // Same name on both providers: say which.
                            let twin = s.settings.accounts.iter().any(|b| {
                                b.id != a.id && b.label.trim().eq_ignore_ascii_case(a.label.trim())
                            });
                            if twin {
                                format!("{} ({})", a.label, crate::state::provider_name(a.provider))
                            } else {
                                a.label.clone()
                            }
                        })
                        .collect();
                    format!(
                        "On · {} {}{}{}",
                        w.time.format("%-H:%M"),
                        w.days_text(),
                        if names.is_empty() {
                            " · no account chosen".to_string()
                        } else {
                            format!(" · {}", names.join(", "))
                        },
                        if w.wake_pc { " · wakes the PC" } else { "" }
                    )
                } else {
                    "Off. Starts your 5-hour windows early, so they reset when you usually run out."
                        .to_string()
                };
                let state = self.state.clone();
                let on_stats = self.on_stats.clone();
                row(
                    "Warm-up",
                    Some(&hint),
                    button("warmup-setup", "Open 5-hour windows", t, false).on_click(
                        move |_, _, cx| {
                            state.update(cx, |s, _| {
                                s.pending_overview_row =
                                    Some(subhub_core::insights::TipRow::Windows)
                            });
                            on_stats(cx);
                        },
                    ),
                    t,
                )
                .into_any_element()
            })
            .child({
                let s = self.state.read(cx);
                let steering = s.settings.steering.enabled
                    && s.settings.accounts.iter().any(|a| a.hub.is_some());
                let hint = if steering {
                    "A notification about 15 minutes before an account's 5-hour window runs out, when another account of the same provider has a fresh one. At most one per window. Off for hub accounts while hub steering is on: the hub moves on by itself."
                } else {
                    "A notification about 15 minutes before an account's 5-hour window runs out, when another account of the same provider has a fresh one. At most one per window."
                };
                let state = self.state.clone();
                row(
                    "Tell me before a window runs out",
                    Some(hint),
                    Switch::new("nudge")
                        .checked(s.settings.nudge.enabled)
                        .on_click(move |v, _, cx| {
                            let v = *v;
                            state.update(cx, |s, cx| {
                                s.set_nudge(v);
                                cx.notify();
                            });
                        }),
                    t,
                )
                .into_any_element()
            })
            .child(row(
                "Close button",
                Some("What ✕ on the popover and this window's close button do. Quit is always in the tray icon's menu."),
                chips(
                    "closeq",
                    &[(false, "Keep running in tray"), (true, "Quit completely")],
                    &self.state.read(cx).display().close_quits,
                    t,
                    {
                        let st = self.state.clone();
                        move |v, cx| {
                            st.update(cx, |s, cx| {
                                s.update_display(|d| d.close_quits = v);
                                cx.notify();
                            })
                        }
                    },
                ),
                t,
            ))
            .child(row(
                "Start with Windows",
                Some(if crate::demo::on() {
                    crate::demo::NOT_AVAILABLE
                } else {
                    "Opens SubHub in the tray when you sign in to Windows. Per user, no administrator rights; the switch shows what Windows has registered."
                }),
                Switch::new("startup")
                    .checked(crate::startup::is_enabled())
                    .disabled(crate::demo::on())
                    .on_click({
                        let me = cx.entity();
                        move |v, _, cx| {
                            let v = *v;
                            me.update(cx, |this, cx| {
                                match crate::startup::set_enabled(v) {
                                    Ok(()) => this.state.update(cx, |s, cx| {
                                        s.user_changed();
                                        cx.notify();
                                    }),
                                    Err(e) => {
                                        this.message =
                                            Some(format!("Could not change startup: {e}"));
                                    }
                                }
                                cx.notify();
                            })
                        }
                    }),
                t,
            ))
            .child(
                disclosure(
                    "advanced",
                    "Advanced",
                    "Sign-in terminal, Claude and Codex CLI paths",
                    self.advanced_open,
                    t,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.advanced_open = !this.advanced_open;
                    cx.notify();
                })),
            )
            .when(self.advanced_open, |d| d.child(self.advanced(t, cx)))
    }

    /// General → Advanced (the former Tools page): what SubHub starts for sign-in and launching.
    pub(super) fn advanced(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.state.read(cx);
        let terminal = s.settings.tools.terminal;
        let wt = subhub_core::launch::find_windows_terminal();
        let terminal_hint = match (terminal, &wt) {
            (subhub_core::settings::Terminal::PowerShell, _) => {
                "Sign-in opens in a PowerShell window.".to_string()
            }
            (_, Some(_)) => "Windows Terminal found.".to_string(),
            (_, None) => "Windows Terminal not found: PowerShell is used instead.".to_string(),
        };
        let st = self.state.clone();
        let mut page = div()
            .flex()
            .flex_col()
            .child(
                div().pb_1().text_xs().text_color(t.muted).child(
                    "What SubHub starts for sign-in and the Launch menus. Both CLIs are detected automatically; set a path only when that fails.",
                ),
            )
            .child(row(
                "Sign-in terminal",
                Some(&terminal_hint),
                chips(
                    "terminal",
                    &[
                        (
                            subhub_core::settings::Terminal::WindowsTerminal,
                            "Windows Terminal",
                        ),
                        (subhub_core::settings::Terminal::PowerShell, "PowerShell"),
                    ],
                    &terminal,
                    t,
                    move |v, cx| {
                        st.update(cx, |s, cx| {
                            s.set_terminal(v);
                            cx.notify();
                        })
                    },
                ),
                t,
            ));
        for (provider, input) in [
            (ProviderKind::Claude, &self.claude_path),
            (ProviderKind::Codex, &self.codex_path),
        ] {
            let found = s.cli_path(provider);
            let (status, color) = match &found {
                Some(p) => (format!("Found: {}", crate::demo::show_path(p)), t.ok),
                None => (
                    "Not found. Install the CLI or enter its path.".to_string(),
                    t.warn,
                ),
            };
            page = page.child(block(
                &format!("{} CLI", provider_name(provider)),
                Some("Path to the executable (.exe, .cmd or .bat); saved with Enter or when you leave the field."),
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().w_full().max_w(px(460.0)).child(Input::new(input)))
                    .child(div().text_xs().text_color(color).child(status)),
                t,
            ));
        }
        page
    }
}
