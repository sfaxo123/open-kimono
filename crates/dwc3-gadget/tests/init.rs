//! Strict register-trace tests for init/connect/disconnect (spec §4), with
//! literal values taken from the spec rather than the driver's constants.

use dwc3_gadget::{Config, Dwc3, Error};
use mmio_replay::{Event, MemModel, Mmio, Replayer, Trace, Width};

const CORE: u64 = 0x0a60_0000;
const QS: u64 = 0x0a6f_8800;
const DMA: u64 = 0x8000_0000;

fn cfg(poll_limit: u32) -> Config {
    Config { poll_limit, ..Config::sdm845_usb1(DMA, |_| {}) }
}

#[derive(Default)]
struct S(Trace);
impl S {
    fn r(&mut self, off: u64, v: u32) -> &mut Self {
        self.0.push(Event::read(CORE + off, Width::W32, v as u64));
        self
    }
    fn w(&mut self, off: u64, v: u32) -> &mut Self {
        self.0.push(Event::write(CORE + off, Width::W32, v as u64));
        self
    }
    fn qr(&mut self, off: u64, v: u32) -> &mut Self {
        self.0.push(Event::read(QS + off, Width::W32, v as u64));
        self
    }
    fn qw(&mut self, off: u64, v: u32) -> &mut Self {
        self.0.push(Event::write(QS + off, Width::W32, v as u64));
        self
    }
    /// PAR0, PAR1, PAR2, DEPCMD write, then one DEPCMD read returning `done`.
    fn cmd(&mut self, phys: u64, par: [u32; 3], depcmd: u32, done: u32) -> &mut Self {
        let b = 0xc800 + 0x10 * phys;
        self.w(b + 8, par[0]).w(b + 4, par[1]).w(b, par[2]).w(b + 0xc, depcmd).r(b + 0xc, done)
    }

    fn init(&mut self) -> &mut Self {
        self.r(0xc120, 0x5533_310a) // GSNPSID
            .r(0xc14c, 0x0021_0000) // GHWPARAMS3: 16 eps, 8 IN
            .r(0xc11c, 0x0000_0000)
            .w(0xc11c, 0x0400_0000) // GUCTL1 DEV_FORCE_20_CLK_FOR_30_CLK
            .r(0xc704, 0x8000_0000) // DCTL: firmware left RUN_STOP on
            .w(0xc704, 0x4000_0000) // RUN_STOP 0, CSFTRST 1
            .r(0xc704, 0x4000_0000)
            .r(0xc704, 0x0000_0000) // reset done
            .r(0xc110, 0x0000_1000) // GCTL PRTCAPDIR host
            .w(0xc110, 0x0000_2000) // -> device
            .w(0xc408, 0x8000_0000) // GEVNTSIZ masked
            .r(0xc11c, 0x0400_0000)
            .w(0xc11c, 0x0400_0000)
    }

    fn connect(&mut self) -> &mut Self {
        self.connect_body()
            .r(0xc70c, 0x0040_0000) // still halted
            .r(0xc70c, 0)
    }

    /// Everything in connect up to and including the RUN_STOP write.
    fn connect_body(&mut self) -> &mut Self {
        self.qr(0x08, 0).qw(0x08, 0x100) // PIPE_UTMI_CLK_DIS
            .qr(0x08, 0x100).qw(0x08, 0x109) // + PIPE_UTMI_CLK_SEL | PIPE3_PHYSTATUS_SW
            .qr(0x08, 0x109).qw(0x08, 0x009) // - PIPE_UTMI_CLK_DIS
            .qr(0x30, 0).qw(0x30, 1 << 24) // LANE0_PWR_PRESENT
            .qr(0x10, 0).qw(0x10, (1 << 20) | (1 << 28)) // VBUS_VALID | SW_SESSVLD_SEL
            .r(0xc200, 0x0000_1540) // GUSB2PHYCFG: SUSPENDUSB20|ENBLSLPM, TRDTIM 5
            .w(0xc200, 0x0000_2400) // both off, TRDTIM 9
            .r(0xc700, 0x0008_0284) // DCFG: addr 0x50, DEVSPD 4 (SS)
            .w(0xc700, 0x0008_0000) // HS, addr 0
            .w(0xc400, 0x8000_0000)
            .w(0xc404, 0)
            .w(0xc408, 0x1000)
            .w(0xc40c, 0)
            .w(0xc708, 0x57) // DEVTEN
            .cmd(0, [0, 0, 0], 0x409, 0x9) // DEPSTARTCFG(0)
            .cmd(0, [0x1000, 0x500, 0], 0x401, 0x1) // DEPCFG ep0 out ctrl mps 512
            .cmd(0, [1, 0, 0], 0x402, 0x2) // DEPXFERCFG
            .cmd(1, [0x1000, 0x0200_0500, 0], 0x401, 0x1) // DEPCFG ep0 in, phys 1
            .cmd(1, [1, 0, 0], 0x402, 0x2)
            .r(0xc720, 0).w(0xc720, 0b11) // DALEPENA
            .cmd(0, [0, 0x8000_1000, 0], 0x406, 0x0001_0006) // STARTXFER SETUP TRB, rsrc 1
            .r(0xc704, 0).w(0xc704, 0x8000_0000) // RUN_STOP
    }
}

fn dwc(t: Trace, limit: u32) -> Dwc3<Replayer, MemModel> {
    Dwc3::new(Replayer::strict(t), MemModel::new(), cfg(limit))
}

fn check(d: Dwc3<Replayer, MemModel>) -> MemModel {
    let (mut r, m) = d.into_parts();
    let rep = r.finish();
    assert!(rep.ok(), "{rep}");
    m
}

#[test]
fn init_connect_golden_trace() {
    let mut s = S::default();
    s.init().connect();
    let mut d = dwc(s.0, 8);
    assert_eq!(d.init(), Ok(()));
    assert_eq!(d.endpoint_counts(), (16, 8));
    assert_eq!(d.connect(), Ok(()));
    assert!(d.is_running());
    let mut m = check(d);
    // SETUP TRB at DMA+0x1000 -> EP0 buffer DMA+0x1400, 8 bytes, SETUP|LST|IOC|HWO.
    assert_eq!(m.read_u32(DMA + 0x1000), 0x8000_1400);
    assert_eq!(m.read_u32(DMA + 0x1004), 0);
    assert_eq!(m.read_u32(DMA + 0x1008), 8);
    assert_eq!(m.read_u32(DMA + 0x100c), (2 << 4) | 0b11 | (1 << 11));
}

#[test]
fn disconnect_drops_vbus_and_halts() {
    let mut s = S::default();
    s.init().connect();
    s.qr(0x30, 1 << 24).qw(0x30, 0)
        .qr(0x10, 0x1010_0000).qw(0x10, 0)
        .r(0xc704, 0x8000_0000).w(0xc704, 0)
        .r(0xc70c, 0x0040_0000)
        .w(0xc408, 0x8000_1000);
    let mut d = dwc(s.0, 8);
    d.init().unwrap();
    d.connect().unwrap();
    assert_eq!(d.disconnect(), Ok(()));
    assert!(!d.is_running());
    check(d);
}

#[test]
fn rejects_unknown_core_after_one_read() {
    let mut s = S::default();
    s.r(0xc120, 0x1234_0001);
    let mut d = dwc(s.0, 8);
    assert_eq!(d.init(), Err(Error::UnsupportedCore(0x1234_0001)));
    check(d);
}

#[test]
fn rejects_bad_hwparams() {
    for hw3 in [0x0000_1000u32, 0x0004_1000, 0x0024_8000, 0x0001_0000] {
        let mut s = S::default();
        s.r(0xc120, 0x3331_0000).r(0xc14c, hw3);
        let mut d = dwc(s.0, 8);
        assert_eq!(d.init(), Err(Error::BadHwParams(hw3)), "{hw3:#x}");
        check(d);
    }
}

#[test]
fn unaligned_dma_touches_nothing() {
    let mut d = Dwc3::new(Replayer::strict(Trace::new()), MemModel::new(), Config { dma_base: DMA + 0x80, ..cfg(8) });
    assert_eq!(d.init(), Err(Error::UnalignedDma(DMA + 0x80)));
    check(d);
}

#[test]
fn soft_reset_timeout_is_bounded() {
    let mut s = S::default();
    s.r(0xc120, 0x5533_310a).r(0xc14c, 0x0021_0000).r(0xc11c, 0).w(0xc11c, 0x0400_0000)
        .r(0xc704, 0).w(0xc704, 0x4000_0000);
    for _ in 0..4 {
        s.r(0xc704, 0x4000_0000);
    }
    let mut d = dwc(s.0, 4);
    assert_eq!(d.init(), Err(Error::ResetTimeout));
    check(d);
}

#[test]
fn connect_before_init_is_refused() {
    let mut d = dwc(Trace::new(), 8);
    assert_eq!(d.connect(), Err(Error::NotInitialized));
    assert_eq!(d.disconnect(), Err(Error::NotInitialized));
    check(d);
}

/// The connect prefix up to (and including) the DEPSTARTCFG write.
fn connect_until_startcfg(s: &mut S) {
    s.qr(0x08, 0).qw(0x08, 0x100).qr(0x08, 0x100).qw(0x08, 0x109).qr(0x08, 0x109).qw(0x08, 0x9)
        .qr(0x30, 0).qw(0x30, 1 << 24).qr(0x10, 0).qw(0x10, 0x1010_0000)
        .r(0xc200, 0).w(0xc200, 0x2400).r(0xc700, 0).w(0xc700, 0)
        .w(0xc400, 0x8000_0000).w(0xc404, 0).w(0xc408, 0x1000).w(0xc40c, 0).w(0xc708, 0x57)
        .w(0xc808, 0).w(0xc804, 0).w(0xc800, 0).w(0xc80c, 0x409);
}

#[test]
fn endpoint_command_timeout_is_bounded() {
    let mut s = S::default();
    s.init();
    connect_until_startcfg(&mut s);
    for _ in 0..3 {
        s.r(0xc80c, 0x409);
    }
    let mut d = dwc(s.0, 3);
    d.init().unwrap();
    assert_eq!(d.connect(), Err(Error::CommandTimeout { phys: 0, cmd: 9 }));
    assert!(!d.is_running());
    check(d);
}

#[test]
fn endpoint_command_status_error_is_reported() {
    let mut s = S::default();
    s.init();
    connect_until_startcfg(&mut s);
    s.r(0xc80c, 0x1009); // CMDSTATUS 1
    let mut d = dwc(s.0, 8);
    d.init().unwrap();
    assert_eq!(d.connect(), Err(Error::CommandFailed { phys: 0, cmd: 9, status: 1 }));
    check(d);
}

#[test]
fn run_stop_halt_timeout_is_reported() {
    let mut s = S::default();
    s.init().connect_body().r(0xc70c, 0x0040_0000).r(0xc70c, 0x0040_0000);
    let mut d = dwc(s.0, 2);
    d.init().unwrap();
    assert_eq!(d.connect(), Err(Error::HaltTimeout { running: true }));
    assert!(!d.is_running());
    check(d);
}
