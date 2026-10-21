//! Embeds the logo and the version information in `purple.exe` (Windows resources).
//!
//! The icon comes from the same code that draws the tray icon (`src/icon.rs`). With the GNU
//! toolchain the resource compiler is `windres`, found next to the C compiler (`$CC`).

#[path = "src/icon.rs"]
#[allow(dead_code)]
mod icon;

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let ico = out.join("purple.ico");
    std::fs::write(&ico, ico_file(&[16, 24, 32, 48, 64, 256])).expect("writing the icon");

    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico.to_str().expect("UTF-8 OUT_DIR"))
        .set("ProductName", "Purple")
        .set("FileDescription", "Purple")
        .set("LegalCopyright", "Purple contributors");
    if let Some(windres) = windres_next_to_cc() {
        res.set_windres_path(windres.to_str().expect("UTF-8 path"));
    }
    if let Err(e) = res.compile() {
        // Not fatal: the program works without an embedded icon.
        println!("cargo:warning=no icon embedded in the executable ({e})");
    }
}

/// `windres.exe` in the directory of `$CC` (w64devkit), when there is one.
fn windres_next_to_cc() -> Option<PathBuf> {
    let cc = std::env::var("CC").ok()?;
    let windres = Path::new(&cc).with_file_name("windres.exe");
    windres.is_file().then_some(windres)
}

/// A `.ico` file with one image per size: PNG for 256 px, uncompressed bitmaps below.
fn ico_file(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&s| if s >= 256 { png(s) } else { dib(s) }).collect();
    let mut file = Vec::new();
    file.extend_from_slice(&[0, 0, 1, 0]);
    file.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&size, image) in sizes.iter().zip(&images) {
        let side = if size >= 256 { 0 } else { size as u8 }; // 0 means 256
        file.extend_from_slice(&[side, side, 0, 0]);
        file.extend_from_slice(&1u16.to_le_bytes()); // planes
        file.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        file.extend_from_slice(&(image.len() as u32).to_le_bytes());
        file.extend_from_slice(&offset.to_le_bytes());
        offset += image.len() as u32;
    }
    for image in images {
        file.extend_from_slice(&image);
    }
    file
}

fn png(size: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, size, size);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("PNG header");
    writer.write_image_data(&icon::rgba(size)).expect("PNG data");
    writer.finish().expect("PNG end");
    bytes
}

/// A 32-bit BGRA bitmap as stored in `.ico` files: bottom-up rows, then a 1-bit AND mask.
fn dib(size: u32) -> Vec<u8> {
    let rgba = icon::rgba(size);
    let mask_row = size.div_ceil(32) * 4;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&40u32.to_le_bytes());
    bytes.extend_from_slice(&(size as i32).to_le_bytes());
    bytes.extend_from_slice(&(2 * size as i32).to_le_bytes()); // colour + mask
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&32u16.to_le_bytes());
    bytes.extend_from_slice(&[0; 24]); // no compression, sizes and palette unused
    for y in (0..size).rev() {
        for x in 0..size {
            let p = ((y * size + x) * 4) as usize;
            bytes.extend_from_slice(&[rgba[p + 2], rgba[p + 1], rgba[p], rgba[p + 3]]);
        }
    }
    bytes.extend(std::iter::repeat_n(0u8, (mask_row * size) as usize)); // alpha does the masking
    bytes
}
