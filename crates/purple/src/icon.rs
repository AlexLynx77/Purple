//! The Purple logo, drawn in code so no image file ships with the program.
//!
//! One of Star Guardian Syndra's orbs: a glowing violet sphere with a neon magenta rim, two
//! pointed ears that flare outwards, and a small, slightly mischievous face (slanted eyes and a
//! `w` mouth). Large sizes add specks of starlight inside the orb and a soft outer glow.
//! `build.rs` includes this file to make the `.exe` icon, so it must not depend on anything.

type Point = (f32, f32);
type Rgb = [f32; 3];

const CENTRE: Point = (0.5, 0.58);
const RADIUS: f32 = 0.34;
/// The ears, as triangles (outer base, tip, inner base): their bases are hidden in the orb.
const EARS: [[Point; 3]; 2] = [
    [(0.20, 0.47), (0.11, 0.16), (0.40, 0.31)],
    [(0.80, 0.47), (0.89, 0.16), (0.60, 0.31)],
];
/// Where the light comes from (upper left, a little towards the viewer).
const LIGHT: [f32; 3] = [-0.5, -0.65, 0.57];

const DEEP: Rgb = [52.0, 6.0, 92.0];
const BODY: Rgb = [142.0, 40.0, 208.0];
const NEON: Rgb = [238.0, 92.0, 255.0];
const FACE: Rgb = [255.0, 238.0, 253.0];
const FACE_GLOW: Rgb = [255.0, 120.0, 240.0];
const STARLIGHT: Rgb = [236.0, 205.0, 255.0];

/// Specks of starlight inside the orb: centre and radius.
const STARS: [(Point, f32); 8] = [
    ((0.35, 0.39), 0.007),
    ((0.57, 0.35), 0.005),
    ((0.72, 0.47), 0.006),
    ((0.27, 0.70), 0.005),
    ((0.42, 0.85), 0.006),
    ((0.63, 0.86), 0.005),
    ((0.77, 0.69), 0.007),
    ((0.49, 0.45), 0.004),
];

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn length(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

/// Distance from `p` to the segment `a`-`b`.
fn segment(p: Point, a: Point, b: Point) -> f32 {
    let (pa, ba) = ((p.0 - a.0, p.1 - a.1), (b.0 - a.0, b.1 - a.1));
    let h = ((pa.0 * ba.0 + pa.1 * ba.1) / (ba.0 * ba.0 + ba.1 * ba.1)).clamp(0.0, 1.0);
    length(pa.0 - ba.0 * h, pa.1 - ba.1 * h)
}

/// Whether `p` is inside the triangle `t` (either winding).
fn in_triangle(t: &[Point; 3], p: Point) -> bool {
    let side = |a: Point, b: Point| (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
    let (d1, d2, d3) = (side(t[0], t[1]), side(t[1], t[2]), side(t[2], t[0]));
    !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
}

/// Signed distance to the face (eyes and mouth): negative inside.
fn face(p: Point, with_mouth: bool) -> f32 {
    // Eyes: almond shapes whose top edge slopes down towards the nose, for a mischievous look.
    let mut d = f32::MAX;
    for side in [-1.0f32, 1.0] {
        let (cx, cy, rx, ry) = (0.5 + side * 0.118, 0.605, 0.072, 0.05);
        let (x, y) = (p.0 - cx, p.1 - cy);
        let ellipse = (length(x / rx, y / ry) - 1.0) * ry;
        // The cut line: higher on the outer side, lower towards the middle.
        let cut = (cy - 0.016 - x * side * 0.62) - p.1;
        d = d.min(ellipse.max(cut));
    }
    if with_mouth {
        // Mouth: a small `w`, two lower half-circles side by side.
        let (r, y0) = (0.022, 0.7);
        for cx in [0.5 - r, 0.5 + r] {
            let arc = if p.1 >= y0 {
                (length(p.0 - cx, p.1 - y0) - r).abs()
            } else {
                length((p.0 - cx).abs() - r, p.1 - y0)
            };
            d = d.min(arc - 0.008);
        }
    }
    d
}

/// The colour (RGB) and coverage of the logo at `p` in the unit square. `detail` adds the
/// mouth, the starlight and the outer glow, which are noise at tray sizes.
fn shade(p: Point, detail: bool) -> Option<(Rgb, f32)> {
    let (dx, dy) = ((p.0 - CENTRE.0) / RADIUS, (p.1 - CENTRE.1) / RADIUS);
    let r = length(dx, dy);
    let ear = EARS.iter().find(|ear| in_triangle(ear, p));

    let body = if r <= 1.0 {
        // Sphere: diffuse light, then the neon rim all around, brightest towards the light.
        let nz = (1.0 - r * r).sqrt();
        let diffuse = (dx * LIGHT[0] + dy * LIGHT[1] + nz * LIGHT[2]).max(0.0);
        let mut c = mix(DEEP, BODY, diffuse * 1.1);
        let towards_light = ((-dx * 0.6 - dy * 0.8) / r.max(1e-3)).max(0.0);
        let rim = smoothstep(0.78, 1.0, r) * (0.75 + 0.25 * towards_light);
        c = mix(c, NEON, rim);
        // A faint gloss on the upper left.
        let gloss = (1.0 - length((dx + 0.36) / 0.24, (dy + 0.5) / 0.13)).clamp(0.0, 1.0);
        c = mix(c, [255.0, 225.0, 255.0], gloss * 0.3);
        if detail {
            for &(s, sr) in &STARS {
                let d = length(p.0 - s.0, p.1 - s.1) / sr;
                c = mix(c, STARLIGHT, (1.0 - d).clamp(0.0, 1.0) * 0.85 + (1.0 - d / 3.0).clamp(0.0, 1.0) * 0.15);
            }
        }
        Some(c)
    } else if let Some(ear) = ear {
        // Ears: darker towards the tip, with the same neon along their edges.
        let tip = ear[1];
        let along = ((p.1 - tip.1) / (ear[0].1 - tip.1)).clamp(0.0, 1.0);
        let c = mix(DEEP, BODY, 0.25 + along * 0.55);
        let edge = segment(p, ear[0], tip).min(segment(p, tip, ear[2]));
        Some(mix(c, NEON, 1.0 - smoothstep(0.0, 0.035, edge)))
    } else {
        None
    };

    match body {
        Some(c) => {
            // The face glows: a bright core with a pink halo.
            let d = face(p, detail);
            let c = mix(c, FACE_GLOW, (1.0 - d.max(0.0) / 0.03).clamp(0.0, 1.0) * 0.55);
            Some((mix(c, FACE, smoothstep(0.004, -0.004, d)), 1.0))
        }
        None if detail => {
            // A soft neon glow around the orb and the ears.
            let ear_edge = EARS
                .iter()
                .map(|e| segment(p, e[0], e[1]).min(segment(p, e[1], e[2])))
                .fold(f32::MAX, f32::min);
            let edge = ((r - 1.0) * RADIUS).min(ear_edge);
            let glow = (1.0 - edge / 0.06).clamp(0.0, 1.0).powi(2) * 0.55;
            (glow > 0.0).then_some((NEON, glow))
        }
        None => None,
    }
}

/// RGBA pixels of a `size` x `size` icon. Small details are left out below 48 px.
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
                    if let Some((c, a)) = shade(p, size >= 48) {
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
        assert_eq!(alpha(0, size - 1), 0, "bottom-left corner is transparent");
        assert_eq!(alpha(size / 2, size / 2 + 2), 255, "the orb is opaque");
    }

    /// `PURPLE_ICON_PREVIEW=<dir> cargo test -p purple icon` writes the icon at several sizes,
    /// and a sheet of the tray sizes on a light and a dark taskbar.
    #[test]
    fn preview() {
        let Some(dir) = std::env::var_os("PURPLE_ICON_PREVIEW") else { return };
        for size in [16u32, 32, 64, 256, 512] {
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
