use super::*;

impl SettingsWindow {
    /// Add accounts → "Choose a folder manually…": provider, name and profile folder (the old
    /// Add account page), for a profile detection does not find.
    pub(super) fn manual_form(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let provider = self.add_provider;
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .pb_2()
                    .text_xs()
                    .text_color(t.muted)
                    .child("Adds a CLI profile folder that detection did not find. A login that is already added is linked, never added twice."),
            )
            .child(row(
                "Provider",
                None,
                chips(
                    "addp",
                    &[
                        (ProviderKind::Claude, "Claude"),
                        (ProviderKind::Codex, "Codex"),
                    ],
                    &provider,
                    t,
                    {
                        let me = cx.entity();
                        move |p, cx| {
                            me.update(cx, |this, cx| {
                                this.add_provider = p;
                                cx.notify();
                            })
                        }
                    },
                ),
                t,
            ))
            .child(row(
                "Name",
                None,
                div().w(px(300.0)).child(Input::new(&self.add_label)),
                t,
            ))
            .child(row(
                "Profile folder",
                None,
                div().w(px(300.0)).child(Input::new(&self.add_dir)),
                t,
            ))
            .child(
                div()
                    .pt_4()
                    .child(button("add", "Add account", t, true).on_click(cx.listener(
                        |this, _, window, cx| {
                            let label = this.add_label.read(cx).value().to_string();
                            let dir = this.add_dir.read(cx).value().to_string();
                            let p = this.add_provider;
                            let r = this.state.update(cx, |s, cx| {
                                let r = s.add_profile(p, &label, &dir);
                                cx.notify();
                                r
                            });
                            this.message = Some(match r {
                                Ok(()) => {
                                    this.add_label
                                        .update(cx, |i, cx| i.set_value("", window, cx));
                                    this.add_dir.update(cx, |i, cx| i.set_value("", window, cx));
                                    this.state.update(cx, |s, cx| s.refresh(false, cx));
                                    "Account added.".into()
                                }
                                Err(e) => e,
                            });
                            cx.notify();
                        },
                    ))),
            )
    }

    /// Runs profile detection again and ticks the rows that are new with a known login.
    pub(super) fn run_detect(&mut self, cx: &mut Context<Self>) {
        self.detected = self.state.read(cx).detect_profiles();
        self.detect_sig = accounts_sig(self.state.read(cx));
        self.picked = self
            .detected
            .iter()
            .filter(|d| d.preselected())
            .map(|d| d.dir.clone())
            .collect();
    }

    /// The account list changed while the page is open (e.g. the first refresh added the hub
    /// logins): statuses are recomputed, ticks on rows that are still importable are kept, rows
    /// that were not importable before get their default tick.
    pub(super) fn redetect_if_accounts_changed(&mut self, cx: &mut Context<Self>) {
        let sig = accounts_sig(self.state.read(cx));
        if self.page != Page::AddAccounts || sig == self.detect_sig {
            return;
        }
        let before: HashSet<PathBuf> = self
            .detected
            .iter()
            .filter(|d| d.importable())
            .map(|d| d.dir.clone())
            .collect();
        let picked = std::mem::take(&mut self.picked);
        self.detected = self.state.read(cx).detect_profiles();
        self.detect_sig = sig;
        self.picked = self
            .detected
            .iter()
            .filter(|d| {
                d.importable()
                    && if before.contains(&d.dir) {
                        picked.contains(&d.dir)
                    } else {
                        d.preselected()
                    }
            })
            .map(|d| d.dir.clone())
            .collect();
    }

    pub(super) fn import_picked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picks: Vec<DetectedProfile> = self
            .detected
            .iter()
            .filter(|d| d.importable() && self.picked.contains(&d.dir))
            .cloned()
            .collect();
        if picks.is_empty() {
            return;
        }
        let (report, msg) = self.state.update(cx, |s, cx| {
            let r = s.import_profiles(&picks, cx);
            cx.notify();
            r
        });
        // Show what was imported: the first new (or linked) account's page.
        match report.added.first().or(report.linked.first()) {
            Some(&id) => self.go(Page::Account(id), window, cx),
            None => self.run_detect(cx),
        }
        self.message = Some(msg);
        cx.notify();
    }

    /// Add accounts (from Accounts): CLI profiles found on this PC (with what importing would do)
    /// and the hub's logins, which the hub adds by itself on each refresh; at the bottom of the
    /// CLI tab the manual folder form.
    pub(super) fn add_accounts_page(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let importable: Vec<&DetectedProfile> =
            self.detected.iter().filter(|d| d.importable()).collect();
        let n_picked = importable
            .iter()
            .filter(|d| self.picked.contains(&d.dir))
            .count();

        let toolbar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .pb_2()
            .child(
                button("det-all", "Select all", t, false).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.picked = this
                            .detected
                            .iter()
                            .filter(|d| d.importable())
                            .map(|d| d.dir.clone())
                            .collect();
                        cx.notify();
                    },
                )),
            )
            .child(
                button("det-none", "Select none", t, false).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.picked.clear();
                        cx.notify();
                    },
                )),
            )
            .child(
                icon_button("det-again", Icon::Refresh, "Detect again", t).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.run_detect(cx);
                        cx.notify();
                    },
                )),
            )
            .child(div().flex_1())
            .child(
                button(
                    "det-import",
                    &format!("Import selected ({n_picked})"),
                    t,
                    n_picked > 0,
                )
                .when(n_picked == 0, |d| d.opacity(0.5).cursor_default())
                .on_click(cx.listener(|this, _, window, cx| this.import_picked(window, cx))),
            );

        let mut list = div().flex().flex_col();
        for p in ProviderKind::ALL {
            let rows: Vec<(usize, &DetectedProfile)> = self
                .detected
                .iter()
                .enumerate()
                .filter(|(_, d)| d.provider == p)
                .collect();
            // App logins (Copilot, Cursor, Antigravity) show only when found on this PC.
            if rows.is_empty() && !p.is_cli_profile() {
                continue;
            }
            list = list.child(provider_heading(p, t));
            if rows.is_empty() {
                list = list.child(
                    div()
                        .py_2()
                        .text_xs()
                        .text_color(t.faint)
                        .child("No profiles found"),
                );
            }
            let count = rows.len();
            let mut g = div().flex().flex_col();
            for (i, d) in rows {
                g = g.child(self.detect_row(i, d, t, cx));
            }
            list = list.child(crate::format::capped(
                &format!("detected-{p:?}"),
                count,
                56.0,
                g,
            ));
        }

        let tab = self.detect_tab;
        let tabs = div().pb_3().child(chips(
            "det-tab",
            &[(DetectTab::Cli, "CLI profiles"), (DetectTab::Hub, "Hub")],
            &tab,
            t,
            {
                let me = cx.entity();
                move |v, cx| {
                    me.update(cx, |this, cx| {
                        this.detect_tab = v;
                        cx.notify();
                    })
                }
            },
        ));
        let page = div()
            .flex()
            .flex_col()
            .child(self.back_to_accounts(t, cx))
            .child(heading("Add accounts"))
            .child(tabs);
        if tab == DetectTab::Hub {
            return page
                .child(div().pb_1().text_xs().text_color(t.muted).child(
                    "Logins in your CLIProxyAPI hub. The hub adds them by itself on each \
                     refresh; a login that also has a CLI profile stays one account.",
                ))
                .child(self.detect_hub_block(t, cx));
        }
        page.child(div().pb_3().text_xs().text_color(t.muted).child(
            "CLI profiles found in your user folder (.claude*, .codex*), in \
                 CLAUDE_CONFIG_DIR and CODEX_HOME, and in .config\\claude, plus the \
                 Copilot, Cursor and Antigravity logins on this PC. Only the login email is \
                 read here; tokens are never touched. A login is never added twice.",
        ))
        .child(toolbar)
        .child(list)
        .child(
            div().pt_4().child(
                disclosure(
                    "manual-add",
                    "Choose a folder manually…",
                    "Provider, name and profile folder",
                    self.manual_add,
                    t,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.manual_add = !this.manual_add;
                    cx.notify();
                })),
            ),
        )
        .when(self.manual_add, |d| d.child(self.manual_form(t, cx)))
    }

    pub(super) fn detect_row(
        &self,
        i: usize,
        d: &DetectedProfile,
        t: &Tokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let importable = d.importable();
        let checked = importable && self.picked.contains(&d.dir);
        let folder = d
            .dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // App logins are one per PC: named after the provider, not their folder.
        let folder = if d.provider.is_cli_profile() {
            folder
        } else {
            d.provider.name().to_string()
        };
        let title = d.email.clone().unwrap_or(folder);
        let path = crate::demo::show_path(&d.dir);
        let path = path.strip_prefix(r"\\?\").unwrap_or(&path).to_string();
        // Importable rows carry no tag: whether the import makes a new account or links the
        // profile to the account that already has this login (e.g. from the hub) is internal
        // (owner, 2026-11-07). Only rows that cannot be imported say why.
        let status = match &d.status {
            DetectStatus::New | DetectStatus::Joins { .. } => None,
            DetectStatus::Added { label, .. } => Some(format!("Already added as {label}")),
            DetectStatus::SameLogin { dir } => {
                let name = dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                Some(format!("Same login as {name}"))
            }
        };
        // Informed, not asked (owner, 2026-11-07): the login is also in the hub; importing makes
        // no second account, it adds this folder as the account's CLI source.
        let in_hub = match &d.status {
            DetectStatus::Joins { account, .. } | DetectStatus::Added { account, .. } => self
                .state
                .read(cx)
                .account(*account)
                .is_some_and(|a| a.hub.is_some()),
            _ => false,
        };
        let dir = d.dir.clone();
        let control: AnyElement = if importable {
            Checkbox::new(("det-pick", i))
                .checked(checked)
                .on_click(cx.listener(move |this, v: &bool, _, cx| {
                    if *v {
                        this.picked.insert(dir.clone());
                    } else {
                        this.picked.remove(&dir);
                    }
                    cx.notify();
                }))
                .into_any_element()
        } else {
            div().size(px(16.0)).into_any_element()
        };
        div()
            .flex()
            .items_center()
            .gap_3()
            .py_3()
            .border_b_1()
            .border_color(t.border)
            .when(!importable, |d| d.opacity(0.6))
            .child(div().flex_none().child(control))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .min_w_0()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .clip("item-title", SharedString::from(title)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .text_xs()
                            .text_color(t.muted)
                            .clip("item-path", SharedString::from(path)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap_1()
                    // App logins read their login at the first refresh, not here.
                    .when(d.email.is_none() && d.provider.is_cli_profile(), |r| {
                        r.child(tag("No login found", t.warn, t))
                    })
                    .when(in_hub, |r| r.child(tag("Also in the hub", t.muted, t)))
                    .when_some(status, |r, s| r.child(tag(&s, t.muted, t))),
            )
    }

    /// Hub logins are read-only here: the hub adds them on every refresh.
    pub(super) fn detect_hub_block(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.state.read(cx);
        let line = match &s.hub {
            HubStatus::Ok(n) => format!(
                "{n} login{} in the hub, added automatically on each refresh.",
                if *n == 1 { "" } else { "s" }
            ),
            HubStatus::Off => "The hub is off.".into(),
            HubStatus::NoKey => "The hub is not set up yet (no management key).".into(),
            HubStatus::Unknown => "The hub has not been checked yet.".into(),
            HubStatus::Unreachable => "The hub is not reachable.".into(),
            HubStatus::Unauthorized => "The hub did not accept the management key.".into(),
            HubStatus::Error(e) => format!("Hub problem: {e}"),
        };
        let set_up = !matches!(s.hub, HubStatus::Ok(_));
        let folder_name = |p: &std::path::Path| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| crate::demo::show_path(p))
        };
        // One account per login (owner, 2026-11-07): a hub login that also has a CLI profile is
        // the same account with both sources; the row says which folder, or that one was found
        // but not imported yet.
        let hub_accounts: Vec<(ProviderKind, String, Option<String>, String, bool)> = s
            .settings
            .accounts
            .iter()
            .filter(|a| a.hub.is_some())
            .map(|a| {
                let cli = match &a.profile_dir {
                    Some(dir) => format!("Also as CLI profile {}", folder_name(dir)),
                    None => match self.detected.iter().find(|d| {
                        d.importable()
                            && d.provider == a.provider
                            && d.email
                                .as_deref()
                                .zip(a.email.as_deref())
                                .is_some_and(|(x, y)| x.eq_ignore_ascii_case(y))
                    }) {
                        Some(d) => {
                            format!("CLI profile {} found, not imported", folder_name(&d.dir))
                        }
                        None => "Hub only".into(),
                    },
                };
                (
                    a.provider,
                    a.label.clone(),
                    a.email.clone(),
                    cli,
                    a.hub.as_ref().is_some_and(|h| h.benched),
                )
            })
            .collect();
        let hub_count = hub_accounts.len();
        let mut rows = div().flex().flex_col();
        for (p, label, email, status, benched) in hub_accounts {
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .py_2()
                    .border_b_1()
                    .border_color(t.border)
                    .child(icon(Icon::Provider(p), 12.0, provider_color(p, t.muted)))
                    .child(div().flex_1().min_w_0().text_sm().clip(
                        "found",
                        SharedString::from(match email {
                            Some(e) => format!("{label} · {e}"),
                            None => label,
                        }),
                    ))
                    .when(benched, |r| r.child(tag("Benched", t.warn, t)))
                    .child(tag(&status, t.muted, t)),
            );
        }
        let source = format!("Source: CLIProxyAPI hub at {}", s.settings.hub.base_url);
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .text_xs()
                    .text_color(t.muted)
                    .child(SharedString::from(source)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .py_2()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(t.muted)
                            .child(SharedString::from(line)),
                    )
                    .when(set_up, |d| {
                        d.child(button("det-hub", "Hub settings", t, false).on_click(
                            cx.listener(|this, _, window, cx| this.go(Page::Hub, window, cx)),
                        ))
                    }),
            )
            .child(crate::format::capped("hub-accounts", hub_count, 40.0, rows))
    }
}
