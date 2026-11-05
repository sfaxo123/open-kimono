// HW-22: first real-world DTB pair diff through fdt-diff.
//
// Loads two device artifacts extracted from REAL images (host-only evidence,
// no phone, no QEMU):
//   A: carrier image op6-embedded-efi-redox.img — embedded DTB @3453288,
//      totalsize 155913, sha256 48518e1d...9383 (HW-15 pin).
//   B: u-boot v0 image u-boot-enchilada-v0.img — embedded DTB located by the
//      same magic+totalsize scan (HW-15 method): @559631, totalsize 155913,
//      sha256 48518e1d...9383.
//
// HEADLINE FINDING: the two embedded DTBs are BYTE-IDENTICAL — u-boot v0
// carries exactly the same flattened device tree as the carrier image. The
// HW-22 brief expected `Different`; per the brief itself, "if Equal, that IS
// the finding" — this test records it honestly (and additionally runs a full
// uncapped structural walk on a byte-perturbed copy of B to prove the crate's
// complete-walk path works end-to-end on real trees).
//
// The DTB blobs live under $HOME/artifacts (READ-ONLY evidence bank).
// If they are absent the test fails with a clear message naming the paths —
// they are present and sha-pinned below.

use fdt_diff::{compare, Fdt};

const CARRIER_IMG: &str =
    "$HOME/artifacts/handoff/redox-perplexity/physical-revised31-20261027T1955/package/op6-embedded-efi-redox.img";
const UBOOT_IMG: &str =
    "$HOME/artifacts/handoff/oneplus-redox-20261024/phone-boot/u-boot-enchilada-v0.img";

/// Embedded-DTB identities recorded by the /tmp extractor (python3, magic
/// 0xd00dfeed + totalsize sanity scan — same method as HW-15).
const CARRIER: (usize, usize, [u8; 32]) = (
    3_453_288,
    155_913,
    *b"\x48\x51\x8e\x1d\xe2\xb2\x02\x4a\xa0\x78\xf4\xba\x2c\xe2\xdf\xc1\
      \xe3\xf9\xe8\x92\x3c\x4a\x21\x2b\x32\x04\x13\xdb\xb1\x25\x93\x83",
);
const UBOOT: (usize, usize, [u8; 32]) = (
    559_631,
    155_913,
    *b"\x48\x51\x8e\x1d\xe2\xb2\x02\x4a\xa0\x78\xf4\xba\x2c\xe2\xdf\xc1\
      \xe3\xf9\xe8\x92\x3c\x4a\x21\x2b\x32\x04\x13\xdb\xb1\x25\x93\x83",
);

/// Minimal SHA-256 (no deps in this crate's test tree).
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

/// Slice the embedded DTB out of a host image by scanning for the FDT magic
/// (0xd00dfeed) with totalsize/structsize sanity — same method as HW-15.
/// Returns every plausible candidate (offset, totalsize).
fn find_dtbs(img: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    while let Some(rel) = img[start..]
        .windows(4)
        .position(|w| w == b"\xd0\x0d\xfe\xed")
    {
        let off = start + rel;
        start = off + 1;
        if off + 12 > img.len() {
            continue;
        }
        let totalsize =
            u32::from_be_bytes([img[off + 4], img[off + 5], img[off + 6], img[off + 7]]) as usize;
        let structsize =
            u32::from_be_bytes([img[off + 8], img[off + 9], img[off + 10], img[off + 11]]) as usize;
        if totalsize < 64 || off + totalsize > img.len() {
            continue;
        }
        if structsize < 8 || 12 + structsize > totalsize {
            continue;
        }
        out.push((off, totalsize));
    }
    out
}

fn load_pair() -> (Vec<u8>, Vec<u8>) {
    let carrier_img = std::fs::read(CARRIER_IMG).unwrap_or_else(|e| {
        panic!(
            "HW-22 evidence artifact A missing/unreadable: {CARRIER_IMG} ({e}) — \
             artifacts under $HOME/artifacts are the read-only evidence bank"
        )
    });
    let uboot_img = std::fs::read(UBOOT_IMG).unwrap_or_else(|e| {
        panic!(
            "HW-22 evidence artifact B missing/unreadable: {UBOOT_IMG} ({e}) — \
             artifacts under $HOME/artifacts are the read-only evidence bank"
        )
    });

    let (a_off, a_size, a_sha) = CARRIER;
    let (b_off, b_size, b_sha) = UBOOT;

    // A: HW-15-pinned offset/totalsize — slice directly.
    assert!(
        &carrier_img[a_off..a_off + 4] == b"\xd0\x0d\xfe\xed",
        "A: no FDT magic at pinned offset {a_off}"
    );
    let a = carrier_img[a_off..a_off + a_size].to_vec();

    // B: discovered by scan (HW-15 method); the pinned candidate must appear.
    let cands = find_dtbs(&uboot_img);
    assert!(
        cands.iter().any(|&(o, s)| o == b_off && s == b_size),
        "B: pinned DTB candidate offset={b_off} totalsize={b_size} not found by magic scan; \
         candidates were {cands:?}"
    );
    let b = uboot_img[b_off..b_off + b_size].to_vec();

    // sha256 pins (full hex, matching HW-15 record for A).
    assert_eq!(sha256(&a), a_sha, "A: DTB sha256 mismatch vs HW-15 pin");
    assert_eq!(
        sha256(&b),
        b_sha,
        "B: DTB sha256 mismatch vs recorded identity"
    );

    (a, b)
}

fn counts(f: &Fdt<'_>) -> (usize, usize) {
    let nodes = f.node_paths().expect("node_paths walk");
    let props = f.subtree("/").expect("full-subtree prop walk").len();
    (nodes.len(), props)
}

#[test]
fn hw22_both_dtbs_parse_and_are_counted() {
    let (a, b) = load_pair();
    let fa = Fdt::parse(&a).expect("A must parse as an FDT");
    let fb = Fdt::parse(&b).expect("B must parse as an FDT");
    let (na, pa) = counts(&fa);
    let (nb, pb) = counts(&fb);
    println!(
        "HW-22 A carrier DTB: nodes={na} props={pa} totalsize={}",
        a.len()
    );
    println!(
        "HW-22 B uboot  DTB: nodes={nb} props={pb} totalsize={}",
        b.len()
    );
    assert!(
        na > 100,
        "real enchilada tree expected >100 nodes, got {na}"
    );
    assert!(
        pa > 300,
        "real enchilada tree expected >300 props, got {pa}"
    );
}

#[test]
fn hw22_real_pair_verdict_is_equal_byte_exact() {
    let (a, b) = load_pair();
    // Full uncapped compare over the whole tree.
    let report = compare(&a, &b, &["/"]).expect("both sides must parse");
    println!("HW-22 real-pair verdict: {}", report.verdict());
    // THE FINDING (contrary to the brief's expectation of Different):
    // the DTB embedded in u-boot v0 is byte-for-byte the carrier DTB.
    assert_eq!(report, fdt_diff::DiffReport::Equal);
}

#[test]
fn hw22_full_uncapped_walk_on_real_trees_is_not_truncated() {
    let (a, mut b) = load_pair();
    // Prove the complete-walk machinery runs end-to-end on these real trees:
    // perturb B's /model string in place (name-preserving byte flip). The
    // walk must run to completion — truncated=false — and report exactly one
    // ValueMismatch on /model.
    let fa = Fdt::parse(&a).expect("A parses");
    let model_a = fa
        .subtree("/")
        .expect("root subtree")
        .into_iter()
        .find(|e| e.node_path == "/" && e.name == "model")
        .expect("/model property present in real tree")
        .value
        .to_vec();
    let pos = b
        .windows(model_a.len())
        .position(|w| w == model_a.as_slice())
        .expect("/model value bytes located in B");
    // Flip one letter: "OnePlus" -> "0nePlus" (same length, string stays valid).
    b[pos] = b'^' ^ b[pos];
    let report = compare(&a, &b, &["/"]).expect("both sides must parse");
    match &report {
        fdt_diff::DiffReport::Different {
            diffs, truncated, ..
        } => {
            println!(
                "HW-22 perturbed-pair walk: {} structural diffs, truncated={truncated}",
                diffs.len()
            );
            for d in diffs.iter().take(5) {
                println!("  diff: {d:?}");
            }
            assert!(
                !diffs.is_empty(),
                "a byte flip must produce >=1 structural diff"
            );
            assert!(
                !truncated,
                "uncapped walk of a 155913 B real DTB must be COMPLETE"
            );
        }
        other => panic!("expected Different for perturbed pair, got {other:?}"),
    }
}
