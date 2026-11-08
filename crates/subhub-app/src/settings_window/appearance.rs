use super::*;

impl SettingsWindow {
    /// Appearance = the popover's look plus the display next to the tray (owner, 2026-11-07).
    /// Rarely changed taskbar rows sit behind "More options".
    pub(super) fn appearance_page(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let d = self.state.read(cx).display().clone();
        let u = self.updater();
        let mut accent_row = div().flex().gap_2();
        for (i, hex) in ACCENTS.iter().enumerate() {
            let active = d.accent.eq_ignore_ascii_case(hex);
            let u = u.clone();
            let color = theme::parse_hex(hex).unwrap_or(t.accent);
            accent_row = accent_row.child(
                div()
                    .id(("accent", i))
                    .size(px(20.0))
                    .rounded_full()
                    .bg(color)
                    .cursor_pointer()
                    .border_2()
                    .border_color(if active { t.text } else { t.bg })
                    .on_click(move |_, _, cx| u(Box::new(move |d| d.accent = hex.to_string()), cx)),
            );
        }
        // One setter per control (each closure owns its handle).
        let set = |f: fn(&mut DisplaySettings, bool)| {
            let u = u.clone();
            move |v: &bool, _: &mut Window, cx: &mut gpui::App| {
                let v = *v;
                u(Box::new(move |d| f(d, v)), cx)
            }
        };

        let popover = div()
            .flex()
            .flex_col()
            .child(section("Popover", t))
            .child(row(
                "Theme",
                None,
                chips(
                    "theme",
                    &[
                        (ThemeMode::System, "System"),
                        (ThemeMode::Light, "Light"),
                        (ThemeMode::Dark, "Dark"),
                    ],
                    &d.theme,
                    t,
                    {
                        let u = u.clone();
                        move |v, cx| {
                            u(Box::new(move |d| d.theme = v), cx);
                            let dark = theme::is_dark(v, cx);
                            theme::apply_component_theme(dark, None, cx);
                        }
                    },
                ),
                t,
            ))
            .child(row("Accent color", None, accent_row, t))
            .child(row(
                "Show usage as",
                Some("Applies to the popover, the taskbar and the hover cards."),
                chips(
                    "style",
                    &[
                        (UsageStyle::Remaining, "% left"),
                        (UsageStyle::Used, "% used"),
                    ],
                    &d.usage_style,
                    t,
                    {
                        let u = u.clone();
                        move |v, cx| u(Box::new(move |d| d.usage_style = v), cx)
                    },
                ),
                t,
            ))
            .child(row(
                "Plan badges",
                None,
                Switch::new("plan")
                    .checked(d.show_plan)
                    .on_click(set(|d, v| d.show_plan = v)),
                t,
            ))
            .child(row(
                "Show emails",
                Some("Under the account names in the popover."),
                Switch::new("email")
                    .checked(d.show_email)
                    .on_click(set(|d, v| d.show_email = v)),
                t,
            ))
            .child(row(
                "Close popover when clicking elsewhere",
                Some("Off: it stays open until you click the tray icon, press Esc or use the close button."),
                Switch::new("blur-close")
                    .checked(d.close_popover_on_blur)
                    .on_click(set(|d, v| d.close_popover_on_blur = v)),
                t,
            ));

        let mut tray = div()
            .flex()
            .flex_col()
            .child(section("Next to the tray", t))
            .child(row(
                "Show usage on the taskbar",
                Some("Always visible next to the tray (hidden during fullscreen); click to open the popover, drag to move. Off: just the tray icon."),
                Switch::new("tb-on")
                    .checked(d.taskbar_enabled)
                    .on_click(set(|d, v| d.taskbar_enabled = v)),
                t,
            ));
        // Style, accounts and the rest only matter while the display is on.
        if d.taskbar_enabled {
            tray = tray
                .child(row(
                    "Style",
                    Some("Badge: provider, account tag, 5-hour and weekly. Bars: name and the 5-hour bar."),
                    chips(
                        "tbm",
                        &[(TaskbarMode::Badge, "Badge"), (TaskbarMode::Bars, "Bars")],
                        &d.taskbar_mode,
                        t,
                        {
                            let u = u.clone();
                            move |v, cx| u(Box::new(move |d| d.taskbar_mode = v), cx)
                        },
                    ),
                    t,
                ))
                .child(self.taskbar_accounts_block(&d, t))
                .child(
                    disclosure(
                        "more-options",
                        "More options",
                        "Position, monitors, details on hover",
                        self.more_open,
                        t,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.more_open = !this.more_open;
                        cx.notify();
                    })),
                )
                .when(self.more_open, |page| {
                    page.child(row(
                        "Position",
                        Some("Left sits next to the weather/widgets button, right next to the tray icons. Dragging the display moves it too."),
                        chips(
                            "tbp",
                            &[
                                (TaskbarPosition::Left, "Left"),
                                (TaskbarPosition::Right, "Right"),
                            ],
                            &d.taskbar_position,
                            t,
                            {
                                let u = u.clone();
                                move |v, cx| {
                                    u(
                                        Box::new(move |d| {
                                            d.taskbar_position = v;
                                            d.reset_taskbar_offsets();
                                        }),
                                        cx,
                                    )
                                }
                            },
                        ),
                        t,
                    ))
                    .child(self.monitors_block(&d, t))
                    .child(row(
                        "Show details on hover",
                        Some("A small card over each taskbar badge: both windows, reset times and status."),
                        Switch::new("hover-on")
                            .checked(d.hover_details)
                            .on_click(set(|d, v| d.hover_details = v)),
                        t,
                    ))
                });
        }

        div()
            .flex()
            .flex_col()
            .child(heading("Appearance"))
            .child(popover)
            .child(tray)
            .child(
                div().pt_6().child(
                    button("reset-display", "Reset appearance", t, false)
                        .on_click(move |_, _, cx| u(Box::new(|d| *d = Default::default()), cx)),
                ),
            )
    }

    /// Which accounts the taskbar shows, compact: Favorites / Focus / Auto and, except for
    /// Focus (one account), how many.
    pub(super) fn taskbar_accounts_block(
        &self,
        d: &DisplaySettings,
        t: &Tokens,
    ) -> impl IntoElement {
        let u = self.updater();
        let u2 = u.clone();
        let hint = match d.taskbar_accounts {
            TaskbarAccounts::Favorites => {
                "Favorites: the accounts starred in the popover (or switched on under Show on taskbar), in star order."
            }
            TaskbarAccounts::Focus => "Focus: only the account new hub threads go to.",
            TaskbarAccounts::Auto => {
                "Auto: the account new hub threads go to, plus any running low (25% left or less)."
            }
        };
        let controls = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(chips(
                "tba",
                &[
                    (TaskbarAccounts::Favorites, "Favorites"),
                    (TaskbarAccounts::Focus, "Focus"),
                    (TaskbarAccounts::Auto, "Auto"),
                ],
                &d.taskbar_accounts,
                t,
                move |v, cx| u(Box::new(move |d| d.taskbar_accounts = v), cx),
            ))
            .when(d.taskbar_accounts != TaskbarAccounts::Focus, |c| {
                c.child(div().pl_2().text_xs().text_color(t.muted).child("up to"))
                    .child(chips(
                        "tbc",
                        &[(1u8, "1"), (2, "2"), (3, "3"), (4, "4"), (5, "5")],
                        &d.taskbar_count,
                        t,
                        move |v, cx| u2(Box::new(move |d| d.taskbar_count = v), cx),
                    ))
            });
        block("Accounts", Some(hint), controls, t)
    }

    /// Appearance → More options → Monitors: main taskbar, all taskbars, or chosen monitors.
    pub(super) fn monitors_block(&self, d: &DisplaySettings, t: &Tokens) -> impl IntoElement {
        #[derive(Clone, PartialEq)]
        enum Pick {
            Main,
            All,
            Choose,
        }
        let monitors = crate::monitors::list();
        let current = match &d.taskbar_monitors {
            TaskbarMonitors::Main => Pick::Main,
            TaskbarMonitors::All => Pick::All,
            TaskbarMonitors::Chosen(_) => Pick::Choose,
        };
        let u = self.updater();
        let main_id: Vec<String> = monitors
            .iter()
            .filter(|m| m.is_main)
            .map(|m| m.id.clone())
            .collect();
        let pick = {
            let u = u.clone();
            move |v: Pick, cx: &mut gpui::App| {
                let main_id = main_id.clone();
                u(
                    Box::new(move |d| {
                        d.taskbar_monitors = match v {
                            Pick::Main => TaskbarMonitors::Main,
                            Pick::All => TaskbarMonitors::All,
                            // Start from the main taskbar; keep an earlier choice.
                            Pick::Choose => match &d.taskbar_monitors {
                                TaskbarMonitors::Chosen(ids) => {
                                    TaskbarMonitors::Chosen(ids.clone())
                                }
                                _ => TaskbarMonitors::Chosen(main_id),
                            },
                        }
                    }),
                    cx,
                )
            }
        };
        let mut col = div().flex().flex_col().child(block(
            "Monitors",
            Some("Where the display shows. The tray icon itself is only on the main taskbar. Other monitors need Windows' \"Show my taskbar on all displays\". A chosen monitor that is unplugged falls back to the main taskbar until it is back."),
            chips(
                "tbmon",
                &[
                    (Pick::Main, "Main taskbar"),
                    (Pick::All, "All taskbars"),
                    (Pick::Choose, "Choose"),
                ],
                &current,
                t,
                pick,
            ),
            t,
        ));
        if let TaskbarMonitors::Chosen(chosen) = &d.taskbar_monitors {
            for m in &monitors {
                let id = m.id.clone();
                let on = chosen.contains(&id);
                let u = u.clone();
                let hint = m
                    .taskbar
                    .is_none()
                    .then_some("No taskbar on this monitor right now.");
                col = col.child(row(
                    &m.label(),
                    hint,
                    Switch::new(SharedString::from(format!("tbmon-{}", m.number)))
                        .checked(on)
                        .disabled(m.taskbar.is_none() && !on)
                        .on_click(move |v, _, cx| {
                            let (v, id) = (*v, id.clone());
                            u(
                                Box::new(move |d| {
                                    if let TaskbarMonitors::Chosen(ids) = &mut d.taskbar_monitors {
                                        ids.retain(|x| *x != id);
                                        if v {
                                            ids.push(id);
                                        }
                                    }
                                }),
                                cx,
                            )
                        }),
                    t,
                ));
            }
        }
        col
    }
}
