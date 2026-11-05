//! P5 stage 2 (pure core): DWC3 peripheral bring-up, gated on
//! DMA legitimacy composed from the stage-1 token.
//!
//! Stage 1 (`smmu_cbbypass`) proves a stream mapping exists and
//! issues a [`VerifiedCbBypass`] token for one (SMR, context
//! bank) pair. A token alone does not authorise DMA: stage 2
//! additionally requires a FRESH sid-gate observation saying the
//! gate reads the SAME pair as `Translated`, and the token's
//! programmed CB_SCTLR to have stage-1 translation off (M = 0).
//! Only then does [`legitimize`] mint a [`DmaLegitimacy`] — the
//! only value the DWC3 init path in this crate accepts.
//!
//! Why the composed mode is [`DmaMode::Bypass`]: the stage-1
//! bank is programmed CBAR TYPE = S1_TRANS_S2_BYPASS with
//! CB_SCTLR.M = 0, so the DWC3's stream passes the SMMU
//! untranslated and the device sees physical addresses. That is
//! exactly the physmap plan's `Bypass` mode, and [`legitimize`]
//! refuses any token whose SCTLR says otherwise, so the claim
//! cannot silently rot if the stage-1 recipe changes.
//!
//! This module is pure: no MMIO of its own, no target gating.
//! The Redox entry that sequences it against real windows is
//! `redox_stage2` (feature `dwc3-bringup`, Redox target only).
//! The gate mechanism is proven by the compiler-driven fixture
//! test at the bottom of this file: code that tries to reach
//! [`init_core`] with a bare token, or to forge a
//! [`DmaLegitimacy`], does not compile.

use dwc3_gadget::{Config, Dwc3, Error as CoreError};
use mmio_replay::Mmio;

use crate::physmap_plan::DmaMode;
use crate::sid_gate::SidObservation;
use crate::smmu_cbbypass::VerifiedCbBypass;

/// Why stage 2 refused to proceed. Every variant is reached
/// before any DWC3 register is touched: [`begin`] and
/// [`legitimize`] take no window parameters at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage2Error {
    /// The fresh gate observation is not `Translated` for the
    /// DWC3's stream (carries the observation for the log).
    NotTranslated(SidObservation),
    /// Gate and token disagree on the (SMR, context bank) pair.
    PairMismatch {
        /// SMR index the token was issued for.
        token_smr: usize,
        /// Context bank the token was issued for.
        token_cb: usize,
        /// SMR index the fresh observation reports.
        gate_smr: usize,
        /// Context bank the fresh observation reports.
        gate_cb: usize,
    },
    /// The token's CB_SCTLR has stage-1 translation enabled
    /// (M = 1): the composed DMA mode would not be `Bypass`,
    /// so the composition this module vouches for is false.
    SctlrTranslationEnabled(u32),
    /// The DWC3 core itself refused (init or connect).
    Core(CoreError),
}

impl core::fmt::Display for Stage2Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Stage2Error::NotTranslated(obs) => {
                write!(f, "gate does not read Translated: {obs:?}")
            }
            Stage2Error::PairMismatch {
                token_smr,
                token_cb,
                gate_smr,
                gate_cb,
            } => write!(
                f,
                "gate/token pair mismatch: token SMR={token_smr} CB={token_cb}, \
                 gate SMR={gate_smr} CB={gate_cb}"
            ),
            Stage2Error::SctlrTranslationEnabled(v) => {
                write!(f, "token CB_SCTLR {v:#010x} has M=1 (translation on)")
            }
            Stage2Error::Core(e) => write!(f, "dwc3 core: {e:?}"),
        }
    }
}

impl std::error::Error for Stage2Error {}

impl From<CoreError> for Stage2Error {
    fn from(e: CoreError) -> Self {
        Stage2Error::Core(e)
    }
}

/// Proof that DMA through the stage-1 bypass bank is legitimate
/// *right now*: a stage-1 token and a fresh gate observation
/// agree on the (SMR, context bank) pair, and the token's
/// programmed CB_SCTLR leaves stage-1 translation off.
///
/// Fields are private and the only constructor is
/// [`legitimize`]; see the fixture test for the compile-time
/// proof that neither a bare token nor a struct literal can
/// stand in for one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaLegitimacy {
    smr: usize,
    context_bank: usize,
}

impl DmaLegitimacy {
    /// SMR index the composed proof covers.
    pub fn smr(&self) -> usize {
        self.smr
    }

    /// Context bank the composed proof covers.
    pub fn context_bank(&self) -> usize {
        self.context_bank
    }

    /// The physmap mode this proof vouches for. Always
    /// [`DmaMode::Bypass`]: [`legitimize`] only mints when the
    /// bank's programming makes the stream untranslated.
    pub fn dma_mode(&self) -> DmaMode {
        DmaMode::Bypass
    }

    /// One-line verdict for the daemon log (contains no
    /// addresses beyond the pair indices).
    pub fn log_line(&self) -> String {
        format!(
            "usbgadgetd: STAGE2 DMA LEGITIMATE SMR={} CB={} MODE=BYPASS",
            self.smr, self.context_bank
        )
    }
}

/// Compose a [`DmaLegitimacy`] from the stage-1 token and a
/// fresh gate observation. Refuses unless the observation is
/// `Translated` for exactly the token's pair and the token's
/// CB_SCTLR has M = 0.
pub fn legitimize(
    token: &VerifiedCbBypass,
    fresh: SidObservation,
) -> Result<DmaLegitimacy, Stage2Error> {
    let (gate_smr, gate_cb) = match fresh {
        SidObservation::Translated {
            smr,
            context_bank,
        } => (smr, context_bank as usize),
        other => return Err(Stage2Error::NotTranslated(other)),
    };
    if gate_smr != token.smr() || gate_cb != token.context_bank() {
        return Err(Stage2Error::PairMismatch {
            token_smr: token.smr(),
            token_cb: token.context_bank(),
            gate_smr,
            gate_cb,
        });
    }
    let sctlr = token.cb_sctlr_value();
    if sctlr & 1 != 0 {
        return Err(Stage2Error::SctlrTranslationEnabled(sctlr));
    }
    Ok(DmaLegitimacy {
        smr: token.smr(),
        context_bank: token.context_bank(),
    })
}

/// Why the daemon-level stage-2 entry refused before composing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BeginRefusal {
    /// No stage-1 token was supplied at all (establishment
    /// disabled, or it errored/refused upstream).
    NoToken,
    /// A token was supplied but composition refused.
    Legitimacy(Stage2Error),
}

impl core::fmt::Display for BeginRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BeginRefusal::NoToken => write!(f, "no stage-1 token"),
            BeginRefusal::Legitimacy(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for BeginRefusal {}

/// Daemon-level composition entry: the stage-2 path starts
/// here, with the token stage 1 produced (or `None`) and the
/// fresh gate observation taken after establishment.
pub fn begin(
    token: Option<&VerifiedCbBypass>,
    fresh: SidObservation,
) -> Result<DmaLegitimacy, BeginRefusal> {
    let token = token.ok_or(BeginRefusal::NoToken)?;
    legitimize(token, fresh).map_err(BeginRefusal::Legitimacy)
}

/// Construct the DWC3 core over caller-supplied windows. The
/// legitimacy proof is a required parameter: there is no path
/// from a bare [`VerifiedCbBypass`] to a constructed core.
pub fn construct_core<R: Mmio, D: Mmio>(
    _legit: &DmaLegitimacy,
    regs: R,
    dma: D,
    config: Config,
) -> Dwc3<R, D> {
    Dwc3::new(regs, dma, config)
}

/// Run core init (identify, soft reset, device mode) on a core
/// constructed by [`construct_core`].
pub fn init_core<R: Mmio, D: Mmio>(
    _legit: &DmaLegitimacy,
    core: &mut Dwc3<R, D>,
) -> Result<(), Stage2Error> {
    core.init().map_err(Stage2Error::from)
}

/// Connect (glue/PHY config, event ring, EP0, RUN/STOP). After
/// this the device answers enumeration on the bus; the USB
/// stack session itself is driven by the caller.
pub fn connect_core<R: Mmio, D: Mmio>(
    _legit: &DmaLegitimacy,
    core: &mut Dwc3<R, D>,
) -> Result<(), Stage2Error> {
    core.connect().map_err(Stage2Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smmu_cbbypass::{CbRecipe, Write32};
    use crate::smmu_probe::{global_addr, Read32, IDR0_OFF, IDR1_OFF, IDR2_OFF, SCR0_OFF};
    use mmio_replay::{MemModel, Width};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    // ---- token minting: mirror of the stage-1 packet's own
    // fake-bus establishment model (probe-2 globals, everything
    // else zero), so tests hold REAL tokens, not stand-ins.

    #[derive(Default)]
    struct FakeBus {
        regs: HashMap<u64, u32>,
    }

    impl FakeBus {
        fn realistic() -> Self {
            let mut bus = FakeBus::default();
            bus.regs.insert(global_addr(SCR0_OFF), 0x002a_0406);
            bus.regs.insert(global_addr(IDR0_OFF), 0x4c01_7e4c);
            bus.regs.insert(global_addr(IDR1_OFF), 0x5000_002d);
            bus.regs.insert(global_addr(IDR2_OFF), 0x0000_5555);
            bus
        }
    }

    impl Read32 for FakeBus {
        fn read32(&mut self, addr: u64) -> u32 {
            *self.regs.get(&addr).unwrap_or(&0)
        }
    }

    impl Write32 for FakeBus {
        fn write32(&mut self, addr: u64, value: u32) {
            self.regs.insert(addr, value);
        }
    }

    fn mint_token() -> VerifiedCbBypass {
        let mut bus = FakeBus::realistic();
        crate::smmu_cbbypass::establish_cb_bypass(&mut bus, &CbRecipe::operator())
            .expect("fake bus must establish")
    }

    fn translated_for(token: &VerifiedCbBypass) -> SidObservation {
        SidObservation::Translated {
            smr: token.smr(),
            context_bank: token.context_bank() as u8,
        }
    }

    #[test]
    fn legitimize_accepts_matching_pair() {
        let token = mint_token();
        let legit = legitimize(&token, translated_for(&token)).unwrap();
        assert_eq!(legit.smr(), token.smr());
        assert_eq!(legit.context_bank(), token.context_bank());
        assert_eq!(legit.dma_mode(), DmaMode::Bypass);
        assert!(legit.log_line().contains("STAGE2 DMA LEGITIMATE"));
    }

    #[test]
    fn legitimize_refuses_non_translated_observations() {
        let token = mint_token();
        for obs in [
            SidObservation::NotMatched,
            SidObservation::Bypass { smr: token.smr() },
            SidObservation::Fault { smr: token.smr() },
        ] {
            assert_eq!(
                legitimize(&token, obs),
                Err(Stage2Error::NotTranslated(obs)),
                "{obs:?} must refuse"
            );
        }
    }

    #[test]
    fn legitimize_refuses_pair_mismatch() {
        let token = mint_token();
        let wrong_smr = SidObservation::Translated {
            smr: token.smr() + 1,
            context_bank: token.context_bank() as u8,
        };
        assert!(matches!(
            legitimize(&token, wrong_smr),
            Err(Stage2Error::PairMismatch { .. })
        ));
        let wrong_cb = SidObservation::Translated {
            smr: token.smr(),
            context_bank: token.context_bank() as u8 + 1,
        };
        assert!(matches!(
            legitimize(&token, wrong_cb),
            Err(Stage2Error::PairMismatch { .. })
        ));
    }

    #[test]
    fn legitimize_refuses_translation_enabled_token() {
        // A recipe variant with CB_SCTLR.M = 1 would make the
        // bank translate at stage 1; the Bypass composition
        // must not vouch for it.
        let mut bus = FakeBus::realistic();
        let recipe = CbRecipe {
            cb_sctlr: 1,
            cbar_vmid: 0,
        };
        let token = crate::smmu_cbbypass::establish_cb_bypass(&mut bus, &recipe).unwrap();
        assert_eq!(token.cb_sctlr_value() & 1, 1);
        assert_eq!(
            legitimize(&token, translated_for(&token)),
            Err(Stage2Error::SctlrTranslationEnabled(1))
        );
    }

    #[test]
    fn begin_refuses_without_token() {
        let token = mint_token();
        assert_eq!(
            begin(None, translated_for(&token)),
            Err(BeginRefusal::NoToken)
        );
        // With a token but a dead gate, the refusal is the
        // legitimacy error, not NoToken.
        assert_eq!(
            begin(Some(&token), SidObservation::NotMatched),
            Err(BeginRefusal::Legitimacy(Stage2Error::NotTranslated(
                SidObservation::NotMatched
            )))
        );
    }

    // ---- init order / gating against traced windows.

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Op {
        Read(u64),
        Write(u64, u64),
    }

    #[derive(Clone)]
    struct TracingMmio {
        model: MemModel,
        trace: Rc<RefCell<Vec<Op>>>,
    }

    impl TracingMmio {
        fn new(trace: Rc<RefCell<Vec<Op>>>) -> Self {
            Self {
                model: MemModel::new(),
                trace,
            }
        }
        fn seed32(&mut self, addr: u64, value: u32) {
            self.model.write(addr, Width::W32, value as u64);
        }
    }

    impl Mmio for TracingMmio {
        fn read(&mut self, addr: u64, width: Width) -> u64 {
            self.trace.borrow_mut().push(Op::Read(addr));
            self.model.read(addr, width)
        }
        fn write(&mut self, addr: u64, width: Width, value: u64) {
            self.trace.borrow_mut().push(Op::Write(addr, value));
            self.model.write(addr, width, value);
        }
    }

    fn noop_delay(_: u32) {}

    fn test_config() -> Config {
        let mut cfg = Config::sdm845_usb1(0x1000_0000, noop_delay);
        cfg.poll_limit = 4;
        cfg
    }

    const CORE: u64 = 0x0a60_0000;
    const GSNPSID: u64 = CORE + 0xc120;
    const GHWPARAMS3: u64 = CORE + 0xc14c;
    const GUCTL1: u64 = CORE + 0xc11c;
    const DCTL: u64 = CORE + 0xc704;

    #[test]
    fn init_core_on_zeroed_windows_reads_id_first_and_stops() {
        let token = mint_token();
        let legit = legitimize(&token, translated_for(&token)).unwrap();
        let trace = Rc::new(RefCell::new(Vec::new()));
        let regs = TracingMmio::new(trace.clone());
        let dma = TracingMmio::new(Rc::new(RefCell::new(Vec::new())));
        let mut core = construct_core(&legit, regs, dma, test_config());
        assert_eq!(
            init_core(&legit, &mut core),
            Err(Stage2Error::Core(CoreError::UnsupportedCore(0)))
        );
        // Exactly one bus transaction happened: the GSNPSID
        // identification read. Nothing was written anywhere.
        assert_eq!(*trace.borrow(), vec![Op::Read(GSNPSID)]);
    }

    #[test]
    fn init_core_order_is_pinned_through_soft_reset() {
        let token = mint_token();
        let legit = legitimize(&token, translated_for(&token)).unwrap();
        let trace = Rc::new(RefCell::new(Vec::new()));
        let mut regs = TracingMmio::new(trace.clone());
        // A DWC3 core id (0x5533 = DWC_usb3) and sane endpoint
        // counts (total 16, in 8) so init proceeds past identify.
        regs.seed32(GSNPSID, 0x5533_300a);
        regs.seed32(GHWPARAMS3, (8 << 18) | (16 << 12));
        let dma = TracingMmio::new(Rc::new(RefCell::new(Vec::new())));
        let mut core = construct_core(&legit, regs, dma, test_config());
        // The model never clears CSFTRST, so init must end in
        // ResetTimeout — after issuing the soft reset.
        assert_eq!(
            init_core(&legit, &mut core),
            Err(Stage2Error::Core(CoreError::ResetTimeout))
        );
        let t = trace.borrow();
        assert_eq!(t[0], Op::Read(GSNPSID), "{t:?}");
        assert_eq!(t[1], Op::Read(GHWPARAMS3), "{t:?}");
        assert_eq!(t[2], Op::Read(GUCTL1), "{t:?}");
        assert!(matches!(t[3], Op::Write(a, _) if a == GUCTL1), "{t:?}");
        assert_eq!(t[4], Op::Read(DCTL), "{t:?}");
        // The DCTL write sets CSFTRST (bit 30) and nothing the
        // model returns afterwards clears it, so the remainder
        // of the trace is DCTL polls only.
        assert!(
            matches!(t[5], Op::Write(a, v) if a == DCTL && v & (1 << 30) != 0),
            "{t:?}"
        );
        assert!(
            t[6..].iter().all(|op| *op == Op::Read(DCTL)),
            "{t:?}"
        );
        assert!(t.len() > 6, "{t:?}");
    }

    // ---- the token gate, proven by the compiler.
    //
    // A doctest cannot prove this: cargo does not run doctests
    // for binary crates (verified empirically on this crate's
    // toolchain — a `compile_fail` doctest in a bin crate is
    // silently ignored). So this test builds a real fixture
    // crate that include!s THIS crate's pure modules verbatim
    // and checks `cargo check` verdicts on snippets.

    /// Inline a crate module's source with its `//!` inner
    /// doc comments neutralised (they are only legal at a
    /// file's head, not inside a braced `mod` block).
    fn inline_module(manifest_dir: &std::path::Path, name: &str) -> String {
        let src = std::fs::read_to_string(manifest_dir.join(format!("src/{name}.rs")))
            .expect("module source must exist");
        let mut out = String::with_capacity(src.len());
        for line in src.lines() {
            let t = line.trim_start();
            if let Some(rest) = t.strip_prefix("//!") {
                out.push_str("//");
                out.push_str(rest);
            } else if t.starts_with("/*!") {
                out.push_str("/*");
                out.push_str(&t[3..]);
            } else {
                out.push_str(line);
            }
            out.push('\n');
        }
        format!("#[allow(dead_code)] mod {name} {{\n{out}}}\n")
    }

    fn write_fixture(dir: &std::path::Path, main_rs: &str) {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            format!(
                "[package]\nname = \"gate-fixture\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n\
                 [dependencies]\n\
                 mmio-replay = {{ path = {mmio:?} }}\n\
                 dwc3-gadget = {{ path = {dwc3:?} }}\n\
                 embassy-usb = {{ version = \"0.6.0\", default-features = false }}\n\
                 embassy-futures = \"0.1.2\"\n",
                mmio = manifest_dir.join("../mmio-replay"),
                dwc3 = manifest_dir.join("../dwc3-gadget"),
            ),
        )
        .unwrap();
        let mut body = String::new();
        for m in [
            "physmap_plan",
            "windows",
            "sid_gate",
            "smmu_probe",
            "wiring",
            "smmu_cbbypass",
            "acm_shell",
            "stage2",
        ] {
            body.push_str(&inline_module(manifest_dir, m));
        }
        body.push_str(main_rs);
        body.push('\n');
        std::fs::write(dir.join("src/main.rs"), body).unwrap();
    }

    fn cargo_check(dir: &std::path::Path) -> (bool, String) {
        let out = std::process::Command::new("cargo")
            .args(["check", "--offline", "--quiet"])
            .current_dir(dir)
            .output()
            .expect("cargo must run");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    #[test]
    fn token_gate_is_compiler_enforced() {
        let base = std::env::temp_dir().join(format!(
            "usbgadgetd-gate-fixture-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);

        // Positive control: the legitimate composition compiles.
        let ok_dir = base.join("ok");
        write_fixture(
            &ok_dir,
            r#"
use smmu_cbbypass::VerifiedCbBypass;
use stage2::DmaLegitimacy;
fn legit_path(legit: &DmaLegitimacy) {
    let _mode = legit.dma_mode();
}
fn compose(token: &VerifiedCbBypass, obs: sid_gate::SidObservation) {
    if let Ok(legit) = stage2::legitimize(token, obs) {
        legit_path(&legit);
    }
}
fn main() {}
"#,
        );
        let (ok, err) = cargo_check(&ok_dir);
        assert!(ok, "positive fixture must compile:\n{err}");

        // Negative 1: a bare token where a DmaLegitimacy is
        // required must NOT compile.
        let bare_dir = base.join("bare-token");
        write_fixture(
            &bare_dir,
            r#"
use smmu_cbbypass::VerifiedCbBypass;
use stage2::DmaLegitimacy;
fn legit_path(legit: &DmaLegitimacy) {
    let _mode = legit.dma_mode();
}
fn misuse(token: &VerifiedCbBypass) {
    legit_path(token);
}
fn main() {}
"#,
        );
        let (ok, err) = cargo_check(&bare_dir);
        assert!(!ok, "bare-token fixture must NOT compile");
        assert!(
            err.contains("DmaLegitimacy"),
            "error must name the missing proof type:\n{err}"
        );

        // Negative 2: forging the proof by struct literal must
        // NOT compile (fields are private to stage2).
        let forge_dir = base.join("forged");
        write_fixture(
            &forge_dir,
            r#"
use stage2::DmaLegitimacy;
fn misuse() -> DmaLegitimacy {
    DmaLegitimacy { smr: 0, context_bank: 0 }
}
fn main() {}
"#,
        );
        let (ok, err) = cargo_check(&forge_dir);
        assert!(!ok, "forged-literal fixture must NOT compile");
        assert!(
            err.contains("DmaLegitimacy"),
            "error must name the proof type:\n{err}"
        );

        let _ = std::fs::remove_dir_all(&base);
    }
}
