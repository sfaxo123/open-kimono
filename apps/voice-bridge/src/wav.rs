//! Minimal RIFF/WAVE reader and writer (no external deps).
//!
//! The reader accepts what the voice pipeline actually carries: PCM
//! (`WAVE_FORMAT_PCM` = 1), 16-bit, any channel count, any sample rate.
//! Everything else (float, 8-/24/32-bit, extensible headers) is rejected
//! with an explicit reason — the bridge never silently mis-reads audio.
//! The writer emits a canonical 44-byte-header PCM file.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

/// One decoded PCM WAV: raw little-endian sample bytes plus the format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcm {
    /// Sample rate in hertz.
    pub rate: u32,
    /// Channel count (mono = 1).
    pub channels: u16,
    /// Sample width in bytes (16-bit = 2).
    pub width: u8,
    /// Raw little-endian PCM bytes (the `data` chunk).
    pub samples: Vec<u8>,
}

impl Pcm {
    /// Little-endian i16 samples (only valid for `width == 2`; the bridge
    /// only handles 16-bit audio).
    pub fn i16_samples(&self) -> impl Iterator<Item = i16> + '_ {
        self.samples.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]))
    }

    /// Root-mean-square level of the i16 samples, normalized to 0.0..=1.0.
    /// Used for the per-chunk `MicFrame` level meter (VISION §3: the user
    /// always sees whether the mic is live).
    pub fn rms_level(&self) -> f32 {
        chunk_rms_level(&self.samples)
    }
}

/// RMS level (0.0..=1.0) of a 16-bit little-endian PCM byte slice.
/// Empty input is silence (0.0). Odd trailing bytes are ignored.
pub fn chunk_rms_level(bytes: &[u8]) -> f32 {
    let mut sum: u64 = 0;
    let mut n: u64 = 0;
    for b in bytes.chunks_exact(2) {
        let s = i16::from_le_bytes([b[0], b[1]]) as i64;
        sum += (s * s) as u64;
        n += 1;
    }
    if n == 0 {
        return 0.0;
    }
    // f64 intermediate: sum can exceed f32's exact-integer range.
    (((sum as f64) / (n as f64)).sqrt() / 32768.0) as f32
}

/// Read a PCM WAV file. Errors carry an explicit reason.
pub fn read_pcm(path: impl AsRef<Path>) -> Result<Pcm, String> {
    let file = File::open(path.as_ref())
        .map_err(|e| format!("{}: {e}", path.as_ref().display()))?;
    let mut r = BufReader::new(file);
    read_pcm_from(&mut r).map_err(|e| format!("{}: {e}", path.as_ref().display()))
}

fn read_u32_le<R: Read>(r: &mut R) -> Result<u32, String> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b).map_err(|_| "file ends mid-header".to_string())?;
    Ok(u32::from_le_bytes(b))
}

fn read_u16_le<R: Read>(r: &mut R) -> Result<u16, String> {
    let mut b = [0u8; 2];
    r.read_exact(&mut b).map_err(|_| "file ends mid-header".to_string())?;
    Ok(u16::from_le_bytes(b))
}

fn read_pcm_from<R: Read + Seek>(r: &mut R) -> Result<Pcm, String> {
    let mut tag = [0u8; 4];
    r.read_exact(&mut tag).map_err(|_| "empty file: not a WAV".to_string())?;
    if &tag != b"RIFF" {
        return Err(format!("expected RIFF tag, got {:?}", String::from_utf8_lossy(&tag)));
    }
    let riff_size = read_u32_le(r)?;
    let _ = riff_size; // chunk walking does not trust the total size
    r.read_exact(&mut tag).map_err(|_| "file ends before WAVE tag".to_string())?;
    if &tag != b"WAVE" {
        return Err(format!("expected WAVE tag, got {:?}", String::from_utf8_lossy(&tag)));
    }

    let mut fmt: Option<(u16, u16, u32, u16, u16)> = None; // (format, channels, rate, block, bits)
    let mut data: Option<Vec<u8>> = None;

    // Walk the chunk list; skip unknown chunks (LIST, fact, …).
    loop {
        match r.read_exact(&mut tag) {
            Ok(()) => {}
            Err(_) => break, // clean end of chunk list
        }
        let size = read_u32_le(r)? as u64;
        if &tag == b"fmt " {
            let format = read_u16_le(r)?;
            let channels = read_u16_le(r)?;
            let rate = read_u32_le(r)?;
            let _byte_rate = read_u32_le(r)?;
            let block_align = read_u16_le(r)?;
            let bits = read_u16_le(r)?;
            fmt = Some((format, channels, rate, block_align, bits));
            // Any fmt extension bytes (WAVE_FORMAT_EXTENSIBLE) are skipped.
            if size > 16 {
                let skip = (size - 16 + 1) & !1; // word-aligned
                r.seek(SeekFrom::Current(skip as i64))
                    .map_err(|_| "cannot skip fmt extension".to_string())?;
            }
        } else if &tag == b"data" {
            let remaining = (size + 1) & !1; // RIFF chunks are word-aligned
            let mut buf = Vec::new();
            // Guard against a bogus huge declared size on a short file.
            let mut take = remaining;
            let mut tmp = [0u8; 8192];
            while take > 0 {
                let want = std::cmp::min(take as usize, tmp.len());
                match r.read(&mut tmp[..want]) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                        take -= n as u64;
                    }
                    Err(_) => break,
                }
            }
            // Drop the alignment pad byte from the sample data.
            if size % 2 == 1 && buf.len() as u64 > size {
                buf.truncate(size as usize);
            }
            let got = buf.len() as i64;
            data = Some(buf);
            if remaining > 0 {
                r.seek(SeekFrom::Current(remaining as i64 - got))
                    .map_err(|_| "cannot seek past data chunk".to_string())?;
            }
        } else {
            // Skip unknown chunks; RIFF chunks are word-aligned, so an odd
            // size carries a pad byte that must be skipped too.
            let skip = (size + 1) & !1;
            r.seek(SeekFrom::Current(skip as i64))
                .map_err(|_| format!("cannot skip chunk {:?}", String::from_utf8_lossy(&tag)))?;
        }
    }

    let (format, channels, rate, block_align, bits) =
        fmt.ok_or_else(|| "no fmt chunk".to_string())?;
    if format != 1 {
        return Err(format!(
            "only PCM (format 1) WAVs are supported, got format {format} \
             (float/extensible/compressed are rejected)"
        ));
    }
    if bits != 16 {
        return Err(format!("only 16-bit samples are supported, got {bits}-bit"));
    }
    if channels == 0 {
        return Err("fmt chunk declares 0 channels".to_string());
    }
    if block_align as u32 != channels as u32 * 2 {
        return Err(format!(
            "block align {block_align} does not match {channels} channels x 16-bit"
        ));
    }
    let samples = data.ok_or_else(|| "no data chunk".to_string())?;
    if samples.is_empty() {
        return Err("data chunk is empty".to_string());
    }
    if samples.len() % (channels as usize * 2) != 0 {
        return Err("data chunk length is not a whole number of frames".to_string());
    }

    Ok(Pcm { rate, channels, width: 2, samples })
}

/// Write a canonical PCM WAV file (44-byte header + data).
pub fn write_pcm(
    path: impl AsRef<Path>,
    rate: u32,
    width: u8,
    channels: u16,
    samples: &[u8],
) -> std::io::Result<()> {
    if width != 2 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "write_pcm only supports 16-bit samples",
        ));
    }
    let mut f = File::create(path)?;
    let byte_rate = rate * channels as u32 * width as u32;
    let block_align = channels * width as u16;
    let data_len = samples.len() as u32;
    let pad = (data_len % 2) as u32;

    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len + pad).to_le_bytes())?;
    f.write_all(b"WAVE")?;
    f.write_all(b"fmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?; // PCM
    f.write_all(&channels.to_le_bytes())?;
    f.write_all(&rate.to_le_bytes())?;
    f.write_all(&byte_rate.to_le_bytes())?;
    f.write_all(&block_align.to_le_bytes())?;
    f.write_all(&(width as u16 * 8).to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    f.write_all(samples)?;
    if pad == 1 {
        f.write_all(&[0u8])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("vb-wav-{}-{name}", std::process::id()));
        p
    }

    /// RIFF/WAVE header + a valid 16 kHz mono 16-bit fmt chunk; the caller
    /// appends further chunks after this.
    fn wav_prefix() -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(b"RIFF");
        body.extend_from_slice(&0u32.to_le_bytes()); // size not trusted by the reader
        body.extend_from_slice(b"WAVE");
        body.extend_from_slice(b"fmt ");
        body.extend_from_slice(&16u32.to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes()); // PCM
        body.extend_from_slice(&1u16.to_le_bytes()); // mono
        body.extend_from_slice(&16000u32.to_le_bytes());
        body.extend_from_slice(&32000u32.to_le_bytes());
        body.extend_from_slice(&2u16.to_le_bytes());
        body.extend_from_slice(&16u16.to_le_bytes());
        body
    }

    #[test]
    fn write_then_read_roundtrips() {
        let path = temp_path("roundtrip.wav");
        let samples: Vec<u8> = (0..100i16).flat_map(|s| s.to_le_bytes()).collect();
        write_pcm(&path, 22050, 2, 1, &samples).unwrap();
        let pcm = read_pcm(&path).unwrap();
        assert_eq!(pcm.rate, 22050);
        assert_eq!(pcm.channels, 1);
        assert_eq!(pcm.width, 2);
        assert_eq!(pcm.samples, samples);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn odd_unknown_chunk_is_skipped_with_pad() {
        // A 3-byte "note" chunk carries a RIFF pad byte; the reader must
        // skip 4 bytes so the following `data` chunk is still found.
        let mut body = wav_prefix();
        body.extend_from_slice(b"note");
        body.extend_from_slice(&3u32.to_le_bytes());
        body.extend_from_slice(&[7, 7, 7, 0]); // 3 bytes + pad
        body.extend_from_slice(b"data");
        body.extend_from_slice(&4u32.to_le_bytes());
        body.extend_from_slice(&[1, 0, 2, 0]);
        let pcm = read_pcm_from(&mut Cursor::new(body)).unwrap();
        assert_eq!(pcm.samples, vec![1, 0, 2, 0]);
    }

    #[test]
    fn rejects_odd_data_length() {
        // 16-bit PCM frames are 2 bytes; an odd data chunk is corrupt.
        let mut body = wav_prefix();
        body.extend_from_slice(b"data");
        body.extend_from_slice(&3u32.to_le_bytes());
        body.extend_from_slice(&[1, 0, 2]);
        let err = read_pcm_from(&mut Cursor::new(body)).unwrap_err();
        assert!(err.contains("whole number of frames"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_float_format() {
        let mut body = Vec::new();
        body.extend_from_slice(b"RIFF");
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(b"WAVE");
        body.extend_from_slice(b"fmt ");
        body.extend_from_slice(&16u32.to_le_bytes());
        body.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&16000u32.to_le_bytes());
        body.extend_from_slice(&32000u32.to_le_bytes());
        body.extend_from_slice(&2u16.to_le_bytes());
        body.extend_from_slice(&16u16.to_le_bytes());
        body.extend_from_slice(b"data");
        body.extend_from_slice(&4u32.to_le_bytes());
        body.extend_from_slice(&[0, 0, 0, 0]);
        let err = read_pcm_from(&mut Cursor::new(body)).unwrap_err();
        assert!(err.contains("format 3"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_8_bit() {
        let mut body = Vec::new();
        body.extend_from_slice(b"RIFF");
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(b"WAVE");
        body.extend_from_slice(b"fmt ");
        body.extend_from_slice(&16u32.to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&16000u32.to_le_bytes());
        body.extend_from_slice(&16000u32.to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&8u16.to_le_bytes());
        body.extend_from_slice(b"data");
        body.extend_from_slice(&2u32.to_le_bytes());
        body.extend_from_slice(&[0, 0]);
        let err = read_pcm_from(&mut Cursor::new(body)).unwrap_err();
        assert!(err.contains("16-bit"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_non_wave() {
        let err = read_pcm_from(&mut Cursor::new(b"NOTAFILE")).unwrap_err();
        assert!(err.contains("RIFF"), "unexpected error: {err}");
    }

    #[test]
    fn skips_unknown_chunks_and_reads_data() {
        // RIFF with a LIST chunk between fmt and data.
        let mut body = wav_prefix();
        body.extend_from_slice(b"LIST");
        body.extend_from_slice(&8u32.to_le_bytes());
        body.extend_from_slice(b"INFO");
        body.extend_from_slice(&[0, 0, 0, 0]);
        body.extend_from_slice(b"data");
        body.extend_from_slice(&4u32.to_le_bytes());
        body.extend_from_slice(&[1, 0, 2, 0]);
        let pcm = read_pcm_from(&mut Cursor::new(body)).unwrap();
        assert_eq!(pcm.samples, vec![1, 0, 2, 0]);
    }

    #[test]
    fn rms_level_is_normalized_and_exact() {
        // Full scale on every sample → 1.0.
        let full: Vec<u8> = vec![0u8, 0x80].repeat(4); // i16::MIN x4
        assert!((chunk_rms_level(&full) - 1.0).abs() < 1e-6);
        // Half scale constant → 0.5.
        let half: Vec<u8> = (0..4).flat_map(|_| (-16384i16).to_le_bytes()).collect();
        assert!((chunk_rms_level(&half) - 0.5).abs() < 1e-6);
        // Silence and empty → 0.0.
        assert_eq!(chunk_rms_level(&[0, 0, 0, 0]), 0.0);
        assert_eq!(chunk_rms_level(&[]), 0.0);
    }
}
