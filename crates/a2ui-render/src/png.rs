//! Minimal PNG writer **and** reader, so the golden-image tests compare real
//! pixels instead of bytes that happen to depend on a zlib version.
//!
//! Writer: RGBA8, filter byte 0 per scanline, one zlib IDAT (flate2),
//! hand-rolled chunks + CRC32 — adapted from `apps/timeline` (same repo, MIT),
//! so the app stays small (VISION §8) and the output opens in any viewer.
//!
//! Reader: the subset this writer can produce, plus every PNG filter type
//! (0–4), because a decoder that only reads its own output proves nothing.
//! CRCs are verified on the way in: a golden that has been mangled by a
//! checkout filter or a bad copy fails loudly rather than comparing garbage.

use std::io::{Read, Write};
use std::path::Path;

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use orbclient::Color;

const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Encode an RGBA pixel buffer (row-major, `Color` = 0xAARRGGBB) as a PNG file.
pub fn write_png(path: &Path, width: u32, height: u32, pixels: &[Color]) -> std::io::Result<()> {
    if pixels.len() != (width as usize) * (height as usize) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("pixel buffer {} != {width}x{height}", pixels.len()),
        ));
    }
    let mut raw = Vec::with_capacity((width as usize * 4 + 1) * height as usize);
    for row in 0..height {
        raw.push(0u8); // filter type 0 (None)
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
    out.extend_from_slice(SIGNATURE);
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8; // bit depth
    ihdr[9] = 6; // color type: RGBA
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

/// A decoded image: row-major RGBA, same layout [`write_png`] takes.
#[derive(Clone, Debug, PartialEq)]
pub struct Png {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<Color>,
}

/// Read a PNG file into a pixel buffer.
pub fn read_png(path: &Path) -> std::io::Result<Png> {
    let bytes = std::fs::read(path)?;
    decode_png(&bytes).map_err(std::io::Error::other)
}

/// Decode a PNG that this crate's writer could have produced: 8-bit RGBA,
/// non-interlaced, any of the five scanline filters.
pub fn decode_png(bytes: &[u8]) -> Result<Png, String> {
    if !bytes.starts_with(SIGNATURE) {
        return Err("not a PNG (bad signature)".into());
    }
    let mut pos = SIGNATURE.len();
    let mut width = 0u32;
    let mut height = 0u32;
    let mut idat = Vec::new();
    let mut seen_ihdr = false;
    let mut seen_iend = false;

    while pos + 8 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]) as usize;
        let kind = [bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]];
        let start = pos + 8;
        let end = start.checked_add(len).ok_or("chunk length overflows")?;
        if end + 4 > bytes.len() {
            return Err("file ends inside a chunk".into());
        }
        let data = &bytes[start..end];
        let want = u32::from_be_bytes([bytes[end], bytes[end + 1], bytes[end + 2], bytes[end + 3]]);
        let got = crc32(&[&kind[..], data].concat());
        if got != want {
            let name = String::from_utf8_lossy(&kind);
            return Err(format!("CRC mismatch in {name} chunk: file is corrupt"));
        }
        match &kind {
            b"IHDR" => {
                if data.len() != 13 {
                    return Err("IHDR must be 13 bytes".into());
                }
                width = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
                height = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
                if data[8] != 8 || data[9] != 6 || data[10] != 0 || data[11] != 0 || data[12] != 0 {
                    return Err(format!(
                        "only 8-bit RGBA non-interlaced PNGs are supported (depth {}, color {}, interlace {})",
                        data[8], data[9], data[12]
                    ));
                }
                seen_ihdr = true;
            }
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => {
                seen_iend = true;
                break;
            }
            _ => {} // ancillary chunks (tEXt, gAMA, …) are fine to skip
        }
        pos = end + 4;
    }
    if !seen_ihdr {
        return Err("no IHDR chunk".into());
    }
    if !seen_iend {
        return Err("no IEND chunk".into());
    }
    if width == 0 || height == 0 {
        return Err("zero-sized image".into());
    }

    let mut raw = Vec::new();
    ZlibDecoder::new(&idat[..])
        .read_to_end(&mut raw)
        .map_err(|e| format!("IDAT inflate failed: {e}"))?;
    let stride = width as usize * 4;
    if raw.len() != (stride + 1) * height as usize {
        return Err(format!("IDAT is {} bytes, expected {}", raw.len(), (stride + 1) * height as usize));
    }

    // Un-filter scanline by scanline. `cur` is rebuilt in place so the Sub and
    // Paeth filters can read already-decoded neighbours.
    let mut out = vec![0u8; stride * height as usize];
    let mut prev = vec![0u8; stride];
    for row in 0..height as usize {
        let filter = raw[row * (stride + 1)];
        let src = &raw[row * (stride + 1) + 1..][..stride];
        let dst = row * stride;
        match filter {
            0 => out[dst..dst + stride].copy_from_slice(src),
            1 => {
                for i in 0..stride {
                    let a = if i >= 4 { out[dst + i - 4] as u32 } else { 0 };
                    out[dst + i] = (src[i] as u32 + a) as u8;
                }
            }
            2 => {
                for i in 0..stride {
                    out[dst + i] = (src[i] as u32 + prev[i] as u32) as u8;
                }
            }
            3 => {
                for i in 0..stride {
                    let a = if i >= 4 { out[dst + i - 4] as u32 } else { 0 };
                    out[dst + i] = (src[i] as u32 + (a + prev[i] as u32) / 2) as u8;
                }
            }
            4 => {
                for i in 0..stride {
                    let a = if i >= 4 { out[dst + i - 4] as u32 } else { 0 };
                    let b = prev[i] as u32;
                    let c = if i >= 4 { prev[i - 4] as u32 } else { 0 };
                    out[dst + i] = (src[i] as u32 + paeth(a, b, c)) as u8;
                }
            }
            other => return Err(format!("unknown PNG filter {other} on row {row}")),
        }
        prev.copy_from_slice(&out[dst..dst + stride]);
    }

    let pixels = out
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| Color::rgba(p[0], p[1], p[2], p[3]))
        .collect();
    Ok(Png { width, height, pixels })
}

/// The Paeth predictor. Signed arithmetic: `a + b - c` goes negative for real
/// pixel data, and a `u32` version panics in debug and wraps in release.
fn paeth(a: u32, b: u32, c: u32) -> u32 {
    let (a, b, c) = (i64::from(a), i64::from(b), i64::from(c));
    let p = a + b - c;
    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
    if pa <= pb && pa <= pc {
        a as u32
    } else if pb <= pc {
        b as u32
    } else {
        c as u32
    }
}
