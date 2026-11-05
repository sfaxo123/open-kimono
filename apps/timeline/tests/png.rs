//! PNG writer tests: known CRC vectors, and a full structural + decompressed
//! round-trip of a written file (no external decoder needed to prove bytes).

use std::io::Read;

use flate2::read::ZlibDecoder;
use orbclient::Color;

use timeline::png::{crc32, write_png};

#[test]
fn crc32_known_vectors() {
    // The PNG spec's own IEND vector.
    assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    assert_eq!(crc32(b"IHDR"), 0xA8A1_AE0A);
    assert_eq!(crc32(b""), 0);
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
}

#[test]
fn write_png_round_trips() {
    let dir = std::env::temp_dir().join(format!("timeline-png-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.png");

    // 2x2: red, black / orange, transparent-black
    let px = vec![
        Color::rgb(255, 0, 0),
        Color::BLACK,
        Color::rgb(255, 140, 0),
        Color::rgba(0, 0, 0, 0),
    ];
    write_png(&path, 2, 2, &px).unwrap();

    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n");

    // IHDR chunk right after the signature.
    let ihdr_len = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    assert_eq!(ihdr_len, 13);
    assert_eq!(&bytes[12..16], b"IHDR");
    let ihdr = &bytes[16..29];
    assert_eq!(u32::from_be_bytes([ihdr[0], ihdr[1], ihdr[2], ihdr[3]]), 2);
    assert_eq!(u32::from_be_bytes([ihdr[4], ihdr[5], ihdr[6], ihdr[7]]), 2);
    assert_eq!(ihdr[8], 8); // bit depth
    assert_eq!(ihdr[9], 6); // RGBA
    // IHDR CRC covers type+data.
    let crc_at = 16 + ihdr_len;
    assert_eq!(&bytes[crc_at..crc_at + 4], &crc32(&bytes[12..crc_at]).to_be_bytes());

    // IDAT: find it, decompress, compare raw scanlines byte for byte.
    let idat_pos = bytes.windows(4).position(|w| w == b"IDAT").unwrap() - 4;
    let idat_len = u32::from_be_bytes([
        bytes[idat_pos],
        bytes[idat_pos + 1],
        bytes[idat_pos + 2],
        bytes[idat_pos + 3],
    ]) as usize;
    let idat = &bytes[idat_pos + 8..idat_pos + 8 + idat_len];
    let mut raw = Vec::new();
    ZlibDecoder::new(idat).read_to_end(&mut raw).unwrap();
    let expected: Vec<u8> = [
        0, 255, 0, 0, 255, 0, 0, 0, 255, // row0: filter, red RGBA, black RGBA
        0, 255, 140, 0, 255, 0, 0, 0, 0, // row1: filter, orange RGBA, transparent
    ]
    .to_vec();
    assert_eq!(raw, expected, "decompressed scanlines must match the input pixels");

    // IEND closes the file: 4-byte length, "IEND", 4-byte CRC.
    assert_eq!(&bytes[bytes.len() - 8..bytes.len() - 4], b"IEND");
    assert_eq!(&bytes[bytes.len() - 4..], &crc32(b"IEND").to_be_bytes());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn write_png_rejects_wrong_buffer_size() {
    let dir = std::env::temp_dir().join(format!("timeline-png-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let err = write_png(&dir.join("x.png"), 4, 4, &[Color::BLACK]).unwrap_err();
    assert!(err.kind() == std::io::ErrorKind::InvalidInput);
    let _ = std::fs::remove_dir_all(&dir);
}
