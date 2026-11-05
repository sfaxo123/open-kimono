//! PNG writer + decoder tests. The decoder is what makes the golden-image
//! tests real: it must read genuine PNGs, not just its own output.

use std::io::Write;

use flate2::write::ZlibEncoder;
use flate2::Compression;
use orbclient::Color;

use a2ui_render::png::{crc32, decode_png, read_png, write_png};

fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::rgba(r, g, b, a)
}

fn data_of(pixels: &[Color]) -> Vec<u32> {
    // `Color`'s PartialEq ignores alpha, so compare the raw words.
    pixels.iter().map(|c| c.data).collect()
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(&[&kind[..], data].concat()).to_be_bytes());
}

/// Assemble a PNG from already-filtered scanlines (`filter` byte + row bytes).
fn png_bytes(width: u32, height: u32, raw: &[u8], idat_split: usize) -> Vec<u8> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(raw).unwrap();
    let idat = enc.finish().unwrap();

    let mut out = Vec::new();
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = 6;
    chunk(&mut out, b"IHDR", &ihdr);
    // Real encoders split IDAT across chunks; the decoder must concatenate.
    let mut at = 0usize;
    while at < idat.len() {
        let n = idat_split.min(idat.len() - at);
        chunk(&mut out, b"IDAT", &idat[at..at + n]);
        at += n;
    }
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Encode `rows` with one filter type, the way a PNG encoder would.
fn filtered_rows(rows: &[Vec<u8>], filter: u8) -> Vec<u8> {
    let mut raw = Vec::new();
    let mut prev = vec![0u8; rows[0].len()];
    for row in rows {
        raw.push(filter);
        for i in 0..row.len() {
            let a = if i >= 4 { row[i - 4] } else { 0 };
            let b = prev[i];
            let c = if i >= 4 { prev[i - 4] } else { 0 };
            let pred: u32 = match filter {
                0 => 0,
                1 => a as u32,
                2 => b as u32,
                3 => (a as u32 + b as u32) / 2,
                4 => {
                    // Signed, like the spec: a + b - c goes negative on real data.
                    let (ai, bi, ci) = (i64::from(a), i64::from(b), i64::from(c));
                    let p = ai + bi - ci;
                    let (pa, pb, pc) = ((p - ai).abs(), (p - bi).abs(), (p - ci).abs());
                    if pa <= pb && pa <= pc {
                        ai as u32
                    } else if pb <= pc {
                        bi as u32
                    } else {
                        ci as u32
                    }
                }
                other => panic!("no such filter {other}"),
            };
            // The encoded byte is (original - predictor) mod 256.
            raw.push((i64::from(row[i]) - i64::from(pred)) as u8);
        }
        prev.copy_from_slice(row);
    }
    raw
}

fn rows_3x2() -> (u32, u32, Vec<Vec<u8>>) {
    (
        3,
        2,
        vec![
            vec![10, 20, 30, 255, 40, 50, 60, 128, 70, 80, 90, 255],
            vec![15, 25, 35, 255, 200, 100, 50, 0, 250, 250, 250, 255],
        ],
    )
}

fn flat(rows: &[Vec<u8>]) -> Vec<Color> {
    rows.iter()
        .flat_map(|r| r.as_chunks::<4>().0.iter().map(|p| rgba(p[0], p[1], p[2], p[3])))
        .collect()
}

#[test]
fn crc32_matches_known_vectors() {
    // The same values `python3 -c "import zlib; zlib.crc32(...)"` prints.
    assert_eq!(crc32(b""), 0);
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    let ihdr = [0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0];
    assert_eq!(crc32(&[b"IHDR".as_slice(), &ihdr].concat()), 0x1F15_C489);
}

#[test]
fn every_filter_type_decodes_back_to_the_same_pixels() {
    let (w, h, rows) = rows_3x2();
    let want = data_of(&flat(&rows));
    for filter in 0..=4u8 {
        let bytes = png_bytes(w, h, &filtered_rows(&rows, filter), 1024);
        let got = decode_png(&bytes).unwrap_or_else(|e| panic!("filter {filter}: {e}"));
        assert_eq!((got.width, got.height), (3, 2), "filter {filter}");
        assert_eq!(data_of(&got.pixels), want, "filter {filter} must round-trip");
    }
}

#[test]
fn split_idat_chunks_are_concatenated() {
    let (w, h, rows) = rows_3x2();
    let bytes = png_bytes(w, h, &filtered_rows(&rows, 4), 3);
    let got = decode_png(&bytes).expect("many small IDAT chunks");
    assert_eq!(data_of(&got.pixels), data_of(&flat(&rows)));
}

#[test]
fn writer_output_round_trips_through_the_decoder() {
    let (w, h) = (17u32, 9u32);
    let pixels: Vec<Color> = (0..(w * h))
        .map(|i| rgba(i as u8, (i * 3) as u8, (i * 7) as u8, (i % 256) as u8))
        .collect();
    let path = std::env::temp_dir().join(format!("a2ui-png-{}-{}.png", std::process::id(), h));
    write_png(&path, w, h, &pixels).expect("write");
    let back = read_png(&path).expect("read back");
    assert_eq!((back.width, back.height), (w, h));
    assert_eq!(data_of(&back.pixels), data_of(&pixels));
    std::fs::remove_file(&path).ok();
}

#[test]
fn corrupt_crc_is_refused_not_decoded() {
    let (w, h, rows) = rows_3x2();
    let mut bytes = png_bytes(w, h, &filtered_rows(&rows, 0), 1024);
    // Flip a byte inside the first IDAT's data, found by its chunk type.
    let at = bytes.windows(4).position(|w| w == b"IDAT").expect("an IDAT chunk");
    bytes[at + 6] ^= 0xFF;
    let err = decode_png(&bytes).expect_err("a mangled file must not decode");
    assert!(err.contains("CRC"), "unexpected error: {err}");
}

#[test]
fn truncated_and_non_rgba_files_are_refused() {
    let (w, h, rows) = rows_3x2();
    let bytes = png_bytes(w, h, &filtered_rows(&rows, 0), 1024);
    let err = decode_png(&bytes[..bytes.len() / 2]).expect_err("truncated file");
    assert!(err.contains("chunk") || err.contains("IEND"), "unexpected: {err}");

    // Same file, IHDR color type changed to 2 (RGB, no alpha).
    let mut other = bytes.clone();
    other[25] = 2;
    // The IHDR CRC no longer matches, which is itself a refusal.
    let err = decode_png(&other).expect_err("tampered IHDR");
    assert!(err.contains("CRC") || err.contains("RGBA"), "unexpected: {err}");

    assert!(decode_png(b"not a png at all").is_err());
}

#[test]
fn decodes_a_png_encoded_by_a_different_encoder() {
    // Produced by `python3` + zlib (level 9, filter 1 on every row) — not by
    // this crate's writer, so the decoder is reading real PNGs.
    let hex = "89504e470d0a1a0a0000000d494844520000000200000002080600000072b60d24\
               000000174944415478da63fccfc0f09ff13f432323900602464700424a07c04da4\
               d74a0000000049454e44ae426082";
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    let got = decode_png(&bytes).expect("python-encoded png");
    assert_eq!((got.width, got.height), (2, 2));
    assert_eq!(
        data_of(&got.pixels),
        data_of(&[rgba(255, 0, 0, 255), rgba(0, 255, 0, 128), rgba(0, 0, 255, 255), rgba(255, 255, 0, 64)])
    );
}

#[test]
fn decodes_a_screenshot_written_by_another_app() {
    // apps/timeline (task C4) writes its own PNGs; ours must read them.
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/workbench/C4-TIMELINE-DEMO.png");
    let got = read_png(&std::path::PathBuf::from(path)).expect("C4 screenshot must decode");
    assert_eq!((got.width, got.height), (480, 800));
    assert_eq!(got.pixels.len(), 480 * 800);
    // The timeline screen is black+orange on black: the top-left corner is the
    // background, and the frame must contain at least one orange pixel.
    assert_eq!((got.pixels[0].r(), got.pixels[0].g(), got.pixels[0].b()), (0, 0, 0));
    let orange = got.pixels.iter().filter(|c| c.r() > 200 && c.g() > 80 && c.b() < 60).count();
    assert!(orange > 100, "expected orange ink in the screenshot, found {orange} px");
}

#[test]
fn writer_rejects_a_buffer_that_is_not_the_image() {
    let path = std::env::temp_dir().join(format!("a2ui-png-bad-{}.png", std::process::id()));
    let err = write_png(&path, 4, 4, &[rgba(0, 0, 0, 255); 15]).expect_err("15 != 4x4");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert!(!path.exists(), "a failed write must leave no file");
}
