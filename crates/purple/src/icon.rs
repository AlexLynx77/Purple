//! The Purple icon, drawn in code (a violet pearl) so no image file has to ship with the program.

/// RGBA pixels of a `size` x `size` icon.
pub fn rgba(size: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    let c = size as f32 / 2.0;
    let radius = c - 1.0;
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            let distance = (dx * dx + dy * dy).sqrt();
            // One pixel of soft edge so the disc is not jagged.
            let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
            // Violet, lighter at the top.
            let t = (y as f32 / size as f32).clamp(0.0, 1.0);
            let lerp = |a: f32, b: f32| a + (b - a) * t;
            let (mut r, mut g, mut b) = (lerp(167.0, 109.0), lerp(139.0, 40.0), lerp(250.0, 217.0));
            // A small white highlight, upper left.
            let (hx, hy) = (dx + size as f32 * 0.16, dy + size as f32 * 0.18);
            let glow = (1.0 - (hx * hx + hy * hy).sqrt() / (size as f32 * 0.22)).clamp(0.0, 1.0);
            r += (255.0 - r) * glow * 0.8;
            g += (255.0 - g) * glow * 0.8;
            b += (255.0 - b) * glow * 0.8;
            pixels.extend_from_slice(&[r as u8, g as u8, b as u8, (coverage * 255.0) as u8]);
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::rgba;

    #[test]
    fn has_the_right_size_with_a_transparent_corner_and_an_opaque_centre() {
        let (size, px) = (32u32, rgba(32));
        assert_eq!(px.len(), (size * size * 4) as usize);
        assert_eq!(px[3], 0, "corner pixel is transparent");
        let centre = ((size / 2 * size + size / 2) * 4 + 3) as usize;
        assert_eq!(px[centre], 255, "centre pixel is opaque");
    }
}
