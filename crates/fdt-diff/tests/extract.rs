//! HW-08S acceptance tests for `extract_dtbs` + `best_candidate_rank`
//! (G3 evidence runner, salvaged onto the strict v17 parser).
//!
//! (a) synthetic container: builder DTB at offset 4096 in 8 KiB junk → exactly 1 candidate at 4096
//! (b) two embedded copies → 2 candidates, ascending
//! (c) magic + garbage totalsize (u32::MAX, totalsize past buffer) → rejected, no panic
//! (d) fixed-seed LCG junk 64 KiB → zero or parse-valid candidates only, never panic
//! (e) candidate slice re-parses and compares Equal to the source builder blob
//! (f) ranking_prefers_byte_equal: structurally-equal-but-not-byte-equal DTB
//!     at LOW offset + byte-equal DTB at HIGH offset → best_candidate_rank
//!     picks the byte-equal one (HW-08's defect, fixed here).

use fdt_diff::{best_candidate_rank, compare, extract_dtbs, DiffReport, DtbBuilder, Fdt};

/// Minimal valid builder blob used across tests.
fn builder_blob() -> Vec<u8> {
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
                fb.prop("format", b"a8r8g8b8\0");
                // (c2) construction: the LAST interned string is 4 chars
                // long ("name"), so its NUL lands exactly at the strings
                // block's end (size_str ≡ 0 mod 4), and its 1-byte value is
                // padded with 3 NULs — the blob's final byte is a pad, not
                // part of any name. Cutting the final pad byte keeps every
                // name NUL-terminated while the buffer is 1 byte short of
                // totalsize. With the totalsize-fits check in `Fdt::parse`
                // disabled, the block slices still resolve inside the
                // clamped buffer, FDT_END is intact, and the walk completes
                // — a phantom candidate. Only the fits check stops it, so
                // the mutation probe genuinely goes RED.
                fb.prop("name", b"ok\0");
            }
        }
    }
    b.finish()
}

/// Deterministic junk filler (never contains FDT_MAGIC by construction:
/// bytes cycle through a pattern that cannot spell d00dfeed at any 4-aligned
/// word — word 0 is 0x0000_0000 and 0x1111_1111-style words never appear).
fn junk(len: usize, seed_byte: u8) -> Vec<u8> {
    (0..len).map(|i| seed_byte.wrapping_add(i as u8)).collect()
}

// ---------------------------------------------------------------------------
// (a) one DTB embedded at offset 4096 inside 8 KiB of junk
// ---------------------------------------------------------------------------

#[test]
fn embedded_at_4096_single_candidate() {
    let dtb = builder_blob();
    let mut container = junk(4096, 0x40);
    container.extend_from_slice(&dtb);
    container.extend_from_slice(&junk(4096 - dtb.len(), 0x80));

    let cands = extract_dtbs(&container);
    assert_eq!(cands.len(), 1, "exactly one candidate expected");
    assert_eq!(cands[0].offset, 4096);
    assert_eq!(cands[0].totalsize, dtb.len());
}

// ---------------------------------------------------------------------------
// (b) two embedded copies → 2 candidates, ascending order
// ---------------------------------------------------------------------------

#[test]
fn two_copies_two_candidates_ascending() {
    let dtb = builder_blob();
    let mut container = Vec::new();
    container.extend_from_slice(&junk(1024, 0x10));
    container.extend_from_slice(&dtb);
    // Fill the gap with 4-aligned zero words (0x00000000 ≠ FDT_MAGIC, so no
    // spurious candidates) to keep the second copy 4-aligned.
    while container.len() % 4 != 0 {
        container.push(0);
    }
    container.extend_from_slice(&vec![0u8; 2048]);
    container.extend_from_slice(&dtb);
    container.extend_from_slice(&vec![0u8; 512]);

    let cands = extract_dtbs(&container);
    assert_eq!(cands.len(), 2);
    assert!(cands[0].offset < cands[1].offset, "ascending order");
    assert_eq!(cands[0].totalsize, dtb.len());
    assert_eq!(cands[1].totalsize, dtb.len());
}

// ---------------------------------------------------------------------------
// (c) magic + garbage totalsize → rejected, no panic
// ---------------------------------------------------------------------------

#[test]
fn garbage_totalsize_rejected_no_panic() {
    let dtb = builder_blob();

    // (c1) u32::MAX totalsize — absurd size, must be rejected.
    let mut big = Vec::new();
    big.extend_from_slice(&[0u8; 64]);
    big.extend_from_slice(&0xd00d_feedu32.to_be_bytes());
    big.extend_from_slice(&u32::MAX.to_be_bytes());
    big.extend_from_slice(&[0u8; 512]);
    assert!(
        extract_dtbs(&big).is_empty(),
        "u32::MAX totalsize must be rejected"
    );

    // (c2) CRAFTED totalsize-fits probe — the non-vacuous mutation gate.
    // The old drop-last-byte case was VACUOUS under the mutation
    // `if (totalsize as usize) > blob.len()` -> `if false` in Fdt::parse:
    // with parse's fit check disabled, the walk still failed once any
    // name reference lost its NUL in the truncated strings block, so no
    // phantom candidate ever appeared and the assert passed for the wrong
    // reason. This probe is designed so a phantom candidate genuinely
    // appears when the totalsize-fits rule is not enforced.
    //
    // Construction: take the builder blob (real length R = 300; every
    // declared block — structure and strings — ends at or before R, and
    // the last interned name's NUL lands exactly at the strings-block
    // end), patch header totalsize to R+12 = 312, then append 12 NUL
    // bytes so the crafted blob is self-consistent on disk (312-byte
    // buffer for ts=312). The container embeds the crafted blob after 128
    // junk bytes and TRUNCATES it to 128 + (R + 6): the handed region
    // covers every declared block end (300 ≤ 306), keeps all name
    // terminators intact, and keeps FDT_END intact — but is 6 bytes short
    // of the declared totalsize. With parse's `TotalsizeExceedsBuffer`
    // check disabled (the mutation posture named in HW-08S), the walk
    // completes over this region and a phantom candidate appears; with
    // the explicit fits check in extract_dtbs active, it never does.
    //
    // NOTE (why R+6, not R−1): empirically, truncating to R−1 or any
    // region < 300 drops the LAST interned name's NUL (str_off 43), so
    // the walk dies with BadStringOffset under EVERY parse posture — no
    // phantom candidate can appear, and the assert passes vacuously. The
    // phantom window for this crafted blob is [300, 311]: a region that
    // covers every declared block end but falls short of ts=312. R+6 =
    // 306 sits inside that window.
    let mut crafted = dtb.clone();
    let real = dtb.len();
    let ts_crafted = (real + 12) as u32;
    crafted[4..8].copy_from_slice(&ts_crafted.to_be_bytes()); // header totalsize := R+12
    crafted.extend_from_slice(&[0u8; 12]); // self-consistent on disk

    // Crafted-blob preconditions (HW-08's unfinished item): the strings
    // block must end exactly 4-aligned — i.e. the last interned name's NUL
    // is the final strings byte — and it must end strictly INSIDE the
    // declared totalsize, so a region of R+6 bytes loses no terminator and
    // trips no block-bounds rule other than the totalsize fit itself. If
    // the builder ever changes and one of these asserts fires, the (c2)
    // probe below would be vacuous; re-craft the blob instead.
    let off_str = u32::from_be_bytes([crafted[12], crafted[13], crafted[14], crafted[15]]) as usize;
    let size_str =
        u32::from_be_bytes([crafted[32], crafted[33], crafted[34], crafted[35]]) as usize;
    assert_eq!(
        u32::from_be_bytes([crafted[4], crafted[5], crafted[6], crafted[7]]),
        ts_crafted
    );
    assert_eq!(size_str % 4, 0, "strings block must end 4-aligned");
    assert_eq!(
        crafted[off_str + size_str - 1],
        0,
        "last name must be NUL-terminated at the strings-block end"
    );
    assert!(
        off_str + size_str < ts_crafted as usize,
        "declared blocks must end well inside the region the container hands over"
    );

    let mut phantom = Vec::new();
    phantom.extend_from_slice(&[0u8; 128]); // junk prefix (no FDT_MAGIC)
    phantom.extend_from_slice(&crafted);
    phantom.truncate(128 + real + 6); // region = R+6: blocks fit, ts does not

    assert!(
        extract_dtbs(&phantom).is_empty(),
        "totalsize past buffer must be rejected — phantom candidate under the \
         totalsize-fits mutation; this assert is the HW-08S2 mutation-gate target"
    );

    // Never panic on degenerate inputs.
    assert!(extract_dtbs(&[]).is_empty());
    assert!(extract_dtbs(&[0u8; 4]).is_empty());
}

// ---------------------------------------------------------------------------
// (d) fixed-seed LCG junk 64 KiB → zero or parse-valid candidates, never panic
// ---------------------------------------------------------------------------

#[test]
fn lcg_junk_never_panics() {
    let mut state: u64 = 0x2545_F491_4F6C_DD1D; // fixed seed
    let mut next = || {
        // LCG (Knuth MMIX)
        state = state
            .wrapping_mul(6364_1362_2384_6793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as u8
    };
    let junk: Vec<u8> = (0..64 * 1024).map(|_| next()).collect();

    let cands = extract_dtbs(&junk);
    // Every returned candidate must independently re-parse (or be empty).
    for c in &cands {
        let end = c.offset + c.totalsize;
        assert!(end <= junk.len());
        Fdt::parse(&junk[c.offset..end]).expect("candidate must parse");
    }
}

// ---------------------------------------------------------------------------
// (e) candidate slice re-parses and compares Equal to source blob
// ---------------------------------------------------------------------------

#[test]
fn candidate_roundtrips_equal() {
    let dtb = builder_blob();
    let mut container = junk(2048, 0x33);
    container.extend_from_slice(&dtb);
    container.extend_from_slice(&junk(1000, 0x77));

    let cands = extract_dtbs(&container);
    assert_eq!(cands.len(), 1);
    let c = &cands[0];
    let slice = &container[c.offset..c.offset + c.totalsize];
    Fdt::parse(slice).expect("candidate slice must re-parse");
    let report = compare(&dtb, slice, &["/"]).expect("compare must not error");
    assert_eq!(report, DiffReport::Equal);
}

// ---------------------------------------------------------------------------
// (f) ranking prefers byte-equal over structurally-equal (HW-08 defect fix)
// ---------------------------------------------------------------------------

/// A second builder blob: SAME tree shape and prop values as
/// `builder_blob`, but the node is spelled `framebuffer` instead of
/// `framebuffer-0`. Compared against `builder_blob` it yields exactly one
/// structural diff (a NameMismatch), so it is neither byte-equal nor
/// structurally equal — it ranks 2 while a copy of the source ranks 0.
/// (Byte-equality is decided on raw bytes, so no string-block permutation
/// could fake it; any not-byte-equal blob that compares with zero diffs is
/// rank 1 and still loses to the byte-equal copy.)
fn variant_blob() -> Vec<u8> {
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
                fb.prop("format", b"a8r8g8b8\0");
                fb.prop("name", b"ok\0");
            }
        }
    }
    b.finish()
}

#[test]
fn ranking_prefers_byte_equal() {
    let src = builder_blob(); // the "captured" blob
    let variant = variant_blob();
    assert_ne!(variant, src);

    // Synthetic img: structurally-comparable-but-not-byte-equal DTB at a
    // LOW offset, byte-equal copy at a HIGH offset. HW-08's runner picked
    // the first zero-diff candidate; the fixed ranking must pick the
    // byte-equal one regardless of order. Inter-DTB gaps are 4-aligned
    // zero words: 0x00000000 != FDT_MAGIC, so they never seed spurious
    // candidates (a ramp-style filler can accidentally spell d00dfeed at
    // an aligned word and inject phantom candidates).
    let mut img = junk(1024, 0x20);
    while img.len() % 4 != 0 {
        img.push(0);
    }
    img.extend_from_slice(&variant);
    img.extend_from_slice(&vec![0u8; 512]);
    img.extend_from_slice(&src);

    let cands = extract_dtbs(&img);
    assert_eq!(cands.len(), 2, "both embedded DTBs must be candidates");
    assert!(cands[0].offset < cands[1].offset);

    let verdicts: Vec<_> = cands
        .iter()
        .map(|c| compare(&src, &img[c.offset..c.offset + c.totalsize], &["/"]))
        .collect();
    // Sanity: neither candidate errors out.
    for v in &verdicts {
        assert!(v.is_ok(), "candidates must compare, got {:?}", v);
    }

    let best = best_candidate_rank(&cands, &verdicts).expect("non-empty ranking");
    let winner = &cands[best];
    let wslice = &img[winner.offset..winner.offset + winner.totalsize];
    assert_eq!(wslice, src.as_slice(), "winner must be BYTE-EQUAL");
    assert_eq!(
        winner.offset, cands[1].offset,
        "byte-equal sits at HIGH offset"
    );

    // Ties break on lowest offset: a dedicated container holding TWO
    // byte-equal copies of `src` (zero-word gap) → ranking must pick the
    // first (lowest-offset) one.
    let mut img3 = junk(1024, 0x20);
    while img3.len() % 4 != 0 {
        img3.push(0);
    }
    img3.extend_from_slice(&src);
    img3.extend_from_slice(&vec![0u8; 512]);
    img3.extend_from_slice(&src);
    let cands3 = extract_dtbs(&img3);
    assert_eq!(cands3.len(), 2, "two byte-equal copies expected");
    let dup_verdicts: Vec<_> = cands3
        .iter()
        .map(|c| compare(&src, &img3[c.offset..c.offset + c.totalsize], &["/"]))
        .collect();
    for v in &dup_verdicts {
        assert!(matches!(v, Ok(DiffReport::Equal)), "all byte-equal");
    }
    let best_dup = best_candidate_rank(&cands3, &dup_verdicts).expect("non-empty ranking");
    assert_eq!(best_dup, 0, "byte-equal tie must break on lowest offset");
}
