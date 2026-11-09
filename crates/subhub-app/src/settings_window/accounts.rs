use super::*;

impl SettingsWindow {
    /// "‹ Accounts" over a page drilled into from the list (saves open edits, like any page
    /// switch).
    pub(super) fn back_to_accounts(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        div().flex().child(
            back_link("back-accounts", "Accounts", t)
                .on_click(cx.listener(|this, _, window, cx| this.go(Page::Accounts, window, cx))),
        )
    }

    /// Accounts: one row per account grouped by provider; a row opens its page. "+ Add accounts"
    /// opens detection / import (owner, 2026-11-07: accounts are no longer menu entries).
    pub(super) fn accounts_page(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        struct RowData {
            id: Uuid,
            label: String,
            hidden: bool,
            tag: String,
            tag_color: Hsla,
            source: &'static str,
            health: Health,
        }
        let groups: Vec<(ProviderKind, Vec<RowData>)> = {
            let s = self.state.read(cx);
            let views = crate::hover_card::account_views(s);
            ProviderKind::ALL
                .into_iter()
                .map(|p| {
                    let rows = views
                        .iter()
                        .filter(|v| v.account.provider == p)
                        .map(|v| {
                            let a = v.account;
                            let (tag, tag_color) = account_tag(s, a);
                            RowData {
                                id: a.id,
                                label: a.label.clone(),
                                hidden: a.hidden,
                                tag,
                                tag_color,
                                source: source_hint(a),
                                health: hover::health(v),
                            }
                        })
                        .collect();
                    (p, rows)
                })
                .collect()
        };

        let mut list = div().flex().flex_col();
        let mut n = 0usize;
        for (p, rows) in groups {
            // App logins (Copilot, Cursor, Antigravity) only show up once this PC has one.
            if rows.is_empty() && !p.is_cli_profile() {
                continue;
            }
            list = list.child(provider_heading(p, t));
            if rows.is_empty() {
                list = list.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_xs()
                        .text_color(t.faint)
                        .child("No accounts"),
                );
            }
            let count = rows.len();
            let mut g = div().flex().flex_col();
            for r in rows {
                let id = r.id;
                let (status, status_color) = match r.health {
                    Health::Fresh => ("Fresh", t.ok),
                    Health::NoReading | Health::Benched => {
                        (r.health.label().unwrap_or_default(), t.muted)
                    }
                    h => (h.label().unwrap_or_default(), t.warn),
                };
                g = g.child(
                    div()
                        .id(("acc-row", n))
                        .flex()
                        .items_center()
                        .gap_3()
                        .mx(px(-12.0))
                        .px_3()
                        .py(px(10.0))
                        .rounded(px(8.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(t.surface))
                        .active(|s| s.bg(t.surface_hover))
                        .child(
                            div()
                                .flex()
                                .flex_1()
                                .min_w_0()
                                .items_center()
                                .gap_3()
                                .when(r.hidden, |d| d.opacity(0.55))
                                .child(tag_chip(&r.tag, r.tag_color))
                                .child(
                                    div()
                                        .min_w_0()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .clip("acct-row", SharedString::from(r.label)),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .text_xs()
                                        .text_color(t.muted)
                                        .child(r.source),
                                )
                                .when(r.hidden, |d| d.child(tag("Hidden", t.muted, t))),
                        )
                        .child(tag(status, status_color, t))
                        .child(icon(Icon::ChevronRight, 14.0, t.faint))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.go(Page::Account(id), window, cx)
                        })),
                );
                n += 1;
            }
            list = list.child(crate::format::capped(
                &format!("accounts-{p:?}"),
                count,
                52.0,
                g,
            ));
        }

        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().flex_1().child(heading("Accounts")))
                    .child(
                        button("add-accounts", "Add accounts", t, true)
                            .child(icon(Icon::Plus, 12.0, gpui::white()))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.go(Page::AddAccounts, window, cx)
                            })),
                    ),
            )
            .child(div().text_xs().text_color(t.muted).child(
                "Hub logins appear by themselves; CLI profiles on this PC are added with Add accounts. Click an account for its name, tag, group, billing date, sign-in and removal.",
            ))
            .child(list)
    }

    pub(super) fn account_page(&self, id: Uuid, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let s = self.state.read(cx);
        let Some(acc) = s.account(id).cloned() else {
            return div().child("Account not found.").into_any_element();
        };
        let (tag_text, tag_color) = account_tag(s, &acc);
        let plan = s.settings.plans.get(&id).cloned();
        let favorite = s.display().taskbar_favorites.contains(&id);
        let preferred = s.settings.steering.preferred == Some(id);
        let groups = s.settings.groups.clone();
        let windows = s
            .settings
            .readings
            .get(&id)
            .map(|r| r.windows.clone())
            .unwrap_or_default();
        let source = source_label(&acc);
        let confirming = self.confirm_remove == Some(id);

        // Header: name, then email / provider / plan / source.
        let header = div()
            .flex()
            .flex_col()
            .gap_2()
            .pb_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(tag_chip(&tag_text, tag_color))
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .clip("acct-title", acc.label.clone()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .text_color(t.muted)
                    .child(acc.email.clone().unwrap_or_else(|| "email unknown".into()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(icon(
                                Icon::Provider(acc.provider),
                                12.0,
                                provider_color(acc.provider, t.muted),
                            ))
                            .child(provider_name(acc.provider)),
                    )
                    .when_some(plan, |d, p| d.child(tag(&p, t.muted, t)))
                    .child(tag(source, t.accent, t)),
            );

        // Group chips.
        let mut group_chips = div().flex().flex_wrap().gap_1();
        for (i, g) in std::iter::once(None)
            .chain(groups.into_iter().map(Some))
            .enumerate()
        {
            let active = acc.group == g;
            let st = self.state.clone();
            let label = g.clone().unwrap_or_else(|| "No group".into());
            group_chips = group_chips.child(button(("grp", i), &label, t, active).on_click(
                move |_, _, cx| {
                    st.update(cx, |s, cx| {
                        s.set_group(id, g.clone());
                        cx.notify();
                    })
                },
            ));
        }
        let group_block = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(group_chips)
            .child(div().w(px(240.0)).child(Input::new(&self.group)));

        // Windows.
        let mut win_list = div().flex().flex_col().gap_1();
        if windows.is_empty() {
            win_list = win_list.child(
                div()
                    .text_xs()
                    .text_color(t.muted)
                    .child("Windows appear after the first successful reading."),
            );
        }
        for (i, w) in windows.iter().enumerate() {
            let hidden = acc.hidden_windows.contains(&w.key);
            let sh = self.state.clone();
            let k2 = w.key.clone();
            win_list = win_list.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .when(hidden, |d| d.text_color(t.faint))
                            .child(w.label.clone()),
                    )
                    .child(
                        button(
                            ("hidew", i),
                            if hidden { "Hidden" } else { "Hide" },
                            t,
                            hidden,
                        )
                        .on_click(move |_, _, cx| {
                            sh.update(cx, |s, cx| {
                                s.toggle_window_hidden(id, &k2);
                                cx.notify();
                            })
                        }),
                    ),
            );
        }

        // Order / hide / remove.
        let (st_up, st_down, st_hide) =
            (self.state.clone(), self.state.clone(), self.state.clone());
        let order = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(
                icon_button("mv-up", Icon::ArrowUp, "Move up", t).on_click(move |_, _, cx| {
                    st_up.update(cx, |s, cx| {
                        s.move_account(id, true);
                        cx.notify();
                    })
                }),
            )
            .child(
                icon_button("mv-down", Icon::ArrowDown, "Move down", t).on_click(
                    move |_, _, cx| {
                        st_down.update(cx, |s, cx| {
                            s.move_account(id, false);
                            cx.notify();
                        })
                    },
                ),
            )
            .child(
                icon_button(
                    "hide-acc",
                    if acc.hidden { Icon::Eye } else { Icon::EyeOff },
                    if acc.hidden { "Show" } else { "Hide" },
                    t,
                )
                .on_click(move |_, _, cx| {
                    st_hide.update(cx, |s, cx| {
                        s.toggle_hidden(id);
                        cx.notify();
                    })
                }),
            );
        let removable_hint = if acc.hub.is_some() {
            "It comes back while the hub lists this login."
        } else {
            "The CLI profile and its login stay untouched."
        };
        let remove_btn = if confirming {
            div()
                .id("remove")
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .px_3()
                .py(px(5.0))
                .rounded(px(8.0))
                .text_xs()
                .cursor_pointer()
                .bg(t.danger)
                .text_color(gpui::white())
                .hover(|s| s.opacity(0.9))
                .child(icon(Icon::Trash, 12.0, gpui::white()))
                .child("Click again to remove")
        } else {
            icon_button("remove", Icon::Trash, "Remove", t)
        }
        .on_click(cx.listener(move |this, _, _, cx| {
            if this.confirm_remove == Some(id) {
                this.state.update(cx, |s, cx| {
                    s.remove(id);
                    cx.notify();
                });
                this.confirm_remove = None;
                this.page = Page::Accounts;
            } else {
                this.confirm_remove = Some(id);
            }
            cx.notify();
        }));

        let st_fav = self.state.clone();
        let st_pref = self.state.clone();
        div()
            .flex()
            .flex_col()
            .child(self.back_to_accounts(t, cx))
            .child(header)
            .child(row(
                "Name",
                None,
                div().w(px(240.0)).child(Input::new(&self.rename)),
                t,
            ))
            .child(row(
                "Taskbar tag",
                Some("Up to 3 characters; empty = automatic."),
                div().w(px(90.0)).child(Input::new(&self.tag)),
                t,
            ))
            .child(row(
                "Show on taskbar",
                Some("Adds it to the favorites the taskbar shows in Favorites mode."),
                Switch::new(SharedString::from(format!("fav-{id}")))
                    .checked(favorite)
                    .on_click(move |v, _, cx| {
                        let v = *v;
                        st_fav.update(cx, |s, cx| {
                            if s.display().taskbar_favorites.contains(&id) != v {
                                s.toggle_taskbar_favorite(id);
                            }
                            cx.notify();
                        })
                    }),
                t,
            ))
            .when(acc.hub.is_some(), |d| {
                d.child(row(
                    "Preferred for new hub threads",
                    Some("Steering puts this account on top in the hub. Off returns to Auto."),
                    Switch::new(SharedString::from(format!("pref-{id}")))
                        .checked(preferred)
                        .on_click(move |v, _, cx| {
                            let v = *v;
                            st_pref.update(cx, |s, cx| {
                                if (s.settings.steering.preferred == Some(id)) != v {
                                    s.toggle_preferred(id, cx);
                                }
                            })
                        }),
                    t,
                ))
            })
            .child(self.sign_in_block(&acc, t, cx))
            .child(self.billing_block(&acc, t, cx))
            .child(block("Group", None, group_block, t))
            .child(block(
                "Windows",
                Some("Hide a window to drop it from the popover, the taskbar and the hover cards."),
                win_list,
                t,
            ))
            .child(block("Order", None, order, t))
            .child(block(
                "Remove",
                Some(removable_hint),
                div().child(remove_btn),
                t,
            ))
            .into_any_element()
    }

    /// Sign in (opens the CLI login for this profile) and Retry (checks usage now).
    pub(super) fn sign_in_block(
        &self,
        acc: &Account,
        t: &Tokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = acc.id;
        let s = self.state.read(cx);
        let note = s.account_notes.get(&id).cloned();
        let busy = s.signing_in.contains(&id);
        let terminal = match s.settings.tools.terminal {
            subhub_core::settings::Terminal::WindowsTerminal => "Windows Terminal",
            subhub_core::settings::Terminal::PowerShell => "PowerShell",
        };
        let cli = acc.provider.is_cli_profile();
        let hint = if !cli {
            format!(
                "{} Retry checks usage now.",
                subhub_core::launch::app_sign_in_hint(acc.provider)
            )
        } else if acc.profile_dir.is_some() {
            format!(
                "Sign in opens the {} login for this profile in {terminal} (change it in General → Advanced). Usage refreshes when it ends. Retry skips failure backoff, never a provider cooldown.",
                provider_name(acc.provider)
            )
        } else {
            "Sign in through the hub: this login belongs to it, SubHub only reads its usage. Retry checks usage now.".to_string()
        };
        let (st_in, st_retry) = (self.state.clone(), self.state.clone());
        let mut actions = div().flex().flex_wrap().items_center().gap_2();
        if acc.profile_dir.is_some() && cli {
            actions = actions.child(
                icon_button(
                    "sign-in",
                    Icon::LogIn,
                    if busy { "Signing in…" } else { "Sign in" },
                    t,
                )
                .on_click(move |_, _, cx| st_in.update(cx, |s, cx| s.sign_in(id, cx))),
            );
        }
        actions = actions.child(
            icon_button("retry-now", Icon::Refresh, "Retry now", t)
                .on_click(move |_, _, cx| st_retry.update(cx, |s, cx| s.refresh_account(id, cx))),
        );
        block(
            "Sign in and retry",
            Some(&hint),
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(actions)
                .when_some(note, |d, n| {
                    d.child(div().text_xs().text_color(t.muted).child(n))
                }),
            t,
        )
        .into_any_element()
    }

    /// Optional monthly renewal / access end date with reminders (CONTEXT: Monthly renewal,
    /// Access end date, Billing reminder).
    pub(super) fn billing_block(
        &self,
        acc: &Account,
        t: &Tokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use subhub_core::billing;
        let id = acc.id;
        let b = self.state.read(cx).settings.billing.get(&id).cloned();
        let today = AppState::today();
        let status = b
            .as_ref()
            .map(|b| billing::description(b, today))
            .unwrap_or_else(|| "Not tracked".to_string());
        let url = billing::billing_url(acc.provider);
        let me = cx.entity();
        let kind = chips(
            "bill-kind",
            &[(true, "Renewal"), (false, "Access ends")],
            &self.bill_renew,
            t,
            move |renew, cx| {
                me.update(cx, |this, cx| {
                    let before = this.bill_renew;
                    this.bill_renew = renew;
                    let r = this.state.update(cx, |s, cx| {
                        let r = s.set_billing_kind(id, renew);
                        cx.notify();
                        r
                    });
                    if let Err(e) = r {
                        this.bill_renew = before;
                        this.message = Some(e);
                    }
                    cx.notify();
                })
            },
        );
        let st_rem = self.state.clone();
        let mut content = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .when(b.is_none(), |d| d.text_color(t.muted))
                    .child(status),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(130.0)).child(Input::new(&self.bill_date)))
                    .child(kind),
            );
        if let Some(b) = &b {
            let on = b.reminders;
            content = content.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Switch::new(SharedString::from(format!("bill-rem-{id}")))
                            .checked(on)
                            .on_click(move |v, _, cx| {
                                let v = *v;
                                st_rem.update(cx, |s, cx| {
                                    s.set_billing_reminders(id, v);
                                    cx.notify();
                                })
                            }),
                    )
                    .child(div().text_sm().child("Remind me 7 and 3 days before")),
            );
        }
        let mut buttons = div().flex().flex_wrap().gap_2().child(
            icon_button("open-billing", Icon::ExternalLink, "Open billing", t)
                .tooltip(|window, cx| {
                    gpui_component::tooltip::Tooltip::new(crate::popover::BILLING_HINT)
                        .build(window, cx)
                })
                .on_click(move |_, _, cx| {
                    // Demo mode opens no real billing page.
                    if !crate::demo::on() {
                        cx.open_url(url)
                    }
                }),
        );
        if b.is_some() {
            buttons = buttons.child(
                icon_button("clear-billing", Icon::Close, "Clear", t).on_click(cx.listener(
                    move |this, _, window, cx| {
                        this.state.update(cx, |s, cx| {
                            s.clear_billing(id);
                            cx.notify();
                        });
                        this.fill_billing(id, window, cx);
                        cx.notify();
                    },
                )),
            );
        }
        block(
            "Monthly billing",
            Some("Optional. Enter the next renewal, or the date paid access ends, from your billing page (YYYY-MM-DD, saved with Enter or when you leave the field). Later monthly dates are estimates; SubHub never changes a subscription. Reminders appear as tray notices while SubHub runs."),
            content.child(buttons),
            t,
        )
        .into_any_element()
    }
}

/// What detection statuses depend on: each account's id, provider, email and profile folder.
pub(super) fn accounts_sig(s: &AppState) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for a in &s.settings.accounts {
        (a.id, a.provider, &a.email, &a.profile_dir, &a.label).hash(&mut h);
    }
    h.finish()
}

pub(super) fn source_label(a: &Account) -> &'static str {
    if !a.provider.is_cli_profile() {
        return if a.profile_dir.is_some() {
            "App login on this PC"
        } else {
            "Not connected"
        };
    }
    match (&a.profile_dir, &a.hub) {
        (_, Some(h)) if h.benched => "Benched",
        (Some(_), Some(_)) => "Hub + CLI profile",
        (None, Some(_)) => "Hub",
        (Some(_), None) => "CLI profile",
        (None, None) => "Not connected",
    }
}

/// The short source hint on an account row, in the popover's words (Hub, Local only).
pub(super) fn source_hint(a: &Account) -> &'static str {
    if !a.provider.is_cli_profile() {
        return if a.profile_dir.is_some() {
            "App login"
        } else {
            "Not connected"
        };
    }
    match (&a.profile_dir, &a.hub) {
        (Some(_), Some(_)) => "Hub + profile",
        (None, Some(_)) => "Hub",
        (Some(_), None) => "Local only",
        (None, None) => "Not connected",
    }
}

/// Provider icon and uppercase name over a group of rows.
pub(super) fn provider_heading(p: ProviderKind, t: &Tokens) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .pt_4()
        .pb_1()
        .child(icon(Icon::Provider(p), 12.0, provider_color(p, t.muted)))
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.faint)
                .child(SharedString::from(provider_name(p).to_uppercase())),
        )
}
