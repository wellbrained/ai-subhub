//! The tray icon's right-click menu, drawn by SubHub in the app's look (theme, Geist, icons)
//! instead of Windows' own menu (owner, 2026-11-08). A small pop-up window at the pointer that
//! closes when it loses focus, on Esc, or once an item is picked. Keyboard: Up / Down, Enter.
//! When the window cannot be opened, the tray falls back to Windows' menu (`Tray::native_menu`).
use std::rc::Rc;

use gpui::{
    App, Context, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    Render, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};

use crate::icons::{Icon, icon};
use crate::state::AppState;
use crate::theme::{self, Tokens};
use crate::tray::TrayEvent;

/// Menu width (logical px).
pub const WIDTH: f32 = 220.0;
const ITEM_H: f32 = 32.0;
const SEP_H: f32 = 9.0;
const PAD: f32 = 4.0;

/// What runs when an action is picked (the tray event it stands for).
pub type OnPick = Rc<dyn Fn(TrayEvent, &mut App)>;

/// One menu line: an action, or a separator.
#[derive(Clone, Copy)]
enum Line {
    Item(&'static str, Icon, TrayEvent),
    Separator,
}

const LINES: [Line; 6] = [
    Line::Item("Open AI SubHub", Icon::AppWindow, TrayEvent::Open),
    Line::Item("Refresh usage", Icon::Refresh, TrayEvent::Refresh),
    Line::Item("Statistics", Icon::Chart, TrayEvent::Statistics),
    Line::Item("Settings", Icon::Settings, TrayEvent::Settings),
    Line::Separator,
    Line::Item("Quit", Icon::Power, TrayEvent::Quit),
];

/// Height of the menu (logical px).
pub fn height() -> f32 {
    let lines: f32 = LINES
        .iter()
        .map(|l| match l {
            Line::Item(..) => ITEM_H,
            Line::Separator => SEP_H,
        })
        .sum();
    lines + 2.0 * PAD
}

/// The actions in order (keyboard moves over these only).
fn items() -> Vec<(&'static str, Icon, TrayEvent)> {
    LINES
        .iter()
        .filter_map(|l| match *l {
            Line::Item(a, b, c) => Some((a, b, c)),
            Line::Separator => None,
        })
        .collect()
}

/// Top-left (logical px) of a menu of `w` x `h` opened at the pointer `at` (physical px) on a
/// monitor with work area `work` (physical px, left/top/right/bottom) and `scale`: above the
/// pointer (bottom edge at the pointer, like Windows' tray menus), below it when there is no
/// room above, and always inside the work area.
pub fn origin(
    at: (i32, i32),
    work: (i32, i32, i32, i32),
    scale: f32,
    w: f32,
    h: f32,
) -> (f32, f32) {
    let s = scale.max(0.5);
    let (px_, py) = (at.0 as f32 / s, at.1 as f32 / s);
    let (wl, wt, wr, wb) = (
        work.0 as f32 / s,
        work.1 as f32 / s,
        work.2 as f32 / s,
        work.3 as f32 / s,
    );
    let x = px_.min(wr - w).max(wl);
    let y = if py - h >= wt { py - h } else { py };
    (x, y.min(wb - h).max(wt))
}

pub struct TrayMenu {
    state: gpui::Entity<AppState>,
    focus: FocusHandle,
    /// Highlighted action (keyboard or pointer).
    hi: Option<usize>,
    on_pick: OnPick,
    _subs: Vec<gpui::Subscription>,
}

impl TrayMenu {
    pub fn new(
        state: gpui::Entity<AppState>,
        on_pick: OnPick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subs = vec![cx.observe_window_activation(window, |_, window, _| {
            if !window.is_window_active() && !crate::demo::hidden() {
                window.remove_window();
            }
        })];
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            state,
            focus,
            hi: None,
            on_pick,
            _subs: subs,
        }
    }

    fn pick(&mut self, ev: TrayEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.remove_window();
        let on_pick = self.on_pick.clone();
        // After this window is gone (the action may open or focus another one).
        cx.defer(move |cx| on_pick(ev, cx));
    }

    fn key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let n = items().len();
        match ev.keystroke.key.as_str() {
            "escape" => window.remove_window(),
            "down" => self.hi = Some(self.hi.map_or(0, |i| (i + 1) % n)),
            "up" => self.hi = Some(self.hi.map_or(n - 1, |i| (i + n - 1) % n)),
            "enter" | "space" => {
                if let Some(i) = self.hi {
                    let ev = items()[i].2;
                    self.pick(ev, window, cx);
                    return;
                }
            }
            _ => return,
        }
        cx.notify();
    }
}

impl Render for TrayMenu {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let d = self.state.read(cx).display().clone();
        let t = Tokens::new(theme::is_dark(d.theme, cx), &d.accent);
        let mut list = div().flex().flex_col().p(px(PAD));
        let mut index = 0;
        for line in LINES {
            match line {
                Line::Separator => {
                    list = list.child(
                        div()
                            .h(px(SEP_H))
                            .flex()
                            .items_center()
                            .px(px(8.0))
                            .child(div().h(px(1.0)).w_full().bg(t.border)),
                    );
                }
                Line::Item(label, ic, ev) => {
                    let i = index;
                    index += 1;
                    let hi = self.hi == Some(i);
                    let quit = matches!(ev, TrayEvent::Quit);
                    list = list.child(
                        div()
                            .id(("tray-menu", i))
                            .h(px(ITEM_H))
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .px(px(10.0))
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .when(hi, |d| d.bg(t.surface_hover))
                            .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                                if *over {
                                    this.hi = Some(i);
                                } else if this.hi == Some(i) {
                                    this.hi = None;
                                }
                                cx.notify();
                            }))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.pick(ev, window, cx);
                            }))
                            .child(icon(ic, 15.0, if quit { t.muted } else { t.text }))
                            .child(div().text_size(px(13.0)).child(label)),
                    );
                }
            }
        }
        div()
            .track_focus(&self.focus)
            .on_key_down(
                cx.listener(|this, ev: &KeyDownEvent, window, cx| this.key(ev, window, cx)),
            )
            .size_full()
            .bg(t.bg)
            .text_color(t.text)
            .font_family(crate::fonts::family())
            // Corners, outline and shadow come from DWM (window_style::round).
            .child(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: (i32, i32, i32, i32) = (0, 0, 1920, 1032);

    #[test]
    fn opens_above_the_pointer_inside_the_work_area() {
        let h = height();
        // Pointer on a bottom taskbar: bottom edge at the pointer, clamped into the work area.
        let (x, y) = origin((1500, 1050), WORK, 1.0, WIDTH, h);
        assert_eq!((x, y), (1500.0, 1032.0 - h));
        // Near the right edge: moved left so it fits.
        let (x, _) = origin((1910, 1050), WORK, 1.0, WIDTH, h);
        assert_eq!(x, 1920.0 - WIDTH);
    }

    #[test]
    fn opens_below_the_pointer_on_a_top_taskbar_and_scales() {
        let h = height();
        let work = (0, 48, 1920, 1080);
        assert_eq!(origin((800, 20), work, 1.0, WIDTH, h), (800.0, 48.0));
        // 150 %: physical pointer and work area to logical px.
        let (x, y) = origin((1500, 1500), (0, 0, 2880, 1548), 1.5, WIDTH, h);
        assert_eq!((x, y), (1000.0, 1000.0 - h));
    }

    #[test]
    fn height_counts_items_separator_and_padding() {
        assert_eq!(height(), 5.0 * ITEM_H + SEP_H + 2.0 * PAD);
        assert_eq!(items().len(), 5);
    }
}
