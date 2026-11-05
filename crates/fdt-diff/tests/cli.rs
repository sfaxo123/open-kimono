// HW-25: integration tests for the `fdt-diff-cli` binary.
//
// Invokes the real built binary via std::process::Command and the
// CARGO_BIN_EXE_fdt-diff-cli env var cargo sets for integration tests.
// Exercises the exit-code contract (0 EQUAL / 1 DIFFERENT / 2 INCOMPLETE /
// 3 usage-or-parse error), the verdict line, and the diff-line output —
// on REAL device trees, not synthetic fixtures.
//
// The carrier DTB is the HW-15/HW-22-pinned artifact: extracted from the
// carrier image op6-embedded-efi-redox.img at offset 3,453,288 (totalsize
// 155,913 B, sha256 48518e1d…9383). It is extracted on demand from the
// read-only evidence bank with a magic scan — same method as HW-22 — and
// cached under target/, so no new binary blob is committed.

use std::path::{Path, PathBuf};
use std::process::Command;

const CARRIER_IMG: &str =
    "$HOME/artifacts/handoff/redox-perplexity/physical-revised31-20261027T1955/package/op6-embedded-efi-redox.img";
/// HW-15 pin: embedded DTB offset/totalsize/sha256 in the carrier image.
const CARRIER_OFF: usize = 3_453_288;
const CARRIER_SIZE: usize = 155_913;

fn bin() -> PathBuf {
    // The hyphenated env key cannot be read with env!(); cargo sets it as a
    // real process env var for integration tests of bin targets.
    PathBuf::from(
        std::env::var("CARGO_BIN_EXE_fdt-diff-cli")
            .expect("cargo must provide CARGO_BIN_EXE_fdt-diff-cli for integration tests"),
    )
}

/// Minimal SHA-256 (no deps; same shape as tests/fdt_real_pair.rs).
fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let (mut h0, mut h1, mut h2, mut h3, mut h4, mut h5, mut h6, mut h7) = (
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    );
    let bitlen = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bitlen.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh): (
            u32,
            u32,
            u32,
            u32,
            u32,
            u32,
            u32,
            u32,
        ) = (h0, h1, h2, h3, h4, h5, h6, h7);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
        h5 = h5.wrapping_add(f);
        h6 = h6.wrapping_add(g);
        h7 = h7.wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, h) in [h0, h1, h2, h3, h4, h5, h6, h7].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&h.to_be_bytes());
    }
    out
}

/// Extract the HW-15-pinned carrier DTB from the read-only evidence image,
/// verify the sha256 pin, and cache it under target/ for reuse by other
/// tests in this suite. Panics with a clear message if the bank is absent.
fn carrier_dtb_path() -> PathBuf {
    let cache = Path::new(env!("CARGO_TARGET_TMPDIR")).join("hw25_carrier.dtb");
    if let Ok(bytes) = std::fs::read(&cache) {
        if bytes.len() == CARRIER_SIZE {
            return cache;
        }
    }
    let img = std::fs::read(CARRIER_IMG).unwrap_or_else(|e| {
        panic!(
            "HW-25 evidence artifact missing/unreadable: {CARRIER_IMG} ({e}) — \
             artifacts under $HOME/artifacts are the read-only evidence bank"
        )
    });
    assert!(
        &img[CARRIER_OFF..CARRIER_OFF + 4] == b"\xd0\x0d\xfe\xed",
        "no FDT magic at HW-15-pinned carrier offset {CARRIER_OFF}"
    );
    let dtb = &img[CARRIER_OFF..CARRIER_OFF + CARRIER_SIZE];
    // HW-15 sha pin: 48518e1d e2b2024a a078f4ba 2ce2dfc1 e3f9e892 3c4a212b 320413db b1259383
    let want: [u8; 32] = [
        0x48, 0x51, 0x8e, 0x1d, 0xe2, 0xb2, 0x02, 0x4a, 0xa0, 0x78, 0xf4, 0xba, 0x2c, 0xe2, 0xdf,
        0xc1, 0xe3, 0xf9, 0xe8, 0x92, 0x3c, 0x4a, 0x21, 0x2b, 0x32, 0x04, 0x13, 0xdb, 0xb1, 0x25,
        0x93, 0x83,
    ];
    assert_eq!(
        sha256(dtb),
        want,
        "carrier DTB sha256 mismatch vs HW-15 pin"
    );
    std::fs::write(&cache, dtb).expect("cache carrier DTB under target tmpdir");
    cache
}

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn fdt-diff-cli");
    (
        out.status.code().expect("no signal"),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn cli_equal_on_identical_pair() {
    let a = carrier_dtb_path();
    let (code, stdout, stderr) = run(&[a.to_str().unwrap(), a.to_str().unwrap()]);
    println!("cli_equal: exit={code} stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(code, 0, "identical real DTB pair must exit 0");
    assert!(
        stdout.contains("EQUAL"),
        "verdict line must contain EQUAL, got {stdout:?}"
    );
}

#[test]
fn cli_different_on_flipped_byte() {
    let a = carrier_dtb_path();
    let mut bytes = std::fs::read(&a).expect("carrier dtb");
    // The /model trick from tests/fdt_real_pair.rs: locate the root `model`
    // value bytes and flip one in place (name-preserving, string stays
    // valid). "OnePlus 6\0" starts at blob offset 164 in this DTB; locate
    // it by content so the test does not hardcode the offset.
    let needle = b"OnePlus 6\x00";
    let pos = bytes
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("/model value bytes located in carrier DTB");
    bytes[pos] = b'^' ^ bytes[pos]; // 'O' -> 0x2f ('/'): "OnePlus 6" -> "/nePlus 6"
    let flipped = Path::new(env!("CARGO_TARGET_TMPDIR")).join("hw25_carrier_flipped.dtb");
    std::fs::write(&flipped, &bytes).expect("write flipped copy");

    let (code, stdout, stderr) = run(&[a.to_str().unwrap(), flipped.to_str().unwrap()]);
    println!("cli_different: exit={code} stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(code, 1, "flipped /model byte must exit 1");
    assert!(
        stdout.contains("DIFFERENT"),
        "verdict line must contain DIFFERENT, got {stdout:?}"
    );
    assert!(
        stdout
            .lines()
            .any(|l| l.contains("model") && l.contains('/')),
        "a diff line must name /model, got {stdout:?}"
    );
}

#[test]
fn cli_cap_incomplete() {
    // HW-09C semantics on the REAL pair: the two HW-22 DTBs are byte-identical,
    // so `compare` short-circuits to Equal BEFORE any budgeting — uncapped,
    // exit 0. With --cap 2 the same short-circuit still fires first, so the
    // CLI reports EQUAL and exits 0 on the identical pair. To exercise the
    // INCOMPLETE path (exit 2) the brief's cap probe must therefore run on a
    // BYTE-PERTURBED pair: cap 2 gives a walk budget of 2*2=4 total entries;
    // the real tree burns the budget on the root begin + its first 3 root
    // props (interrupt-parent, #address-cells, #size-cells) and aborts at
    // root prop #4 (chassis-type) — BEFORE reaching /model at root prop #6.
    // The flip is thus invisible to the capped walk: zero diffs emitted +
    // walk blindness => DiffReport::Incomplete => exit 2. This test asserts
    // exactly that, documenting both behaviors.
    let a = carrier_dtb_path();
    let (code, stdout, stderr) = run(&[a.to_str().unwrap(), a.to_str().unwrap(), "--cap", "2"]);
    println!("cli_cap_incomplete identical-pair: exit={code} stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(
        code, 0,
        "identical pair with --cap 2: byte-equality shortcut fires before budgeting, exit 0"
    );
    assert!(stdout.contains("EQUAL"), "got {stdout:?}");

    // Now the perturbed pair under the same cap: INCOMPLETE, exit 2.
    let mut bytes = std::fs::read(&a).expect("carrier dtb");
    let needle = b"OnePlus 6\x00";
    let pos = bytes
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("/model value bytes located in carrier DTB");
    bytes[pos] = b'^' ^ bytes[pos];
    let flipped = Path::new(env!("CARGO_TARGET_TMPDIR")).join("hw25_carrier_flipped_cap.dtb");
    std::fs::write(&flipped, &bytes).expect("write flipped copy");

    let (code, stdout, stderr) =
        run(&[a.to_str().unwrap(), flipped.to_str().unwrap(), "--cap", "2"]);
    println!("cli_cap_incomplete perturbed-pair: exit={code} stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(
        code, 2,
        "cap-2 walk burns its 4-entry budget at the root, never reaches the flipped /model: \
         zero diffs + walk blindness must be INCOMPLETE, exit 2"
    );
    assert!(
        stdout.contains("INCOMPLETE"),
        "verdict line must contain INCOMPLETE, got {stdout:?}"
    );
    assert!(
        stdout.contains("byte diff at 0xa4"),
        "byte_diff_at must be reported (first differing byte at 0xa4 = 164, the /model value), \
         got {stdout:?}"
    );
}

#[test]
fn cli_usage_error() {
    let (code, stdout, stderr) = run(&[]);
    println!("cli_usage: exit={code} stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(code, 3, "no args must exit 3");
    assert!(
        stdout.is_empty(),
        "usage errors go to stderr, got {stdout:?}"
    );
    assert!(
        stderr.contains("usage:"),
        "stderr must carry the usage line, got {stderr:?}"
    );
    // A missing file is also a usage/parse error (exit 3).
    let (code, _, _) = run(&["nope.dtb", "also-nope.dtb"]);
    assert_eq!(code, 3, "unreadable file must exit 3");
}
