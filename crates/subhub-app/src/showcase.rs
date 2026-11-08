//! Demo only (`--demo --demo-hidden --demo-page taskbar`): the taskbar display and one hover
//! card drawn on a mock Windows taskbar in one ordinary window, for README screenshots. The real
//! display sits on the taskbar and is off in hidden demo runs (nothing may appear on the desktop),
//! so this view embeds the same two views (`TaskbarDisplay`, `HoverCard`) instead.
use gpui::{
    App, AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div,
    linear_color_stop, linear_gradient, px, rgb,
};

use crate::hover_card::HoverCard;
use crate::state::AppState;
use crate::taskbar::TaskbarDisplay;

/// Window size (logical px).
pub const WIDTH: f32 = 680.0;
pub const HEIGHT: f32 = 270.0;
/// Height of the mock taskbar (Windows 11: 48 px).
const BAR_H: f32 = 48.0;
/// Room right of the display for tray icons and the clock.
const TRAY_W: f32 = 150.0;

pub struct Showcase {
    display: Entity<TaskbarDisplay>,
    display_w: f32,
    card: Option<(Entity<HoverCard>, f32, f32)>,
}

impl Showcase {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let display_w = crate::taskbar::width_for(state.read(cx));
        let display = cx.new(|cx| TaskbarDisplay::showcase(state.clone(), cx));
        // The card of the first account on the taskbar display (the one under the pointer).
        let first = crate::taskbar::shown_ids(state.read(cx)).into_iter().next();
        let card = first.and_then(|id| crate::hover_card::showcase_card(state, id, cx));
        Self {
            display,
            display_w,
            card,
        }
    }
}

impl Render for Showcase {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let display_right = TRAY_W;
        let display_left = WIDTH - display_right - self.display_w;
        let glyph = || div().size(px(14.0)).rounded(px(3.0)).bg(rgb(0x5a5a60));
        let mut root = div()
            .relative()
            .size_full()
            .bg(linear_gradient(
                135.0,
                linear_color_stop(rgb(0x1d2b3d), 0.0),
                linear_color_stop(rgb(0x2b2142), 1.0),
            ))
            .child(
                // The taskbar: the display, then tray icons (SubHub's ring among them) and clock.
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(BAR_H))
                    .bg(rgb(0x1c1c1c))
                    .border_t_1()
                    .border_color(rgb(0x2e2e2e))
                    .child(
                        div()
                            .absolute()
                            .top(px(2.0))
                            .bottom_0()
                            .left(px(display_left))
                            .w(px(self.display_w))
                            .child(self.display.clone()),
                    )
                    .child(
                        div()
                            .absolute()
                            .right(px(12.0))
                            .top_0()
                            .bottom_0()
                            .flex()
                            .items_center()
                            .gap(px(12.0))
                            .child(glyph())
                            .child(crate::logo::app_mark(16.0, true))
                            .child(glyph())
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .items_end()
                                    .text_size(px(12.0))
                                    .line_height(px(16.0))
                                    .text_color(rgb(0xf2f2f2))
                                    .child("16:42")
                                    .child("08/11/2026"),
                            ),
                    ),
            );
        if let Some((card, w, h)) = &self.card {
            root = root.child(
                div()
                    .absolute()
                    .left(px((display_left - 24.0).max(12.0)))
                    .bottom(px(BAR_H + 10.0))
                    .w(px(*w))
                    .h(px(*h))
                    .rounded(px(8.0))
                    .overflow_hidden()
                    .border_1()
                    .border_color(rgb(0x3a3a40))
                    .shadow_lg()
                    .child(card.clone()),
            );
        }
        root
    }
}

/// Opens the showcase window off-screen (hidden demo runs only).
pub fn open(state: Entity<AppState>, cx: &mut App) {
    use gpui::{TitlebarOptions, WindowBounds, WindowOptions, size};
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(WIDTH), px(HEIGHT)), cx)),
        titlebar: Some(TitlebarOptions {
            title: Some("AI SubHub showcase".into()),
            appears_transparent: true,
            ..Default::default()
        }),
        show: false,
        focus: false,
        ..Default::default()
    };
    if let Ok(h) = cx.open_window(options, |_, cx| cx.new(|cx| Showcase::new(state, cx))) {
        let _ = h.update(cx, |_, window, cx| {
            crate::window_style::show_off_screen(
                window,
                cx,
                crate::demo::OFF_SCREEN,
                crate::demo::OFF_SCREEN,
                WIDTH,
                HEIGHT,
            );
        });
    }
}
