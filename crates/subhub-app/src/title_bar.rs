//! The merged title bar of the Statistics and Settings windows (owner decision 2026-11-07).
//!
//! The windows are created with `TitlebarOptions { appears_transparent: true }`, so Windows
//! draws no caption of its own and GPUI's Windows platform layer takes over the non-client hit
//! test. What it returns for a point comes from the elements under the mouse that carry a
//! `WindowControlArea`:
//!
//! - `Drag` -> `HTCAPTION`: Windows itself moves the window, maximizes on double click and
//!   offers Aero Snap (drag to an edge or corner).
//! - `Min` / `Max` / `Close` -> `HTMINBUTTON` / `HTMAXBUTTON` / `HTCLOSE`: GPUI performs the
//!   action on mouse up; hovering `HTMAXBUTTON` makes Windows 11 show the snap layouts flyout.
//! - Resizing at the edges: GPUI hit-tests the top edge itself (`HTTOP*`), the other edges are
//!   the window's resize frame (`WS_THICKFRAME`).
//!
//! Drag areas are never ancestors of buttons: an ancestor's hitbox also counts as hovered under
//! its children, which would turn a click on a button into a caption click. So the bar is built
//! from sibling pieces: title group (drag), tabs, spacer (drag), trailing controls, window
//! buttons.
use std::rc::Rc;

use gpui::{
    AnyElement, FontWeight, Hsla, IntoElement, ParentElement, SharedString, Stateful, Styled,
    Window, WindowControlArea, div, prelude::*, px,
};

use crate::icons::{Icon, icon};
use crate::logo;
use crate::theme::Tokens;

/// Height of the bar including its bottom border.
pub const HEIGHT: f32 = 46.0;
/// Width of each window button.
const BUTTON_W: f32 = 46.0;
/// Windows' close-button red, white glyph on hover.
const CLOSE_HOVER: u32 = 0xc42b1c;
const CLOSE_PRESSED: u32 = 0xa72619;

/// One window button (minimize, maximize/restore, close). It must not have a click handler of
/// its own: the action runs from the non-client mouse messages of its control area.
fn window_button(
    id: &'static str,
    glyph: Icon,
    area: WindowControlArea,
    close: bool,
    t: &Tokens,
) -> impl IntoElement {
    let hover_bg: Hsla = if close {
        gpui::rgb(CLOSE_HOVER).into()
    } else {
        t.surface
    };
    let pressed_bg: Hsla = if close {
        gpui::rgb(CLOSE_PRESSED).into()
    } else {
        t.surface_hover
    };
    let hover_fg: Hsla = if close {
        gpui::rgb(0xffffff).into()
    } else {
        t.text
    };
    div()
        .id(id)
        .group(id)
        .window_control_area(area)
        .w(px(BUTTON_W))
        .h_full()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .hover(move |s| s.bg(hover_bg))
        .active(move |s| s.bg(pressed_bg))
        .child(
            icon(glyph, 10.0, t.muted)
                .group_hover(id, move |s| s.text_color(hover_fg))
                .into_any_element(),
        )
}

/// The bar. `after_title` sits right of the title (tab switch); `trailing` is right-aligned
/// before the window buttons (range switch, action buttons). Pass only elements that are
/// controls: the empty space between them is the drag area.
pub fn bar(
    id: &'static str,
    title: &'static str,
    t: &Tokens,
    window: &Window,
    after_title: Option<AnyElement>,
    trailing: Vec<AnyElement>,
) -> AnyElement {
    let maximized = window.is_maximized();
    let (max_icon, max_id) = if maximized {
        (Icon::WinRestore, "win-restore")
    } else {
        (Icon::WinMaximize, "win-maximize")
    };
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .w_full()
        .h(px(HEIGHT))
        .bg(t.bg)
        .border_b_1()
        .border_color(t.border)
        .child(
            div()
                .window_control_area(WindowControlArea::Drag)
                .h_full()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(10.0))
                .pl(px(14.0))
                .pr(px(14.0))
                .child(logo::app_mark(26.0))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(SharedString::from(title)),
                ),
        )
        .children(after_title)
        .child(
            div()
                .window_control_area(WindowControlArea::Drag)
                .h_full()
                .flex_1()
                .min_w(px(12.0)),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.0))
                .pl(px(12.0))
                .pr(px(10.0))
                .children(trailing),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .h_full()
                .border_l_1()
                .border_color(t.border)
                .child(window_button(
                    "win-minimize",
                    Icon::WinMinimize,
                    WindowControlArea::Min,
                    false,
                    t,
                ))
                .child(window_button(
                    max_id,
                    max_icon,
                    WindowControlArea::Max,
                    false,
                    t,
                ))
                .child(window_button(
                    "win-close",
                    Icon::WinClose,
                    WindowControlArea::Close,
                    true,
                    t,
                )),
        )
        .into_any_element()
}

/// A segmented switch (tabs, range, breakdown): recessed track, the active option as a raised
/// chip; hover and pressed states on the others.
pub fn seg<T: PartialEq + Clone + 'static>(
    id: &'static str,
    options: &[(T, &'static str)],
    current: &T,
    t: &Tokens,
    on_pick: impl Fn(T, &mut gpui::App) + 'static,
) -> AnyElement {
    let on_pick = Rc::new(on_pick);
    let mut bar = div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(28.0))
        .gap(px(2.0))
        .p(px(2.0))
        .rounded(px(8.0))
        .bg(t.surface)
        .border_1()
        .border_color(t.border);
    for (i, (value, label)) in options.iter().enumerate() {
        let active = value == current;
        let value = value.clone();
        let on_pick = on_pick.clone();
        let (hover_bg, pressed_bg, text, text_hover) = (t.surface_hover, t.border, t.muted, t.text);
        bar = bar.child(
            div()
                .id((id, i))
                .h_full()
                .flex()
                .items_center()
                .px(px(10.0))
                .rounded(px(6.0))
                .text_xs()
                .border_1()
                .border_color(gpui::transparent_black())
                .cursor_pointer()
                .when(active, |d| {
                    d.bg(t.bg)
                        .border_color(t.border)
                        .text_color(t.text)
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .when(!active, |d| {
                    d.text_color(text)
                        .hover(move |s| s.bg(hover_bg).text_color(text_hover))
                        .active(move |s| s.bg(pressed_bg))
                })
                .child(*label)
                .on_click(move |_, _, cx| on_pick(value.clone(), cx)),
        );
    }
    bar.into_any_element()
}

/// A 28 px icon button for the bar.
pub fn icon_button(id: &'static str, glyph: Icon, t: &Tokens) -> Stateful<gpui::Div> {
    let (hover_bg, pressed_bg) = (t.surface, t.surface_hover);
    div()
        .id(id)
        .size(px(28.0))
        .flex_none()
        .rounded(px(7.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |s| s.bg(hover_bg))
        .active(move |s| s.bg(pressed_bg))
        .child(icon(glyph, 15.0, t.muted))
}
