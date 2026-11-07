//! The app logo in two forms (ADR 0001):
//! - **App icon** (window, title bar, exe, Start): `assets/logo.svg`, "Night glow" (owner pick,
//!   2026-11-07): the skeleton badge on a violet squircle, rendered with resvg.
//! - **Tray mark**: the same skeleton badge without tile or bar tracks, drawn on a 16-unit grid
//!   with 4x4 supersampling so it stays crisp at 16 px: orange dot with the tag pill overlapping
//!   its lower edge, a long blue bar over a short amber bar. Both are static, never live values.

/// The app icon SVG for 48 px and up (also served to GPUI as `icons/app-logo.svg`).
pub const APP_SVG: &[u8] = include_bytes!("../assets/logo.svg");
/// Small-size master of the app icon for 16-32 px: the same mark with fewer, thicker shapes and
/// no glow, so it stays recognisable in title bars and small icons (owner, 2026-11-07).
pub const APP_SMALL_SVG: &[u8] = include_bytes!("../assets/logo-small.svg");
/// Sizes up to this use the small master.
const SMALL_MAX: u32 = 32;

/// One shape in grid units: circle (cx, cy, r) or rounded rect (x, y, w, h, radius).
enum Shape {
    Circle(f32, f32, f32),
    Rect(f32, f32, f32, f32, f32),
}

/// Tray mark, painted back to front (the tag is drawn over the dot).
const SHAPES: [(Shape, [u8; 3], f32); 5] = [
    // 5-hour (long, blue) and weekly (short, amber).
    (
        Shape::Rect(9.2, 4.0, 6.0, 3.0, 1.5),
        [0x5b, 0x93, 0xf8],
        1.0,
    ),
    (
        Shape::Rect(9.2, 9.6, 3.2, 3.0, 1.5),
        [0xf5, 0xa5, 0x24],
        1.0,
    ),
    // Provider dot, lowered so the tag overlaps it.
    (Shape::Circle(4.5, 6.9, 3.1), [0xe0, 0x7a, 0x55], 1.0),
    // Cut-out ring separates tag from dot, like the real badge.
    (Shape::Rect(0.6, 9.0, 7.8, 4.8, 2.4), [0, 0, 0], 0.0),
    // Tag pill.
    (
        Shape::Rect(1.2, 9.6, 6.6, 3.6, 1.8),
        [0xe8, 0x59, 0x0c],
        1.0,
    ),
];

fn coverage(shape: &Shape, x: f32, y: f32) -> bool {
    match *shape {
        Shape::Circle(cx, cy, r) => (x - cx).powi(2) + (y - cy).powi(2) <= r * r,
        Shape::Rect(rx, ry, w, h, rad) => {
            // A radius larger than half the side would invert the clamp range below.
            let rad = rad.min(w / 2.0).min(h / 2.0);
            if x < rx || y < ry || x > rx + w || y > ry + h {
                return false;
            }
            // Distance into the rounded corners. max/min instead of clamp: with rad == side/2
            // the bounds can differ by float rounding (11.1 vs 11.099999) and clamp panics.
            let cx = x.max(rx + rad).min((rx + w - rad).max(rx + rad));
            let cy = y.max(ry + rad).min((ry + h - rad).max(ry + rad));
            (x - cx).powi(2) + (y - cy).powi(2) <= rad * rad
        }
    }
}

/// Straight (non-premultiplied) RGBA, `size` x `size`, transparent background.
pub fn rgba(size: u32) -> Vec<u8> {
    const SS: u32 = 4;
    let scale = 16.0 / size as f32;
    let mut out = vec![0u8; (size * size * 4) as usize];
    for py in 0..size {
        for px in 0..size {
            // Composite in premultiplied space, averaged over subsamples.
            let (mut r, mut g, mut b, mut a) = (0f32, 0f32, 0f32, 0f32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) * scale;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) * scale;
                    let (mut cr, mut cg, mut cb, mut ca) = (0f32, 0f32, 0f32, 0f32);
                    for (shape, rgb, alpha) in SHAPES.iter() {
                        if !coverage(shape, x, y) {
                            continue;
                        }
                        // alpha 0.0 = cut-out (clears what is below).
                        if *alpha == 0.0 {
                            (cr, cg, cb, ca) = (0.0, 0.0, 0.0, 0.0);
                            continue;
                        }
                        let s = |c: u8| c as f32 / 255.0 * alpha;
                        cr = s(rgb[0]) + cr * (1.0 - alpha);
                        cg = s(rgb[1]) + cg * (1.0 - alpha);
                        cb = s(rgb[2]) + cb * (1.0 - alpha);
                        ca = alpha + ca * (1.0 - alpha);
                    }
                    r += cr;
                    g += cg;
                    b += cb;
                    a += ca;
                }
            }
            let n = (SS * SS) as f32;
            let (r, g, b, a) = (r / n, g / n, b / n, a / n);
            let i = ((py * size + px) * 4) as usize;
            if a > 0.0 {
                out[i] = (r / a * 255.0).round().min(255.0) as u8;
                out[i + 1] = (g / a * 255.0).round().min(255.0) as u8;
                out[i + 2] = (b / a * 255.0).round().min(255.0) as u8;
                out[i + 3] = (a * 255.0).round() as u8;
            }
        }
    }
    out
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

/// Renders an SVG to `size` x `size` straight RGBA (falls back to the tray mark).
fn svg_rgba(svg: &[u8], size: u32) -> Vec<u8> {
    use resvg::{tiny_skia, usvg};
    let Ok(tree) = usvg::Tree::from_data(svg, &usvg::Options::default()) else {
        return rgba(size);
    };
    let Some(mut pixmap) = tiny_skia::Pixmap::new(size, size) else {
        return rgba(size);
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

/// The app icon in a title bar, `size` px square (the small master: title bars are 16-32 px).
pub fn app_mark(size: f32) -> gpui::Img {
    use gpui::{Styled, px};
    gpui::img("icons/app-logo-small.svg")
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
