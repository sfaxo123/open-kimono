#![no_main]
#![no_std]

extern crate alloc;

use alloc::string::String;
use core::fmt::Write;
use core::time::Duration;
use font8x8::{UnicodeFonts, BASIC_FONTS};
use uefi::boot::{self, MemoryType};
use uefi::mem::memory_map::MemoryMap;
use uefi::proto::console::gop::{BltOp, BltPixel, BltRegion, GraphicsOutput, PixelFormat};
use uefi::{entry, guid, println, system, Status};

const DTB_GUID: uefi::Guid = guid!("b1b621d5-f19c-41a5-830b-d9152c69aae0");

struct Screen {
    gop: Option<uefi::boot::ScopedProtocol<GraphicsOutput>>,
    row: usize,
}

impl Screen {
    fn new() -> Self {
        let gop = boot::get_handle_for_protocol::<GraphicsOutput>()
            .ok()
            .and_then(|h| boot::open_protocol_exclusive::<GraphicsOutput>(h).ok());
        Self { gop, row: 0 }
    }

    fn line(&mut self, message: &str) {
        println!("{message}");
        if let Some(gop) = self.gop.as_mut() {
            let info = gop.current_mode_info();
            let (width, height) = info.resolution();
            let y = self.row * 10 + 8;
            for (col, ch) in message.chars().enumerate() {
                let x = col * 8 + 8;
                if x + 8 > width || y + 8 > height {
                    break;
                }
                let mut pixels = [BltPixel::new(0, 0, 0); 64];
                if let Some(glyph) = BASIC_FONTS.get(ch) {
                    for (dy, bits) in glyph.iter().enumerate() {
                        for dx in 0..8 {
                            if bits & (1 << dx) != 0 {
                                pixels[dy * 8 + dx] = BltPixel::new(255, 255, 255);
                            }
                        }
                    }
                }
                let _ = gop.blt(BltOp::BufferToVideo {
                    buffer: &pixels,
                    src: BltRegion::Full,
                    dest: (x, y),
                    dims: (8, 8),
                });
            }
        }
        self.row += 1;
    }
}

fn line_fmt(screen: &mut Screen, args: core::fmt::Arguments<'_>) {
    let mut s = String::new();
    let _ = s.write_fmt(args);
    screen.line(&s);
}

fn be32(bytes: &[u8], offset: usize) -> Option<usize> {
    Some(u32::from_be_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?) as usize)
}

fn dt_string(bytes: &[u8], start: usize, end: usize) -> &str {
    let raw = bytes.get(start..end).unwrap_or(&[]);
    core::str::from_utf8(raw.split(|b| *b == 0).next().unwrap_or(&[])).unwrap_or("<invalid utf8>")
}

fn report_dtb(screen: &mut Screen) {
    let addr = system::with_config_table(|table| {
        table
            .iter()
            .find(|e| e.guid == DTB_GUID)
            .map(|e| e.address as usize)
    });
    let Some(addr) = addr else {
        screen.line("DTB: EFI configuration table entry absent");
        return;
    };
    line_fmt(screen, format_args!("DTB address: {addr:#x}"));
    if addr == 0 {
        screen.line("DTB: null pointer");
        return;
    }
    // SAFETY: Firmware owns the configuration table and its live DTB pointer.
    let header = unsafe { core::slice::from_raw_parts(addr as *const u8, 40) };
    if be32(header, 0) != Some(0xd00d_feed) {
        screen.line("DTB: invalid magic");
        return;
    }
    let size = be32(header, 4).unwrap_or(0);
    line_fmt(screen, format_args!("DTB size: {size} bytes"));
    if !(40..=16 * 1024 * 1024).contains(&size) {
        screen.line("DTB: implausible size");
        return;
    }
    // SAFETY: A valid FDT header declares the contiguous blob size; cap above limits reads.
    let dtb = unsafe { core::slice::from_raw_parts(addr as *const u8, size) };
    let (Some(struct_off), Some(strings_off), Some(struct_size), Some(strings_size)) =
        (be32(dtb, 8), be32(dtb, 12), be32(dtb, 36), be32(dtb, 32))
    else {
        return;
    };
    let Some(structure) = dtb.get(struct_off..struct_off.saturating_add(struct_size)) else {
        return;
    };
    let Some(strings) = dtb.get(strings_off..strings_off.saturating_add(strings_size)) else {
        return;
    };
    let mut pos = 0;
    let mut depth = 0usize;
    while pos + 4 <= structure.len() {
        let token = be32(structure, pos).unwrap_or(0);
        pos += 4;
        match token {
            1 => {
                let Some(end) = structure
                    .get(pos..)
                    .and_then(|s| s.iter().position(|b| *b == 0))
                else {
                    break;
                };
                pos = (pos + end + 4) & !3;
                depth += 1;
            }
            2 => depth = depth.saturating_sub(1),
            3 => {
                let (Some(len), Some(nameoff)) = (be32(structure, pos), be32(structure, pos + 4))
                else {
                    break;
                };
                pos += 8;
                let Some(value) = structure.get(pos..pos.saturating_add(len)) else {
                    break;
                };
                if depth == 1 {
                    let name = dt_string(strings, nameoff, strings.len());
                    if name == "model" {
                        line_fmt(
                            screen,
                            format_args!("DTB model: {}", dt_string(value, 0, value.len())),
                        );
                    } else if name == "compatible" {
                        for part in value.split(|b| *b == 0).filter(|p| !p.is_empty()) {
                            line_fmt(
                                screen,
                                format_args!(
                                    "DTB compatible: {}",
                                    core::str::from_utf8(part).unwrap_or("<invalid utf8>")
                                ),
                            );
                        }
                    }
                }
                pos = (pos + len + 3) & !3;
            }
            4 => {}
            9 => break,
            _ => break,
        }
    }
}

#[entry]
fn main() -> Status {
    if uefi::helpers::init().is_err() {
        return Status::ABORTED;
    }
    let mut screen = Screen::new();
    screen.line("OnePlus 6 EFI diagnostic");
    let el: u64;
    // SAFETY: Reading CurrentEL changes no machine state.
    unsafe {
        core::arch::asm!("mrs {0}, CurrentEL", out(reg) el);
    }
    line_fmt(&mut screen, format_args!("CurrentEL: EL{}", (el >> 2) & 3));
    match boot::memory_map(MemoryType::LOADER_DATA) {
        Ok(map) => {
            let mut count = 0u64;
            let mut pages = 0u64;
            for entry in map.entries() {
                count += 1;
                pages += entry.page_count;
            }
            line_fmt(
                &mut screen,
                format_args!(
                    "EFI memory map: {count} regions, {pages} pages ({} MiB)",
                    pages / 256
                ),
            );
            for entry in map.entries().take(8) {
                line_fmt(
                    &mut screen,
                    format_args!(
                        "  {:?} {:#x} {} pages",
                        entry.ty, entry.phys_start, entry.page_count
                    ),
                );
            }
        }
        Err(e) => line_fmt(
            &mut screen,
            format_args!("EFI memory map error: {:?}", e.status()),
        ),
    }
    report_dtb(&mut screen);
    if let Some(gop) = screen.gop.as_mut() {
        let info = gop.current_mode_info();
        let (w, h) = info.resolution();
        let stride = info.stride();
        let format = info.pixel_format();
        let base = if format == PixelFormat::BltOnly {
            None
        } else {
            Some(gop.frame_buffer().as_mut_ptr() as usize)
        };
        line_fmt(
            &mut screen,
            format_args!("GOP: {w}x{h} stride={stride} format={format:?} fb={base:x?}"),
        );
    } else {
        screen.line("GOP: unavailable");
    }
    screen.line("Press a key or wait 60 seconds...");
    for _ in 0..600 {
        if system::with_stdin(|input| input.read_key().ok().flatten().is_some()) {
            break;
        }
        boot::stall(Duration::from_millis(100));
    }
    Status::SUCCESS
}

// LLVM may lower a UTF-16 terminator scan in uefi 0.41 to this C symbol.
// The UEFI target has no C runtime, so provide the tiny equivalent.
#[no_mangle]
pub unsafe extern "C" fn wcslen(mut p: *const u16) -> usize {
    let mut len = 0;
    // SAFETY: Callers pass a valid NUL-terminated UTF-16 string.
    unsafe {
        while *p != 0 {
            len += 1;
            p = p.add(1);
        }
    }
    len
}
