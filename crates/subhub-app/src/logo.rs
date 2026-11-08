//! The app logo, "Hub ring" (owner pick, 2026-11-08): a centre with three coloured arcs around
//! it, your accounts around one hub. Static, never live values (ADR 0001). The SVGs are
//! generated (see `docs/features/app.md`, "Logo and icons") and rendered with resvg.
//! - **App icon** (exe, Start, window icon): dark tile; `logo.svg` from 48 px, the thicker small
//!   master `logo-small.svg` up to 32 px.
//! - **Bare mark** (tray, title bar, popover header): the ring without a tile on a transparent
//!   background (owner, 2026-11-08), its centre light on dark backgrounds and dark on light ones
//!   (`tray-dark.svg` / `tray-light.svg`). The tray follows the Windows mode, the UI its theme.

/// The app icon SVG for 48 px and up (also served to GPUI as `icons/app-logo.svg`).
pub const APP_SVG: &[u8] = include_bytes!("../assets/logo.svg");
/// Small master of the app icon for 16-32 px: thicker arcs and wider gaps, dark tile.
pub const APP_SMALL_SVG: &[u8] = include_bytes!("../assets/logo-small.svg");
/// The bare mark (no tile) for dark and light backgrounds: tray and in-app.
pub const TRAY_DARK_SVG: &[u8] = include_bytes!("../assets/tray-dark.svg");
pub const TRAY_LIGHT_SVG: &[u8] = include_bytes!("../assets/tray-light.svg");
/// Sizes up to this use the small master.
const SMALL_MAX: u32 = 32;

/// The tray mark, `size` px square, straight (non-premultiplied) RGBA.
pub fn tray_rgba(size: u32, dark_taskbar: bool) -> Vec<u8> {
    svg_rgba(
        if dark_taskbar {
            TRAY_DARK_SVG
        } else {
            TRAY_LIGHT_SVG
        },
        size,
    )
}

/// The app icon, `size` px square: the small master up to 32 px, the full icon above;
/// straight (non-premultiplied) RGBA.
pub fn app_rgba(size: u32) -> Vec<u8> {
    svg_rgba(
        if size <= SMALL_MAX {
            APP_SMALL_SVG
        } else {
            APP_SVG
        },
        size,
    )
}

/// Renders an SVG to `size` x `size` straight RGBA (transparent if it cannot be read).
fn svg_rgba(svg: &[u8], size: u32) -> Vec<u8> {
    use resvg::{tiny_skia, usvg};
    let empty = vec![0u8; (size * size * 4) as usize];
    let Ok(tree) = usvg::Tree::from_data(svg, &usvg::Options::default()) else {
        return empty;
    };
    let Some(mut pixmap) = tiny_skia::Pixmap::new(size, size) else {
        return empty;
    };
    let k = size as f32 / tree.size().width();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(k, k),
        &mut pixmap.as_mut(),
    );
    pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

/// The logo in the app (title bar, popover header), `size` px square: the bare ring on a
/// transparent background, its centre light on dark backgrounds and dark on light ones.
pub fn app_mark(size: f32, dark_bg: bool) -> gpui::Img {
    use gpui::{Styled, px};
    gpui::img(if dark_bg {
        "icons/mark-dark.svg"
    } else {
        "icons/mark-light.svg"
    })
    .size(px(size))
    .flex_none()
}

/// Window icon for GPUI (`WindowOptions.icon`).
pub fn window_icon() -> Option<std::sync::Arc<image::RgbaImage>> {
    // Windows shows the window icon at 16-32 px (taskbar, alt-tab): the small master.
    image::RgbaImage::from_raw(64, 64, svg_rgba(APP_SMALL_SVG, 64)).map(std::sync::Arc::new)
}

/// Multi-size .ico (16–256 px, PNG entries) for the exe resource; written by `--write-icon`.
pub fn ico_bytes() -> Vec<u8> {
    let sizes = [16u32, 20, 24, 32, 48, 64, 256];
    let pngs: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&s| {
            let mut buf = std::io::Cursor::new(Vec::new());
            image::RgbaImage::from_raw(s, s, app_rgba(s))
                .expect("size matches")
                .write_to(&mut buf, image::ImageFormat::Png)
                .expect("png encode");
            buf.into_inner()
        })
        .collect();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (s, png) in sizes.iter().zip(&pngs) {
        let dim = if *s >= 256 { 0 } else { *s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for png in pngs {
        out.extend_from_slice(&png);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every logo SVG renders (a broken file would give a blank icon, not an error).
    #[test]
    fn all_svgs_render() {
        for svg in [APP_SVG, APP_SMALL_SVG, TRAY_DARK_SVG, TRAY_LIGHT_SVG] {
            let px = svg_rgba(svg, 32);
            assert!(px.chunks(4).any(|p| p[3] > 0), "blank render");
        }
    }

    /// The tray marks differ in the centre: light on a dark taskbar, dark on a light one.
    #[test]
    fn tray_centre_follows_the_taskbar() {
        let centre = |v: &[u8]| {
            let i = ((16 * 32 + 16) * 4) as usize;
            v[i] as u32 + v[i + 1] as u32 + v[i + 2] as u32
        };
        assert!(centre(&tray_rgba(32, true)) > 600);
        assert!(centre(&tray_rgba(32, false)) < 200);
    }
}
