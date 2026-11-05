//! P5 stage 2 — Redox entry (`cfg(target_os = "redox")`,
//! feature `dwc3-bringup`): sequence the pure stage-2
//! composition against real hardware windows and run the
//! embassy session.
//!
//! Sequence (each step fail-closed, one log line per verdict):
//!
//! 1. Stage-1 front half, mirrored from
//!    `redox_smmu::run_cb_bypass`: read-only pre-check of the
//!    SMMU globals against the pinned layout, then the
//!    read/write window and `establish_cb_bypass`. Any refusal
//!    exits 3 with no DMA — the same contract as stage 1.
//! 2. A FRESH gate observation (`redox_smmu::run`: the wiring
//!    sequence's two sequenced probe scans, taken after
//!    establishment).
//! 3. `stage2::begin` composes token + observation into a
//!    `DmaLegitimacy`. Only a legitimate composition proceeds;
//!    on refusal the DWC3 has not been mapped, let alone
//!    touched, and the exit is 3.
//! 4. `redox_glue::map_windows` under the composed mode
//!    (`Bypass`, no translated bus), core construction through
//!    `stage2::construct_core`, then `init_core` /
//!    `connect_core`. A core failure exits 4 — a new state
//!    (legitimacy held, the controller refused) worth its own
//!    code in the run log.
//! 5. The embassy session: the stage-3 ACM session when
//!    feature `cdc-acm-shell` is on, else a bare device (no
//!    functions) serving control transfers only — stage 2's
//!    exit state. The session never returns; reaching the end
//!    of this function means it ended, exit 0.
//!
//! Compile status: the pure layers are host-tested; this glue
//! is type-checked against the real `dwc3-gadget` /
//! `embassy-usb` APIs in a host fixture (see
//! STAGE23-EVIDENCE.md), but the Redox-target build itself
//! happens when root adopts the packet into the tree build —
//! the same status stage 1's glue had in its annex.

use std::cell::RefCell;

use dwc3_gadget::{Config, Driver};
use embassy_usb::Builder;
use mmio_replay::{Mmio, Width};

use crate::acm_shell;
use crate::redox_glue::map_windows;
use crate::redox_mmio::{DmaPtrMmio, RegsPtrMmio};
use crate::redox_smmu::{run as probe_run, SmmuReader, SmmuRw};
use crate::sid_gate::APPS_SMMU_BASE;
use crate::smmu_cbbypass::{
    establish_cb_bypass, layout_checked, verdict_line, CbRecipe, Stage1Verdict,
    VerdictFacts,
};
use crate::smmu_probe::dump_globals;
use crate::stage2;

/// Forwarding `Mmio` adapter over a raw pointer to one of
/// `HwWindows`' window impls. `Dwc3` owns its two windows,
/// but `HwWindows` lends them one `&mut` at a time (its
/// mappings must outlive the impls), mmio-replay has no
/// `&mut T` blanket impl, and the two borrows cannot be live
/// at once — so the entry takes a pointer per window first.
///
/// SAFETY discipline (the same one `redox_mmio`'s own
/// pointer-backed impls document): the pointers name two
/// DISTINCT fields of the `HwWindows` in `run_bringup`, which
/// outlives the core built from them (declared earlier,
/// dropped later), and every dereference happens through an
/// `Mmio` call on the daemon's single thread.
struct PtrMmio<T>(*mut T);

impl<T: Mmio> Mmio for PtrMmio<T> {
    fn read(&mut self, addr: u64, width: Width) -> u64 {
        // SAFETY: per the struct doc — distinct field, owner
        // alive, single-threaded.
        unsafe { (*self.0).read(addr, width) }
    }
    fn write(&mut self, addr: u64, width: Width, value: u64) {
        // SAFETY: per the struct doc.
        unsafe { (*self.0).write(addr, width, value) }
    }
}

/// Stage-2 entry, called from `redox_main` after
/// `common::init()`. See the module header for the sequence
/// and exit codes.
pub fn run_bringup() -> i32 {
    // 1 — stage-1 establishment (mirrors run_cb_bypass).
    let mut reader = match SmmuReader::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("usbgadgetd: STAGE2 ABORT: {e}; no writes, no DMA");
            return 3;
        }
    };
    let globals = dump_globals(&mut reader);
    eprintln!(
        "usbgadgetd: STAGE2 pre-check: IDR0={:#010x} IDR1={:#010x} GFSR={:#010x}",
        globals.idr0, globals.idr1, globals.gfsr
    );
    let layout = match layout_checked(globals.idr0, globals.idr1) {
        Some(l) => l,
        None => {
            // Mirror run_cb_bypass exactly: the self-contained
            // stage-1 verdict line (the raw values judged) for
            // this same refusal prints first, so one attended
            // cycle tells the next fix without another cycle.
            // Logging only - flow, registers and exit code are
            // unchanged.
            let facts = VerdictFacts {
                idr0: Some(globals.idr0),
                idr1: Some(globals.idr1),
                gfsr: Some(globals.gfsr),
                scr0: Some(globals.scr0),
                base: Some(APPS_SMMU_BASE),
                gfar: None,
                gfsynr0: None,
                gfsynr1: None,
                gfsynr2: None,
            };
            eprintln!(
                "usbgadgetd: {}",
                verdict_line(facts, &Stage1Verdict::Refused)
            );
            eprintln!(
                "usbgadgetd: STAGE2 REFUSED: IDR counts/window disagree with the pinned SDM845 facts; no writes"
            );
            return 3;
        }
    };
    drop(reader);

    let mut rw = match SmmuRw::new(APPS_SMMU_BASE, layout.window_len()) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("usbgadgetd: STAGE2 ABORT: {e}; no writes, no DMA");
            return 3;
        }
    };
    let token = match establish_cb_bypass(&mut rw, &CbRecipe::operator()) {
        Ok(t) => {
            eprintln!("usbgadgetd: {}", t.log_line());
            t
        }
        Err(e) => {
            eprintln!(
                "usbgadgetd: STAGE2 CB-BYPASS FAILED (fail-closed): {e:?}; no DMA attempted"
            );
            return 3;
        }
    };
    drop(rw);

    // 2 — fresh gate observation, after establishment.
    let probe = match probe_run() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("usbgadgetd: STAGE2 ABORT: fresh probe failed: {e}; DWC3 untouched");
            return 3;
        }
    };
    let fresh = probe.report.second().obs();

    // 3 — compose. No DWC3 mapping exists before this passes.
    let legit = match stage2::begin(Some(&token), fresh) {
        Ok(l) => {
            eprintln!("usbgadgetd: {}", l.log_line());
            l
        }
        Err(e) => {
            eprintln!("usbgadgetd: STAGE2 REFUSED: {e}; DWC3 untouched, no DMA");
            return 3;
        }
    };

    // 4 — windows, core, init, connect.
    let mut hw = match map_windows(legit.dma_mode(), None) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("usbgadgetd: STAGE2 ABORT: window mapping failed: {e}; no DMA");
            return 3;
        }
    };
    let config = Config::sdm845_usb1(hw.dma_plan().bus(), crate::delay_us);
    // Pointers first (see PtrMmio): the two window borrows
    // cannot be live simultaneously.
    let regs_ptr: *mut RegsPtrMmio = hw.regs();
    let dma_ptr: *mut DmaPtrMmio = hw.dma();
    let mut core = stage2::construct_core(
        &legit,
        PtrMmio(regs_ptr),
        PtrMmio(dma_ptr),
        config,
    );
    if let Err(e) = stage2::init_core(&legit, &mut core) {
        eprintln!("usbgadgetd: STAGE2 INIT FAILED: {e}");
        return 4;
    }
    if let Err(e) = stage2::connect_core(&legit, &mut core) {
        eprintln!("usbgadgetd: STAGE2 CONNECT FAILED: {e}");
        return 4;
    }
    eprintln!("usbgadgetd: STAGE2 ENUMERATION LIVE");

    // 5 — embassy session.
    let core = RefCell::new(core);
    let driver = Driver::new(&core);
    let mut usb_config = embassy_usb::Config::new(acm_shell::VID, acm_shell::PID);
    usb_config.manufacturer = Some(acm_shell::MANUFACTURER);
    usb_config.product = Some(acm_shell::PRODUCT);
    usb_config.serial_number = Some(acm_shell::SERIAL);
    usb_config.max_packet_size_0 = acm_shell::MAX_PACKET_SIZE_0;
    #[cfg(feature = "cdc-acm-shell")]
    {
        // The ACM function groups two interfaces under an IAD.
        usb_config.composite_with_iads = true;
        usb_config.device_class = 0xef;
        usb_config.device_sub_class = 0x02;
        usb_config.device_protocol = 0x01;
    }
    let mut cfg_buf = [0u8; 256];
    let mut bos_buf = [0u8; 64];
    let mut msos_buf = [0u8; 0];
    let mut ctl_buf = [0u8; 128];
    // Drop order: the ACM State must be declared BEFORE the
    // Builder - the class registers the state with the
    // builder, and values drop in reverse declaration order,
    // so the builder must drop first (E0597 otherwise; the
    // canonical embassy-usb ordering).
    #[cfg(feature = "cdc-acm-shell")]
    let mut acm_state = embassy_usb::class::cdc_acm::State::new();
    let mut builder = Builder::new(
        driver,
        usb_config,
        &mut cfg_buf,
        &mut bos_buf,
        &mut msos_buf,
        &mut ctl_buf,
    );
    #[cfg(feature = "cdc-acm-shell")]
    let acm_class = embassy_usb::class::cdc_acm::CdcAcmClass::new(
        &mut builder,
        &mut acm_state,
        acm_shell::ACM_MAX_PACKET_SIZE,
    );
    let mut usb = builder.build();
    #[cfg(feature = "cdc-acm-shell")]
    crate::redox_stage3::run_session(usb, acm_class);
    #[cfg(not(feature = "cdc-acm-shell"))]
    embassy_futures::block_on(usb.run());
    0
}
