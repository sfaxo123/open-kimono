//! Deterministic HW-07 demo: build two minimal DTBs in memory (no fixtures,
//! no I/O), diff them, and print a stable report. Running this example twice
//! must produce byte-identical stdout.
//!
//! Scenario mirrors G1/G3: side A has `/chosen/framebuffer-0` (rich bootargs)
//! and `/reserved-memory/splash@9d400000`; side B renamed the framebuffer
//! node to `framebuffer` (same body) and dropped the splash node.

use fdt_diff::{compare, Diff, DiffReport, DtbBuilder};

const SUBTREES: &[&str] = &["/chosen", "/reserved-memory"];

fn blob_a() -> Vec<u8> {
    let mut b = DtbBuilder::new().open_root();
    {
        let mut root = b.root_props();
        root.prop("compatible", b"test\0");
        {
            let mut ch = root.node("chosen");
            ch.prop("stdout-path", b"serial0:115200n8\0");
            ch.prop("bootargs", b"console=ttyS0 root=/dev/mmcblk0p2\0");
            {
                let mut fb = ch.node("framebuffer-0");
                fb.prop("compatible", b"simple-framebuffer\0");
                fb.prop("width", &0x438u32.to_be_bytes());
                fb.prop("height", &0x8e8u32.to_be_bytes());
                fb.prop("format", b"a8r8g8b8\0");
            }
        }
        {
            let mut rm = root.node("reserved-memory");
            rm.prop("#address-cells", &2u32.to_be_bytes());
            rm.prop("#size-cells", &1u32.to_be_bytes());
            {
                let mut sp = rm.node("splash@9d400000");
                sp.prop(
                    "reg",
                    &[
                        0, 0, 0, 0x9d, 0x40, 0x00, 0x00, 0x00, 0x24, 0x00, 0x00, 0x00,
                    ],
                );
                sp.prop("no-map", &[]);
            }
        }
    }
    b.finish()
}

fn blob_b() -> Vec<u8> {
    // Same framebuffer body under a different name; splash node gone;
    // bootargs slightly different (one extra word).
    let mut b = DtbBuilder::new().open_root();
    {
        let mut root = b.root_props();
        root.prop("compatible", b"test\0");
        {
            let mut ch = root.node("chosen");
            ch.prop("stdout-path", b"serial0:115200n8\0");
            ch.prop("bootargs", b"console=ttyS0 root=/dev/mmcblk0p2 quiet\0");
            {
                let mut fb = ch.node("framebuffer");
                fb.prop("compatible", b"simple-framebuffer\0");
                fb.prop("width", &0x438u32.to_be_bytes());
                fb.prop("height", &0x8e8u32.to_be_bytes());
                fb.prop("format", b"a8r8g8b8\0");
            }
        }
        {
            let mut rm = root.node("reserved-memory");
            rm.prop("#address-cells", &2u32.to_be_bytes());
            rm.prop("#size-cells", &1u32.to_be_bytes());
        }
    }
    b.finish()
}

fn main() {
    let a = blob_a();
    let b = blob_b();

    println!("blob A: {} bytes", a.len());
    println!("blob B: {} bytes", b.len());

    match compare(&a, &b, SUBTREES) {
        Ok(DiffReport::Equal) => println!("verdict: EQUAL"),
        Ok(DiffReport::StructurallyEqual { byte_diff_at }) => {
            println!("verdict: STRUCTURALLY EQUAL (first byte diff at 0x{byte_diff_at:x})")
        }
        Ok(DiffReport::Incomplete { .. }) => {
            println!("INCOMPLETE (budget exhausted, no diffs seen)");
            return;
        }
        Ok(DiffReport::Different {
            diffs,
            byte_diff_at,
            truncated,
        }) => {
            match byte_diff_at {
                Some(at) => println!("verdict: DIFFERENT (first byte diff at 0x{at:x})"),
                None => println!("verdict: DIFFERENT"),
            }
            if truncated {
                println!("verdict: TRUNCATED (diff collection stopped at MAX_DIFFS)");
            }
            for d in &diffs {
                match d {
                    Diff::NameMismatch {
                        parent,
                        a_name,
                        b_name,
                    } => {
                        println!("  NameMismatch   {parent}: a=`{a_name}` <-> b=`{b_name}`")
                    }
                    Diff::MissingInB { path } => println!("  MissingInB     {path}"),
                    Diff::MissingInA { path } => println!("  MissingInA     {path}"),
                    Diff::PropMissingInB { path, name } => {
                        println!("  PropMissingInB {path}#{name}")
                    }
                    Diff::PropMissingInA { path, name } => {
                        println!("  PropMissingInA {path}#{name}")
                    }
                    Diff::ValueMismatch {
                        path,
                        name,
                        a_off,
                        b_off,
                        a_len,
                        b_len,
                    } => {
                        println!(
                            "  ValueMismatch  {path}#{name} a=0x{a_off:x}({a_len}B) b=0x{b_off:x}({b_len}B)"
                        )
                    }
                }
            }
        }
        Err((side, e)) => println!("parse error on side {side}: {e:?}"),
    }
}
