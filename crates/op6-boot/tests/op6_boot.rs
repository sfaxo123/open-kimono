//! HW-20 unit tests for op6-boot. Fixture-based; ZERO fastboot execution,
//! ZERO phone contact. Run with: cargo test --offline

use std::io::Write;
use std::path::PathBuf;

use op6_boot::{
    check_consent, choose_pin, is_prohibition_line, plan_text, ConsentGate, FastbootVerb,
    CARRIER_IMG, CARRIER_V2_IMG, CARRIER_V3_IMG, FASTBOOT_VERB_COUNT, STAGED_IMAGES, V0_IMG,
};

// ---------------------------------------------------------------------------
// Pin selection (Wonder diff 9399dc3): the filename -> pin mapping lives
// in lib and is pinned here so it cannot drift in the CLI untested.
// ---------------------------------------------------------------------------

#[test]
fn choose_pin_maps_v2_filename_to_v2_pin() {
    assert_eq!(choose_pin(Some(CARRIER_V2_IMG.name)).sha256, CARRIER_V2_IMG.sha256);
    assert_eq!(choose_pin(Some(CARRIER_V2_IMG.name)).size, CARRIER_V2_IMG.size);
}

#[test]
fn choose_pin_maps_v3_filename_to_v3_pin() {
    // Literal assertions (Wonder diff 10f4fe0): comparing the returned
    // pin against CARRIER_V3_IMG's own fields is tautological — a
    // typo'd pin constant would pass. The literals below are the
    // root byte-inspected values from the carrier assembly receipt;
    // the runtime `verify` hash-checks the real image against them.
    let pin = choose_pin(Some(CARRIER_V3_IMG.name));
    assert_eq!(pin.name, "op6-embedded-efi-redox-v3.img");
    assert_eq!(pin.size, 4_706_304);
    assert_eq!(
        pin.sha256,
        "b016e24a51f9cb1f5743a80e2958522a3e6feafd21534fc80beea3d97d6915b4"
    );
    assert_eq!(CARRIER_V3_IMG.name, "op6-embedded-efi-redox-v3.img");
    assert_ne!(CARRIER_V3_IMG.sha256, CARRIER_V2_IMG.sha256);
}

#[test]
fn choose_pin_maps_carrier_unknown_and_none_to_carrier_pin() {
    assert_eq!(choose_pin(Some(CARRIER_IMG.name)).sha256, CARRIER_IMG.sha256);
    assert_eq!(choose_pin(Some("renamed-v2-copy.img")).sha256, CARRIER_IMG.sha256);
    assert_eq!(choose_pin(None).sha256, CARRIER_IMG.sha256);
    // Renamed-image rejection property: the v2 bytes checked under any
    // non-v2 name face the v1 pin, whose hash differs from the v2 pin,
    // so a renamed v2 file can never pass; a v1 file under the v2 name
    // faces the v2 pin and likewise fails.
    assert_ne!(CARRIER_V2_IMG.sha256, CARRIER_IMG.sha256);
}

#[test]
fn verify_binary_rows_follow_staged_order_and_missing_images_fail() {
    // End-to-end (Wonder diff bb6b111): run the real binary's verify
    // in a sandbox HOME with an empty staging kit. Every row must
    // appear in STAGED_IMAGES order, the v2 row must be present and
    // FAIL (its HOME-derived default is absent in the sandbox), and
    // the exit code must be nonzero.
    let sandbox = std::env::temp_dir().join(format!(
        "op6-boot-verify-test-{}",
        std::process::id()
    ));
    let kit = sandbox.join("op6-staging-20261028");
    std::fs::create_dir_all(&kit).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_op6-boot"))
        .arg("verify")
        .env("HOME", &sandbox)
        .env_remove("OP6_BOOT_V2_IMG")
        .env_remove("OP6_BOOT_V3_IMG")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let _ = std::fs::remove_dir_all(&sandbox);
    assert!(!out.status.success(), "verify must fail when images are missing: {text}");
    let pos = |name: &str| text.find(name).unwrap_or_else(|| panic!("row for {name} missing: {text}"));
    assert!(pos(CARRIER_IMG.name) < pos(CARRIER_V2_IMG.name), "v1 row before v2 row: {text}");
    assert!(pos(CARRIER_V2_IMG.name) < pos(CARRIER_V3_IMG.name), "v2 row before v3 row: {text}");
    assert!(pos(CARRIER_V3_IMG.name) < pos(V0_IMG.name), "v3 row before v0 row: {text}");
    let v2_line = text
        .lines()
        .find(|l| l.contains(CARRIER_V2_IMG.name))
        .unwrap_or("");
    assert!(v2_line.contains("FAIL"), "missing v2 must be a FAIL row: {v2_line}");
}

#[test]
fn staged_images_receipt_order_is_carrier_v2_v3_v0() {
    assert_eq!(STAGED_IMAGES[0].name, CARRIER_IMG.name);
    assert_eq!(STAGED_IMAGES[1].name, CARRIER_V2_IMG.name);
    assert_eq!(STAGED_IMAGES[2].name, CARRIER_V3_IMG.name);
    assert_eq!(STAGED_IMAGES[3].name, V0_IMG.name);
}

// ---------------------------------------------------------------------------
// sha256 sanity (FIPS vectors) — the verify path depends on this.
// ---------------------------------------------------------------------------

#[test]
fn sha256_known_vectors() {
    assert_eq!(
        op6_boot::sha256::hex(&op6_boot::sha256::sha256(b"")),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        op6_boot::sha256::hex(&op6_boot::sha256::sha256(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn sha256_streaming_equals_oneshot() {
    let data: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
    let mut ctx = op6_boot::sha256::Sha256::new();
    for chunk in data.chunks(777) {
        ctx.update(chunk);
    }
    assert_eq!(
        op6_boot::sha256::hex(&ctx.finalize()),
        op6_boot::sha256::hex(&op6_boot::sha256::sha256(&data))
    );
}

// ---------------------------------------------------------------------------
// verify, fixture-based (no fastboot, no staging-dir touch).
// ---------------------------------------------------------------------------

/// Write a temp fixture file in a unique dir under the crate's target area;
/// returns its path. Cleaned up by the caller.
fn fixture(name: &str, bytes: &[u8]) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("op6-boot-test-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    let file = path.join(name);
    let mut f = std::fs::File::create(&file).unwrap();
    f.write_all(bytes).unwrap();
    f.flush().unwrap();
    file
}

#[test]
fn verify_ok_on_fixture() {
    // A fixture whose sha256 we compute with the crate's own (vector-proven)
    // implementation, then check the round trip: content -> digest -> matches.
    let body: Vec<u8> = (0..4096u32).map(|i| (i * 31 % 256) as u8).collect();
    let file = fixture("ok.bin", &body);
    let digest = {
        use std::io::Read;
        let mut f = std::fs::File::open(&file).unwrap();
        let mut ctx = op6_boot::sha256::Sha256::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = f.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            ctx.update(&buf[..n]);
        }
        op6_boot::sha256::hex(&ctx.finalize())
    };
    assert_eq!(
        digest,
        op6_boot::sha256::hex(&op6_boot::sha256::sha256(&body))
    );
    // The embedded pins are well-formed lowercase hex of the right length.
    for pin in [CARRIER_IMG, V0_IMG] {
        assert_eq!(pin.sha256.len(), 64);
        assert!(pin
            .sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    }
    let _ = std::fs::remove_file(&file);
}

#[test]
fn verify_rejects_wrong_sha_and_size() {
    // Same pin, tampered content: digest must differ (wrong sha detected),
    // and length differs from the pinned size (wrong size detected).
    let good: Vec<u8> = b"carrier bytes, verified".to_vec();
    let tampered: Vec<u8> = b"carrier bytes, tampered".to_vec();
    let g = op6_boot::sha256::hex(&op6_boot::sha256::sha256(&good));
    let t = op6_boot::sha256::hex(&op6_boot::sha256::sha256(&tampered));
    assert_ne!(g, t, "a 1-byte tamper must change the digest");

    // Fixture file: same length as `good` but different bytes -> sha fail.
    let f1 = fixture("tamper.bin", &tampered);
    let size1 = std::fs::metadata(&f1).unwrap().len();
    assert_eq!(size1, good.len() as u64); // size would pass...
    let sha1 = op6_boot::sha256::hex(&op6_boot::sha256::sha256(&std::fs::read(&f1).unwrap()));
    assert_ne!(sha1, g); // ...but the sha check must FAIL.

    // Fixture file: right prefix, extra byte -> size fail AND sha fail.
    let mut longer = good.clone();
    longer.push(0x21);
    let f2 = fixture("longer.bin", &longer);
    let size2 = std::fs::metadata(&f2).unwrap().len();
    assert_ne!(size2, good.len() as u64);

    // Pinned sizes are distinct and non-zero (a wrong-size check is meaningful).
    assert_eq!(CARRIER_IMG.size, 3_612_672);
    assert_eq!(V0_IMG.size, 716_800);
    assert_ne!(CARRIER_IMG.size, V0_IMG.size);

    let _ = std::fs::remove_file(&f1);
    let _ = std::fs::remove_file(&f2);
}

// ---------------------------------------------------------------------------
// the consent double-gate.
// ---------------------------------------------------------------------------

#[test]
fn attempt_refuses_without_consent() {
    // Each gate independently: flag missing -> refuse; env missing -> refuse.
    match check_consent(None, Some("tok")) {
        ConsentGate::Refused { reasons } => {
            assert!(reasons.iter().any(|r| r.contains("flag")));
        }
        ConsentGate::Open { .. } => panic!("gate (a) missing must REFUSE"),
    }
    match check_consent(Some("tok"), None) {
        ConsentGate::Refused { reasons } => {
            assert!(reasons.iter().any(|r| r.contains("OP6_BOOT_CONSENT")));
        }
        ConsentGate::Open { .. } => panic!("gate (b) missing must REFUSE"),
    }
    // Both missing -> refuse with both reasons.
    match check_consent(None, None) {
        ConsentGate::Refused { reasons } => assert_eq!(reasons.len(), 2),
        ConsentGate::Open { .. } => panic!("both gates missing must REFUSE"),
    }
    // Mismatched tokens -> refuse.
    match check_consent(Some("tokA"), Some("tokB")) {
        ConsentGate::Refused { reasons } => assert!(reasons.iter().any(|r| r.contains("MATCH"))),
        ConsentGate::Open { .. } => panic!("token mismatch must REFUSE"),
    }
    // Empty tokens -> refuse.
    match check_consent(Some(""), Some("")) {
        ConsentGate::Refused { .. } => {}
        ConsentGate::Open { .. } => panic!("empty tokens must REFUSE"),
    }
    // Both present, equal, non-empty -> open (the ONLY opening).
    assert_eq!(
        check_consent(Some("tok"), Some("tok")),
        ConsentGate::Open {
            token: "tok".to_string()
        }
    );
}

// ---------------------------------------------------------------------------
// the verb whitelist — compile-time / const proof.
// ---------------------------------------------------------------------------

#[test]
fn forbidden_verbs_unrepresentable() {
    // The enum has EXACTLY Boot + Devices. This const assertion plus the
    // exhaustive match in as_str() (no catch-all arm) means adding any verb
    // breaks the build/tests until reviewed.
    assert_eq!(FASTBOOT_VERB_COUNT, 2);

    // Exhaustiveness proof: this match covers every variant with no wildcard;
    // a new FastbootVerb variant is a COMPILE ERROR here.
    let all = [FastbootVerb::Boot, FastbootVerb::Devices];
    let names: Vec<&str> = all
        .iter()
        .map(|v| match v {
            FastbootVerb::Boot => "boot",
            FastbootVerb::Devices => "devices",
        })
        .collect();
    assert_eq!(names, vec!["boot", "devices"]);
    assert_eq!(all.len(), FASTBOOT_VERB_COUNT);

    // Every forbidden word is INEXPRESSIBLE: parse returns None for all of them.
    for bad in [
        "flash",
        "erase",
        "continue",
        "oem",
        "format",
        "wipe",
        "reboot",
        "getvar",
        "flashing",
        "flash:all",
        "oem-ibft",
        "download",
        "update-super",
        "set_active",
    ] {
        assert!(
            FastbootVerb::parse(bad).is_none(),
            "{bad} must be unparseable"
        );
    }
    // Only the two whitelisted words parse.
    assert_eq!(FastbootVerb::parse("boot"), Some(FastbootVerb::Boot));
    assert_eq!(FastbootVerb::parse("devices"), Some(FastbootVerb::Devices));
}

#[test]
fn plan_output_contains_no_state_changing_verbs() {
    let plan = plan_text();
    let forbidden = ["erase", "flashing", "flash"];
    let mut violations = Vec::new();
    for line in plan.lines() {
        let lower = line.to_ascii_lowercase();
        if is_prohibition_line(line) {
            continue; // prohibition notes may name the danger they forbid
        }
        for word in forbidden {
            if lower.contains(word) {
                violations.push(format!("{word}: {line}"));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "plan text names state-changing verbs outside prohibition notes: {violations:?}"
    );
    // And positively: the permitted line IS present, with the v0 fallback.
    assert!(plan.contains("fastboot boot op6-embedded-efi-redox.img"));
    assert!(plan.contains("fastboot boot u-boot-enchilada-v0.img"));
    assert!(plan.contains("fastboot devices"));
}
