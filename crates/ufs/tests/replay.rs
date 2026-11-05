//! Golden MMIO trace: bring-up + GPT reads recorded against the model, frozen in
//! `tests/fixtures/bringup_gpt.trace` (JSONL; `*.jsonl` is gitignored in this repo) and replayed
//! strictly. Re-bless after an intended sequence change with `UFS_BLESS=1 cargo test`.
//! This is also the reference a phone-recorded trace is diffed against (`specs/ufs.md` §7).

mod common;

use common::device::UfsDevice;
use common::{config, phone_device, rig, SharedDma, BASE};
use mmio_replay::{Compare, MismatchKind, Mmio, Op, Recorder, Replayer, Trace, Width};
use ufs_core::gpt::Gpt;
use ufs_core::regs::{self, qcom};
use ufs_core::{Config, DeviceInfo, Error, NoDelay, Ufshc};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/bringup_gpt.trace"
);

/// Replays register reads from a trace and runs the device model on each doorbell write,
/// standing in for the DMA side effects a register trace cannot carry.
struct Harness {
    rep: Replayer,
    dev: UfsDevice,
    dma: SharedDma,
    utrlba: u64,
}

impl Mmio for Harness {
    fn read(&mut self, addr: u64, width: Width) -> u64 {
        self.rep.read(addr, width)
    }
    fn write(&mut self, addr: u64, width: Width, value: u64) {
        self.rep.write(addr, width, value);
        match addr - BASE {
            regs::UTRLBA => self.utrlba = (self.utrlba & !0xffff_ffff) | value,
            regs::UTRLBAU => self.utrlba = (self.utrlba & 0xffff_ffff) | value << 32,
            regs::UTRLDBR => {
                for slot in (0..32).filter(|s| value & (1 << s) != 0) {
                    self.dev.process(&self.dma, self.utrlba + 32 * slot);
                }
            }
            _ => {}
        }
    }
}

type Session = Result<(DeviceInfo, Gpt, Gpt), Error>;

fn session<M: Mmio>(io: M, dma: SharedDma, cfg: Config) -> (Session, M) {
    let mut hc = Ufshc::new(io, BASE, dma, NoDelay, cfg).unwrap();
    let mut run = || -> Session {
        let info = hc.bring_up()?;
        Ok((info, hc.read_gpt(0, 4096)?, hc.read_gpt(1, 4096)?))
    };
    let r = run();
    (r, hc.into_parts().0)
}

fn record() -> (Trace, (DeviceInfo, Gpt, Gpt)) {
    let (mut m, dma) = rig(phone_device());
    let (r, rec) = session(Recorder::new(&mut m), dma, config());
    (rec.into_trace(), r.unwrap())
}

fn golden() -> Trace {
    let text = std::fs::read_to_string(FIXTURE).expect("fixture missing: run with UFS_BLESS=1");
    Trace::from_jsonl(&text).unwrap()
}

fn replay(mode: Compare, cfg: Config) -> (Session, mmio_replay::ReplayReport) {
    let dma = common::dma();
    let h = Harness {
        rep: Replayer::new(golden(), mode),
        dev: phone_device(),
        dma: dma.clone(),
        utrlba: 0,
    };
    let (r, mut h) = session(h, dma, cfg);
    (r, h.rep.finish())
}

#[test]
fn recorded_trace_matches_the_frozen_fixture() {
    let text = record().0.to_jsonl();
    if std::env::var_os("UFS_BLESS").is_some() {
        std::fs::create_dir_all(std::path::Path::new(FIXTURE).parent().unwrap()).unwrap();
        std::fs::write(FIXTURE, &text).unwrap();
    }
    assert_eq!(
        text,
        std::fs::read_to_string(FIXTURE).unwrap(),
        "MMIO sequence drifted; re-bless if intended"
    );
}

#[test]
fn strict_replay_of_the_fixture_reproduces_the_session() {
    let (_, expected) = record();
    let (got, report) = replay(Compare::Strict, config());
    assert!(report.ok(), "{report:?}");
    assert_eq!(report.warnings(), 0);
    assert_eq!(got.unwrap(), expected);
}

#[test]
fn strict_replay_catches_a_changed_register_value() {
    let mut cfg = config();
    cfg.qcom = Some(ufs_core::Qcom {
        unipro_core_clk_mhz: 100,
    });
    let (_, report) = replay(Compare::Strict, cfg);
    assert!(!report.ok());
    assert!(report
        .mismatches
        .iter()
        .any(|m| m.kind == MismatchKind::WriteValue));
    // Lenient mode tolerates the different value as a warning.
    let (got, report) = replay(Compare::Lenient, cfg);
    assert!(
        report.ok() && report.warnings() > 0 && got.is_ok(),
        "{report:?}"
    );
}

#[test]
fn golden_trace_only_writes_allowlisted_registers() {
    let allowed = [
        regs::IE,
        regs::HCE,
        regs::IS,
        regs::UCMDARG1,
        regs::UCMDARG2,
        regs::UCMDARG3,
        regs::UICCMD,
        regs::UTRLBA,
        regs::UTRLBAU,
        regs::UTRLRSR,
        regs::UTRLDBR,
        qcom::CFG1,
    ];
    let t = golden();
    let writes: Vec<_> = t.events.iter().filter(|e| e.op == Op::Write).collect();
    assert!(writes.iter().all(|e| allowed.contains(&(e.addr - BASE))));
    assert!(t.events.iter().all(|e| e.width == Width::W32));
    // One SET_FLAG; every doorbell rings slot 0 only.
    let bells = writes
        .iter()
        .filter(|e| e.addr == BASE + regs::UTRLDBR)
        .count();
    assert!(writes
        .iter()
        .filter(|e| e.addr == BASE + regs::UTRLDBR)
        .all(|e| e.value == 1));
    // NOP, SET_FLAG, READ_FLAG, device, geometry, 8 units, then 2 × (header + entries).
    assert_eq!(bells, 1 + 1 + 1 + 2 + 8 + 2 * 2);
}
