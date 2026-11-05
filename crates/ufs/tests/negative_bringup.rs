//! Bring-up failure paths: each ends in a typed error before the list runs.

mod common;

use common::model::UfsModel;
use common::{config, phone_device, rig, SharedDma, BASE, DMA_PHYS};
use mmio_replay::{Replayer, Trace};
use ufs_core::regs;
use ufs_core::{Error, NoDelay, Stage, Ufshc};

fn up(m: &mut UfsModel, d: SharedDma) -> Ufshc<&mut UfsModel, SharedDma, NoDelay> {
    let mut hc = Ufshc::new(m, BASE, d, NoDelay, config()).unwrap();
    hc.bring_up().unwrap();
    hc
}

fn bring_up_err(f: impl FnOnce(&mut UfsModel)) -> (Error, UfsModel) {
    let (mut m, d) = rig(phone_device());
    f(&mut m);
    let e = Ufshc::new(&mut m, BASE, d, NoDelay, config())
        .unwrap()
        .bring_up()
        .unwrap_err();
    (e, m)
}

#[test]
fn link_startup_failure_reports_uic_result() {
    let (e, m) = bring_up_err(|m| m.hw.link_result = 0x01);
    assert_eq!(
        e,
        Error::Uic {
            opcode: regs::DME_LINKSTARTUP,
            result: 1
        }
    );
    assert!(!m.link_up && m.rsr == 0, "list never started");
}

#[test]
fn link_up_without_device_present() {
    assert_eq!(bring_up_err(|m| m.hw.no_device = true).0, Error::NoDevice);
}

#[test]
fn dead_phy_is_detected_before_link_startup() {
    let (e, m) = bring_up_err(|m| m.hw.phy_dead = true);
    assert_eq!(e, Error::PhyNotReady(0));
    assert_eq!(
        m.uic_log.len(),
        config().polls.phy as usize,
        "bounded HIBERN8 polling"
    );
    assert!(m.uic_log.iter().all(|u| u.0 == regs::DME_GET));
}

#[test]
fn hce_enable_and_disable_timeouts() {
    assert_eq!(
        bring_up_err(|m| m.hw.hce_stuck_off = true).0,
        Error::Timeout(Stage::HceEnable)
    );
    let (e, _) = bring_up_err(|m| {
        m.hce = true;
        m.hw.hce_stuck_on = true;
    });
    assert_eq!(e, Error::Timeout(Stage::HceDisable));
}

#[test]
fn uic_command_that_never_completes() {
    assert_eq!(
        bring_up_err(|m| m.hw.uic_silent = true).0,
        Error::Timeout(Stage::UicComplete)
    );
}

#[test]
fn list_not_ready_after_link_startup() {
    assert_eq!(
        bring_up_err(|m| m.hw.list_not_ready = true).0,
        Error::ListNotReady
    );
}

#[test]
fn nop_failing_every_attempt_returns_last_ocs() {
    let (e, m) = bring_up_err(|m| m.dev.faults.nop_ocs = vec![0x05; 10]);
    assert_eq!(e, Error::Ocs(0x05));
    assert_eq!(m.dev.seen.len(), config().nop_attempts as usize);
}

#[test]
fn device_init_that_never_clears() {
    let (e, _) = bring_up_err(|m| m.dev.faults.device_init_busy = u32::MAX);
    assert_eq!(e, Error::Timeout(Stage::DeviceInit));
}

#[test]
fn descriptor_with_wrong_idn_is_rejected() {
    assert_eq!(
        bring_up_err(|m| m.dev.faults.wrong_idn = Some(0x09)).0,
        Error::Descriptor(0x00)
    );
}

#[test]
fn dma_layout_and_capability_checks() {
    let r = || Replayer::strict(Trace::new());
    let small = SharedDma::new(DMA_PHYS, 0x1000);
    assert!(matches!(
        Ufshc::new(r(), BASE, small, NoDelay, config()),
        Err(Error::DmaLayout)
    ));
    let unaligned = SharedDma::new(DMA_PHYS + 0x80, 0x2000);
    assert!(matches!(
        Ufshc::new(r(), BASE, unaligned, NoDelay, config()),
        Err(Error::DmaLayout)
    ));

    let high = SharedDma::new(0x1_8000_0000, 0x1000 + 0x10000);
    let mut m = UfsModel::sdm845(high.clone(), phone_device());
    m.cap &= !regs::CAP_64AS;
    let e = Ufshc::new(&mut m, BASE, high, NoDelay, config())
        .unwrap()
        .bring_up();
    assert_eq!(e, Err(Error::Unsupported("64-bit addressing")));

    let (mut m, d) = rig(phone_device());
    m.cap |= regs::CAP_LSDBS;
    let e = Ufshc::new(&mut m, BASE, d, NoDelay, config())
        .unwrap()
        .bring_up();
    assert_eq!(e, Err(Error::Unsupported("single doorbell")));
}

#[test]
fn bring_up_on_a_32_lun_device_scans_all_32() {
    let mut dev = phone_device();
    dev.max_lu_code = 1;
    let (mut m, d) = rig(dev);
    let info = up(&mut m, d).enumerate().unwrap();
    assert_eq!(info.max_lu, 32);
    assert_eq!(info.units.len(), 3);
}
