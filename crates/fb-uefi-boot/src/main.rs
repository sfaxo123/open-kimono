//! HW-16: prove the fb-write console path RUNS on UEFI firmware.
//! HW-17: prove the FRAMEBUFFER WRITE path — render the HW-11 banner
//! THROUGH fb-write's `Frame` directly onto the GOP linear framebuffer,
//! then read the firmware-owned memory back and digest it on the spot.
//!
//! Output lines (stdout, serial-captured under QEMU):
//! - `HW16-DIGEST <hex64>` — private-buffer render (HW-16, unchanged)
//! - `HW17-GOP <w>x<h> stride=<bytes_per_line> fmt=<pixel format>`
//! - `HW17-FB-DIGEST <hex64>` — SHA-256 over the top-left 320x128 region
//!   of the GOP framebuffer, row-packed with no stride gaps. Must equal
//!   the HW-11 pin `b2115ecb…69d2` if the pixels actually landed.
//! - `HW17-GOP-FMT-UNSUPPORTED <fmt>` / `HW17-GOP-UNAVAILABLE` — honest
//!   fallbacks; no fake digests.
//!
//! Layout match (verified in code, not assumed): `Frame` stores X888 LE
//! pixels as bytes `[B, G, R, 0]` (u32 `0x00RRGGBB`). GOP
//! `PixelFormat::Rgb` is BGRR in *numerical* bit-field order
//! (red=0x00FF0000 …); on a little-endian machine that bitfield maps to
//! memory bytes `[B, G, R, reserved]` — byte-for-byte the `Frame` layout,
//! so a direct blit is valid.
#![no_std]
#![no_main]

extern crate alloc;

mod uartpath;

use uefi::prelude::*;
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat};

/// Hex digits for rendering the 32-byte digest without alloc-heavy formatting.
const HEX: &[u8; 16] = b"0123456789abcdef";

/// Print one ASCII line to UEFI stdout (UCS-2 conversion inline).
fn println(line: &[u8]) {
    let mut buf = [0u16; 160];
    let n = line.len().min(158);
    for i in 0..n {
        buf[i] = line[i] as u16;
    }
    buf[n] = b'\n' as u16;
    buf[n + 1] = 0;
    if let Ok(text) = uefi::CStr16::from_u16_with_nul(&buf[..n + 2]) {
        uefi::system::with_stdout(|stdout| {
            let _ = stdout.output_string_lossy(text);
        });
    }
}

/// Append decimal digits of `v` to `out` at `pos`, returning the new pos.
fn put_dec(out: &mut [u8; 160], mut pos: usize, v: u64) -> usize {
    let mut tmp = [0u8; 20];
    let mut n = 0;
    let mut x = v;
    loop {
        tmp[n] = b'0' + (x % 10) as u8;
        n += 1;
        x /= 10;
        if x == 0 {
            break;
        }
    }
    while n > 0 {
        n -= 1;
        out[pos] = tmp[n];
        pos += 1;
    }
    pos
}

/// Append hex digits of the 32-byte digest to `out` at `pos`.
fn put_hex64(out: &mut [u8; 160], mut pos: usize, digest: &[u8; 32]) -> usize {
    for &byte in digest {
        out[pos] = HEX[(byte >> 4) as usize];
        out[pos + 1] = HEX[(byte & 0xf) as usize];
        pos += 2;
    }
    pos
}

#[entry]
fn main() -> Status {
    // --- HW-24: geni-uart text path, COMPILED INTO this EFI image but
    // runtime-gated OFF. The sink (real geni MMIO at
    // `geni_uart::DEBUG_UART_BASE`) is constructed ONLY under the
    // compile-time const `uartpath::GENI_UART_ENABLE`, which is false:
    // QEMU virt has no geni device, and enabling on real hardware is a
    // separate gated decision (clock tree + operator sign-off, HW-23).
    // When false we print exactly the DISABLED marker — the geni driver
    // code is linked in (build proof) but never executed here.
    if uartpath::GENI_UART_ENABLE {
        // SAFETY (real-hardware path only, never taken on virt): the base
        // is a valid SE mapping of >= SE_WINDOW_SIZE bytes on sdm845
        // (DT reg = <0xa84000 0x4000>, HW-CONSOLE.md §2).
        let mut uart =
            unsafe { geni_uart::GeniUart::new(geni_uart::Mmio::new(geni_uart::DEBUG_UART_BASE)) };
        let sink_opt: Option<&mut dyn geni_uart::TxSink> = Some(&mut uart);
        let _ = uartpath::emit_banner_if_enabled(sink_opt);
    } else {
        println(b"HW24-UART-PATH=DISABLED");
    }

    // --- HW-16 (unchanged): render the banner into a private buffer and
    // hash it with fb-write's own code path.
    let digest = fb_write::evidence::frame_digest(320, 128);
    {
        let mut line = [0u8; 160];
        let mut pos = 0;
        for &b in b"HW16-DIGEST " {
            line[pos] = b;
            pos += 1;
        }
        pos = put_hex64(&mut line, pos, &digest);
        println(&line[..pos]);
    }

    // --- HW-17: GOP framebuffer write + readback digest.
    let gop_result = uefi::boot::get_handle_for_protocol::<GraphicsOutput>()
        .and_then(|handle| uefi::boot::open_protocol_exclusive::<GraphicsOutput>(handle));

    let mut gop = match gop_result {
        Ok(gop) => gop,
        Err(err) => {
            let mut line = [0u8; 160];
            let mut pos = 0;
            for &b in b"HW17-GOP-UNAVAILABLE status=0x" {
                line[pos] = b;
                pos += 1;
            }
            let code = err.status().0;
            for shift in (0..8).rev() {
                line[pos] = HEX[((code >> (shift * 4)) & 0xf) as usize];
                pos += 1;
            }
            println(&line[..pos]);
            return Status::SUCCESS;
        }
    };

    let info = gop.current_mode_info();
    let (w, h) = info.resolution();
    let stride_px = info.stride();
    let stride_bytes = stride_px * 4;
    let format = info.pixel_format();

    // "HW17-GOP <w>x<h> stride=<bytes_per_line> fmt=<pixel format>"
    {
        let mut line = [0u8; 160];
        let mut pos = 0;
        for &b in b"HW17-GOP " {
            line[pos] = b;
            pos += 1;
        }
        pos = put_dec(&mut line, pos, w as u64);
        line[pos] = b'x';
        pos += 1;
        pos = put_dec(&mut line, pos, h as u64);
        for &b in b" stride=" {
            line[pos] = b;
            pos += 1;
        }
        pos = put_dec(&mut line, pos, stride_bytes as u64);
        for &b in b" fmt=" {
            line[pos] = b;
            pos += 1;
        }
        let name: &[u8] = match format {
            PixelFormat::Rgb => b"Rgb",
            PixelFormat::Bgr => b"Bgr",
            PixelFormat::Bitmask => b"Bitmask",
            PixelFormat::BltOnly => b"BltOnly",
        };
        for &b in name {
            line[pos] = b;
            pos += 1;
        }
        println(&line[..pos]);
    }

    // Frame's X888 LE layout ([B,G,R,0] in memory) matches GOP Rgb on a
    // little-endian machine (see module docs). Bgr swaps R/B — unsupported
    // for a direct blit. Bitmask: accept only the exact B8G8R8X8 bitmask.
    let supported = match format {
        PixelFormat::Rgb => true,
        PixelFormat::Bitmask => match info.pixel_bitmask() {
            Some(bm) => bm.red == 0x00ff0000 && bm.green == 0x0000ff00 && bm.blue == 0x000000ff,
            None => false,
        },
        _ => false,
    };
    if !supported {
        let mut line = [0u8; 160];
        let mut pos = 0;
        for &b in b"HW17-GOP-FMT-UNSUPPORTED " {
            line[pos] = b;
            pos += 1;
        }
        let name: &[u8] = match format {
            PixelFormat::Rgb => b"Rgb",
            PixelFormat::Bgr => b"Bgr",
            PixelFormat::Bitmask => b"Bitmask",
            PixelFormat::BltOnly => b"BltOnly",
        };
        for &b in name {
            line[pos] = b;
            pos += 1;
        }
        println(&line[..pos]);
        return Status::SUCCESS;
    }

    // Full-screen console render through fb-write's Frame directly onto
    // the GOP linear framebuffer (stride = pixels_per_scan_line).
    let mut fb = gop.frame_buffer();
    let fb_bytes = fb.size();
    let rows_fit = fb_bytes / stride_bytes;
    let frame_h = h.min(rows_fit);
    if frame_h == 0 || stride_px == 0 {
        println(b"HW17-GOP-FB-EMPTY");
        return Status::SUCCESS;
    }
    // SAFETY: format verified above (32bpp LE), base+size come from the
    // firmware mode info, and all accesses stay within `rows_fit` rows of
    // `stride_bytes` bytes each. The Frame geometry invariant additionally
    // clamps width/height to what the slice can hold.
    let lfb: &mut [u32] = unsafe {
        let p = fb.as_mut_ptr() as *mut u32;
        let n = rows_fit * stride_px;
        core::slice::from_raw_parts_mut(p, n)
    };
    {
        let frame = fb_write::Frame::new(lfb, w, frame_h, stride_px);
        let mut console = fb_write::Console::new(frame, 0x00FFFFFF, 0x00000000);
        console.write_str(fb_write::evidence::BANNER);
    }

    // Read back the top-left 320x128 region of the GOP framebuffer row by
    // row (respecting stride) into a fresh row-packed buffer and digest it
    // exactly like `frame_digest` (u32 -> LE bytes, SHA-256).
    let rw = 320usize.min(w);
    let rh = 128usize.min(frame_h);
    let mut readback: alloc::vec::Vec<u8> = alloc::vec![0u8; rw * rh * 4];
    for y in 0..rh {
        for x in 0..rw {
            let src = lfb[y * stride_px + x].to_le_bytes();
            let dst = (y * rw + x) * 4;
            readback[dst..dst + 4].copy_from_slice(&src);
        }
    }
    let fb_digest = fb_write::evidence::sha256(&readback);
    {
        let mut line = [0u8; 160];
        let mut pos = 0;
        for &b in b"HW17-FB-DIGEST " {
            line[pos] = b;
            pos += 1;
        }
        pos = put_hex64(&mut line, pos, &fb_digest);
        println(&line[..pos]);
    }

    Status::SUCCESS
}
