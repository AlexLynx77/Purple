//! The Purple logo, drawn in code so no image file ships with the program.
//!
//! A dark violet orb with two little pointed ears, after the orbs of Star Guardian Syndra: a
//! shaded sphere, a pink rim light on its upper-left edge, a glossy highlight, and a small star.
//! `build.rs` includes this file to make the `.exe` icon, so it must not depend on anything.

type Point = (f32, f32);

const CENTRE: Point = (0.5, 0.57);
const RADIUS: f32 = 0.37;
/// The ears, as triangles: their tips lean outwards.
const EARS: [[Point; 3]; 2] = [
    [(0.205, 0.40), (0.22, 0.135), (0.42, 0.27)],
    [(0.795, 0.40), (0.78, 0.135), (0.58, 0.27)],
];
/// Where the light comes from (upper left, a little towards the viewer).
const LIGHT: [f32; 3] = [-0.55, -0.65, 0.52];

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Whether `p` is inside the triangle `t` (either winding).
fn in_triangle(t: &[Point; 3], p: Point) -> bool {
    let side = |a: Point, b: Point| (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
    let (d1, d2, d3) = (side(t[0], t[1]), side(t[1], t[2]), side(t[2], t[0]));
    !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
}

/// A four-pointed star centred on `c`, of half-size `r`: its coverage at `p` (0 to 1).
fn star(p: Point, c: Point, r: f32) -> f32 {
    let (dx, dy) = ((p.0 - c.0).abs() / r, (p.1 - c.1).abs() / r);
    (1.0 - (dx.sqrt() + dy.sqrt())).clamp(0.0, 1.0).powf(0.4)
}

/// The colour (RGB) and coverage of the logo at `p` in the unit square.
fn shade(p: Point, with_star: bool) -> Option<([f32; 3], f32)> {
    const DEEP: [f32; 3] = [30.0, 8.0, 52.0];
    const BODY: [f32; 3] = [88.0, 30.0, 128.0];
    const RIM: [f32; 3] = [255.0, 170.0, 240.0];

    let (dx, dy) = ((p.0 - CENTRE.0) / RADIUS, (p.1 - CENTRE.1) / RADIUS);
    let d2 = dx * dx + dy * dy;
    let color = if d2 <= 1.0 {
        // Sphere: diffuse light, then a rim light where the surface turns away (upper left).
        let nz = (1.0 - d2).sqrt();
        let diffuse = (dx * LIGHT[0] + dy * LIGHT[1] + nz * LIGHT[2]).max(0.0);
        let mut c = mix(DEEP, BODY, diffuse * 1.15);
        let facing_light = ((-dx * 0.6 - dy * 0.8) / d2.sqrt().max(1e-3)).max(0.0);
        let rim = (1.0 - nz).powf(0.9) * facing_light.powf(0.7);
        c = mix(c, RIM, rim * 1.1);
        // A soft violet bounce light on the lower right.
        let bounce = (1.0 - nz).powf(3.0) * ((dx * 0.5 + dy * 0.85) / d2.sqrt().max(1e-3)).max(0.0);
        c = mix(c, [150.0, 90.0, 230.0], bounce * 0.6);
        // Glossy highlight: a small soft oval on the upper left.
        let (hx, hy) = ((dx + 0.34) / 0.2, (dy + 0.46) / 0.12);
        let gloss = (1.0 - (hx * hx + hy * hy)).clamp(0.0, 1.0);
        mix(c, [255.0, 236.0, 255.0], gloss * 0.9)
    } else if let Some(ear) = EARS.iter().find(|ear| in_triangle(ear, p)) {
        // Ears: lit like the sphere's top, their outer edge catching the rim light.
        let tip = ear[1];
        let along = ((p.1 - tip.1) / (ear[0].1 - tip.1)).clamp(0.0, 1.0);
        let c = mix(BODY, DEEP, 0.35 + along * 0.5);
        // The left ear faces the light: its outer edge glows like the sphere's rim.
        let left = ear[0].0 < 0.5;
        let edge = (p.0 - ear[0].0.min(tip.0)).abs();
        let lit = if left { (1.0 - edge / 0.05).clamp(0.0, 1.0) } else { 0.0 };
        mix(c, RIM, lit * 0.85 + (1.0 - along) * 0.15)
    } else {
        [0.0; 3]
    };
    let inside = d2 <= 1.0 || EARS.iter().any(|ear| in_triangle(ear, p));
    let sparkle = if with_star { star(p, (0.86, 0.16), 0.11) } else { 0.0 };
    match (inside, sparkle > 0.0) {
        (true, _) => Some((mix(color, [255.0, 240.0, 255.0], sparkle), 1.0)),
        (false, true) => Some(([255.0, 214.0, 250.0], sparkle)),
        (false, false) => None,
    }
}

/// RGBA pixels of a `size` x `size` icon. The star is left out below 24 px, where it is noise.
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
                    if let Some((c, a)) = shade(p, size >= 24) {
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
