//! Small dialog after an update: "Updated to vX.Y.Z" with the patch notes of every version since
//! the last one seen (skipped versions too) and an OK button. Compact style, theme tokens, same
//! frameless window setup as the popover (`window_style::round` in `main`).
use gpui::{
    Context, Entity, FontWeight, IntoElement, ParentElement, Render, SharedString, Styled, Window,
    div, prelude::*, px,
};
use subhub_core::changelog::Section;
use subhub_core::update::Version;

use crate::state::AppState;
use crate::theme::{self, Tokens};

pub const WIDTH: f32 = 400.0;
const MIN_HEIGHT: f32 = 190.0;
const MAX_HEIGHT: f32 = 520.0;

/// A height that fits the notes without scrolling, within limits (longer notes scroll).
pub fn height_for(sections: &[Section]) -> f32 {
    // Title + button + padding; per version a header line (when several); per bullet its wrapped
    // lines at ~52 characters.
    let mut h = 128.0;
    for s in sections {
        if sections.len() > 1 {
            h += 28.0;
        }
        for b in &s.bullets {
            h += 20.0 * (b.chars().count().div_ceil(52).max(1) as f32) + 6.0;
        }
    }
    h.clamp(MIN_HEIGHT, MAX_HEIGHT)
}

pub struct WhatsNew {
    state: Entity<AppState>,
    version: Version,
    sections: Vec<Section>,
    focus: gpui::FocusHandle,
}

impl WhatsNew {
    pub fn new(
        state: Entity<AppState>,
        version: Version,
        sections: Vec<Section>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            state,
            version,
            sections,
            focus,
        }
    }
}

impl Render for WhatsNew {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let d = self.state.read(cx).display().clone();
        let t = Tokens::new(theme::is_dark(d.theme, cx), &d.accent);
        let several = self.sections.len() > 1;

        let mut notes = div().flex().flex_col().gap_3();
        for s in &self.sections {
            let mut block = div().flex().flex_col().gap(px(6.0));
            if several {
                let title = match &s.date {
                    Some(date) => format!("v{} · {date}", s.version),
                    None => format!("v{}", s.version),
                };
                block = block.child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.muted)
                        .child(SharedString::from(title)),
                );
            }
            for b in &s.bullets {
                block = block.child(
                    div()
                        .flex()
                        .gap_2()
                        .text_sm()
                        .child(
                            div()
                                .flex_none()
                                .mt(px(7.0))
                                .size(px(4.0))
                                .rounded_full()
                                .bg(t.faint),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(SharedString::from(b.clone())),
                        ),
                );
            }
            notes = notes.child(block);
        }

        div()
            .track_focus(&self.focus)
            .on_key_down(|ev: &gpui::KeyDownEvent, window, _cx| {
                if matches!(ev.keystroke.key.as_str(), "escape" | "enter") {
                    window.remove_window();
                }
            })
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .bg(t.bg)
            .text_color(t.text)
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(SharedString::from(format!("Updated to v{}", self.version))),
            )
            .child(
                div()
                    .id("whats-new-notes")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(notes),
            )
            .child(
                div().flex().justify_end().child(
                    div()
                        .id("whats-new-ok")
                        .px_4()
                        .py(px(5.0))
                        .rounded(px(8.0))
                        .text_xs()
                        .cursor_pointer()
                        .bg(t.accent)
                        .text_color(gpui::white())
                        .hover(|s| s.opacity(0.9))
                        .child("OK")
                        .on_click(|_, window, _| window.remove_window()),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(bullets: usize, len: usize) -> Section {
        Section {
            version: Version::parse("0.1.0").unwrap(),
            date: None,
            bullets: vec!["x".repeat(len); bullets],
        }
    }

    #[test]
    fn height_grows_with_notes_within_limits() {
        let small = height_for(&[section(3, 40)]);
        let longer = height_for(&[section(6, 120)]);
        assert!((MIN_HEIGHT..=MAX_HEIGHT).contains(&small));
        assert!(longer > small);
        assert_eq!(height_for(&[section(40, 200)]), MAX_HEIGHT);
        assert_eq!(height_for(&[]), MIN_HEIGHT);
    }
}
