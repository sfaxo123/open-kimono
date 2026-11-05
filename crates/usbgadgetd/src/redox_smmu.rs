//! Payload v2 — Redox physmap-backed SMMU reader (`cfg(target_os
//! = "redox")` only), the hardware half of the log-only wiring.
//!
//! Maps the `apps_smmu` global register window READ-ONLY
//! (`Prot::RO`, `MemoryType::DeviceMemory`) through the part-1
//! window plan (`smmu_probe::smmu_window_plan`), and backs the
//! probe's `Read32` abstraction with volatile 32-bit reads at
//! the plan's `translate()` offsets — the same arithmetic the
//! host tests pin. It writes no register, ever: the mapping is
//! not writable, so a stray write path cannot exist.
//!
//! The caller (`main`) must have called `common::init()` exactly
//! once before `run()`: `PhysBorrowed::map` goes through the
//! memory-scheme root fd that `init` installs.
//!
//! Failure discipline: every fallible step returns a typed
//! `String` error for the caller to log; nothing here panics,
//! unwraps a runtime result, or touches DMA. After the gate
//! sequence, `run` makes ONE extra read pair — the matched
//! SMR/S2CR raw values, for the log line only. That read is
//! explicitly NOT a gate input: the gate consumed exactly the
//! two sequenced probe scans, and the log read's values are
//! cross-checked by a human against the pmOS record, per the
//! gate-lite criterion for the first wired run. That readback
//! goes through `wiring::matched_pair`, which refuses an
//! out-of-range SMR index with a typed error rather than
//! inventing values.

use common::{MemoryType, PhysBorrowed, Prot};

use crate::physmap_plan::WindowPlan;
use crate::smmu_probe::{
    dump_globals, global_addr, smmu_window_plan, GlobalDump, Read32, SmmuProbe, GFAR_OFF,
    GFSR_OFF, GFSYNR0_OFF, GFSYNR1_OFF, GFSYNR2_OFF, SCR0_OFF,
};
use crate::wiring::{matched_pair, probe_twice_and_verify, LogOnlyReport};

/// Volatile register source over the physmap'ed SMMU window.
pub struct SmmuReader {
    map: PhysBorrowed,
    plan: WindowPlan,
}

impl SmmuReader {
    pub fn new() -> Result<Self, String> {
        let plan = smmu_window_plan().map_err(|e| format!("smmu window plan refused: {e:?}"))?;
        let map = PhysBorrowed::map(
            plan.map_base() as usize,
            plan.map_len() as usize,
            Prot::RO,
            MemoryType::DeviceMemory,
        )
        .map_err(|e| format!("smmu window physmap failed: {e:?}"))?;
        Ok(SmmuReader { map, plan })
    }
}

/// Read/write register window over the EXTENDED SMMU region
/// (globals + GR1 + context-bank space), for the P5 stage-1
/// CB-bypass path only. Same translation discipline as
/// `SmmuReader` (plan-checked volatile u32 access), but mapped
/// `Prot::RW`. Out-of-window or unaligned writes are dropped:
/// the establishment state machine's verification readbacks
/// then disagree with what was "written", which surfaces as a
/// fail-closed ReadbackMismatch, never a wild store.
pub struct SmmuRw {
    map: PhysBorrowed,
    plan: WindowPlan,
}

impl SmmuRw {
    pub fn new(base: u64, len: u64) -> Result<Self, String> {
        let plan = crate::physmap_plan::plan_window(base, len)
            .map_err(|e| format!("smmu RW window plan refused: {e:?}"))?;
        let map = PhysBorrowed::map(
            plan.map_base() as usize,
            plan.map_len() as usize,
            Prot::RW,
            MemoryType::DeviceMemory,
        )
        .map_err(|e| format!("smmu RW window physmap failed: {e:?}"))?;
        Ok(SmmuRw { map, plan })
    }
}

impl Read32 for SmmuRw {
    fn read32(&mut self, addr: u64) -> u32 {
        if addr % 4 != 0 {
            return 0;
        }
        match self.plan.translate(addr, 4) {
            Some(off) => {
                // SAFETY: as in SmmuReader::read32 — translate()
                // checked the access lies inside the mapping,
                // which this adapter owns. Single-threaded.
                unsafe {
                    std::ptr::read_volatile(
                        (self.map.as_ptr() as *const u8).add(off as usize) as *const u32,
                    )
                }
            }
            None => 0,
        }
    }
}

impl Read32 for &mut SmmuRw {
    fn read32(&mut self, addr: u64) -> u32 {
        (**self).read32(addr)
    }
}

impl crate::smmu_cbbypass::Write32 for SmmuRw {
    fn write32(&mut self, addr: u64, value: u32) {
        if addr % 4 != 0 {
            return;
        }
        if let Some(off) = self.plan.translate(addr, 4) {
            // SAFETY: translate() checked the 4-byte access
            // lies fully inside the RW mapping owned by this
            // adapter. Single-threaded daemon access.
            unsafe {
                std::ptr::write_volatile(
                    (self.map.as_ptr() as *mut u8).add(off as usize) as *mut u32,
                    value,
                )
            }
        }
    }
}

impl crate::smmu_cbbypass::Write32 for &mut SmmuRw {
    fn write32(&mut self, addr: u64, value: u32) {
        (**self).write32(addr, value)
    }
}

impl Read32 for SmmuReader {
    fn read32(&mut self, addr: u64) -> u32 {
        // Alignment, enforced at the unsafe boundary: the
        // volatile read below is a u32 read, so an unaligned
        // address would be UB even inside the window.
        // `translate` checks extent only. An unaligned ask reads
        // as an unprogrammed register, never a wild access. (The
        // probe's own addresses are base + 4*n by construction;
        // this guard is defence in depth at the one unsafe site.)
        if addr % 4 != 0 {
            return 0;
        }
        match self.plan.translate(addr, 4) {
            Some(off) => {
                // SAFETY: translate() checked the 4-byte access
                // lies fully inside the window; the pointer is
                // the mapping base plus that offset, and the
                // mapping is owned by this reader and outlives
                // the read. Single-threaded daemon access.
                unsafe {
                    std::ptr::read_volatile(
                        (self.map.as_ptr() as *const u8).add(off as usize) as *const u32,
                    )
                }
            }
            // The probe only ever names in-window addresses; an
            // out-of-window ask reads as an unprogrammed register
            // (VALID clear), never a wild pointer.
            None => 0,
        }
    }
}

impl Read32 for &mut SmmuReader {
    fn read32(&mut self, addr: u64) -> u32 {
        (**self).read32(addr)
    }
}

/// One completed on-device log-only run: the wiring report plus
/// the matched pair's raw register values for the log line
/// (`None` when the verified observation is `NotMatched`).
pub struct DeviceProbe {
    pub report: LogOnlyReport,
    pub matched: Option<(usize, u32, u32)>,
    /// Probe iteration 2: raw SMMU global-register dump
    /// (log-only, never a gate input).
    pub globals: GlobalDump,
}

/// Map the SMMU window and run the log-only wiring sequence.
/// No DMA is planned or attempted here or by the caller of this
/// first wired run.
pub fn run() -> Result<DeviceProbe, String> {
    let mut reader = SmmuReader::new()?;
    let report = {
        let mut probe = SmmuProbe::new(&mut reader);
        probe_twice_and_verify(&mut probe).map_err(|e| format!("{e:?}"))?
        // probe dropped here: the reader borrow is released.
    };
    // Logging-only readback of the matched pair (never a gate
    // input). An out-of-range index is a typed error, NOT a
    // fallback read at some other address: fabricating raw
    // values would corrupt the gate-lite cross-check evidence.
    // Probe iteration 2: dump the global registers AFTER the
    // gate sequence, so the dump can never perturb the gate's
    // two sequenced scans. Log-only, like the matched pair.
    let globals = dump_globals(&mut reader);
    let matched = match matched_pair(&mut reader, report.second().obs()) {
        None => None,
        Some(Ok(pair)) => Some(pair),
        Some(Err(e)) => return Err(format!("matched-pair readback refused: {e:?}")),
    };
    Ok(DeviceProbe { report, matched, globals })
}

/// P5 stage 1 entry (feature `smmu-cb-bypass`): cross-check
/// the register layout over the read-only window first, then
/// open the extended window read/write and run the CB-bypass
/// establishment state machine. Returns 3 on every outcome —
/// verified, refused, or failed — because stage 1's job ends
/// at the verdict; it never proceeds to DMA.
pub fn run_cb_bypass() -> i32 {
    use crate::sid_gate::APPS_SMMU_BASE;
    use crate::smmu_cbbypass::{
        establish_cb_bypass, layout_checked, verdict_line, CbRecipe, Stage1Verdict,
        VerdictFacts,
    };

    let mut reader = match SmmuReader::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "usbgadgetd: {}",
                verdict_line(VerdictFacts::default(), &Stage1Verdict::Aborted(&e))
            );
            return 3;
        }
    };
    let globals = dump_globals(&mut reader);
    eprintln!(
        "usbgadgetd: CB-BYPASS pre-check: IDR0={:#010x} IDR1={:#010x} GFSR={:#010x} SCR0={:#010x} BASE={:#010x}",
        globals.idr0, globals.idr1, globals.gfsr, globals.scr0, APPS_SMMU_BASE
    );
    let facts = VerdictFacts {
        idr0: Some(globals.idr0),
        idr1: Some(globals.idr1),
        gfsr: Some(globals.gfsr),
        scr0: Some(globals.scr0),
        base: Some(APPS_SMMU_BASE),
        // The fault block is read at failure time, in the
        // establishment error path below - never from this
        // pre-check snapshot.
        gfar: None,
        gfsynr0: None,
        gfsynr1: None,
        gfsynr2: None,
    };
    let layout = match layout_checked(globals.idr0, globals.idr1) {
        Some(l) => l,
        None => {
            eprintln!("usbgadgetd: {}", verdict_line(facts, &Stage1Verdict::Refused));
            return 3;
        }
    };
    drop(reader);

    let mut rw = match SmmuRw::new(APPS_SMMU_BASE, layout.window_len()) {
        Ok(w) => w,
        Err(e) => {
            eprintln!(
                "usbgadgetd: {}",
                verdict_line(facts, &Stage1Verdict::Aborted(&e))
            );
            return 3;
        }
    };
    match establish_cb_bypass(&mut rw, &CbRecipe::operator()) {
        Ok(token) => {
            eprintln!("usbgadgetd: {}", token.log_line());
            eprintln!("usbgadgetd: stage 1 stops at the verdict; no DMA attempted");
        }
        Err(e) => {
            // Verdict-time globals: re-read SCR0 and the
            // fault block through the RW window while it is
            // still open, so the FAILED line reports the
            // state the machine ended in - including WHICH
            // stream faulted, if one did - not the pre-check
            // snapshot. Reads only; the machine has already
            // restored or refused by this point.
            let mut fail_facts = facts;
            fail_facts.scr0 = Some(rw.read32(global_addr(SCR0_OFF)));
            let gfsr_now = rw.read32(global_addr(GFSR_OFF));
            fail_facts.gfsr = Some(gfsr_now);
            if gfsr_now != 0 {
                fail_facts.gfar = Some(rw.read32(global_addr(GFAR_OFF)));
                fail_facts.gfsynr0 = Some(rw.read32(global_addr(GFSYNR0_OFF)));
                fail_facts.gfsynr1 = Some(rw.read32(global_addr(GFSYNR1_OFF)));
                fail_facts.gfsynr2 = Some(rw.read32(global_addr(GFSYNR2_OFF)));
            }
            eprintln!(
                "usbgadgetd: {}",
                verdict_line(fail_facts, &Stage1Verdict::Failed(&e))
            );
        }
    }
    3
}
