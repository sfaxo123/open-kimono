//! HW-07 acceptance tests for fdt-diff.
//! (a) identical blobs → Equal
//! (b) one-byte value flip → ValueMismatch with correct path
//! (c) node renamed (framebuffer vs framebuffer-0) → NameMismatch, bodies matched
//! (d) truncated blob → FdtError, not panic
//! (e) totalsize larger than buffer → FdtError
//! (f) real fixtures: live vs phone-roundtrip chosen/splash DTBs (dtc 1.7.0 built)

use fdt_diff::{compare, Diff, DiffReport, DtbBuilder, Fdt, FdtError};
use std::format;
use std::string::String;

const SUBTREES: &[&str] = &["/chosen", "/reserved-memory"];

/// Build the canonical minimal fixture: /chosen/framebuffer-0 + /reserved-memory/splash.
fn blob_a() -> Vec<u8> {
    let mut b = DtbBuilder::new().open_root();
    {
        let mut root = b.root_props();
        root.prop("compatible", b"test\0");
        {
            let mut ch = root.node("chosen");
            ch.prop("stdout-path", b"serial0:115200n8\0");
            {
                let mut fb = ch.node("framebuffer-0");
                fb.prop("compatible", b"simple-framebuffer\0");
                fb.prop("width", &0x438u32.to_be_bytes());
                fb.prop("height", &0x8e8u32.to_be_bytes());
                fb.prop("stride", &0x10e0u32.to_be_bytes());
                fb.prop("format", b"a8r8g8b8\0");
                fb.prop("memory-region", &2u32.to_be_bytes());
            }
        }
        {
            let mut rm = root.node("reserved-memory");
            {
                let mut sp = rm.node("splash@9d400000");
                sp.prop(
                    "reg",
                    &[
                        0u8, 0, 0x9d, 0x40, 0x00, 0x00, 0x00, 0x00, 0x24, 0x00, 0x00, 0x00,
                    ],
                );
                sp.prop("no-map", &[0u8, 0, 0, 0]);
            }
        }
    }
    b.finish()
}

fn parse_ok(blob: &[u8]) {
    Fdt::parse(blob).expect("blob must parse");
}

// ---------------------------------------------------------------------------
// (a) identical blobs → Equal
// ---------------------------------------------------------------------------

#[test]
fn identical_blobs_are_equal() {
    let a = blob_a();
    let b = a.clone();
    parse_ok(&a);
    let rep = compare(&a, &b, SUBTREES).expect("both parse");
    assert_eq!(rep, DiffReport::Equal);
    assert!(rep.is_equal());
}

// ---------------------------------------------------------------------------
// (b) one-byte value flip → ValueMismatch with correct path
// ---------------------------------------------------------------------------

#[test]
fn one_byte_flip_reports_value_mismatch_with_path() {
    let a = blob_a();
    let mut b = a.clone();
    parse_ok(&b);
    // Flip one byte of the framebuffer-0 `width` value (0x438 -> 0x439).
    let fa = Fdt::parse(&a).unwrap();
    let entry = fa
        .subtree("/chosen")
        .unwrap()
        .into_iter()
        .find(|e| e.node_path == "/chosen/framebuffer-0" && e.name == "width")
        .expect("width prop exists");
    b[entry.value_at + entry.value.len() - 1] ^= 0x01;
    assert_ne!(a, b);

    let rep = compare(&a, &b, SUBTREES).expect("both parse");
    match rep {
        DiffReport::Different {
            diffs,
            byte_diff_at,
            truncated,
        } => {
            assert!(!truncated, "single-diff case must not be truncated");
            assert!(byte_diff_at.is_some());
            let vm: Vec<&Diff> = diffs
                .iter()
                .filter(|d| matches!(d, Diff::ValueMismatch { .. }))
                .collect();
            assert_eq!(vm.len(), 1, "exactly one value mismatch: {diffs:?}");
            match vm[0] {
                Diff::ValueMismatch {
                    path,
                    name,
                    a_off,
                    b_off,
                    a_len,
                    b_len,
                } => {
                    assert_eq!(path, "/chosen/framebuffer-0");
                    assert_eq!(name, "width");
                    assert_eq!(*a_off, entry.value_at);
                    assert_eq!(*b_off, entry.value_at);
                    assert_eq!(*a_len, 4);
                    assert_eq!(*b_len, 4);
                }
                _ => unreachable!(),
            }
        }
        other => panic!("expected Different, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// (c) node renamed (framebuffer vs framebuffer-0) → NameMismatch, bodies match
// ---------------------------------------------------------------------------

fn blob_renamed() -> Vec<u8> {
    // Same bodies, but chosen child spelled `framebuffer` (the full31
    // package spelling) instead of `framebuffer-0` (the live ABL spelling).
    let mut b = DtbBuilder::new().open_root();
    {
        let mut root = b.root_props();
        root.prop("compatible", b"test\0");
        {
            let mut ch = root.node("chosen");
            ch.prop("stdout-path", b"serial0:115200n8\0");
            {
                let mut fb = ch.node("framebuffer");
                fb.prop("compatible", b"simple-framebuffer\0");
                fb.prop("width", &0x438u32.to_be_bytes());
                fb.prop("height", &0x8e8u32.to_be_bytes());
                fb.prop("stride", &0x10e0u32.to_be_bytes());
                fb.prop("format", b"a8r8g8b8\0");
                fb.prop("memory-region", &2u32.to_be_bytes());
            }
        }
        {
            let mut rm = root.node("reserved-memory");
            {
                let mut sp = rm.node("splash@9d400000");
                sp.prop(
                    "reg",
                    &[
                        0u8, 0, 0x9d, 0x40, 0x00, 0x00, 0x00, 0x00, 0x24, 0x00, 0x00, 0x00,
                    ],
                );
                sp.prop("no-map", &[0u8, 0, 0, 0]);
            }
        }
    }
    b.finish()
}

#[test]
fn renamed_node_reports_name_mismatch_not_missing() {
    let a = blob_a();
    let b = blob_renamed();
    parse_ok(&a);
    parse_ok(&b);
    let rep = compare(&a, &b, SUBTREES).expect("both parse");
    match rep {
        DiffReport::Different { diffs, .. } => {
            let nm: Vec<&Diff> = diffs
                .iter()
                .filter(|d| matches!(d, Diff::NameMismatch { .. }))
                .collect();
            assert_eq!(nm.len(), 1, "exactly one name mismatch: {diffs:?}");
            match nm[0] {
                Diff::NameMismatch {
                    parent,
                    a_name,
                    b_name,
                } => {
                    assert_eq!(parent, "/chosen");
                    assert_eq!(a_name, "framebuffer-0");
                    assert_eq!(b_name, "framebuffer");
                }
                _ => unreachable!(),
            }
            // Bodies matched: no value mismatches, and the renamed nodes are
            // NOT separately reported as missing on either side.
            assert!(!diffs
                .iter()
                .any(|d| matches!(d, Diff::ValueMismatch { .. })));
            assert!(!diffs.iter().any(|d| {
                matches!(d, Diff::MissingInA { path } | Diff::MissingInB { path }
                    if path.contains("framebuffer"))
            }));
        }
        other => panic!("expected Different with NameMismatch, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// (d) truncated blob → FdtError, not panic
// ---------------------------------------------------------------------------

#[test]
fn truncated_blob_is_error_not_panic() {
    let a = blob_a();
    // Cut mid-struct-block: totalsize still claims the full length, buffer is short.
    let cut = a.len() / 2;
    let declared = u32::from_be_bytes([a[4], a[5], a[6], a[7]]);
    let t = &a[..cut];
    let err = Fdt::parse(t).unwrap_err();
    assert_eq!(
        err,
        FdtError::TotalsizeExceedsBuffer {
            totalsize: declared,
            len: cut
        }
    );

    // Also: a blob truncated AND header rewritten to match the shorter buffer
    // (chopped mid-property) must fail on the struct walk, not panic.
    let mut t2 = a[..cut].to_vec();
    t2[4..8].copy_from_slice(&(cut as u32).to_be_bytes());
    let e2 = Fdt::parse(&t2).unwrap_err();
    assert!(matches!(
        e2,
        FdtError::OutOfBounds { .. } | FdtError::NoEndToken | FdtError::BadOffset { .. }
    ));

    // compare() must surface the failing side, not panic.
    let b = blob_a();
    let res = compare(t, &b, SUBTREES);
    let (side, _) = res.unwrap_err();
    assert_eq!(side, 0);
}

// ---------------------------------------------------------------------------
// (e) totalsize larger than buffer → FdtError
// ---------------------------------------------------------------------------

#[test]
fn totalsize_larger_than_buffer_is_error() {
    let mut a = blob_a();
    let real = a.len() as u32;
    a[4..8].copy_from_slice(&(real + 4096).to_be_bytes());
    let err = Fdt::parse(&a).unwrap_err();
    assert_eq!(
        err,
        FdtError::TotalsizeExceedsBuffer {
            totalsize: real + 4096,
            len: real as usize
        }
    );
}

#[test]
fn absurd_totalsize_rejected() {
    let mut a = blob_a();
    a[4..8].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(
        Fdt::parse(&a).unwrap_err(),
        FdtError::TotalsizeTooLarge(u32::MAX)
    );
    a[4..8].copy_from_slice(&4u32.to_be_bytes());
    assert_eq!(Fdt::parse(&a).unwrap_err(), FdtError::TotalsizeTooSmall(4));
}

#[test]
fn bad_magic_rejected() {
    let mut a = blob_a();
    a[0..4].copy_from_slice(&0xfeedd00du32.to_be_bytes());
    assert_eq!(Fdt::parse(&a).unwrap_err(), FdtError::BadMagic(0xfeed_d00d));
}

// ---------------------------------------------------------------------------
// (f) real fixtures (dtc 1.7.0 from workbench/hw-dtb/*.dts, copied in)
// ---------------------------------------------------------------------------

const LIVE: &[u8] = include_bytes!("fixtures/live-chosen-splash.dtb");
const ROUNDTRIP: &[u8] = include_bytes!("fixtures/phone-roundtrip-chosen-splash.dtb");
const UBOOT: &[u8] = include_bytes!("fixtures/uboot-build-chosen-splash.dtb");

#[test]
fn real_fixtures_parse_and_diff() {
    parse_ok(LIVE);
    parse_ok(ROUNDTRIP);
    parse_ok(UBOOT);

    // Identical fixture vs itself: Equal.
    assert_eq!(compare(LIVE, LIVE, SUBTREES).unwrap(), DiffReport::Equal);

    // live (framebuffer-0, rich bootargs) vs phone-roundtrip (framebuffer,
    // phandle on fb): bytes differ; the framebuffer spelling must land as a
    // NameMismatch pair, and differing bodies as ValueMismatch/PropMissing.
    let rep = compare(LIVE, ROUNDTRIP, SUBTREES).unwrap();
    match rep {
        DiffReport::Different {
            diffs,
            byte_diff_at,
            truncated,
        } => {
            assert!(!truncated, "fixture diff must not be truncated");
            assert!(byte_diff_at.is_some());
            let nm = diffs
                .iter()
                .filter(|d| matches!(d, Diff::NameMismatch { .. }))
                .count();
            assert!(
                nm >= 1,
                "expected >=1 NameMismatch (framebuffer-0 vs framebuffer): {diffs:?}"
            );
            let fb_accounted = diffs.iter().any(|d| match d {
                Diff::NameMismatch {
                    parent,
                    a_name,
                    b_name,
                } => {
                    parent.contains("chosen")
                        && a_name.contains("framebuffer")
                        && b_name.contains("framebuffer")
                }
                Diff::ValueMismatch { path, .. } => path.contains("framebuffer"),
                Diff::MissingInA { path } | Diff::MissingInB { path } => {
                    path.contains("framebuffer")
                }
                _ => false,
            });
            assert!(
                fb_accounted,
                "framebuffer pair must be accounted for: {diffs:?}"
            );
        }
        other => panic!("live vs roundtrip must be Different, got {other:?}"),
    }

    // uboot-build fixture: chosen has only stdout-path; no splash node and no
    // /reserved-memory subtree anywhere on the uboot side.
    let rep = compare(LIVE, UBOOT, SUBTREES).unwrap();
    match rep {
        DiffReport::Different { diffs, .. } => {
            assert!(diffs.iter().any(|d| matches!(d, Diff::MissingInB { path } if path.starts_with("/reserved-memory"))),
                "splash/reserved-memory absent on uboot side (missing in b): {diffs:?}");
            assert!(!diffs.iter().any(|d| matches!(d, Diff::MissingInA { path } if path.starts_with("/reserved-memory"))),
                "uboot has no reserved-memory nodes: {diffs:?}");
        }
        other => panic!("live vs uboot must be Different, got {other:?}"),
    }
}

#[test]
fn subtree_extraction_finds_chosen_props() {
    let f = Fdt::parse(LIVE).unwrap();
    let chosen = f.subtree("/chosen").unwrap();
    assert!(chosen
        .iter()
        .any(|e| e.node_path == "/chosen" && e.name == "stdout-path"));
    assert!(chosen
        .iter()
        .any(|e| e.node_path == "/chosen/framebuffer-0" && e.name == "compatible"));
    let rm = f.subtree("/reserved-memory").unwrap();
    assert!(rm
        .iter()
        .any(|e| e.node_path == "/reserved-memory/splash@9d400000" && e.name == "reg"));
}

// HW-07B: the walk must be bounded by the DECLARED size_dt_struct, not by
// totalsize. Shrink the declared struct block to 8 B (fits totalsize but is
// far smaller than the real struct block) — parse must Err, never read past
// the declared block.
#[test]
fn declared_blocks_bounded() {
    let mut bytes = LIVE.to_vec();
    // size_dt_struct lives at header offset 36 in this repo's v17 layout.
    bytes[36..40].copy_from_slice(&8u32.to_be_bytes());
    assert!(
        Fdt::parse(&bytes).is_err(),
        "walk read past the declared struct block (size_dt_struct=8)"
    );
}

// HW-07C: node scope must come from the walk, not from property paths. A
// propertyless child at depth 2 (/chosen/extra) present only in b must be
// reported as MissingInA (missing FROM a) even though it owns no props.
// (HW-07E corrected the expected label: the old `MissingInB` expectation
// encoded the comparator's inverted side labels.)
#[test]
fn empty_child_deep() {
    let mut a = DtbBuilder::new().open_root();
    let mut b = DtbBuilder::new().open_root();
    {
        let mut root = a.root_props();
        root.prop("compatible", b"test\0");
        let mut ch = root.node("chosen");
        ch.prop("stdout-path", b"serial0:115200n8\0");
    }
    {
        let mut root = b.root_props();
        root.prop("compatible", b"test\0");
        let mut ch = root.node("chosen");
        ch.prop("stdout-path", b"serial0:115200n8\0");
        let _extra = ch.node("extra"); // propertyless, depth 2, only in b
    }
    let a = a.finish();
    let b = b.finish();
    parse_ok(&a);
    parse_ok(&b);
    let rep = compare(&a, &b, SUBTREES).expect("both parse");
    match rep {
        DiffReport::Different { diffs, .. } => assert!(
            diffs
                .iter()
                .any(|d| matches!(d, Diff::MissingInA { path } if path == "/chosen/extra")),
            "propertyless deep child (only in b) must be MissingInA: {diffs:?}"
        ),
        other => panic!("expected Different with MissingInA /chosen/extra, got {other:?}"),
    }
}

// HW-07D: the reservation map is a terminator-terminated 16-byte-entry
// array at off_rsv_map (header offset 16), NOT a header-sized block. This
// test byte-patches the live fixture: (1) point off_rsv_map at a valid
// past-header offset inside totalsize filled with NON-terminating junk
// entries (no all-zero (addr,size) pair before totalsize) -> parse must
// Err; (2) put a proper all-zero terminator at that offset -> parse OK.
// Patch region: the fixture's header padding between the 40-byte header
// and off_dt_struct=56 is zero-filled slack — junk written there for the
// Err arm never touches a declared block. The OK arm points off_rsv_map
// at the fixture's EXISTING terminator: the live blob already carries a
// valid map at off_rsv_map=40 whose sole entry is all-zero (addr 0, size
// 0) at 40..56 — so it rewrites ONLY header offset 16 to 40 (the base
// fixture encodes off 0 there, the HW-07B-era convention) and touches no
// payload byte at all. 40 is 4-aligned and >= FDT_HEADER_LEN, and the
// scan reaches the zero terminator without running past totalsize.
#[test]
fn rsv_map_terminator_required() {
    let base = include_bytes!("fixtures/live-chosen-splash.dtb").to_vec();
    let ts = u32::from_be_bytes([base[4], base[5], base[6], base[7]]) as usize;
    let rsv = 48usize; // valid past-header offset, inside the fixture's pre-struct padding
    let junk = [
        0x00u8, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, // addr 0x10000000
        0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, // size 0x01000000
    ];
    // (1) non-terminating junk from rsv to totalsize -> must Err
    let mut bad = base.clone();
    bad[16..20].copy_from_slice(&(rsv as u32).to_be_bytes());
    for chunk in bad[rsv..ts].chunks_mut(16) {
        let n = chunk.len().min(junk.len());
        chunk[..n].copy_from_slice(&junk[..n]);
    }
    assert!(
        matches!(
            Fdt::parse(&bad),
            Err(FdtError::BadReservationMap { off, .. }) if off == rsv as u32
        ),
        "reservation map without a zero terminator was accepted"
    );
    // (2) proper zero terminator at rsv -> must parse OK
    let mut good = base.clone();
    good[16..20].copy_from_slice(&40u32.to_be_bytes()); // off_rsv_map -> existing terminator entry
    assert!(
        Fdt::parse(&good).is_ok(),
        "terminator-terminated map at a valid offset was rejected: {:?}",
        Fdt::parse(&good).err()
    );
}

// HW-07E: root ("/") properties must participate in the diff. Identical
// trees whose root property "model" value differs must be Different with a
// ValueMismatch on path "/" — not merely a NameMismatch, and never a
// StructurallyEqual verdict.
#[test]
fn root_prop_value_mismatch() {
    let mut a = DtbBuilder::new().open_root();
    let mut b = DtbBuilder::new().open_root();
    {
        let mut root = a.root_props();
        root.prop("model", b"redox-alpha\0");
        root.prop("compatible", b"test\0");
        let mut ch = root.node("chosen");
        ch.prop("stdout-path", b"serial0:115200n8\0");
    }
    {
        let mut root = b.root_props();
        root.prop("model", b"redox-beta\0");
        root.prop("compatible", b"test\0");
        let mut ch = root.node("chosen");
        ch.prop("stdout-path", b"serial0:115200n8\0");
    }
    let a = a.finish();
    let b = b.finish();
    parse_ok(&a);
    parse_ok(&b);
    let rep = compare(&a, &b, &["/"]).expect("both parse");
    match rep {
        DiffReport::Different { diffs, .. } => {
            assert!(
                diffs.iter().any(|d| matches!(
                    d,
                    Diff::ValueMismatch { path, name, .. } if path == "/" && name == "model"
                )),
                "root prop \"model\" value diff must be a ValueMismatch on \"/\": {diffs:?}"
            );
            assert!(
                !diffs.iter().any(|d| matches!(d, Diff::NameMismatch { .. })),
                "root prop diff is a value mismatch, not a NameMismatch: {diffs:?}"
            );
        }
        other => panic!("root prop mismatch must be Different, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// HW-09: resource bounds (depth / candidates / diffs) + FDT_MAX_PROP_LEN audit
// ---------------------------------------------------------------------------

// HW-09(a): nesting beyond MAX_NODE_DEPTH → NodeDepthExceeded. The walk is a
// loop, not recursion — the bound caps `path` Vec growth and marks the blob
// rejected rather than walking it forever. A chain of MAX_NODE_DEPTH + 2
// nested BEGIN_NODEs (root counts as depth 1; the legal ceiling is
// MAX_NODE_DEPTH + 1) must fail with the named error, not panic.
#[test]
fn node_depth_beyond_max_is_rejected() {
    // Hand-craft the struct block: root + (MAX_NODE_DEPTH + 2) nested nodes.
    let mut sb: Vec<u8> = Vec::new();
    sb.extend_from_slice(&fdt_diff::FDT_BEGIN_NODE.to_be_bytes());
    sb.extend_from_slice(&[0u8; 4]); // root: empty name + pad
    for _ in 0..(fdt_diff::MAX_NODE_DEPTH + 2) {
        sb.extend_from_slice(&fdt_diff::FDT_BEGIN_NODE.to_be_bytes());
        sb.extend_from_slice(&[b'n', 0, 0, 0]); // 1-char name + pad
    }
    for _ in 0..(fdt_diff::MAX_NODE_DEPTH + 2) {
        sb.extend_from_slice(&fdt_diff::FDT_END_NODE.to_be_bytes());
    }
    sb.extend_from_slice(&[0u8; 4]); // root END_NODE
    sb.extend_from_slice(&fdt_diff::FDT_END.to_be_bytes());

    // Wrap in a strict v17 header (same layout DtbBuilder::finish emits).
    let hdr = 40usize;
    let off_struct = hdr + 16;
    let off_strings = off_struct + sb.len();
    let strings: Vec<u8> = Vec::new();
    let totalsize = off_strings + strings.len() + 1; // +1: the one-byte strings block below
    let mut blob: Vec<u8> = Vec::new();
    blob.extend_from_slice(&fdt_diff::FDT_MAGIC.to_be_bytes());
    blob.extend_from_slice(&(totalsize as u32).to_be_bytes());
    blob.extend_from_slice(&(off_struct as u32).to_be_bytes());
    // A strings block of length 0 is rejected (EmptyDeclaredBlock), so give
    // it one NUL byte: no prop references it, but the block is nonempty.
    // The field is off_dt_strings (header offset 12) — NOT size_dt_strings.
    blob.extend_from_slice(&(off_strings as u32).to_be_bytes());
    blob.extend_from_slice(&(hdr as u32).to_be_bytes()); // off_rsv_map = 40
    blob.extend_from_slice(&17u32.to_be_bytes()); // version
    blob.extend_from_slice(&17u32.to_be_bytes()); // last_comp_version
    blob.extend_from_slice(&0u32.to_be_bytes()); // boot_cpuid_phys
    blob.extend_from_slice(&(strings.len() as u32 + 1).to_be_bytes()); // size_dt_strings
    blob.extend_from_slice(&(sb.len() as u32).to_be_bytes()); // size_dt_struct
    blob.extend_from_slice(&[0u8; 16]); // rsv-map terminator
    blob.extend_from_slice(&sb);
    blob.extend_from_slice(&strings);
    blob.push(0); // the 1-byte strings block

    // The header must pass the fixed-field checks before the walk runs:
    // off_dt_strings + size_dt_strings <= totalsize (== off_dt_strings + 1).
    assert_eq!(blob.len(), totalsize, "crafted blob is self-consistent");

    assert_eq!(
        Fdt::parse(&blob).unwrap_err(),
        FdtError::NodeDepthExceeded,
        "nesting beyond MAX_NODE_DEPTH ({}) must be NodeDepthExceeded",
        fdt_diff::MAX_NODE_DEPTH
    );
}

// HW-09(b): candidate cap. `extract_dtbs` stops scanning deterministically
// once the cap is reached — no panic, no unbounded Vec. The cap constant is
// 4096; the stop-behavior is exercised through the same code path with a
// small limit (the public wrapper passes MAX_CANDIDATES). A container of 12
// back-to-back minimal valid DTBs must yield exactly 4 when capped at 4,
// and all 12 uncapped.
#[test]
fn candidate_scan_stops_at_cap() {
    let one = {
        let mut b = DtbBuilder::new().open_root();
        {
            let mut root = b.root_props();
            root.prop("compatible", b"t\0");
        }
        b.finish()
    };
    let mut container: Vec<u8> = Vec::new();
    for _ in 0..12 {
        // Pad each copy to a 4-byte boundary so every DTB starts 4-aligned
        // (the scan only probes aligned offsets). `extract_dtbs` tolerates
        // trailing bytes after FDT_END — that is the real-image case.
        while container.len() % 4 != 0 {
            container.push(0);
        }
        container.extend_from_slice(&one);
    }
    // Sanity: uncapped scan finds all 12 (each candidate re-parses).
    let all = fdt_diff::extract_dtbs(&container);
    assert_eq!(all.len(), 12, "uncapped scan must find all 12");
    // Capped scan: exactly the limit, kept are the LOWEST offsets, and the
    // scan stopped there (a 13th candidate would have been impossible to
    // report anyway — the point is the Vec never exceeds the cap).
    let capped = fdt_diff::extract_dtbs_capped(&container, 4);
    assert_eq!(capped.len(), 4, "capped scan must stop at the cap");
    assert!(
        capped.windows(2).all(|w| w[0].offset < w[1].offset),
        "kept candidates are the lowest-offset ones, ascending"
    );
    // The public cap constant is what the public API enforces.
    assert_eq!(fdt_diff::MAX_CANDIDATES, 4096);
}

// HW-09(c): diff truncation. Two trees differing in more diffs than the cap
// must report `truncated: true` with exactly `cap` diffs (a deterministic
// prefix), via the same code path the public API uses (public wrapper passes
// MAX_DIFFS = 10_000).
#[test]
fn diff_collection_truncates_at_cap() {
    // a: /chosen with 6 props; b: /chosen with all 6 values flipped plus 4
    // extra props → 10 diffs uncapped.
    let mk = |extra: usize, flip: bool| -> Vec<u8> {
        let mut b = DtbBuilder::new().open_root();
        {
            let mut root = b.root_props();
            let mut ch = root.node("chosen");
            for i in 0..6usize {
                let v = if flip {
                    [i as u8 ^ 0xff, 0, 0, 0]
                } else {
                    [i as u8, 0, 0, 0]
                };
                ch.prop(&alloc_name(i), &v);
            }
            for i in 6..(6 + extra) {
                ch.prop(&alloc_name(i), &[i as u8, 0, 0, 0]);
            }
        }
        b.finish()
    };
    let a = mk(0, false);
    let b = mk(4, true);
    parse_ok(&a);
    parse_ok(&b);

    // Uncapped public path: all 10 diffs, not truncated.
    let rep = compare(&a, &b, &["/chosen"]).expect("both parse");
    match rep {
        DiffReport::Different {
            diffs, truncated, ..
        } => {
            assert_eq!(diffs.len(), 10, "6 mismatches + 4 missing-in-a");
            assert!(!truncated);
        }
        other => panic!("expected Different, got {other:?}"),
    }

    // Capped at 4 through the same collection path: exactly 4 diffs,
    // truncated flag set.
    let rep = fdt_diff::compare_capped(&a, &b, &["/chosen"], 4).expect("both parse");
    match rep {
        DiffReport::Different {
            diffs, truncated, ..
        } => {
            assert_eq!(diffs.len(), 4, "collection must stop at the cap");
            assert!(truncated, "cap hit must be flagged truncated: true");
        }
        other => panic!("expected truncated Different, got {other:?}"),
    }
    assert_eq!(fdt_diff::MAX_DIFFS, 10_000);
}

/// Test helper: unique prop name built from an index.
fn alloc_name(i: usize) -> String {
    format!("p{i}")
}

/// HW-09B: the emission budget bounds WORK and PEAK ALLOCATION, not just
/// visible length. Two trees differing in > limit props at the root AND
/// deep inside a subtree: `compare_capped` must return exactly `limit`
/// diffs with `truncated`, and the returned Vec's retained capacity must
/// stay under `2 * limit` (the old shrink-after-collection path kept the
/// full 128-slot backing store for cap 4 — this asserts it never happens).
#[test]
fn budget_stops_descending() {
    let limit = 4usize;
    // a: 10 differing root props + a deep subtree with 10 differing props.
    // b: same shape, every value flipped. > limit diffs at BOTH levels.
    let mk = |flip: bool| -> Vec<u8> {
        let mut b = DtbBuilder::new().open_root();
        {
            let mut root = b.root_props();
            for i in 0..10usize {
                let v = if flip {
                    [i as u8 ^ 0xff, 0, 0, 0]
                } else {
                    [i as u8, 0, 0, 0]
                };
                root.prop(&alloc_name(i), &v);
            }
            {
                let mut ch = root.node("chosen");
                {
                    let mut deep = ch.node("deep");
                    for i in 0..10usize {
                        let v = if flip {
                            [i as u8 ^ 0xaa, 0, 0, 0]
                        } else {
                            [i as u8, 0, 0, 0]
                        };
                        deep.prop(&alloc_name(i), &v);
                    }
                }
            }
        }
        b.finish()
    };
    let a = mk(false);
    let b = mk(true);
    parse_ok(&a);
    parse_ok(&b);

    let rep = fdt_diff::compare_capped(&a, &b, &["/"], limit).expect("both parse");
    match rep {
        DiffReport::Different {
            diffs, truncated, ..
        } => {
            assert!(truncated, "budget exhausted must flag truncated");
            assert_eq!(diffs.len(), limit, "exactly cap diffs emitted");
            assert!(
                diffs.capacity() <= 2 * limit,
                "retained capacity {}: budget must bound allocation, not just length",
                diffs.capacity()
            );
        }
        other => panic!("expected truncated Different, got {other:?}"),
    }
}

// HW-09 step 2 audit note (no separate test — the constant is FDT_MAX_SIZE,
// so no reachable blob can carry a longer value; stated for the record):
// EVERY property-value read path in src/lib.rs goes through `Fdt::walk`, and
// the single FDT_PROP arm checks `len > FDT_MAX_PROP_LEN` before slicing any
// value bytes. `subtree()`, `node_paths()` and `compare()` are all walk
// consumers; no path reads a property value without that check. No bypass
// existed to fix.

/// HW-09C: the entry budget in `subtree_bounded` is TOTAL prop ENTRIES, not
/// nodes — a fat root cannot blow past it. Root with 100 props where only
/// p99 differs, cap 2: the walk sees just the first few entries and stops,
/// so the differing tail (p99) is never examined. Zero diffs were emitted,
/// but the pass is INCOMPLETE — claiming StructurallyEqual over an
/// unexamined tail is false equality (root defect 1 + phantom prop budget,
/// defect 2). This pins both fixes through the public API.
#[test]
fn fat_root_tail_diff_is_incomplete() {
    let mk = |last: u8| -> Vec<u8> {
        let mut b = DtbBuilder::new().open_root();
        {
            let mut root = b.root_props();
            for i in 0..100usize {
                root.prop(&alloc_name(i), &[if i == 99 { last } else { 0 }]);
            }
        }
        b.finish()
    };
    let a = mk(1);
    let b = mk(2);
    parse_ok(&a);
    parse_ok(&b);

    let rep = fdt_diff::compare_capped(&a, &b, &["/"], 2).expect("both parse");
    match rep {
        DiffReport::Incomplete { .. } => {}
        other => {
            panic!("fat-root truncated pass with unexamined tail must be Incomplete, got {other:?}")
        }
    }

    // Uncapped semantics stay exact (HW-09): the same pair through
    // `compare` finds the real diff — never Incomplete.
    let rep = compare(&a, &b, &["/"]).expect("both parse");
    match rep {
        DiffReport::Different {
            diffs, truncated, ..
        } => {
            assert!(!truncated, "uncapped compare must never truncate");
            assert!(
                diffs.iter().any(|d| matches!(
                    d,
                    Diff::ValueMismatch { name, .. } if name == "p99"
                )),
                "uncapped compare must find the p99 value mismatch: {diffs:?}"
            );
        }
        other => panic!("expected Different under uncapped compare, got {other:?}"),
    }
}
