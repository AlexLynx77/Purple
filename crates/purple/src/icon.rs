//! The Purple logo: an isometric cube in violet glass, drawn in code so no image file ships.
//!
//! Three faces seen from above, the right one split along a diagonal into a light and a dark
//! facet. Each facet is a soft gradient; the outline and the seams between facets catch the light,
//! like the rims of a glass object. `build.rs` includes this file to make the `.exe` icon, so it
//! must not depend on anything.

type Point = (f32, f32);

// The hexagon outline (pointy top) around the centre C.
const TOP: Point = (0.5, 0.05);
const UPPER_RIGHT: Point = (0.89, 0.275);
const LOWER_RIGHT: Point = (0.89, 0.725);
const BOTTOM: Point = (0.5, 0.95);
const LOWER_LEFT: Point = (0.11, 0.725);
const UPPER_LEFT: Point = (0.11, 0.275);
const C: Point = (0.5, 0.5);

const OUTLINE: [Point; 6] = [TOP, UPPER_RIGHT, LOWER_RIGHT, BOTTOM, LOWER_LEFT, UPPER_LEFT];
const SEAMS: [(Point, Point); 4] = [(C, UPPER_LEFT), (C, UPPER_RIGHT), (C, BOTTOM), (C, LOWER_RIGHT)];

/// A facet: its corners, and the colours (RGBA) at its top and at its bottom.
struct Facet {
    corners: &'static [Point],
    top: [f32; 4],
    bottom: [f32; 4],
}

const FACETS: [Facet; 4] = [
    // Top face: the brightest, almost white where the light hits.
    Facet { corners: &[TOP, UPPER_RIGHT, C, UPPER_LEFT], top: [226.0, 216.0, 255.0, 245.0], bottom: [181.0, 156.0, 251.0, 238.0] },
    // Left face.
    Facet { corners: &[UPPER_LEFT, C, BOTTOM, LOWER_LEFT], top: [158.0, 120.0, 248.0, 228.0], bottom: [104.0, 62.0, 214.0, 235.0] },
    // Right face, upper facet: the light "pointer" slice.
    Facet { corners: &[C, UPPER_RIGHT, LOWER_RIGHT], top: [190.0, 166.0, 252.0, 232.0], bottom: [139.0, 100.0, 240.0, 232.0] },
    // Right face, lower facet: the shadow.
    Facet { corners: &[C, LOWER_RIGHT, BOTTOM], top: [96.0, 56.0, 200.0, 238.0], bottom: [62.0, 30.0, 150.0, 245.0] },
];

/// Whether `p` is inside the convex polygon `poly` (either winding), edges included.
fn inside(poly: &[Point], p: Point) -> bool {
    let mut sign = 0.0f32;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let cross = (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
        if cross.abs() < 1e-7 {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}

/// Distance from `p` to the segment `a`-`b`.
fn segment_distance(p: Point, a: Point, b: Point) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    let (x, y) = (a.0 + t * dx - p.0, a.1 + t * dy - p.1);
    (x * x + y * y).sqrt()
}

fn mix(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t]
}

/// The colour of the logo at `p` (unit square), or `None` outside it. `px` is one pixel in
/// unit lengths, so the rims keep a visible width at every size.
fn shade(p: Point, px: f32) -> Option<[f32; 4]> {
    let facet = FACETS.iter().find(|f| inside(f.corners, p))?;
    let (min_y, max_y) = facet.corners.iter().fold((1.0f32, 0.0f32), |(lo, hi), c| (lo.min(c.1), hi.max(c.1)));
    let mut color = mix(facet.top, facet.bottom, ((p.1 - min_y) / (max_y - min_y)).clamp(0.0, 1.0));

    // Glass rims: a light line along the outline, brighter towards the top.
    let rim_width = (0.035f32).max(1.2 * px);
    let to_outline = (0..6).map(|i| segment_distance(p, OUTLINE[i], OUTLINE[(i + 1) % 6])).fold(f32::MAX, f32::min);
    let rim = (1.0 - to_outline / rim_width).clamp(0.0, 1.0);
    let rim_strength = 0.75 - 0.4 * p.1;
    color = mix(color, [255.0, 255.0, 255.0, 255.0], rim * rim * rim_strength);

    // Seams between facets: thinner and softer than the rim.
    let seam_width = (0.012f32).max(0.7 * px);
    let to_seam = SEAMS.iter().map(|&(a, b)| segment_distance(p, a, b)).fold(f32::MAX, f32::min);
    let seam = (1.0 - to_seam / seam_width).clamp(0.0, 1.0);
    color = mix(color, [255.0, 255.0, 255.0, 255.0], seam * 0.45);

    // A soft specular glow on the upper left of the top face.
    let (gx, gy) = ((p.0 - 0.36) / 0.22, (p.1 - 0.2) / 0.1);
    let glow = (1.0 - (gx * gx + gy * gy).sqrt()).clamp(0.0, 1.0);
    color = mix(color, [255.0, 255.0, 255.0, 255.0], glow * glow * 0.35);
    Some(color)
}

/// RGBA pixels of a `size` x `size` icon.
pub fn rgba(size: u32) -> Vec<u8> {
    const SAMPLES: u32 = 4;
    let px = 1.0 / size as f32;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            // Premultiplied sum of the sub-samples, un-premultiplied once at the end.
            let mut acc = [0.0f32; 4];
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let p = (
                        (x as f32 + (sx as f32 + 0.5) / SAMPLES as f32) * px,
                        (y as f32 + (sy as f32 + 0.5) / SAMPLES as f32) * px,
                    );
                    if let Some(c) = shade(p, px) {
                        let a = c[3] / 255.0;
                        acc[0] += c[0] * a;
                        acc[1] += c[1] * a;
                        acc[2] += c[2] * a;
                        acc[3] += a;
                    }
                }
            }
            if acc[3] > 0.0 {
                let channel = |v: f32| (v / acc[3]).round().clamp(0.0, 255.0) as u8;
                let alpha = (acc[3] / (SAMPLES * SAMPLES) as f32 * 255.0).round() as u8;
                pixels.extend_from_slice(&[channel(acc[0]), channel(acc[1]), channel(acc[2]), alpha]);
            } else {
                pixels.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::rgba;

    #[test]
    fn has_the_right_size_with_a_transparent_corner_and_a_solid_centre() {
        let (size, px) = (32u32, rgba(32));
        assert_eq!(px.len(), (size * size * 4) as usize);
        let alpha = |x: u32, y: u32| px[((y * size + x) * 4 + 3) as usize];
        assert_eq!(alpha(0, 0), 0, "corner is transparent");
        assert!(alpha(size / 2, size / 2) > 200, "centre is nearly opaque");
    }

    /// `PURPLE_ICON_PREVIEW=<dir> cargo test -p purple icon` writes the icon at several sizes,
    /// and a sheet of the tray sizes on a light and a dark taskbar.
    #[test]
    fn preview() {
        let Some(dir) = std::env::var_os("PURPLE_ICON_PREVIEW") else { return };
        for size in [16u32, 32, 64, 256] {
            let path = std::path::Path::new(&dir).join(format!("icon-{size}.png"));
            image::save_buffer(path, &rgba(size), size, size, image::ColorType::Rgba8).unwrap();
        }
        let mut sheet = vec![0u8; 96 * 48 * 4];
        for (i, size) in [16u32, 24, 32].into_iter().enumerate() {
            let px = rgba(size);
            for (row, bg) in [(0u32, 243u8), (24, 32)] {
                for y in 0..24 {
                    for x in 0..32 {
                        let o = (((row + y) * 96 + i as u32 * 32 + x) * 4) as usize;
                        let mut c = [bg, bg, bg];
                        if x < size && y < size {
                            let p = ((y * size + x) * 4) as usize;
                            let a = f32::from(px[p + 3]) / 255.0;
                            for k in 0..3 {
                                c[k] = (f32::from(px[p + k]) * a + f32::from(bg) * (1.0 - a)) as u8;
                            }
                        }
                        sheet[o..o + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
                    }
                }
            }
        }
        let path = std::path::Path::new(&dir).join("icon-tray.png");
        image::save_buffer(path, &sheet, 96, 48, image::ColorType::Rgba8).unwrap();
    }
}
