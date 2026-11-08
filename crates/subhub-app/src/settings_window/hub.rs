use super::*;

impl SettingsWindow {
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
                "Reads accounts and usage from your CLIProxyAPI hub so you don't add them by hand. The hub keeps renewing its own logins. The only thing SubHub changes there is account priority, for steering.",
            ))
            .child(row("Use the hub", None, Switch::new("hub-on").checked(enabled).on_click(move |v, _, cx| {
                let v = *v;
                st.update(cx, |s, cx| { s.set_hub_enabled(v); s.refresh(true, cx); cx.notify(); })
            }), t))
            .child(row("Status", None, div().text_sm().text_color(color).child(status), t))
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
