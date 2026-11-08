//! The app's font: Geist (Vercel, SIL Open Font License 1.1, `assets/fonts/OFL.txt`), built into
//! the exe so it looks the same on every PC (owner, 2026-11-08). The taskbar badges keep Windows'
//! own "Segoe UI Variable Text" so they read as part of the taskbar.
use std::borrow::Cow;

use gpui::App;

/// Family name of the built-in font.
pub const UI: &str = "Geist";

/// Geist, variable weight (100-900).
const GEIST: &[u8] = include_bytes!("../assets/fonts/Geist-Variable.ttf");

/// The family the app's windows and hover cards use (`--demo-font` overrides it in the demo).
pub fn family() -> &'static str {
    crate::demo::font().unwrap_or(UI)
}

/// Registers the built-in font with the text system (once, at start).
pub fn load(cx: &mut App) {
    // On failure the windows fall back to the system font.
    let _ = cx.text_system().add_fonts(vec![Cow::Borrowed(GEIST)]);
    if let Some(bytes) = crate::demo::font_file().and_then(|p| std::fs::read(p).ok()) {
        // Demo font comparison: a font file that is not installed (`--demo-font-file`).
        let _ = cx.text_system().add_fonts(vec![Cow::Owned(bytes)]);
    }
}

/// Names the font on gpui-component's theme, which every `Root` window renders with. Called at
/// start and after each theme change (a theme change may load the theme's own font back).
pub fn apply(cx: &mut App) {
    gpui_component::Theme::global_mut(cx).font_family = family().into();
}
