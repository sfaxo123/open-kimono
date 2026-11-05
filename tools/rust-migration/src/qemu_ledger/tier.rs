//! Tier rules for the QEMU source ledger — explicit path prefixes plus
//! one explicit file list, no judgement left implicit (every rule is
//! documented at its site).
//!
//! - **T1** — the OnePlus 6 machine and its direct device files. The
//!   brief's explicit list is `hw/arm/oneplus6.c`,
//!   `hw/input/rmi4-s3706.c`, `hw/intc/qcom-pdc.c`, `hw/gpio/qcom-tlmm.c`,
//!   qcom-ufshc, `ui/input.c`; the list below is that set closed under
//!   the fork's own naming: every file whose basename carries one of the
//!   custom name fragments (`oneplus6`, `qcom`, `rmi4`, `geni`, `smem`,
//!   `pdc`, `tlmm`, `ufshc`, `s3706`) under `hw/` or `include/hw/` —
//!   the machine source, its SMEM/cmd-db support, the GENI UART/I2C
//!   devices + testbox, the GDSC/SMMUv2/SPMI-PMIC-arbiter misc devices,
//!   the UFS host controller, and each one's header — plus the exact
//!   file `ui/input.c` (the input surface the rmi4 device drives).
//! - **T2** — the rest of `hw/` and `include/hw/` (every QEMU device
//!   class subtree) and the qtest surfaces (`tests/qtest/` plus the
//!   python qtest harness under `python/qemu/`).
//! - **T3** — everything else.

/// The tier a QEMU tree path belongs to. Rules are ordered: T1 file
/// rules first, then the T2 prefix rules; anything unmatched is T3.
pub fn tier(path: &str) -> &'static str {
    // T1: the explicit input surface (brief list).
    if path == "ui/input.c" {
        return "T1";
    }
    // T1: the machine + its direct device files (closed name list,
    // basename-matched — see `t1_custom_name`).
    if (path.starts_with("hw/") || path.starts_with("include/hw/")) && t1_custom_name(path) {
        return "T1";
    }
    // T2: every other device surface.
    if path.starts_with("hw/") || path.starts_with("include/hw/") {
        return "T2";
    }
    // T2: the qtest surfaces — the C qtest suite and the python harness.
    if path.starts_with("tests/qtest/") || path.starts_with("python/qemu/") {
        return "T2";
    }
    "T3"
}

/// True when the file's basename carries one of the fork's custom name
/// fragments — the same set the classifier uses for `machine`, kept in
/// sync by the contract test. Basename-only, so no directory name can
/// drag an unrelated file up a tier; case-insensitive so upstream files
/// cannot collide by substring (at the pin: upstream `vmgenid` contains
/// `geni` case-sensitively and must stay T2).
fn t1_custom_name(path: &str) -> bool {
    let name = match path.rsplit_once('/') {
        Some((_, n)) => n,
        None => path,
    };
    const NAMES: [&str; 9] = [
        "oneplus6", "qcom", "rmi4", "geni", "smem", "pdc", "tlmm", "ufshc", "s3706",
    ];
    let lower = name.to_lowercase();
    NAMES.iter().any(|n| lower.contains(n))
}
