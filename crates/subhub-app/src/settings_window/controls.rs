use super::*;

// ---------- building blocks ----------

pub(super) fn heading(text: &str) -> impl IntoElement {
    div()
        .pb_2()
        .text_lg()
        .font_weight(FontWeight::SEMIBOLD)
        .child(SharedString::from(text.to_string()))
}

pub(super) fn row(
    label: &str,
    hint: Option<&str>,
    control: impl IntoElement,
    t: &Tokens,
) -> impl IntoElement {
    div()
        .flex()
        // Wrap: when label + control don't fit side by side, the control moves onto its own
        // line, right-aligned, instead of spilling past the edge (chips clipped "25%").
        .flex_wrap()
        .items_center()
        .gap_x_4()
        .gap_y_2()
        .py_3()
        .border_b_1()
        .border_color(t.border)
        .child(
            div()
                .flex_1()
                .min_w(px(240.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(div().text_sm().child(SharedString::from(label.to_string())))
                .when_some(hint.map(|h| h.to_string()), |d, h| {
                    // Lines stay readable now that the column is wider.
                    d.child(
                        div()
                            .max_w(px(640.0))
                            .text_xs()
                            .text_color(t.muted)
                            .child(h),
                    )
                }),
        )
        .child(div().flex_none().ml_auto().child(control))
}

/// Like `row`, but the control sits under the label (wide controls: chips, lists).
pub(super) fn block(
    label: &str,
    hint: Option<&str>,
    content: impl IntoElement,
    t: &Tokens,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .py_3()
        .border_b_1()
        .border_color(t.border)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(div().text_sm().child(SharedString::from(label.to_string())))
                .when_some(hint.map(|h| h.to_string()), |d, h| {
                    d.child(div().text_xs().text_color(t.muted).child(h))
                }),
        )
        .child(content)
}

/// Segmented chips for an enum-like choice.
pub(super) fn chips<T: PartialEq + Clone + 'static>(
    id: &'static str,
    options: &[(T, &'static str)],
    current: &T,
    t: &Tokens,
    on_pick: impl Fn(T, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let on_pick = Rc::new(on_pick);
    let mut bar = div()
        .flex()
        .flex_none()
        .gap_1()
        .p(px(3.0))
        .rounded(px(10.0))
        .bg(t.surface);
    for (i, (value, label)) in options.iter().enumerate() {
        let active = value == current;
        let value = value.clone();
        let on_pick = on_pick.clone();
        bar = bar.child(
            div()
                .id((id, i))
                .px_3()
                .py(px(4.0))
                .rounded(px(7.0))
                .text_xs()
                .cursor_pointer()
                .when(active, |d| {
                    d.bg(t.bg)
                        .text_color(t.text)
                        .font_weight(FontWeight::MEDIUM)
                        .shadow_sm()
                })
                .when(!active, |d| {
                    d.text_color(t.muted).hover(|s| s.text_color(t.text))
                })
                .child(*label)
                .on_click(move |_, _, cx| on_pick(value.clone(), cx)),
        );
    }
    bar
}

pub(super) fn button(
    id: impl Into<gpui::ElementId>,
    label: &str,
    t: &Tokens,
    primary: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .px_3()
        .py(px(5.0))
        .rounded(px(8.0))
        .text_xs()
        .cursor_pointer()
        .when(primary, |d| {
            d.bg(t.accent)
                .text_color(gpui::white())
                .hover(|s| s.opacity(0.9))
        })
        .when(!primary, |d| {
            d.bg(t.surface)
                .text_color(t.text)
                .hover(|s| s.bg(t.surface_hover))
        })
        .child(SharedString::from(label.to_string()))
}

/// A secondary button with a drawn icon before the label.
pub(super) fn icon_button(
    id: impl Into<gpui::ElementId>,
    ic: Icon,
    label: &str,
    t: &Tokens,
) -> gpui::Stateful<gpui::Div> {
    button(id, label, t, false).child(icon(ic, 12.0, t.muted))
}

/// A small square icon-only button; `enabled` false dims it and ignores clicks' visual hover.
pub(super) fn square_button(
    id: impl Into<gpui::ElementId>,
    ic: Icon,
    tip: &'static str,
    enabled: bool,
    t: &Tokens,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(26.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(8.0))
        .bg(t.surface)
        .when(enabled, |d| {
            d.cursor_pointer().hover(|s| s.bg(t.surface_hover))
        })
        .when(!enabled, |d| d.opacity(0.35))
        .child(icon(ic, 13.0, t.muted))
        .tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(tip).build(window, cx))
}

pub(super) fn tag(text: &str, color: Hsla, t: &Tokens) -> impl IntoElement {
    div()
        .px(px(6.0))
        .py(px(1.0))
        .rounded(px(6.0))
        .bg(t.surface)
        .text_xs()
        .text_color(color)
        .child(SharedString::from(text.to_string()))
}

/// The account's taskbar tag as a coloured chip.
pub(super) fn tag_chip(text: &str, color: Hsla) -> impl IntoElement {
    div()
        .flex_none()
        .min_w(px(26.0))
        .px(px(5.0))
        .py(px(1.0))
        .rounded(px(6.0))
        .bg(color)
        .text_color(gpui::white())
        .text_size(px(10.0))
        .font_weight(FontWeight::SEMIBOLD)
        .flex()
        .justify_center()
        .child(SharedString::from(text.to_string()))
}

/// A section heading inside a page ("Popover", "Next to the tray").
pub(super) fn section(text: &str, t: &Tokens) -> impl IntoElement {
    div()
        .pt_5()
        .pb_1()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.faint)
        .child(SharedString::from(text.to_uppercase()))
}

/// A collapsible group header ("Advanced", "More options"): a chevron that points right while
/// closed and down while open, the label, and a muted summary of what is inside while closed.
/// The caller adds the click.
pub(super) fn disclosure(
    id: &'static str,
    label: &str,
    summary: &str,
    open: bool,
    t: &Tokens,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .mt_2()
        .py_2()
        .px_2()
        .mx(px(-8.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(|s| s.bg(t.surface))
        .active(|s| s.bg(t.surface_hover))
        .child(icon(
            if open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            },
            14.0,
            t.muted,
        ))
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(SharedString::from(label.to_string())),
        )
        .when(!open, |d| {
            d.child(
                div()
                    .min_w_0()
                    .text_xs()
                    .text_color(t.muted)
                    .clip("summary", SharedString::from(summary.to_string())),
            )
        })
}

/// "‹ Accounts" at the top of a page drilled into from the account list. The caller adds the
/// click.
pub(super) fn back_link(id: &'static str, label: &str, t: &Tokens) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .mb_3()
        .ml(px(-6.0))
        .pl(px(2.0))
        .pr_2()
        .py_1()
        .rounded(px(6.0))
        .text_sm()
        .text_color(t.muted)
        .cursor_pointer()
        .hover(|s| s.bg(t.surface).text_color(t.text))
        .active(|s| s.bg(t.surface_hover))
        .child(icon(Icon::ChevronLeft, 14.0, t.muted))
        .child(SharedString::from(label.to_string()))
}
