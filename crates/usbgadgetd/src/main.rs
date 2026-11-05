//! `usbgadgetd` — V1 compile spike (see `docs/workbench/USBGADGETD-WIRING.md`).
//!
//! Scope of V1, honestly stated: the two `Mmio` window impls (host
//! `MemModel`-backed in `windows.rs`, Redox pointer-backed in
//! `redox_mmio.rs`) plus proof that the daemon binary builds against
//! the real `dwc3-gadget` core and fails CLEANLY when there is no
//! DWC3 hardware behind the windows — the exact behaviour the V2
//! QEMU negative-path staging test requires.
//!
//! Payload v2 (2026-10-30): `main` IS wired — on the Redox
//! target only. It calls `common::init()` exactly once, maps the
//! `apps_smmu` window read-only (`redox_smmu.rs`), and runs the
//! wiring sequence (`wiring.rs`): two sequenced probe scans
//! through the SID 0x740 establish-and-verify gate
//! (`sid_gate.rs`), whose verified output is the only legitimate
//! source of a `DmaMode`. THE FIRST WIRED RUN IS LOG-ONLY
//! (gate-lite criterion, Wonder progress review 20261030-0955):
//! the daemon logs both sequence numbers, both observations,
//! the matched SMR/S2CR raw values, and the gate's mode answer,
//! then exits 3 having attempted NO DMA, constructed NO
//! `HwWindows`, and NOT init'ed the DWC3 core — the readback
//! must be cross-checked against the pmOS record before any
//! later packet enables DMA. On QEMU the verified result is
//! `NotMatched` -> `SidNotMatched` (reads return 0), which is
//! the binary-path negative demo. The host target keeps the V1
//! model-windows path unchanged (`UnsupportedCore(0)`, exit 2).
//! Still not present: the `usbcon` scheme, the embassy-usb run
//! loop, and the `ramsh` bridge. Both paths fail cleanly with a
//! typed log line and a non-zero exit — designed failures, not
//! stubs pretending to be a daemon.

mod physmap_plan;
mod sid_gate;
mod smmu_cbbypass;
mod smmu_probe;
mod windows;
mod wiring;
mod acm_shell;
mod stage2;

#[cfg(all(target_os = "redox", feature = "dwc3-bringup"))]
mod redox_cs;
#[cfg(all(target_os = "redox", feature = "dwc3-bringup"))]
mod redox_stage2;
#[cfg(all(target_os = "redox", feature = "dwc3-bringup", feature = "cdc-acm-shell"))]
mod redox_stage3;

#[cfg(target_os = "redox")]
mod redox_glue;
#[cfg(target_os = "redox")]
mod redox_mmio;
#[cfg(target_os = "redox")]
mod redox_smmu;

use dwc3_gadget::{Config, Dwc3};
use windows::{DmaMmio, RegsMmio};

fn delay_us(us: u32) {
    // Busy-wait: the glue sequence needs ~100us steps and sleep
    // granularity is not trusted for that (wiring design section 2).
    let start = std::time::Instant::now();
    let want = std::time::Duration::from_micros(us as u64);
    while start.elapsed() < want {
        std::hint::spin_loop();
    }
}

/// Redox entry point (payload v2, log-only first wired run):
/// `common::init()` exactly once (it panics on a second call,
/// per the `common` docs), then the SMMU probe-and-verify
/// sequence over the read-only physmap window. Every outcome —
/// verified mode, verified refusal, protocol failure, mapping
/// failure — is logged on ONE line and exits 3; no DMA is
/// attempted in any of them.
#[cfg(target_os = "redox")]
fn redox_main() -> i32 {
    common::init();

    // P5 stage 1 (opt-in): attempt CB-bypass establishment
    // for the USB stream and exit with the verdict. The
    // writable window is opened only inside redox_smmu's
    // CB-bypass entry; the log-only path below keeps its
    // read-only window. Stage 1 never proceeds to DMA.
    // P5 stage 2/3 (opt-in): full bring-up - stage-1
    // establishment, DMA-legitimacy composition, DWC3
    // init/connect and the embassy session. Subsumes the
    // stage-1-only exit below when enabled.
    #[cfg(feature = "dwc3-bringup")]
    {
        return redox_stage2::run_bringup();
    }
    #[cfg(feature = "smmu-cb-bypass")]
    {
        return redox_smmu::run_cb_bypass();
    }
    match redox_smmu::run() {
        Ok(dev) => {
            let report = dev.report;
            let matched = match dev.matched {
                Some((i, smr, s2cr)) => {
                    format!("SMR[{i}]={smr:#010x} S2CR[{i}]={s2cr:#010x}")
                }
                None => "no matched SMR".to_string(),
            };
            eprintln!(
                "usbgadgetd: SMMU globals: {}",
                dev.globals.log_line(),
            );
            eprintln!(
                "usbgadgetd: SMMU log-only probe: seq {} -> {:?}; seq {} -> {:?}; {}; verified DMA mode: {:?}; DMA refused: log-only first wired run, no DMA attempted",
                report.first().seq(),
                report.first().obs(),
                report.second().seq(),
                report.second().obs(),
                matched,
                report.mode(),
            );
            3
        }
        Err(e) => {
            eprintln!(
                "usbgadgetd: SMMU log-only probe failed cleanly: {e}; no DMA attempted"
            );
            3
        }
    }
}

fn main() {
    #[cfg(target_os = "redox")]
    {
        std::process::exit(redox_main());
    }
    // Host-model DMA bus address: page-aligned placeholder used only
    // against the MemModel backing store; no hardware is addressed.
    let bus: u64 = 0x8000_0000;
    let mut core = Dwc3::new(
        RegsMmio::new(),
        DmaMmio::new(bus),
        Config::sdm845_usb1(bus, delay_us),
    );
    match core.init() {
        Ok(()) => {
            eprintln!("usbgadgetd: unexpected init success against model windows");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!(
                "usbgadgetd: no DWC3 hardware behind model windows; init failed cleanly: {e:?}"
            );
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod smoke {
    #[test]
    fn delay_is_at_least_requested() {
        let t = std::time::Instant::now();
        super::delay_us(200);
        assert!(t.elapsed() >= std::time::Duration::from_micros(200));
    }
}
