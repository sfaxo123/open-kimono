//! Minimal PNG writer for the workbench screenshot evidence: RGBA8, filter 0
//! per scanline, one zlib IDAT (flate2), hand-rolled chunks + CRC32. No image
//! crate — the app stays small (VISION §8) and the output is verifiable with
//! any PNG decoder.

use std::io::Write;
use std::path::Path;

use flate2::write::ZlibEncoder;
use flate2::Compression;
use orbclient::Color;

/// Encode an RGBA pixel buffer (row-major, `Color` = 0xAARRGGBB) as a PNG file.
pub fn write_png(path: &Path, width: u32, height: u32, pixels: &[Color]) -> std::io::Result<()> {
    if pixels.len() != (width as usize) * (height as usize) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("pixel buffer {} != {width}x{height}", pixels.len()),
        ));
    }

    // Raw scanlines: filter byte 0 + RGBA.
    let mut raw = Vec::with_capacity((width as usize * 4 + 1) * height as usize);
    for row in 0..height {
        raw.push(0u8);
        for c in &pixels[(row as usize * width as usize)..((row + 1) as usize * width as usize)] {
            raw.push(c.r());
            raw.push(c.g());
            raw.push(c.b());
            raw.push(c.a());
        }
    }
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&raw)?;
    let idat = enc.finish()?;

    let mut out = Vec::new();
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8; // bit depth
    ihdr[9] = 6; // color type RGBA
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    std::fs::write(path, out)
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

/// PNG/zip CRC-32 (reflected, polynomial 0xEDB88320).
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for b in data {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}
