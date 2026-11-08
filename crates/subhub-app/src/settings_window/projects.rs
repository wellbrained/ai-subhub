use super::*;

impl SettingsWindow {
    /// Projects: saved local folders shared by all accounts, in the order of the Launch menus.
    /// Flat rows (label + path), reorder, Show/Hide, rename, Remove (only the shortcut).
    pub(super) fn projects_page(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let list = self.state.read(cx).settings.projects.clone();
        let last = list.len().saturating_sub(1);
        let mut rows = div().flex().flex_col();
        for (i, p) in list.iter().enumerate() {
            let id = p.id;
            let editing = self.editing_project == Some(id);
            let confirming = self.confirm_remove_project == Some(id);
            let missing = !p.directory.is_dir();
            let state = self.state.clone();

            let title: AnyElement = if editing {
                div()
                    .w_full()
                    .max_w(px(320.0))
                    .child(Input::new(&self.project_label))
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .min_w_0()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .clip("project", SharedString::from(p.label.clone())),
                    )
                    .when(p.hidden, |d| d.child(tag("Hidden", t.muted, t)))
                    .when(missing, |d| d.child(tag("Folder missing", t.warn, t)))
                    .into_any_element()
            };

            let up_state = state.clone();
            let down_state = state.clone();
            let vis_state = state.clone();
            let rm_state = state.clone();
            let hidden = p.hidden;
            let label = p.label.clone();
            let remove_button = if confirming {
                button(("proj-rm", i), "Click again to remove", t, false).text_color(t.danger)
            } else {
                icon_button(("proj-rm", i), Icon::Trash, "Remove", t)
            };
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .py_3()
                    .border_b_1()
                    .border_color(t.border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .when(hidden, |d| d.opacity(0.6))
                            .child(title)
                            .child(div().min_w_0().text_xs().text_color(t.muted).clip(
                                "project-path",
                                SharedString::from(crate::demo::show_path(&p.directory)),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap_1()
                            .child(
                                square_button(("proj-up", i), Icon::ArrowUp, "Move up", i > 0, t)
                                    .on_click(move |_, _, cx| {
                                        up_state.update(cx, |s, cx| {
                                            s.move_project(id, true);
                                            cx.notify();
                                        })
                                    }),
                            )
                            .child(
                                square_button(
                                    ("proj-down", i),
                                    Icon::ArrowDown,
                                    "Move down",
                                    i < last,
                                    t,
                                )
                                .on_click(move |_, _, cx| {
                                    down_state.update(cx, |s, cx| {
                                        s.move_project(id, false);
                                        cx.notify();
                                    })
                                }),
                            )
                            .child(
                                icon_button(
                                    ("proj-vis", i),
                                    if hidden { Icon::Eye } else { Icon::EyeOff },
                                    if hidden { "Show" } else { "Hide" },
                                    t,
                                )
                                .on_click(move |_, _, cx| {
                                    vis_state.update(cx, |s, cx| {
                                        s.set_project_hidden(id, !hidden);
                                        cx.notify();
                                    })
                                }),
                            )
                            .child(
                                square_button(("proj-edit", i), Icon::Pencil, "Rename", true, t)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.commit_project_label(cx);
                                        this.editing_project = Some(id);
                                        this.confirm_remove_project = None;
                                        let text = label.clone();
                                        this.project_label
                                            .update(cx, |i, cx| i.set_value(text, window, cx));
                                        this.project_label
                                            .read(cx)
                                            .focus_handle(cx)
                                            .focus(window, cx);
                                        cx.notify();
                                    })),
                            )
                            .child(remove_button.on_click(cx.listener(move |this, _, _, cx| {
                                if this.confirm_remove_project == Some(id) {
                                    this.confirm_remove_project = None;
                                    rm_state.update(cx, |s, cx| {
                                        s.remove_project(id);
                                        cx.notify();
                                    });
                                } else {
                                    this.confirm_remove_project = Some(id);
                                }
                                cx.notify();
                            }))),
                    ),
            );
        }
        if list.is_empty() {
            rows = rows.child(
                div()
                    .py_4()
                    .text_sm()
                    .text_color(t.muted)
                    .child("No saved folders yet."),
            );
        }

        div()
            .flex()
            .flex_col()
            .child(heading("Projects"))
            .child(
                div().pb_3().text_xs().text_color(t.muted).child(
                    "Local folders shared by all accounts. Each account's Launch menu lists the visible ones in this order. Removing a project only removes the shortcut, never the folder.",
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .pb_2()
                    .child(
                        button("proj-add", "Add project", t, true)
                            .child(icon(Icon::FolderPlus, 12.0, gpui::white()))
                            .on_click(cx.listener(|this, _, _, cx| this.pick_project_folders(cx))),
                    )
            )
            .child(crate::format::capped("projects", list.len(), 52.0, rows))
    }
}
