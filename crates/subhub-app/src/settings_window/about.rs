use super::*;

impl SettingsWindow {
    /// About & updates: what SubHub is, then update mode, version, last check, Check now, the
    /// status with Install, and the settings file location. Messages come from `updater` and are
    /// already safe (no URLs, paths or tokens).
    pub(super) fn about_page(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::updater::{Phase, running_version};
        use subhub_core::settings::UpdateMode;
        let (mode, last_check, phase, available, downloaded, error, dev) = {
            let s = self.state.read(cx);
            (
                s.settings.updates.mode,
                s.settings.updates.last_check,
                s.update.phase,
                s.update.available.as_ref().map(|r| r.version),
                s.update.downloaded,
                s.update.error.clone(),
                s.update.dev_build,
            )
        };
        let busy = phase != Phase::Idle;
        let version = running_version();
        let last = match last_check {
            Some(at) => crate::format::ago(at, chrono::Utc::now()),
            None => "never".to_string(),
        };

        let state = self.state.clone();
        let mode_row = row(
            "Update mode",
            Some(
                "Auto checks about every 6 hours, downloads in the background and restarts SubHub when no window is open. Manual only tells you here and waits for Install.",
            ),
            chips(
                "update-mode",
                &[(UpdateMode::Auto, "Auto"), (UpdateMode::Manual, "Manual")],
                &mode,
                t,
                move |m, cx| {
                    state.update(cx, |s, cx| {
                        s.set_update_mode(m);
                        cx.notify();
                    })
                },
            ),
            t,
        );
        let version_row = row(
            "Current version",
            dev.then_some("Development build: updates are off"),
            div().text_sm().child(format!("v{version}")),
            t,
        );
        let check_row = row(
            "Last checked",
            None,
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(div().text_sm().text_color(t.muted).child(last))
                .child(
                    button(
                        "update-check",
                        if phase == Phase::Checking {
                            "Checking…"
                        } else {
                            "Check now"
                        },
                        t,
                        false,
                    )
                    .when(busy, |d| d.opacity(0.5))
                    .when(!busy, |d| {
                        d.on_click(cx.listener(|this, _, _, cx| {
                            this.state.update(cx, |s, cx| s.check_update(cx));
                        }))
                    }),
                ),
            t,
        );

        // Status line: what is happening, what is available, or the last problem.
        let (status, status_color, hint) = match (phase, available) {
            (Phase::Checking, _) => ("Checking…".to_string(), t.muted, None),
            (Phase::Downloading, v) => (
                format!(
                    "Downloading{}…",
                    v.map(|v| format!(" v{v}")).unwrap_or_default()
                ),
                t.muted,
                None,
            ),
            (Phase::Installing, v) => (
                format!(
                    "Installing{}…",
                    v.map(|v| format!(" v{v}")).unwrap_or_default()
                ),
                t.muted,
                Some("SubHub restarts in a moment."),
            ),
            (Phase::Idle, Some(v)) => {
                let hint = if dev {
                    None
                } else {
                    Some(match (mode, downloaded) {
                        (UpdateMode::Manual, _) => {
                            "Install downloads it, replaces SubHub and restarts it."
                        }
                        (UpdateMode::Auto, true) => {
                            "Downloaded. It installs when no popover, Settings or Statistics window is open and no sign-in is running."
                        }
                        (UpdateMode::Auto, false) => {
                            "It downloads in the background and installs when SubHub is idle."
                        }
                    })
                };
                (format!("Update available: v{v}"), t.accent_text(), hint)
            }
            (Phase::Idle, None) if last_check.is_some() && error.is_none() => {
                ("SubHub is up to date.".to_string(), t.muted, None)
            }
            (Phase::Idle, None) => (String::new(), t.muted, None),
        };
        let can_install = !dev && !busy && available.is_some();
        let install_label = if mode == UpdateMode::Auto {
            "Install now"
        } else {
            "Install"
        };
        let status_row = (!status.is_empty() || error.is_some()).then(|| {
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x_4()
                .gap_y_2()
                .py_3()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(240.0))
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .when(!status.is_empty(), |d| {
                            d.child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(status_color)
                                    .child(SharedString::from(status.clone())),
                            )
                        })
                        .when_some(hint, |d, h| {
                            d.child(div().text_xs().text_color(t.muted).child(h))
                        })
                        .when_some(error.clone(), |d, e| {
                            d.child(
                                div()
                                    .text_xs()
                                    .text_color(t.danger)
                                    .child(SharedString::from(e)),
                            )
                        }),
                )
                .when(can_install, |d| {
                    d.child(div().flex_none().ml_auto().child(
                        button("update-install", install_label, t, true).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.state.update(cx, |s, cx| s.install_update(cx));
                            },
                        )),
                    ))
                })
        });

        div()
            .flex()
            .flex_col()
            .child(heading("About & updates"))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .pb_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(t.muted)
                            .child("Built with Rust + GPUI. Free, open source (MIT), local: no account, backend or telemetry."),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(t.muted)
                            .child("Claude usage uses an undocumented endpoint and can break when the provider changes it."),
                    ),
            )
            .child(version_row)
            .when(crate::demo::on(), |d| {
                d.child(row(
                    "Updates",
                    Some("No checks, downloads or installs in the demo."),
                    div().text_sm().text_color(t.muted).child("Demo mode"),
                    t,
                ))
            })
            .when(!crate::demo::on(), |d| {
                d.child(mode_row)
                    .child(check_row)
                    .when_some(status_row, |d, r| d.child(r))
            })
            .child(block(
                "Settings file",
                Some("Preferences, accounts and the last readings; never keys or tokens."),
                div()
                    .text_xs()
                    .text_color(t.muted)
                    .child(crate::demo::show_path(&subhub_core::settings::default_path())),
                t,
            ))
    }
}
