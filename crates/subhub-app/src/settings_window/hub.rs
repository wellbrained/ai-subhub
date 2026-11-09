use super::*;

impl SettingsWindow {
    /// The hub's logins with Remove (second click confirms), then Add (sign in through the hub).
    fn hub_logins(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.state.read(cx);
        let mut logins: Vec<&Account> = s
            .settings
            .accounts
            .iter()
            .filter(|a| a.hub.is_some())
            .collect();
        logins.sort_by_key(|a| a.provider != ProviderKind::Claude);
        let mut list = div().flex().flex_col();
        if logins.is_empty() {
            list = list.child(
                div()
                    .py_2()
                    .text_sm()
                    .text_color(t.muted)
                    .child("The hub has no Claude or Codex logins yet."),
            );
        }
        for a in logins {
            let id = a.id;
            let file = a
                .hub
                .as_ref()
                .map(|h| h.file_name.clone())
                .unwrap_or_default();
            let benched = a.hub.as_ref().is_some_and(|h| h.benched);
            let removing = s.hub_removing.contains(&file);
            let confirming = self.confirm_hub_remove == Some(id);
            let action = if removing {
                div()
                    .text_xs()
                    .text_color(t.muted)
                    .child("Removing…")
                    .into_any_element()
            } else {
                let b = if confirming {
                    div()
                        .id(("hub-remove", id.as_u128() as u64 as usize))
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
                    icon_button(
                        ("hub-remove", id.as_u128() as u64 as usize),
                        Icon::Trash,
                        "Remove from hub",
                        t,
                    )
                };
                b.on_click(cx.listener(move |this, _, _, cx| {
                    if this.confirm_hub_remove == Some(id) {
                        this.confirm_hub_remove = None;
                        this.state.update(cx, |s, cx| s.remove_hub_login(id, cx));
                    } else {
                        this.confirm_hub_remove = Some(id);
                    }
                    cx.notify();
                }))
                .into_any_element()
            };
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .py(px(6.0))
                    .border_b_1()
                    .border_color(t.border)
                    .child(icon(
                        Icon::Provider(a.provider),
                        14.0,
                        provider_color(a.provider, t.muted),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().text_sm().text_color(t.text).child(a.label.clone()))
                            .child(div().text_xs().text_color(t.faint).child(format!(
                                "{} · {}",
                                provider_name(a.provider),
                                a.email.clone().unwrap_or_else(|| file.clone())
                            ))),
                    )
                    .when(benched, |d| d.child(tag("Benched", t.muted, t)))
                    .child(action),
            );
        }
        let add = match &s.hub_login {
            Some((p, _)) => div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().text_sm().text_color(t.muted).child(format!(
                    "Waiting for the {} sign-in in your browser…",
                    provider_name(*p)
                )))
                .child(
                    button("hub-login-cancel", "Cancel", t, false).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.state.update(cx, |s, cx| s.cancel_hub_login(cx));
                        },
                    )),
                )
                .into_any_element(),
            None => {
                let pick = |kind: ProviderKind, id: &'static str, label: &'static str| {
                    icon_button(id, Icon::Provider(kind), label, t).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.state.update(cx, |s, cx| s.start_hub_login(kind, cx));
                        },
                    ))
                };
                div()
                    .flex()
                    .gap_2()
                    .child(pick(ProviderKind::Claude, "hub-login-claude", "Claude"))
                    .child(pick(ProviderKind::Codex, "hub-login-codex", "Codex"))
                    .into_any_element()
            }
        };
        div()
            .flex()
            .flex_col()
            .child(section("Logins in the hub", t))
            .child(div().pb_1().text_xs().text_color(t.muted).child(
                "Remove deletes the login from the hub; the account stays in SubHub (as a CLI account if it has a profile here). Signing in again brings it back.",
            ))
            .child(list)
            .child(row(
                "Add a login",
                Some("Opens the sign-in in your browser; the hub keeps the login."),
                add,
                t,
            ))
    }

    pub(super) fn hub(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.state.read(cx);
        let enabled = s.settings.hub.enabled;
        let (status, color) = match &s.hub {
            HubStatus::Off => ("Off".to_string(), t.muted),
            HubStatus::NoKey => ("No key stored".into(), t.warn),
            HubStatus::Unknown => ("Not checked yet".into(), t.muted),
            HubStatus::Ok(n) => (format!("Connected · {n} logins"), t.ok),
            HubStatus::Unreachable => ("Not reachable".into(), t.warn),
            HubStatus::Unauthorized => ("Key rejected".into(), t.danger),
            HubStatus::Error(e) => (format!("Error: {e}"), t.danger),
        };
        let steering = &s.settings.steering;
        let steer_on = steering.enabled;
        let label_of =
            |id: Option<uuid::Uuid>| id.and_then(|id| s.account(id)).map(|a| a.label.clone());
        let (steer_status, steer_color) = match (&s.steering_error, steer_on) {
            (Some(e), _) => (format!("Not applied: {e}"), t.danger),
            (None, false) => ("Off · hub priorities are as you set them".into(), t.muted),
            (None, true) => match (label_of(steering.preferred), label_of(steering.current)) {
                (Some(p), _) => (
                    format!("Preferred: {p} (switch it off on the account page for Auto)"),
                    t.text,
                ),
                (None, Some(c)) => (format!("Auto: new threads go to {c}"), t.text),
                (None, None) => ("Auto: waiting for fresh readings".into(), t.muted),
            },
        };
        let st = self.state.clone();
        let st_check = self.state.clone();
        let key_input = self.hub_key.clone();
        div()
            .flex()
            .flex_col()
            .child(heading("Hub"))
            .child(div().pb_2().text_xs().text_color(t.muted).child(
                "Reads accounts and usage from your CLIProxyAPI hub so you don't add them by hand. The hub keeps renewing its own logins. SubHub changes account priority there for steering, and adds or removes logins when you ask below.",
            ))
            .child(row("Use the hub", None, Switch::new("hub-on").checked(enabled).on_click(move |v, _, cx| {
                let v = *v;
                st.update(cx, |s, cx| { s.set_hub_enabled(v); s.refresh(true, cx); cx.notify(); })
            }), t))
            .child(row("Status", None, div().text_sm().text_color(color).child(status), t))
            .when(enabled && matches!(s.hub, HubStatus::Ok(_)), |d| d.child(self.hub_logins(t, cx)))
            .child(row(
                "Steer new threads",
                Some("Puts the account whose weekly leftover expires first on top in the hub (Auto), or the one you marked Preferred on its account page. Affects new threads in every hub app (e.g. T3 Code). Off restores the original priorities."),
                Switch::new("steer-on").checked(steer_on).on_click({
                    let st = self.state.clone();
                    move |v, _, cx| {
                        let v = *v;
                        st.update(cx, |s, cx| s.set_steering_enabled(v, cx))
                    }
                }),
                t,
            ))
            .child(row("Steering now", None, div().text_sm().text_color(steer_color).child(steer_status), t))
            .child(row("Address", Some("Local by default; the Tailscale address works too."), div().w(px(260.0)).child(Input::new(&self.hub_url)), t))
            .when(crate::demo::on(), |d| {
                // Saving or removing would change the real Windows Credential Manager.
                d.child(row(
                    "Management key",
                    Some(crate::demo::NOT_AVAILABLE),
                    div()
                        .flex()
                        .gap_2()
                        .opacity(0.5)
                        .child(button("save-key", "Save", t, true))
                        .child(button("clear-key", "Remove", t, false)),
                    t,
                ))
            })
            .when(!crate::demo::on(), |d| d.child(row(
                "Management key",
                Some("Stored in Windows Credential Manager, never in the settings file."),
                div()
                    .flex()
                    .gap_2()
                    .child(div().w(px(180.0)).child(Input::new(&self.hub_key)))
                    .child(button("save-key", "Save", t, true).on_click(cx.listener(move |this, _, window, cx| {
                        let v = key_input.read(cx).value().to_string();
                        this.message = Some(match secrets::set_hub_key(&v) {
                            Ok(()) => "Key saved.".into(),
                            Err(e) => e.to_string(),
                        });
                        key_input.update(cx, |i, cx| i.set_value("", window, cx));
                        st_check.update(cx, |s, cx| s.refresh(true, cx));
                        cx.notify();
                    })))
                    .child(button("clear-key", "Remove", t, false).on_click(cx.listener(|this, _, _, cx| {
                        this.message = Some(match secrets::clear_hub_key() {
                            Ok(()) => "Key removed.".into(),
                            Err(e) => e.to_string(),
                        });
                        this.state.update(cx, |s, cx| s.refresh(true, cx));
                        cx.notify();
                    }))),
                t,
            )))
    }
}
