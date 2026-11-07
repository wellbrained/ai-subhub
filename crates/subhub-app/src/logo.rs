//! The app logo: a static "skeleton badge" (ADR 0001, 2026-11-06). A Claude-orange provider dot
//! with the tag pill overlapping its lower edge (like the real taskbar badge), then a long blue
//! bar over a short amber bar. Never shows live values. Defined on a 16-unit grid and rendered
//! here with 4x4 supersampling, so tray, window and exe icons are the exact same mark.

/// One shape in grid units: circle (cx, cy, r) or rounded rect (x, y, w, h, radius).
enum Shape {
    Circle(f32, f32, f32),
    Rect(f32, f32, f32, f32, f32),
}

/// Painted back to front (the tag is drawn over the dot).
const SHAPES: [(Shape, [u8; 3], f32); 7] = [
    // Bar tracks.
    (
        Shape::Rect(8.0, 5.0, 7.0, 2.6, 1.3),
        [0x8a, 0x8a, 0x92],
        0.45,
    ),
    (
        Shape::Rect(8.0, 9.9, 7.0, 2.6, 1.3),
        [0x8a, 0x8a, 0x92],
        0.45,
    ),
    // 5-hour (long, blue) and weekly (short, amber).
    (
        Shape::Rect(8.0, 5.0, 6.4, 2.6, 1.3),
        [0x5b, 0x93, 0xf8],
        1.0,
    ),
    (
        Shape::Rect(8.0, 9.9, 2.2, 2.6, 1.3),
        [0xf5, 0x9e, 0x0b],
        1.0,
    ),
    // Provider dot, lowered so the tag overlaps it.
    (Shape::Circle(4.0, 7.3, 2.9), [0xd9, 0x77, 0x57], 1.0),
    // Thin ring in the background colour separates tag from dot, like the real badge.
    (
        Shape::Rect(1.0, 9.3, 6.0, 3.6, 1.8),
        [0x1c, 0x1c, 0x1f],
        0.0,
    ),
    // Tag pill.
    (
        Shape::Rect(1.4, 9.7, 5.2, 2.8, 1.4),
        [0xe6, 0x51, 0x00],
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

/// Window icon for GPUI (`WindowOptions.icon`).
pub fn window_icon() -> Option<std::sync::Arc<image::RgbaImage>> {
    image::RgbaImage::from_raw(64, 64, rgba(64)).map(std::sync::Arc::new)
}

/// Multi-size .ico (16–256 px, PNG entries) for the exe resource; written by `--write-icon`.
pub fn ico_bytes() -> Vec<u8> {
    let sizes = [16u32, 20, 24, 32, 48, 64, 256];
    let pngs: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&s| {
            let mut buf = std::io::Cursor::new(Vec::new());
            image::RgbaImage::from_raw(s, s, rgba(s))
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
