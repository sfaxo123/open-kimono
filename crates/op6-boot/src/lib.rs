//! op6-boot — Rust replacement for the bash `BOOT-ATTEMPT.sh` (HW-20).
//!
//! Root-of-record advisory (2026-10-28 18:04 PT): the bash `BOOT-ATTEMPT.sh`
//! staged in `workbench/HW-PREP-BOOT.md` §9 conflicts with the Rust-only
//! helper requirement and is preserved as HISTORICAL EVIDENCE only. This crate
//! replaces its decision logic in typed Rust.
//!
//! Three subcommands (see `main.rs`):
//! - `verify`  — streamed SHA-256 + size check of the two staged images
//!   against embedded pins; prints a receipt table. Read-only.
//! - `plan`    — prints the exact operator-hands checklist (mirrors the
//!   STAGING-RECEIPT.md / §4 numbering) and the single permitted command
//!   line. NEVER runs anything.
//! - `attempt` — HARD-GATED: refuses unless BOTH `--operator-consent <token>`
//!   and `OP6_BOOT_CONSENT=<same token>` are present. When gated open it runs
//!   exactly `fastboot devices` then `fastboot boot <img>` — nothing else.
//!
//! The dangerous surface is minimized BY CONSTRUCTION: the only expressible
//! fastboot verbs are [`FastbootVerb::Boot`] and [`FastbootVerb::Devices`].
//! `flash`, `erase`, `continue`, `oem`, `reboot`, `getvar` and every other
//! state-changing or informational verb cannot be represented in the type
//! system, so no code path in this crate can invoke them.
//!
//! No external dependencies. `cargo --offline` compatible.

pub mod sha256;

use std::fmt;
use std::path::Path;

// ---------------------------------------------------------------------------
// Pinned image identity (from HW-PREP-BOOT.md §2 / §9, re-verified on the
// 3090 by HW-13 with zero drift).
// ---------------------------------------------------------------------------

/// One staged image: name, expected byte size, expected SHA-256 (lowercase hex).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImagePin {
    pub name: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    pub role: &'static str,
}

/// The v0 fallback image (`u-boot-enchilada-v0.img`), fallback only per
/// HW-DTB §3.1 — it carries no DTB.
pub const V0_IMG: ImagePin = ImagePin {
    name: "u-boot-enchilada-v0.img",
    size: 716_800,
    sha256: "2f86f373c21d3eb083624844db3ed002d674edea52eadf86ce3a08c34ab89f72",
    role: "fallback only (no DTB, HW-DTB §3.1)",
};

/// The full31 carrier image (`op6-embedded-efi-redox.img`) — the image the
/// attempt boots (RAM load via `fastboot boot`).
pub const CARRIER_IMG: ImagePin = ImagePin {
    name: "op6-embedded-efi-redox.img",
    size: 3_612_672,
    sha256: "1385d2d1d48a4b88fbcb9b51e424a1badddbf42d9d8c5ff23b9e3ddd6dc78be5",
    role: "the full31 carrier the attempt boots",
};

/// The payload-v2 carrier image (`op6-embedded-efi-redox-v2.img`) — the
/// image Attempt #2 boots (RAM load via `fastboot boot`). Same U-Boot pin,
/// same DTB pin, same kernel as the full31 carrier; only the ramboot
/// initfs changed (adds the log-only SMMU probe daemon `usbgadgetd`,
/// staged binary sha256 1a3399d0e878ec30...). Byte-inspected by root
/// 2026-10-30: embedded payload sha256 == the step-2 EFI, FDT sha256 ==
/// the pinned phone DTB.
pub const CARRIER_V2_IMG: ImagePin = ImagePin {
    name: "op6-embedded-efi-redox-v2.img",
    size: 4_706_304,
    sha256: "cb4c64ccc531492f4e160c15b857478daccede0c3d53cc9f42a83bbd36450c33",
    role: "the payload-v2 carrier Attempt #2 boots (log-only)",
};

/// The probe-2 carrier image (`op6-embedded-efi-redox-v3.img`) — the
/// image the probe-2 SMMU global-register readback boots (RAM load via
/// `fastboot boot`). Same U-Boot pin, same DTB pin, same kernel as the
/// v2 carrier; only the ramboot initfs changed (usbgadgetd rebuilt with
/// the corrected IDR0 NUMSMRG decode + the SMMU global-register dump,
/// staged binary sha256 c8d1f9d33aaeba...). Byte-inspected by root
/// 2026-10-30: embedded payload sha256 == the probe-2 EFI, FDT sha256 ==
/// the pinned phone DTB.
pub const CARRIER_V3_IMG: ImagePin = ImagePin {
    name: "op6-embedded-efi-redox-v3.img",
    size: 4_706_304,
    sha256: "b016e24a51f9cb1f5743a80e2958522a3e6feafd21534fc80beea3d97d6915b4",
    role: "the probe-2 carrier the SMMU global readback boots (log-only)",
};

/// The staged images, in receipt-table order.
pub const STAGED_IMAGES: [ImagePin; 4] = [CARRIER_IMG, CARRIER_V2_IMG, CARRIER_V3_IMG, V0_IMG];

/// Pin selection for an attempt's chosen image, by file name: the v2
/// carrier name selects [`CARRIER_V2_IMG`] and the v3 carrier name
/// selects [`CARRIER_V3_IMG`]; every other name (including the v1
/// carrier name, an unknown name, or no name) selects
/// [`CARRIER_IMG`]. Fail-closed: a v2 or v3 file under any other name
/// is checked against the v1 pin and fails its hash check, and a v1
/// file under the v2 or v3 name fails that carrier's pin — renaming
/// can never make a drifted image pass. Lives in lib (not the CLI)
/// so the mapping is unit-tested (Wonder diff 9399dc3).
pub fn choose_pin(file_name: Option<&str>) -> &'static ImagePin {
    match file_name {
        Some(n) if n == CARRIER_V2_IMG.name => &CARRIER_V2_IMG,
        Some(n) if n == CARRIER_V3_IMG.name => &CARRIER_V3_IMG,
        _ => &CARRIER_IMG,
    }
}

// ---------------------------------------------------------------------------
// The verb whitelist — the whole safety argument lives in this type.
// ---------------------------------------------------------------------------

/// The ONLY fastboot verbs this crate can express. `Boot` = RAM load via ABL
/// (nothing persisted); `Devices` = read-only enumeration. Every state-changing
/// verb (flash, erase, continue, oem, format, wipe, reboot-to-bootloader, ...)
/// is UNREPRESENTABLE: there is no variant, no parse path, and no `From`/cast
/// that yields one. Exhaustiveness is pinned by [`FASTBOOT_VERB_COUNT`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FastbootVerb {
    Boot,
    Devices,
}

/// Exact number of variants in [`FastbootVerb`]. The unit test
/// `forbidden_verbs_unrepresentable` asserts this equals 2 AND that the
/// exhaustive `match` in [`FastbootVerb::as_str`] compiles — adding any
/// variant breaks that test, so the whitelist cannot silently grow.
pub const FASTBOOT_VERB_COUNT: usize = 2;

impl FastbootVerb {
    /// The literal fastboot argument for this verb. The match is EXHAUSTIVE
    /// with no catch-all arm: any new variant is a compile error here until
    /// reviewed, and the const-count test refuses an unreviewed growth.
    pub fn as_str(self) -> &'static str {
        match self {
            FastbootVerb::Boot => "boot",
            FastbootVerb::Devices => "devices",
        }
    }

    /// Parse a requested verb. ONLY "boot" and "devices" succeed; everything
    /// else — including every forbidden word — returns `None`, so a forbidden
    /// verb can never reach an exec path.
    pub fn parse(s: &str) -> Option<FastbootVerb> {
        match s {
            "boot" => Some(FastbootVerb::Boot),
            "devices" => Some(FastbootVerb::Devices),
            _ => None,
        }
    }
}

impl fmt::Display for FastbootVerb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// The consent double-gate.
// ---------------------------------------------------------------------------

/// Outcome of evaluating the consent double-gate for `attempt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsentGate {
    /// Both gates satisfied: flag token present AND env token equal to it.
    Open { token: String },
    /// Gate refused, with the reason(s) in operator-facing text.
    Refused { reasons: Vec<String> },
}

/// The refusal text shown whenever the gate does not open.
pub const REFUSAL_TEXT: &str = "REFUSED: the boot attempt is HARD-GATED. Both of \
these are required, independently:\n  \
  (a) flag  --operator-consent <token>   on the command line\n  \
  (b) env   OP6_BOOT_CONSENT=<same token> in the shell environment\n\
The attempt also remains operator-hands gated per HW-PREP-BOOT.md §4: fastboot \
mode entry, cable, and the one-shot rule are HUMAN steps. This tool will not \
run any fastboot verb without BOTH consent inputs.";

/// Evaluate the double-gate. BOTH inputs are required and must carry the SAME
/// non-empty token. Either one missing, or a mismatch, refuses.
pub fn check_consent(flag_token: Option<&str>, env_token: Option<&str>) -> ConsentGate {
    let mut reasons = Vec::new();

    match flag_token {
        None => reasons.push("missing flag --operator-consent <token> (gate a)".to_string()),
        Some("") => {
            reasons.push("flag --operator-consent given an EMPTY token (gate a)".to_string())
        }
        Some(_) => {}
    }

    match env_token {
        None => reasons.push("missing env OP6_BOOT_CONSENT (gate b)".to_string()),
        Some("") => reasons.push("env OP6_BOOT_CONSENT is EMPTY (gate b)".to_string()),
        Some(_) => {}
    }

    if let (Some(f), Some(e)) = (flag_token, env_token) {
        if !f.is_empty() && !e.is_empty() && f != e {
            reasons.push(
                "flag token and env OP6_BOOT_CONSENT token DO NOT MATCH (gates a+b)".to_string(),
            );
        }
    }

    if reasons.is_empty() {
        ConsentGate::Open {
            token: flag_token.unwrap().to_string(),
        }
    } else {
        ConsentGate::Refused { reasons }
    }
}

// ---------------------------------------------------------------------------
// The plan: operator-hands checklist + the single permitted command line.
// ---------------------------------------------------------------------------

/// The single permitted command line (NEVER executed by this tool).
pub const PERMITTED_COMMAND: &str = "fastboot boot op6-embedded-efi-redox.img";

/// The documented fallback command line (v0 img, no DTB — HW-DTB §3.1). Use
/// only on a coordinator decision after the carrier attempt.
pub const FALLBACK_COMMAND: &str = "fastboot boot u-boot-enchilada-v0.img";

/// The operator-hands checklist. Numbering mirrors the STAGING-RECEIPT.md /
/// HW-PREP-BOOT.md §4 procedure as staged in the 3090 kit
/// (`~/op6-staging-20261028/STAGING-RECEIPT.md`). Steps 1-3 are HUMAN; step 4
/// is the ONLY phone-affecting command, typed by the operator.
///
/// Prohibition note (the ONLY place forbidden words appear in this output):
/// PROHIBITED VERBS — never run: flash, erase, continue, oem, format, wipe,
/// reboot-to-bootloader. `fastboot boot` is a RAM load via ABL; nothing is
/// written; pmOS is untouchable by the boot itself.
pub fn plan_text() -> String {
    let mut s = String::new();
    s.push_str("OP6 FIRST-BOOT ATTEMPT — OPERATOR-HANDS CHECKLIST (this tool runs NOTHING here)\n");
    s.push_str("===============================================================================\n");
    s.push_str("\n");
    s.push_str("PROHIBITION NOTE: the ONLY permitted fastboot verbs are `devices`\n");
    s.push_str("(read-only enumeration) and `boot` (RAM load via ABL). PROHIBITED —\n");
    s.push_str("never run, and unrepresentable in this tool: any state-changing verb.\n");
    s.push_str("\n");
    s.push_str("Pre-flight (no phone contact):\n");
    s.push_str("  0. Battery >= 30% — read-only check BEFORE unplugging, while pmOS is up:\n");
    s.push_str("       ssh oneplus 'cat /sys/class/power_supply/bq27545-0/capacity'\n");
    s.push_str("       (use whichever bq27* path exists). Operator consent for the whole\n");
    s.push_str("       sequence: required.\n");
    s.push_str("  0b. Run `op6-boot verify` on the staging host; both images must PASS\n");
    s.push_str("      the size+sha256 receipt BEFORE any phone step.\n");
    s.push_str("\n");
    s.push_str("Phone into fastboot mode (OH-2a, button combo — hands-light):\n");
    s.push_str("  1. Phone USB-attached to the staging host (3090, REAR port), pmOS running.\n");
    s.push_str("  2. Hold Power + Volume Down ~10-15 s. Expect the fastboot/ABL download\n");
    s.push_str("     screen, or a BLACK screen (this device shows black-screen fastboot).\n");
    s.push_str("     Do not panic at black.\n");
    s.push_str("  3. On the staging host: fastboot devices -> expect exactly\n");
    s.push_str("     `0a1b2c3d fastboot`. Empty -> wait 30 s, re-check ONCE; still empty ->\n");
    s.push_str("     recovery: hold Power ~10-15 s (force power-off) -> normal boot -> pmOS.\n");
    s.push_str("     No harm done.\n");
    s.push_str("\n");
    s.push_str("The RAM boot (OH-3 — the ONLY phone-affecting command):\n");
    s.push_str("  4. Type, by operator hands, path verified against §2 in the same session:\n");
    s.push_str(&format!("       {PERMITTED_COMMAND}\n"));
    s.push_str("     Fallback (documented, use ONLY on a coordinator decision after the\n");
    s.push_str("     carrier attempt; v0 carries no DTB, HW-DTB §3.1):\n");
    s.push_str(&format!("       {FALLBACK_COMMAND}\n"));
    s.push_str("  5. Expect: fastboot reports transfer, then host silence (ABL jumps into\n");
    s.push_str("     headless U-Boot, CONFIG_BOOTDELAY=-2). Only output channel: the panel.\n");
    s.push_str("  6. Panel evidence ladder: ABL splash persists -> boot-console text via\n");
    s.push_str("     op6/fbcon (K14) = success signal — or the Redox shell/login prompt.\n");
    s.push_str("  7. Hang, no panel change ~2 min: hold Power ~15-20 s (hardware\n");
    s.push_str("     force-reboot) -> ABL -> pmOS. NO retries without a coordinator\n");
    s.push_str("     decision (PERPLEXITY-REVIEW: one boot attempt only, capture, no retries).\n");
    s.push_str("\n");
    s.push_str("STOP rule (applies at every step): ANY sign of a partition write — a\n");
    s.push_str("state-changing fastboot invocation, a `target confirmed`/wipe message, the\n");
    s.push_str(
        "phone enumerating as EDL (05c6:9008), or a screen saying the device is being
",
    );
    s.push_str("updated to firmware —\n");
    s.push_str("ABORT: touch nothing further, photograph the screen, call the coordinator.\n");
    s.push_str("\n");
    s.push_str("Capture (set up BEFORE step 4): continuous video of the panel from the\n");
    s.push_str("button combo to final state, plus host-side `script`/`tee` of the session.\n");
    s
}

/// True if `text` contains no state-changing fastboot verb outside lines
/// marked as prohibition notes (lines containing "PROHIBIT" — case-insensitive
/// check done by the caller's test). Used by the plan test.
pub fn is_prohibition_line(line: &str) -> bool {
    line.to_ascii_lowercase().contains("prohibit")
}

// ---------------------------------------------------------------------------
// HW-27: the process seam + attempt logic (outcome corrections, device
// selection contract, double verify).
//
// Root-of-record advisory (2026-10-28 ~19:54 PT) named two concrete outcome
// bugs in the old inline `attempt()`:
//   1. `fastboot devices` output handling IGNORED the exit status — a nonzero
//      exit with non-empty stdout proceeded to boot.
//   2. `fastboot boot` mapped Ok(status) to exit 0 REGARDLESS of
//      `status.success()` — fabricating CLI success on a failed boot.
// Both are fixed here, and the whole process surface now goes through an
// injectable seam ([`FastbootRunner`]) so the logic is testable with a pure
// in-process fake — no process spawn in any test of this module.
// ---------------------------------------------------------------------------

/// The outcome of one process execution through the seam: `ok` is the exit
/// status's success flag (what `ExitStatus::success()` reported), plus the
/// captured stdout/stderr as lossy UTF-8. HW-27 bug 1 was exactly a dropped
/// `ok`; this type makes dropping it impossible to hide in review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcOutcome {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Object-safe process seam. `args` are the fastboot arguments WITHOUT the
/// program name (e.g. `["devices"]`, `["-s", "<serial>", "boot", "<img>"]` —
/// HW-30 binds the selected serial into the boot invocation). The real impl is
/// [`RealFastboot`]; tests use a pure fake that never spawns anything.
pub trait FastbootRunner {
    fn run(&self, args: &[&str]) -> Result<ProcOutcome, std::io::Error>;
}

/// The real runner: `std::process::Command::new("fastboot").args(args)`.
/// Behavior unchanged from the old inline attempt code — capture output,
/// report the exit status's success flag honestly.
pub struct RealFastboot;

impl FastbootRunner for RealFastboot {
    fn run(&self, args: &[&str]) -> Result<ProcOutcome, std::io::Error> {
        let out = std::process::Command::new("fastboot").args(args).output()?;
        Ok(ProcOutcome {
            ok: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

/// Outcome of [`attempt_run`]. The CLI maps these to exit codes:
/// `Booted` → 0, the abort variants → 1, `BootFailed` → 4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttemptOutcome {
    /// devices enumerated cleanly, selection contract satisfied, and the boot
    /// process reported success. The ONLY outcome that maps to exit 0.
    Booted,
    /// A failed boot PROCESS (ok == false): distinct from every abort so the
    /// operator can tell "the phone rejected the boot" from "we never got
    /// there". HW-27 bug 2 fabricated success here; now it cannot.
    BootFailed { message: String },
    /// `fastboot devices` itself failed (nonzero exit or unrunnable).
    DevicesAbort { reason: String },
    /// The enumeration did not satisfy the selection contract (see below).
    SelectionAbort { reason: String },
    /// The second (pre-boot) pin verify returned false — TOCTOU mitigation.
    VerifyAbort { reason: String },
}

impl AttemptOutcome {
    /// The CLI exit code for this outcome (HW-27 table): 0 boot-ok,
    /// 1 pre-flight/consent/devices abort, 4 boot-failed.
    pub fn exit_code(&self) -> i32 {
        match self {
            AttemptOutcome::Booted => 0,
            AttemptOutcome::BootFailed { .. } => 4,
            AttemptOutcome::DevicesAbort { .. }
            | AttemptOutcome::SelectionAbort { .. }
            | AttemptOutcome::VerifyAbort { .. } => 1,
        }
    }

    /// Operator-facing reason text (empty for `Booted`).
    pub fn reason(&self) -> &str {
        match self {
            AttemptOutcome::Booted => "",
            AttemptOutcome::BootFailed { message }
            | AttemptOutcome::DevicesAbort { reason: message }
            | AttemptOutcome::SelectionAbort { reason: message }
            | AttemptOutcome::VerifyAbort { reason: message } => message,
        }
    }
}

/// One parsed line of `fastboot devices` output: `<serial>\t<state>` (the tab
/// may be a run of spaces — both real fastboot layouts are tolerated).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceLine {
    pub serial: String,
    pub state: String,
}

/// Parse `fastboot devices` stdout into device lines. Blank/whitespace-only
/// lines are skipped. A line with no separable state field is rejected
/// (`None`) so garbage output aborts rather than boots.
pub fn parse_devices_lines(stdout: &str) -> Option<Vec<DeviceLine>> {
    let mut lines = Vec::new();
    for raw in stdout.lines() {
        if raw.trim().is_empty() {
            continue;
        }
        // Preferred separator: the tab fastboot actually prints.
        let (serial, state) = match raw.split_once('\t') {
            Some((s, st)) => (s.trim(), st.trim()),
            None => {
                // Tolerate a run of spaces instead of the tab.
                let mut it = raw.split_whitespace();
                let serial = it.next()?;
                let state = it.next()?;
                if it.next().is_some() {
                    return None; // more than serial+state: unrecognized layout
                }
                (serial, state)
            }
        };
        if serial.is_empty() || state.is_empty() {
            return None;
        }
        lines.push(DeviceLine {
            serial: serial.to_string(),
            state: state.to_string(),
        });
    }
    Some(lines)
}

/// The device-selection contract, enforced here (HW-27):
/// - EXACTLY ONE enumerated line must have state `fastboot`; 0 or >1 aborts;
/// - if `expect_serial` is given, the selected serial must match it exactly.
/// Returns the selected serial, or the abort reason.
pub fn select_device<'a>(
    lines: impl IntoIterator<Item = &'a DeviceLine>,
    expect_serial: Option<&str>,
) -> Result<String, String> {
    let fastboot: Vec<&DeviceLine> = lines
        .into_iter()
        .filter(|l| l.state == "fastboot")
        .collect();
    match fastboot.len() {
        0 => Err("no device in state `fastboot` in the enumeration".to_string()),
        1 => {
            let serial = fastboot[0].serial.clone();
            if let Some(want) = expect_serial {
                if serial != want {
                    return Err(format!(
                        "selected serial {serial} does not match --expect-serial {want}"
                    ));
                }
            }
            Ok(serial)
        }
        n => Err(format!(
            "{n} devices in state `fastboot` — exactly one is required (ambiguous target, abort)"
        )),
    }
}

/// The attempt decision logic, process execution behind [`FastbootRunner`]
/// only. `verify` is the image-pin verification closure; it is called TWICE
/// inside this function: once BEFORE any process runs (pre-flight) and again
/// immediately after a successful devices enumeration, right before boot
/// (double verify).
///
/// TOCTOU status (honest): the double verify narrows the window between
/// "path verified" and "fastboot reads the file", but a RESIDUAL RACE REMAINS
/// — fastboot reopens the image BY PATH after we return, so an attacker (or a
/// sync bug) swapping the file between our second verify and fastboot's open
/// is not fully closed. Closing it completely needs fd-passing fastboot
/// support (hand it an already-open, verified fd) — out of scope for this
/// crate and this fleet's fastboot.
pub fn attempt_run(
    runner: &dyn FastbootRunner,
    chosen: &Path,
    expect_serial: Option<&str>,
    mut verify: impl FnMut() -> bool,
) -> AttemptOutcome {
    let chosen_str = chosen.display().to_string();

    // First verify: BEFORE any process touches the phone path.
    if !verify() {
        return AttemptOutcome::VerifyAbort {
            reason: format!("pre-flight verify FAILED for {chosen_str} — do not boot this file"),
        };
    }

    // `fastboot devices` — HW-27 bug 1 fix: the exit status IS honored. A
    // nonzero exit aborts, with stderr surfaced, even if stdout is non-empty.
    let devices_verb = FastbootVerb::Devices.to_string();
    let dev = match runner.run(&[devices_verb.as_str()]) {
        Err(e) => {
            return AttemptOutcome::DevicesAbort {
                reason: format!("cannot run fastboot {devices_verb}: {e}"),
            }
        }
        Ok(o) => o,
    };
    if !dev.ok {
        let err = dev.stderr.trim();
        return AttemptOutcome::DevicesAbort {
            reason: format!(
                "`fastboot {devices_verb}` exited nonzero — {}",
                if err.is_empty() {
                    "(no stderr reported)".to_string()
                } else {
                    err.to_string()
                }
            ),
        };
    }

    // Selection contract: parse `<serial>\t<state>` lines (multiple spaces
    // tolerated), require EXACTLY ONE line in state `fastboot`, and honor
    // `expect_serial`. Anything else aborts before any boot is attempted.
    let lines = match parse_devices_lines(&dev.stdout) {
        Some(l) => l,
        None => {
            return AttemptOutcome::SelectionAbort {
                reason: format!(
                    "unrecognized `fastboot devices` output line — refusing to guess: {:?}",
                    dev.stdout.trim()
                ),
            }
        }
    };
    let serial = match select_device(&lines, expect_serial) {
        Ok(s) => s,
        Err(r) => return AttemptOutcome::SelectionAbort { reason: r },
    };
    println!("attempt: selected device serial {serial} (state fastboot)");

    // Double verify (TOCTOU mitigation): re-run the pin verify NOW, after the
    // enumeration succeeded and immediately before boot. Residual reopen race
    // remains — see the doc comment on this function.
    if !verify() {
        return AttemptOutcome::VerifyAbort {
            reason: format!(
                "second (pre-boot) verify FAILED for {chosen_str} — image changed after \
                 enumeration; aborting before boot"
            ),
        };
    }

    // `fastboot -s <serial> boot <img>` — HW-30: the boot invocation is BOUND
    // to the serial that passed the selection contract (verbatim from the
    // parsed devices line, or the --expect-serial match). Before HW-30 the
    // selection was advisory only: args lacked `-s <serial>`, so the verified
    // target was not the target the boot actually addressed.
    // HW-27 bug 2 fix (unchanged): exit 0 ONLY when the boot process reported
    // ok. A failed boot maps to BootFailed (exit 4) with the reported status
    // in the message.
    let boot_verb = FastbootVerb::Boot.to_string();
    let serial_flag = "-s";
    let boot = match runner.run(&[
        serial_flag,
        serial.as_str(),
        boot_verb.as_str(),
        chosen_str.as_str(),
    ]) {
        Err(e) => {
            return AttemptOutcome::DevicesAbort {
                reason: format!("cannot run fastboot {boot_verb}: {e}"),
            }
        }
        Ok(o) => o,
    };
    if boot.ok {
        AttemptOutcome::Booted
    } else {
        let detail = {
            let err = boot.stderr.trim();
            let out = boot.stdout.trim();
            let mut m = String::new();
            if !out.is_empty() {
                m.push_str(out);
            }
            if !err.is_empty() {
                if !m.is_empty() {
                    m.push_str(" | ");
                }
                m.push_str(err);
            }
            if m.is_empty() {
                m.push_str("(no output reported)");
            }
            m
        };
        AttemptOutcome::BootFailed {
            message: format!("`fastboot {boot_verb}` reported FAILURE: {detail}"),
        }
    }
}
