//! Design tokens for the popover and settings window. Light and dark palettes plus a user accent;
//! the gpui-component theme is kept in the same mode so its controls match.
use gpui::{App, Hsla, Window, rgb};
use subhub_core::settings::ThemeMode;

#[derive(Clone, Copy)]
pub struct Tokens {
    pub dark: bool,
    pub bg: Hsla,
    pub surface: Hsla,
    pub surface_hover: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub faint: Hsla,
    pub track: Hsla,
    pub accent: Hsla,
    pub warn: Hsla,
    pub danger: Hsla,
    pub ok: Hsla,
}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

pub fn parse_hex(s: &str) -> Option<Hsla> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    u32::from_str_radix(s, 16).ok().map(|v| rgb(v).into())
}

impl Tokens {
    pub fn new(dark: bool, accent: &str) -> Self {
        let accent = parse_hex(accent).unwrap_or(c(0x3b82f6));
        if dark {
            Self {
                dark,
                bg: c(0x1c1c1f),
                surface: c(0x26262a),
                surface_hover: c(0x2e2e33),
                border: c(0x34343a),
                text: c(0xf4f4f5),
                muted: c(0xa1a1aa),
                faint: c(0x6b6b73),
                track: c(0x3a3a40),
                accent,
                warn: c(0xf59e0b),
                danger: c(0xef4444),
                ok: c(0x22c55e),
            }
        } else {
            Self {
                dark,
                bg: c(0xffffff),
                surface: c(0xf4f4f5),
                surface_hover: c(0xebebee),
                border: c(0xe4e4e7),
                text: c(0x18181b),
                muted: c(0x71717a),
                faint: c(0xa1a1aa),
                track: c(0xe4e4e7),
                accent,
                warn: c(0xd97706),
                danger: c(0xdc2626),
                ok: c(0x16a34a),
            }
        }
    }

    /// The accent as text on `bg`: darkened (light theme) or lightened (dark theme) until it has
    /// at least 4.5:1 contrast, so a user accent never makes selected controls unreadable.
    pub fn accent_text(&self) -> Hsla {
        let lum = |c: Hsla| {
            let r: gpui::Rgba = c.into();
            let f = |v: f32| {
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * f(r.r) + 0.7152 * f(r.g) + 0.0722 * f(r.b)
        };
        let bg = lum(self.bg);
        let contrast = |c: Hsla| {
            let l = lum(c);
            (l.max(bg) + 0.05) / (l.min(bg) + 0.05)
        };
        let mut c = self.accent;
        for _ in 0..30 {
            if contrast(c) >= 4.5 {
                break;
            }
            c.l = if self.dark {
                (c.l + 0.03).min(1.0)
            } else {
                (c.l - 0.03).max(0.0)
            };
        }
        c
    }

    /// Color for a "% left" value: accent, then warn at/below the threshold, danger at/below 10%.
    pub fn level(&self, remaining: f64, threshold: u8) -> Hsla {
        if remaining <= 10.0 {
            self.danger
        } else if remaining <= threshold as f64 {
            self.warn
        } else {
            self.accent
        }
    }
}

/// Resolves System to the OS appearance.
pub fn is_dark(mode: ThemeMode, cx: &App) -> bool {
    match mode {
        ThemeMode::Dark => true,
        ThemeMode::Light => false,
        ThemeMode::System => matches!(
            cx.window_appearance(),
            gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
        ),
    }
}

pub fn apply_component_theme(dark: bool, window: Option<&mut Window>, cx: &mut App) {
    let mode = if dark {
        gpui_component::ThemeMode::Dark
    } else {
        gpui_component::ThemeMode::Light
    };
    gpui_component::Theme::change(mode, window, cx);
    // Scrollbars are always visible when content scrolls (not only while scrolling); the
    // OS auto-hide setting would otherwise pick fading bars.
    gpui_component::Theme::set_scrollbar_mode(gpui_component::scroll::ScrollbarMode::Always, cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lum(c: Hsla) -> f32 {
        let r: gpui::Rgba = c.into();
        let f = |v: f32| {
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(r.r) + 0.7152 * f(r.g) + 0.0722 * f(r.b)
    }

    #[test]
    fn accent_text_keeps_contrast_in_both_themes() {
        for accent in ["#3b82f6", "#22c55e", "#f97316", "#14b8a6", "#8b5cf6"] {
            for dark in [true, false] {
                let t = Tokens::new(dark, accent);
                let (a, b) = (lum(t.accent_text()), lum(t.bg));
                let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                assert!(ratio >= 4.5, "{accent} dark={dark}: {ratio}");
            }
        }
    }
}
