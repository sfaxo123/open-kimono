//! gzip compression byte-identical to Python's `gzip.GzipFile` as the
//! original packager used it: filename="", compresslevel=9, mtime=0.
//!
//! Python's gzip module writes the header itself and deflates with the
//! system zlib (raw deflate, level 9, memLevel 8, default strategy).
//! This module does exactly the same: hand-built header/trailer,
//! DEFLATE via FFI to the same system libz python3 links, CRC32
//! implemented in-crate. Proven pre-port: re-running Python's gzip on
//! the stage-1 u-boot-nodtb.bin reproduces the original run's stored
//! .gz sha256 exactly (4cabb7d0...), and the crate test below pins a
//! Python-generated vector.

use std::os::raw::{c_char, c_int, c_uint, c_ulong, c_void};

#[repr(C)]
struct ZStream {
    next_in: *mut u8,
    avail_in: c_uint,
    total_in: c_ulong,
    next_out: *mut u8,
    avail_out: c_uint,
    total_out: c_ulong,
    msg: *mut c_char,
    state: *mut c_void,
    zalloc: Option<extern "C" fn(*mut c_void, c_uint, c_uint) -> *mut c_void>,
    zfree: Option<extern "C" fn(*mut c_void, *mut c_void)>,
    opaque: *mut c_void,
    data_type: c_int,
    adler: c_ulong,
    reserved: c_ulong,
}

#[link(name = "z")]
extern "C" {
    fn zlibVersion() -> *const c_char;
    fn deflateInit2_(
        strm: *mut ZStream,
        level: c_int,
        method: c_int,
        window_bits: c_int,
        mem_level: c_int,
        strategy: c_int,
        version: *const c_char,
        stream_size: c_int,
    ) -> c_int;
    fn deflate(strm: *mut ZStream, flush: c_int) -> c_int;
    fn deflateEnd(strm: *mut ZStream) -> c_int;
    fn deflateBound(strm: *mut ZStream, source_len: c_ulong) -> c_ulong;
}

const Z_DEFLATED: c_int = 8;
const Z_DEFAULT_STRATEGY: c_int = 0;
const Z_FINISH: c_int = 4;
const Z_OK: c_int = 0;
const Z_STREAM_END: c_int = 1;

fn raw_deflate_level9(data: &[u8]) -> Result<Vec<u8>, String> {
    unsafe {
        let mut strm = ZStream {
            next_in: data.as_ptr() as *mut u8,
            avail_in: data.len() as c_uint,
            total_in: 0,
            next_out: std::ptr::null_mut(),
            avail_out: 0,
            total_out: 0,
            msg: std::ptr::null_mut(),
            state: std::ptr::null_mut(),
            zalloc: None,
            zfree: None,
            opaque: std::ptr::null_mut(),
            data_type: 0,
            adler: 0,
            reserved: 0,
        };
        let rc = deflateInit2_(
            &mut strm,
            9,
            Z_DEFLATED,
            -15, // raw deflate, like zlib.compressobj(wbits=-MAX_WBITS)
            8,   // DEF_MEM_LEVEL, Python's default
            Z_DEFAULT_STRATEGY,
            zlibVersion(),
            std::mem::size_of::<ZStream>() as c_int,
        );
        if rc != Z_OK {
            return Err(format!("deflateInit2 failed: rc={}", rc));
        }
        let bound = deflateBound(&mut strm, data.len() as c_ulong) as usize;
        let mut out = vec![0u8; bound.max(64)];
        strm.next_out = out.as_mut_ptr();
        strm.avail_out = out.len() as c_uint;
        let rc = deflate(&mut strm, Z_FINISH);
        let produced = strm.total_out as usize;
        let rc2 = deflateEnd(&mut strm);
        if rc != Z_STREAM_END {
            return Err(format!("deflate did not finish: rc={} end_rc={}", rc, rc2));
        }
        out.truncate(produced);
        Ok(out)
    }
}

/// CRC-32 (IEEE 802.3, reflected) - the same checksum zlib's crc32 and
/// therefore Python's gzip trailer use.
pub fn crc32(data: &[u8]) -> u32 {
    static mut TABLE: [u32; 256] = [0; 256];
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        for i in 0..256u32 {
            let mut c = i;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb88320 ^ (c >> 1) } else { c >> 1 };
            }
            TABLE[i as usize] = c;
        }
    });
    let table = unsafe { &*(&raw const TABLE) };
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc = table[((crc ^ b as u32) & 0xff) as usize] ^ (crc >> 8);
    }
    crc ^ 0xffff_ffff
}

/// gzip with the Python packager's exact parameters: header
/// 1f 8b 08 00, mtime 0, XFL 2 (level 9), OS 255, no filename.
pub fn gzip_level9_mtime0(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(data.len() / 2 + 64);
    out.extend_from_slice(&[0x1f, 0x8b, 0x08, 0x00, 0, 0, 0, 0, 0x02, 0xff]);
    out.extend_from_slice(&raw_deflate_level9(data)?);
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sha256::hex;

    #[test]
    fn crc32_vector() {
        assert_eq!(crc32(b"123456789"), 0xcbf43926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn gzip_matches_python_vector() {
        // Generated with CPython: GzipFile(filename="", mode="wb",
        // compresslevel=9, mtime=0) over b"oneplus6-packager-gzip-vector" * 7.
        let input = b"oneplus6-packager-gzip-vector".repeat(7);
        let got = gzip_level9_mtime0(&input).expect("gzip");
        assert_eq!(
            hex(&got),
            "1f8b08000000000002ffcbcf4b2dc8292d36d32d484cce4e4c4f2dd24dafca2cd02d4b4d2ec92fca1f52920028a372b3cb000000"
        );
    }
}
