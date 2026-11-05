//! Host tests for the geni debug-UART TX driver (mock MMIO only).

use super::console::BANNER;
use super::mock::{AutoSe, FailAtByte, MockSe};
use super::*;
use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

/// Clock divider the oracle would compute for 115200 baud from a 48 MHz SER
/// clock: `get_clk_div_rate` (c:134–149) picks ser_clk=48e6 (multiple of
/// 115200*32), div = 48_000_000 / (115200 * 32) = 13.
const CLK_DIV_115200: u32 = 13;

fn w(off: RegOffset, val: u32) -> super::mock::Write {
    // HW-21 seam plumbing only: transcript entries keep their raw numeric
    // offset+value; the driver now speaks validated `RegOffset` consts.
    super::mock::Write(off.get(), val)
}

/// Exact init write sequence, each entry annotated with the oracle line it mirrors.
fn expected_init_writes() -> Vec<super::mock::Write> {
    vec![
        // abort_rx (c:360–364):
        w(SE_GENI_S_CMD_CTRL_REG, S_GENI_CMD_ABORT_BIT), // c:360 writel(S_GENI_CMD_ABORT,...)
        w(SE_GENI_S_IRQ_CLEAR, S_CMD_DONE | S_CMD_ABORT), // c:363 writel(S_CMD_DONE_EN|S_CMD_ABORT_EN,...)
        w(GENI_FORCE_DEFAULT_REG, FORCE_DEFAULT),         // c:364 writel(FORCE_DEFAULT,...)
        // geni_serial_init packing (c:502–505):
        w(SE_GENI_TX_PACKING_CFG0, 0xf), // c:502 UART_PACKING_CFG0 (c:91)
        w(SE_GENI_TX_PACKING_CFG1, 0),   // c:503 UART_PACKING_CFG1 (c:92)
        w(SE_GENI_RX_PACKING_CFG0, 0xf), // c:504
        w(SE_GENI_RX_PACKING_CFG1, 0),   // c:505
        // geni_serial_init line config (c:507–513):
        w(SE_UART_TX_TRANS_CFG, 1 << 1), // c:507 tx_trans_cfg = UART_CTS_MASK (c:68,492)
        w(SE_UART_TX_PARITY_CFG, 0),     // c:508
        w(SE_UART_RX_TRANS_CFG, 0),      // c:509
        w(SE_UART_RX_PARITY_CFG, 0),     // c:510
        w(SE_UART_TX_WORD_LEN, 8),       // c:511 bits_per_char = BITS_PER_BYTE (c:493)
        w(SE_UART_RX_WORD_LEN, 8),       // c:512
        w(SE_UART_TX_STOP_BIT_LEN, 0),   // c:513
        // geni_serial_baud (c:195–196): SER_CLK_EN | div<<4; div=13 -> 0xd0|1.
        w(
            GENI_SER_M_CLK_CFG,
            SER_CLK_EN | (CLK_DIV_115200 << CLK_DIV_SHFT),
        ),
        w(
            GENI_SER_S_CLK_CFG,
            SER_CLK_EN | (CLK_DIV_115200 << CLK_DIV_SHFT),
        ),
        // _debug_uart_init rx watermark (c:623):
        w(SE_GENI_RX_WATERMARK_REG, DEF_RX_WM), // c:58 DEF_RX_WM=2
        // qcom_geni_serial_start_tx (c:336–337): TX wm then M_IRQ_EN |= wm|done.
        w(SE_GENI_TX_WATERMARK_REG, DEF_TX_WM), // c:336 DEF_TX_WM=2 (c:57)
        w(SE_GENI_M_IRQ_EN, M_TX_FIFO_WATERMARK | M_CMD_DONE), // c:337 (irq_en read 0 in mock)
    ]
}

/// Exact per-byte putc write sequence up to (and including) the FIFO push.
fn expected_putc_writes(b: u8) -> Vec<super::mock::Write> {
    vec![
        w(SE_GENI_TX_WATERMARK_REG, DEF_TX_WM),      // c:387 / c:632
        w(SE_UART_TX_TRANS_LEN, 1),                  // c:274 (setup_tx(base, 1) from c:388/633)
        w(SE_GENI_M_CMD0, M_CMD_UART_START_TX),      // c:275–276 UART_START_TX<<27 (c:67,65)
        w(SE_GENI_TX_FIFOn, b as u32), // c:393 / c:637 writel(ch, base + SE_GENI_TX_FIFOn)
        w(SE_GENI_M_IRQ_CLEAR, M_TX_FIFO_WATERMARK), // c:394 / c:638
        w(SE_GENI_M_IRQ_CLEAR, M_CMD_DONE), // c:293 (poll_tx_done irq_clear = M_CMD_DONE_EN)
    ]
}

#[test]
fn tx_writes_expected_registers() {
    let mut uart = GeniUart::new(AutoSe::new());
    uart.init(CLK_DIV_115200).expect("init");
    uart.write_byte(b'A').expect("tx 'A'");

    let got = uart.regs().inner_writes();
    let mut expected = expected_init_writes();
    expected.extend(expected_putc_writes(b'A'));
    assert_eq!(got, expected.as_slice(), "exact register write sequence (offset+value) must mirror the oracle lines cited in expected_*_writes()");

    // Spot-check semantics beyond the transcript:
    assert_eq!(uart.regs().get(SE_UART_TX_WORD_LEN.get()), 8); // 8-bit words (c:511)
    assert_eq!(
        uart.regs().get(GENI_SER_M_CLK_CFG.get()),
        SER_CLK_EN | (13 << 4)
    ); // clock on + div (c:192–195)
}

#[test]
fn tx_respects_fifo_full() {
    // Mock keeps M_TX_FIFO_WATERMARK (the "FIFO has room" bit, c:390–391) CLEAR:
    // the FIFO is full / not accepting. With a tiny poll budget the driver must
    // time out WITHOUT ever writing to the TX FIFO data port.
    let mut mock = MockSe::new();
    mock.seed(SE_GENI_M_IRQ_STATUS, 0); // watermark never set => FIFO full
    let mut uart = GeniUart::new(mock).with_poll_budget(4);
    let err = uart.write_byte(b'X').unwrap_err();
    assert_eq!(err, TxError::FifoNotReady);

    let regs = uart.regs_mut();
    assert!(
        regs.writes_to(SE_GENI_TX_FIFOn.get()).is_empty(),
        "driver must NOT write data while the FIFO-full condition holds"
    );
    // It did program the transfer (setup_tx) and polled, but stopped before data.
    assert_eq!(regs.writes_to(SE_UART_TX_TRANS_LEN.get()), vec![1]);
    assert_eq!(
        regs.writes_to(SE_GENI_M_CMD0.get()),
        vec![M_CMD_UART_START_TX]
    );

    // Second half: once the bit clears to set, the same driver writes through.
    let mut mock = MockSe::new();
    mock.seed(SE_GENI_M_IRQ_STATUS, M_TX_FIFO_WATERMARK | M_CMD_DONE);
    let mut uart = GeniUart::new(mock).with_poll_budget(4);
    uart.write_byte(b'Y').expect("tx once FIFO has room");
    assert_eq!(
        uart.regs_mut().writes_to(SE_GENI_TX_FIFOn.get()),
        vec![b'Y' as u32]
    );
}

#[test]
fn tx_string_matches_banner_prefix() {
    let mut uart = GeniUart::new(AutoSe::new());
    uart.init(CLK_DIV_115200).expect("init");
    uart.write_bytes(b"REDOX").expect("tx banner");

    let bytes: Vec<u8> = uart
        .regs()
        .inner_writes()
        .iter()
        .filter(|wr| wr.0 == SE_GENI_TX_FIFOn.get())
        .map(|wr| wr.1 as u8)
        .collect();
    assert_eq!(
        bytes,
        b"REDOX".to_vec(),
        "bytes must hit the FIFO port in order"
    );
}

#[test]
fn dtb_carrier_node_facts() {
    // Pinned against workbench/HW-CONSOLE.md §1–2 (READ-ONLY cross-check):
    // console = geni debug UART at 0x00a84000, node serial@a84000 under
    // geniqup@ac0000 under soc@0, compatible "qcom,geni-debug-uart",
    // stdout-path serial0:115200n8.
    assert_eq!(DEBUG_UART_BASE, 0x00a8_4000);
    assert_eq!(DEBUG_UART_COMPATIBLE, "qcom,geni-debug-uart");
    assert!(DEBUG_UART_DT_PATH.ends_with("/serial@a84000"));
    assert!(DEBUG_UART_DT_PATH.contains("geniqup@ac0000"));
    assert_eq!(CONSOLE_BAUD, 115200);

    // The HW-CONSOLE.md text itself must still state these facts (guard against
    // the doc drifting out from under the consts).
    const DOC: &str = include_str!("../../../workbench/HW-CONSOLE.md");
    assert!(
        DOC.contains("0x00a84000"),
        "HW-CONSOLE.md must pin base 0x00a84000"
    );
    assert!(
        DOC.contains("\"qcom,geni-debug-uart\"") || DOC.contains("qcom,geni-debug-uart"),
        "HW-CONSOLE.md must pin the compatible string"
    );
    assert!(
        DOC.contains("serial@a84000"),
        "HW-CONSOLE.md must pin the DT node"
    );
}

#[test]
fn init_aborts_when_sequencer_stuck() {
    // M_GENI_CMD_ACTIVE stuck (h:108) -> init must bail with SequencerBusy
    // before touching config, mirroring the oracle's idle check (c:327).
    let mut mock = MockSe::new();
    mock.seed(SE_GENI_STATUS, M_GENI_CMD_ACTIVE);
    let mut uart = GeniUart::new(mock).with_poll_budget(3);
    assert_eq!(uart.init(CLK_DIV_115200), Err(TxError::SequencerBusy));
    assert!(
        uart.regs().writes.is_empty(),
        "no config writes while sequencer is busy"
    );
}

// ---------------------------------------------------------------------------
// HW-19: init/baud + RX path tests
// ---------------------------------------------------------------------------

/// HW-19 full-init transcript: init + start_rx, every write asserted by
/// offset+value, each annotated with the oracle line it mirrors.
#[test]
fn init_writes_expected_config_registers() {
    let mut uart = GeniUart::new(AutoSe::new());
    uart.init(CLK_DIV_115200).expect("init");
    uart.start_rx();

    let mut expected = expected_init_writes();
    // msm_geni_serial_setup_rx / start_rx (c:376–380):
    expected.push(w(SE_GENI_S_CMD0, UART_START_READ_OPCODE << 27)); // c:408/c:376 geni_se_setup_s_cmd(UART_START_READ, 0)
    expected.push(w(SE_GENI_RX_WATERMARK_REG, DEF_RX_WM)); // c:378 DEF_RX_WM=2 (c:58)
    expected.push(w(SE_GENI_S_IRQ_EN, S_RX_FIFO_WATERMARK | S_RX_FIFO_LAST)); // c:379 setbits S_RX_FIFO_WATERMARK_EN|S_RX_FIFO_LAST_EN (h:209–210); S_IRQ_EN read 0 in mock
    expected.push(w(
        SE_GENI_M_IRQ_EN,
        M_TX_FIFO_WATERMARK | M_CMD_DONE | M_RX_FIFO_WATERMARK | M_RX_FIFO_LAST,
    )); // c:380 setbits M_RX_FIFO_WATERMARK_EN|M_RX_FIFO_LAST_EN (h:179–180) on top of the c:337 value

    assert_eq!(
        uart.regs().inner_writes(),
        expected.as_slice(),
        "full init+start_rx write transcript must match offset+value, oracle lines annotated inline"
    );

    // Semantics spot-checks beyond the transcript:
    assert_eq!(
        uart.regs().get(SE_UART_RX_WORD_LEN.get()),
        8, // 8-bit RX words (c:512)
    );
    assert_eq!(
        uart.regs().get(SE_GENI_RX_PACKING_CFG0.get()),
        0xf, // UART_PACKING_CFG0 (c:91, written c:504)
    );
    assert_eq!(
        uart.regs().get(GENI_SER_S_CLK_CFG.get()),
        SER_CLK_EN | (13 << 4), // S-side SER clock on + div 13 (c:196)
    );
}

/// HW-19 baud math: the driver's own `ser_clock_for_baud` must reproduce the
/// oracle's `get_clk_div_rate` (c:134–149) for 115200, and `set_baud` must
/// write the resulting divider into both SER clk-cfg registers.
#[test]
fn baud_divider_math_115200() {
    // The oracle's parent-clock const: root_freq[] DFS table (c:103–104),
    // mirroring GCC ftbl_gcc_qupv3_wrap0_s0_clk_src (clock-sdm845.c:30–39).
    assert_eq!(ROOT_FREQ_HZ[5], 48_000_000, "48 MHz DFS entry (c:104)");
    assert_eq!(UART_OVERSAMPLING, 32, "UART_OVERSAMPLING (c:22)");

    // Formula: desired = baud * oversampling = 115200 * 32 = 3_686_400 (c:139).
    // The scan (c:115–117) skips 7_372_800 (desired*2 = 7_372_800, but the
    // table entry 14_745_600 = desired*4 is a multiple while 7_372_800 is NOT:
    // 7_372_800 = desired * 2.0 exactly? No — 3_686_400 * 2 = 7_372_800 IS a
    // multiple, so the oracle's FIRST hit is 7_372_800, not 48 MHz).
    // clk_div = 7_372_800 / 3_686_400 = 2 (c:147).
    let (ser_clk, clk_div) =
        ser_clock_for_baud(115200, UART_OVERSAMPLING).expect("115200 supported");
    assert_eq!(
        3_686_400u32 * 2,
        7_372_800,
        "desired*2 == first table entry (c:103)"
    );
    assert_eq!(
        ser_clk, 7_372_800,
        "ser_clk = FIRST root_freq multiple (c:115–117)"
    );
    assert_eq!(clk_div, 2, "clk_div = ser_clk / (baud*32) (c:147)");
    assert_eq!(115200u64 * 32, 3_686_400, "desired_clk (c:139)");
    assert_eq!(7_372_800u64 / 3_686_400, 2, "worked example");
    // Exactness across the whole DFS table (c:103–104): only exact multiples
    // qualify; 48 MHz is NOT one (48e6 % 3.6864e6 = 76_800), which is why the
    // oracle's scan lands on 7.3728 MHz for 115200.
    assert_eq!(
        48_000_000u32 % 3_686_400,
        76_800,
        "48 MHz is NOT an exact multiple (c:116)"
    );

    // The debug-UART early path pins the same math as a macro:
    // CLK_DIV = DEBUG_UART_CLOCK / (BAUDRATE * OVERSAMPLING) (c:612–613).
    // NOTE integer division there truncates like any C `/` — at 48 MHz that
    // is 13 (with the c:614 remainder guard failing for 115200; see below).
    assert_eq!(
        48_000_000u32 / (115200 * UART_OVERSAMPLING),
        13,
        "integer-truncated macro value"
    );

    // set_baud writes SER_CLK_EN | div<<4 to both clk-cfg regs (c:192–196).
    let mut uart = GeniUart::new(MockSe::new());
    uart.set_baud(115200).expect("setbrg");
    let regs = uart.regs_mut();
    let cfg = SER_CLK_EN | (2 << CLK_DIV_SHFT); // div=2 from the 7.3728 MHz hit
    assert_eq!(regs.writes_to(GENI_SER_M_CLK_CFG.get()), vec![cfg]); // c:195
    assert_eq!(regs.writes_to(GENI_SER_S_CLK_CFG.get()), vec![cfg]); // c:196

    // The early debug-UART variant pins the divider from a FIXED parent clock
    // instead of the DFS scan: CLK_DIV = DEBUG_UART_CLOCK / (BAUD * OVERSAMPLING)
    // with an #error guard when it does not divide exactly (c:612–616). A 48 MHz
    // debug clock FAILS that guard for 115200 (remainder 76_800); the largest
    // table parent that passes is 14_745_600 -> div 4:
    assert_eq!(
        14_745_600u32 % (115200 * UART_OVERSAMPLING),
        0,
        "guard c:614 passes"
    );
    assert_eq!(
        14_745_600u32 / (115200 * UART_OVERSAMPLING),
        4,
        "CLK_DIV macro c:612–613"
    );

    // A rate with no DFS entry must error like the oracle's -EINVAL (c:209–212).
    assert_eq!(
        uart.set_baud(115_201),
        Err(BaudError::UnsupportedRate),
        "no root_freq multiple of 115201*32 -> oracle returns -EINVAL (c:211)"
    );
}

/// HW-19 RX happy path: mock seeds RX fifo words + sets the status bits the
/// oracle's getc polls (M_SEC_IRQ c:410, RX_FIFO_WC c:417) -> the driver
/// returns the bytes in order and clears both sequencers' IRQs (c:415–416).
#[test]
fn rx_reads_bytes_in_order() {
    let mut auto = AutoSe::new();
    auto.seed_rx(0x141); // word low byte 'A' (getc returns rx_fifo & 0xff, c:424)
    auto.seed_rx(0x142); // 'B'
    auto.seed_rx(0x143); // 'C'

    let mut uart = GeniUart::new(auto);
    uart.init(CLK_DIV_115200).expect("init");
    uart.start_rx();

    assert_eq!(uart.read_byte(), Ok(b'A'));
    assert_eq!(uart.read_byte(), Ok(b'B'));
    assert_eq!(uart.read_byte(), Ok(b'C'));
    // FIFO drained: fourth read finds wc == 0.
    assert_eq!(uart.read_byte(), Err(RxError::WouldReadNothing));

    // After arming UART_START_READ (c:408), the driver must have echoed both
    // IRQ status snapshots into the CLEAR registers (c:413–416).
    let got = uart.regs().inner_writes();
    let s_cmd0: Vec<u32> = got
        .iter()
        .filter(|wr| wr.0 == SE_GENI_S_CMD0.get())
        .map(|wr| wr.1)
        .collect();
    assert_eq!(
        s_cmd0.as_slice(),
        &[UART_START_READ_OPCODE << 27; 5][..],
        "one UART_START_READ re-arm per getc (c:408): start_rx + 3 bytes + 1 empty probe"
    );
    // M_IRQ_CLEAR writes happened (watermark/done from TX + RX snapshot c:415).
    assert!(
        got.iter()
            .any(|wr| wr.0 == SE_GENI_M_IRQ_CLEAR.get() && wr.1 & M_SEC_IRQ != 0),
        "M_IRQ_CLEAR must include the M_SEC_IRQ snapshot bit (c:415)"
    );
}

/// HW-19 RX empty path: no M_SEC_IRQ / no RX_FIFO_WC -> explicit
/// WouldReadNothing-style error and NO read of the RX FIFO data port.
#[test]
fn rx_empty_returns_would_read_nothing() {
    // MockSe never sets M_SEC_IRQ or RX_FIFO_WC. Seed the summary bit so the
    // driver gets past the secondary poll and reaches the FIFO gate, proving
    // the gate itself (not the poll) produces WouldReadNothing.
    let mut mock = MockSe::new();
    mock.seed(SE_GENI_M_IRQ_STATUS, M_SEC_IRQ);
    mock.seed(SE_GENI_RX_FIFO_STATUS, 0); // wc == 0: nothing readable (c:420)
    let mut uart = GeniUart::new(mock).with_poll_budget(4);
    assert_eq!(uart.read_byte(), Err(RxError::WouldReadNothing));

    let regs = uart.regs_mut();
    // It re-armed the read and cleared IRQs like the oracle (c:408, c:415–416)...
    assert_eq!(
        regs.writes_to(SE_GENI_S_CMD0.get()),
        vec![UART_START_READ_OPCODE << 27]
    );
    assert_eq!(regs.writes_to(SE_GENI_M_IRQ_CLEAR.get()), vec![M_SEC_IRQ]);
    // ...but NEVER touched the RX FIFO data port.
    assert!(
        regs.writes_to(SE_GENI_RX_FIFOn.get()).is_empty(),
        "empty FIFO must not trigger a FIFO data-port access"
    );

    // And with the secondary IRQ never firing at all, the bounded poll must
    // time out explicitly instead of the oracle's silent fall-through (c:410).
    let mut bare = MockSe::new();
    bare.seed(SE_GENI_RX_FIFO_STATUS, 1);
    let mut uart = GeniUart::new(bare).with_poll_budget(4);
    assert_eq!(uart.read_byte(), Err(RxError::SecondaryTimeout));
}

// -------------------------------------------------------------------------
// HW-21 (root advisory): bounded MMIO seam — malformed offsets are rejected
// with explicit errors and reach ZERO pointer-level access. All of this runs
// on the abstract `SeRegs` seam (mocks); no real MMIO is ever exercised.
// -------------------------------------------------------------------------

#[test]
fn offset_rejects_unaligned() {
    // The old safe API accepted read(1)/write(2,..) and did base.add(off)
    // volatile u32 access — UB. The typed seam makes such offsets
    // unconstructable: the checked constructor rejects them explicitly...
    assert_eq!(RegOffset::new(1), Err(OffsetError::Unaligned));
    assert_eq!(RegOffset::new(2), Err(OffsetError::Unaligned));
    assert_eq!(RegOffset::new(0x601), Err(OffsetError::Unaligned));
    // ...and `SeRegs::read`/`write`/`read_fifo` take `RegOffset`, not `u32`,
    // so there is no safe call path that hands an unaligned offset to the
    // pointer arithmetic at all. (A call like `mmio.read(1)` does not
    // compile; the crate's register consts are proven aligned by the
    // const-checked constructor — see window_size_matches_oracle_map.)

    // Zero pointer access through the seam: with the driver running, the
    // mock's access counter equals exactly the number of transcript entries
    // — no access beyond the validated writes ever reaches the model.
    let mut uart = GeniUart::new(AutoSe::new());
    uart.init(CLK_DIV_115200).expect("init");
    uart.write_byte(b'A').expect("tx 'A'");
    let got = uart.regs().inner_writes();
    assert_eq!(
        uart.regs().accesses() as usize,
        got.len(),
        "every model access must be a validated transcript entry"
    );
}

#[test]
fn offset_rejects_out_of_window() {
    // Offsets at/above the window bound are rejected with an explicit error.
    assert_eq!(
        RegOffset::new(SE_WINDOW_SIZE),
        Err(OffsetError::OutOfWindow)
    );
    assert_eq!(
        RegOffset::new(SE_WINDOW_SIZE + 4),
        Err(OffsetError::OutOfWindow)
    );
    assert_eq!(RegOffset::new(0xFFFF_FFFC), Err(OffsetError::OutOfWindow));
    // Boundary: the last in-window word (window - 4) is accepted, and its
    // round-trip value is the raw offset.
    let last = RegOffset::new(SE_WINDOW_SIZE - 4).expect("boundary offset in window");
    assert_eq!(last.get(), SE_WINDOW_SIZE - 4);
    // An unaligned offset that is ALSO out of window reports alignment first
    // (both are UB-class defects; order is documented, not load-bearing).
    assert_eq!(
        RegOffset::new(SE_WINDOW_SIZE + 1),
        Err(OffsetError::Unaligned)
    );

    // Zero pointer access: seed a sentinel far outside the window on the
    // AutoSe model, then run the full HW-18+HW-19 flow. Nothing may land on
    // the sentinel — the driver can only address validated offsets.
    let mut auto = AutoSe::new();
    auto.seed_raw_at(SE_WINDOW_SIZE + 0x40, 0xBAD);
    let mut uart = GeniUart::new(auto);
    uart.init(CLK_DIV_115200).expect("init");
    uart.write_byte(b'A').expect("tx");
    let regs = uart.regs_mut();
    assert_eq!(
        regs.get(SE_WINDOW_SIZE + 0x40),
        0xBAD,
        "out-of-window sentinel must be untouched"
    );
    assert!(
        regs.writes_to_raw(SE_WINDOW_SIZE + 0x40).is_empty(),
        "zero write accesses beyond the window"
    );
}

#[test]
fn validated_offsets_still_work() {
    // The HW-18/HW-19 transcripts pass through the new seam with IDENTICAL
    // asserted offsets and values (only the plumbing — RegOffset consts vs
    // raw u32 — changed; see the unadjusted tests above, all green).
    let mut uart = GeniUart::new(AutoSe::new());
    uart.init(CLK_DIV_115200).expect("init");
    uart.write_byte(b'A').expect("tx 'A'");
    let mut expected = expected_init_writes();
    expected.extend(expected_putc_writes(b'A'));
    assert_eq!(uart.regs().inner_writes(), expected.as_slice());

    // And the RX transcript too (HW-19 getc cycle).
    let mut rx = AutoSe::new();
    rx.seed_rx(0x41);
    let mut uart = GeniUart::new(rx);
    uart.init(CLK_DIV_115200).expect("init");
    assert_eq!(uart.read_byte(), Ok(b'A'));
}

#[test]
fn window_size_matches_oracle_map() {
    // Compile-time proof that the window bound covers the oracle register
    // map: every static register const in this crate is built with
    // `RegOffset::new_unchecked`, which CONST-PANICS on any offset >=
    // SE_WINDOW_SIZE (or unaligned). If the bound ever dropped below the
    // highest used register + 4, the crate would stop compiling. The checks
    // below make that relationship explicit and testable at runtime too.
    const _: RegOffset = RegOffset::new_unchecked(0x810); // SE_GENI_RX_WATERMARK_REG (h:64) — highest named register in the oracle map
    assert!(
        SE_WINDOW_SIZE >= SE_GENI_RX_WATERMARK_REG.get() + 4,
        "window must cover the highest oracle-named register (SE_GENI_RX_WATERMARK_REG = 0x810, geni-se.h h:64) plus one u32"
    );
    // Highest offset actually USED by the driver's transcripts:
    assert!(SE_WINDOW_SIZE >= SE_GENI_M_CMD0.get() + 4);
    assert!(SE_WINDOW_SIZE >= SE_GENI_RX_FIFOn.get() + 4);
    // And the bound stays conservative vs the real DT mapping (0x4000,
    // HW-CONSOLE.md §2) — we never claim more window than the node provides.
    assert!(SE_WINDOW_SIZE <= 0x4000);
    // Every const is aligned by construction; spot-check the FIFO ports:
    assert_eq!(SE_GENI_TX_FIFOn.get() % 4, 0);
    assert_eq!(SE_GENI_RX_FIFOn.get() % 4, 0);
}

// -------------------------------------------------------------------------
// HW-23: text-output bridge — write_str newline policy, banner emission
// over TxSink, object safety, mid-string error propagation. Host mock only.
// -------------------------------------------------------------------------

/// Bytes the driver pushed to the TX FIFO data port, in order.
fn fifo_bytes(writes: &[super::mock::Write]) -> Vec<u8> {
    writes
        .iter()
        .filter(|wr| wr.0 == SE_GENI_TX_FIFOn.get())
        .map(|wr| wr.1 as u8)
        .collect()
}

/// HW-23 newline policy: `\n` is emitted as `\n` (serial convention, LF only
/// — no CR insertion). "a\nb" must hit the FIFO port as exactly a, \n, b.
#[test]
fn write_str_crlf_policy() {
    let mut uart = GeniUart::new(AutoSe::new());
    uart.init(CLK_DIV_115200).expect("init");
    uart.write_str("a\nb").expect("tx \"a\\nb\"");

    assert_eq!(
        fifo_bytes(&uart.regs().inner_writes()),
        vec![b'a', b'\n', b'b'],
        "newline policy is LF-only: no CR may be inserted, no byte dropped"
    );
}

/// HW-23 banner bridge: `emit_banner` must put the EXACT fb-write BANNER
/// bytes + one trailing `\n` on the FIFO port, and return that byte count.
/// The expected string is scanned out of the read-only fb-write evidence
/// source itself (`include_str!`), so the copied const in console.rs cannot
/// silently drift from its provenance.
#[test]
fn emit_banner_matches_fb_write() {
    // Provenance pin: the original BANNER literal, extracted from the
    // read-only fb-write evidence source (no cross-crate dependency).
    const EVIDENCE_SRC: &str = include_str!("../../fb-write/src/evidence.rs");
    const MARKER: &str = "pub const BANNER: &str = \"";
    let start = EVIDENCE_SRC
        .find(MARKER)
        .expect("fb-write evidence.rs must still declare pub const BANNER");
    let rest = &EVIDENCE_SRC[start + MARKER.len()..];
    // The literal's own terminator is the first UNESCAPED `"` — the banner
    // contains `\"` (escaped quote) before it, so a naive `"` scan lands
    // inside the string.
    let mut end = 0;
    let mut escaped = false;
    for (i, ch) in rest.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            end = i;
            break;
        }
    }
    assert!(end > 0, "BANNER literal terminator in evidence.rs");
    let literal = &rest[..end];

    // The copied const must equal the original byte-for-byte (the banner is
    // ASCII with no escapes in its VALUE — the source escapes the `"` glyph
    // only — so unescaping the raw literal recovers the evaluated bytes).
    let unescaped = literal.replace("\\\"", "\"");
    assert_eq!(
        BANNER.as_bytes(),
        unescaped.as_bytes(),
        "console::BANNER must be a byte-for-byte copy of fb-write evidence.rs BANNER"
    );

    let mut uart = GeniUart::new(AutoSe::new());
    uart.init(CLK_DIV_115200).expect("init");
    let written = console::emit_banner(&mut uart).expect("emit banner");

    let mut expected: Vec<u8> = BANNER.as_bytes().to_vec();
    expected.push(b'\n');
    assert_eq!(
        written,
        expected.len(),
        "return value must be the byte count"
    );
    assert_eq!(written, 94, "93 banner bytes + LF");
    assert_eq!(
        fifo_bytes(&uart.regs().inner_writes()),
        expected,
        "FIFO-port bytes must equal the exact BANNER bytes + \\n"
    );
}

/// HW-23: `TxSink` is object-safe — the driver works through a `&mut dyn
/// TxSink` trait object and the writes land on the FIFO port.
#[test]
fn sink_trait_object_safe() {
    let mut uart = GeniUart::new(AutoSe::new());
    uart.init(CLK_DIV_115200).expect("init");

    // The very construction below is the object-safety proof: `dyn TxSink`
    // only compiles if the trait passes the object-safety rules.
    {
        let sink: &mut dyn TxSink = &mut uart;
        sink.write_str("hi\n").expect("write via dyn TxSink");
    }

    assert_eq!(
        fifo_bytes(&uart.regs().inner_writes()),
        vec![b'h', b'i', b'\n']
    );

    // And the banner bridge works over the erased sink too.
    let written = console::emit_banner(&mut uart).expect("emit banner via TxSink");
    assert_eq!(written, 94);
}

/// HW-23: a TX failure mid-string must surface as an error while the bytes
/// already pushed remain on the FIFO port (no buffering, no rollback). The
/// mock drops the 3rd FIFO push (watermark/done never re-arm) — the first
/// two putc cycles complete and their bytes stay visible on the port; the
/// third cycle arms but its data never reaches the model.
#[test]
fn tx_error_propagates_midstring() {
    let fail = FailAtByte::new(3); // 3rd FIFO push is refused by the model
    let mut uart = GeniUart::new(fail).with_poll_budget(4);
    let err = uart.write_str("abcdef").unwrap_err();

    assert!(
        matches!(err, TxError::FifoNotReady | TxError::TxNotDone),
        "failure must surface from write_str, got {err:?}"
    );
    // Bytes 1-2 completed their full putc cycles and stay on the port; the
    // refused 3rd push did not land. No byte past the failure was written.
    assert_eq!(
        uart.regs().fifo_bytes(),
        vec![b'a', b'b'],
        "bytes written before the failure must stay on the FIFO port"
    );

    // Same via the sink seam: the error maps into TxSinkError::Tx.
    let mut uart = GeniUart::new(FailAtByte::new(3)).with_poll_budget(4);
    let err = {
        let sink: &mut dyn TxSink = &mut uart;
        sink.write_str("abcdef").unwrap_err()
    };
    assert!(
        matches!(
            err,
            TxSinkError::Tx(TxError::FifoNotReady) | TxSinkError::Tx(TxError::TxNotDone)
        ),
        "sink seam must surface the same failure, got {err:?}"
    );
    assert_eq!(uart.regs().fifo_bytes(), vec![b'a', b'b']);
}

// -------------------------------------------------------------------------
// HW-26: RX line discipline — LF/CR line rules, backspace editing,
// too-long discard, control filtering. Host mock (AutoSe RX feed) only.
// -------------------------------------------------------------------------

/// Feed bytes into the mock's modelled RX FIFO (low byte = char, popped in
/// order by the driver's getc cycle — see mock.rs `seed_rx`).
fn feed_rx(auto: &mut AutoSe, bytes: &[u8]) {
    for &b in bytes {
        auto.seed_rx(b as u32);
    }
}

/// HW-26: LF terminates a line and is consumed; the count BEFORE the LF is
/// returned. "ab\ncd" → first call yields "ab" (2). HW-29 gap contract: with
/// "cd" buffered but unterminated, a further poll returns pending `Ok(0)`
/// (NOT the partial as a line — that was the HW-26 gap-split defect); after
/// feeding the missing LF, the preserved partial completes as "cd" (2).
#[test]
fn line_ends_on_lf() {
    // Scripted byte source so we can feed more bytes while the reader is
    // alive (the register mock cannot be re-seeded through the reader's
    // mutable borrow of `uart`). Gap semantics on the real mock are pinned
    // by the root probe and `gap_pending`.
    let script = ScriptedRx::shared(b"ab\ncd");
    let mut rx = ScriptedRx::from_shared(Rc::clone(&script));
    let mut reader = rxline::LineReader::new(&mut rx);
    let mut buf = [0u8; 16];

    assert_eq!(reader.read_line(&mut buf), Ok(2));
    assert_eq!(&buf[..2], b"ab", "LF consumed, not returned");

    // HW-29 ADAPTED assertion: "cd" is still buffered mid-line — with the
    // source drained and no LF seen, this call must NOT emit the partial as
    // a line (HW-26 returned Ok(2) here; that was the gap-split defect the
    // root probe pins). Pending instead: Ok(0), nothing completed.
    assert_eq!(reader.read_line(&mut buf), Ok(0));
    assert!(!reader.last_completed(), "mid-line FIFO drain is pending");

    // Feed the missing terminator: the preserved partial completes as one
    // line "cd" on the SAME reader.
    feed_script(&script, &[Some(b'\n')]);
    assert_eq!(reader.read_line(&mut buf), Ok(2));
    assert!(reader.last_completed(), "LF completes the buffered line");
    assert_eq!(&buf[..2], b"cd");

    // Same discipline through the real register mock: init/start and the
    // first line still work end-to-end.
    let mut auto = AutoSe::new();
    feed_rx(&mut auto, b"ab\n");
    let mut uart = GeniUart::new(auto);
    uart.init(CLK_DIV_115200).expect("init");
    uart.start_rx();
    let mut reader = rxline::LineReader::new(&mut uart);
    assert_eq!(reader.read_line(&mut buf), Ok(2));
    assert_eq!(&buf[..2], b"ab");
}

/// HW-26: CRLF is ONE line, not two. "x\r\n" → one line "x"; the LF of the
/// pair is swallowed by the pending-CR state.
#[test]
fn crlf_is_one_line() {
    let mut auto = AutoSe::new();
    feed_rx(&mut auto, b"x\r\n");
    let mut uart = GeniUart::new(auto);
    uart.init(CLK_DIV_115200).expect("init");
    uart.start_rx();

    let mut reader = rxline::LineReader::new(&mut uart);
    let mut buf = [0u8; 16];

    assert_eq!(reader.read_line(&mut buf), Ok(1));
    assert_eq!(&buf[..1], b"x");
    // The LF of the CRLF pair must NOT produce a second (empty) line: the
    // next call finds the FIFO empty and returns Ok(0).
    assert_eq!(reader.read_line(&mut buf), Ok(0));
}

/// HW-26: backspace editing. The brief pins the exact case: DEL (0x7F)
/// arriving after 'd' erases 'd' — feed "abcd\x7F\n" and assert the line
/// is exactly "abc".
#[test]
fn backspace_edits() {
    let mut auto = AutoSe::new();
    // "abcd" then DEL (0x7F) erases the last buffered byte 'd', then LF.
    feed_rx(&mut auto, b"abcd\x7f\n");
    let mut uart = GeniUart::new(auto);
    uart.init(CLK_DIV_115200).expect("init");
    uart.start_rx();

    let mut reader = rxline::LineReader::new(&mut uart);
    let mut buf = [0u8; 16];

    assert_eq!(reader.read_line(&mut buf), Ok(3));
    assert_eq!(&buf[..3], b"abc", "0x7F after 'd' must erase 'd'");

    // BS (0x08) behaves identically, and backspace on an empty buffer is
    // ignored (no underflow, no error).
    let mut auto2 = AutoSe::new();
    feed_rx(&mut auto2, b"\x08hi\x08\n");
    let mut uart2 = GeniUart::new(auto2);
    uart2.init(CLK_DIV_115200).expect("init");
    uart2.start_rx();
    let mut reader2 = rxline::LineReader::new(&mut uart2);
    assert_eq!(reader2.read_line(&mut buf), Ok(1));
    assert_eq!(&buf[..1], b"h", "leading BS ignored; second BS erases 'i'");
}

/// HW-27 (brief-example sequence from HW-26): backspace MID-WORD. Feed the
/// literal `b"abc\x7fd\n"` — DEL arrives between 'c' and 'd', so it erases
/// 'c' and the subsequently typed 'd' survives: the line is exactly "abd".
#[test]
fn backspace_midword() {
    let mut auto = AutoSe::new();
    feed_rx(&mut auto, b"abc\x7fd\n");
    let mut uart = GeniUart::new(auto);
    uart.init(CLK_DIV_115200).expect("init");
    uart.start_rx();

    let mut reader = rxline::LineReader::new(&mut uart);
    let mut buf = [0u8; 16];

    assert_eq!(reader.read_line(&mut buf), Ok(3));
    assert_eq!(&buf[..3], b"abd", "DEL after 'c' erases 'c'; 'd' survives");
}

/// Test source that emits a scripted sequence of bytes and `None`s (the
/// `None` entries model a FIFO gap → `WouldReadNothing`), then stays empty.
/// The script lives behind `Rc<RefCell<_>>` so tests can feed more bytes
/// while a `LineReader` already holds the source as `&mut dyn ByteSource`.
/// Used by the HW-29 tests; independent of the register mocks.
struct ScriptedRx {
    script: alloc::rc::Rc<core::cell::RefCell<Vec<Option<u8>>>>,
}

impl ScriptedRx {
    fn shared(bytes: &[u8]) -> alloc::rc::Rc<core::cell::RefCell<Vec<Option<u8>>>> {
        Rc::new(RefCell::new(bytes.iter().map(|&b| Some(b)).collect()))
    }
    fn from_shared(script: alloc::rc::Rc<core::cell::RefCell<Vec<Option<u8>>>>) -> Self {
        Self { script }
    }
}

impl ByteSource for ScriptedRx {
    fn read_byte(&mut self) -> Result<u8, ByteSourceError> {
        let mut s = self.script.borrow_mut();
        if s.is_empty() {
            return Err(ByteSourceError::Rx(RxError::WouldReadNothing));
        }
        match s.remove(0) {
            Some(b) => Ok(b),
            None => Err(ByteSourceError::Rx(RxError::WouldReadNothing)),
        }
    }
}

/// Append more bytes to a shared script (may be called while a reader holds
/// the source — interior mutability, no aliasing conflict).
fn feed_script(script: &RefCell<Vec<Option<u8>>>, bytes: &[Option<u8>]) {
    let mut s = script.borrow_mut();
    s.extend_from_slice(bytes);
}

/// Append plain bytes to a shared script as `Some` entries (HW-33 shorthand).
fn feed_bytes(script: &RefCell<Vec<Option<u8>>>, bytes: &[u8]) {
    let mut s = script.borrow_mut();
    s.extend(bytes.iter().map(|&b| Some(b)));
}

/// Recording TX sink for the REPL tests: collects every byte written.
#[derive(Default)]
struct TxLogShared {
    bytes: Rc<RefCell<Vec<u8>>>,
    /// If set, the n-th write_byte (1-based) fails instead of recording.
    fail_at: Option<usize>,
    writes: usize,
}

impl TxSink for TxLogShared {
    fn write_byte(&mut self, b: u8) -> Result<(), TxSinkError> {
        self.writes += 1;
        if self.fail_at == Some(self.writes) {
            return Err(TxSinkError::Tx(TxError::FifoNotReady));
        }
        self.bytes.borrow_mut().push(b);
        Ok(())
    }
}

fn tx_str(bytes: &RefCell<Vec<u8>>) -> alloc::string::String {
    alloc::string::String::from_utf8(bytes.borrow().clone()).expect("tx log is utf-8")
}

/// HW-29 REPL: a full line "hi\n" completes → `Some(2)` and the TX echo is
/// exactly `you said: hi\n`.
#[test]
fn echo_roundtrip() {
    let script = ScriptedRx::shared(b"hi\n");
    let tx_bytes = Rc::new(RefCell::new(Vec::<u8>::new()));
    let mut rx = ScriptedRx::from_shared(script);
    let mut tx = TxLogShared {
        bytes: Rc::clone(&tx_bytes),
        ..Default::default()
    };
    let mut console = repl::Console::new(&mut tx, &mut rx);
    let mut buf = [0u8; 16];
    assert_eq!(console.echo_line(&mut buf), Ok(Some(2)));
    assert_eq!(&buf[..2], b"hi");
    drop(console);
    assert_eq!(tx_str(&tx_bytes), "you said: hi\n");
}

/// HW-29 REPL + gap contract: `a, b, GAP` → `Ok(None)` with NOTHING on TX;
/// then `c, LF` completes the preserved partial → `Some(3)` and the echo is
/// exactly `you said: abc\n` (one line, never split across the gap).
#[test]
fn gap_pending() {
    let script = ScriptedRx::shared(b"ab");
    feed_script(&script, &[None]); // a FIFO gap after "ab"
    let tx_bytes = Rc::new(RefCell::new(Vec::<u8>::new()));
    let mut rx = ScriptedRx::from_shared(Rc::clone(&script));
    let mut tx = TxLogShared {
        bytes: Rc::clone(&tx_bytes),
        ..Default::default()
    };
    let mut console = repl::Console::new(&mut tx, &mut rx);
    let mut buf = [0u8; 16];
    assert_eq!(console.echo_line(&mut buf), Ok(None), "gap must not echo");
    assert_eq!(tx_str(&tx_bytes), "", "nothing written while pending");
    // Feed the rest only NOW — the partial "ab" survives inside the reader.
    script.borrow_mut().push(Some(b'c'));
    script.borrow_mut().push(Some(b'\n'));
    assert_eq!(console.echo_line(&mut buf), Ok(Some(3)));
    assert_eq!(&buf[..3], b"abc");
    drop(console);
    assert_eq!(tx_str(&tx_bytes), "you said: abc\n");
}

/// HW-29 REPL: `write_prompt("redox>")` emits exactly `redox>\n` (LF-only
/// newline policy, see `TxSink::write_str`).
#[test]
fn prompt_written() {
    let script = ScriptedRx::shared(b"");
    let tx_bytes = Rc::new(RefCell::new(Vec::<u8>::new()));
    let mut rx = ScriptedRx::from_shared(script);
    let mut tx = TxLogShared {
        bytes: Rc::clone(&tx_bytes),
        ..Default::default()
    };
    let mut console = repl::Console::new(&mut tx, &mut rx);
    assert_eq!(console.write_prompt("redox>"), Ok(7));
    drop(console);
    assert_eq!(tx_str(&tx_bytes), "redox>\n");
}

/// HW-29 REPL: a TX failure mid-echo surfaces as `ConsoleError::Tx` with the
/// raw `TxSinkError` preserved; bytes before the failing one did go out.
#[test]
fn tx_failure_surfaces() {
    let script = ScriptedRx::shared(b"hi\n");
    // Echo starts with "you said: ": write #1='y' #2='o' #3='u' fails.
    let tx_bytes = Rc::new(RefCell::new(Vec::<u8>::new()));
    let mut rx = ScriptedRx::from_shared(script);
    let mut tx = TxLogShared {
        bytes: Rc::clone(&tx_bytes),
        fail_at: Some(3),
        ..Default::default()
    };
    let mut console = repl::Console::new(&mut tx, &mut rx);
    let mut buf = [0u8; 16];
    assert_eq!(
        console.echo_line(&mut buf),
        Err(repl::ConsoleError::Tx(TxSinkError::Tx(
            TxError::FifoNotReady
        ))),
        "TX failure must surface wrapped in ConsoleError::Tx"
    );
    drop(console);
    assert_eq!(
        tx_str(&tx_bytes),
        "yo",
        "bytes before the failing one were written"
    );
}

/// HW-29: buffer full → `LineTooLong(cap)` and the partial line is
/// DISCARDED. Capacity 4, feed "abcdef\n": 'e' overflows → LineTooLong(4)
/// and "abcd" is thrown away; the reader resumes collecting, 'f' is then
/// swallowed by the LF that ends the junk line, and bytes after the LF
/// read cleanly as a new line.
#[test]
fn too_long_discards() {
    let mut auto = AutoSe::new();
    feed_rx(&mut auto, b"abcdef\nok\n");
    let mut uart = GeniUart::new(auto);
    uart.init(CLK_DIV_115200).expect("init");
    uart.start_rx();

    let mut reader = rxline::LineReader::new(&mut uart);
    let mut buf = [0u8; 4];

    // 'a'..'d' fill the 4-byte capacity; 'e' is the offending byte.
    assert_eq!(
        reader.read_line(&mut buf),
        Err(rxline::RxLineError::LineTooLong(4)),
        "overflow must report the capacity that was exceeded"
    );
    // Discard policy: the partial "abcd" is gone. The reader resumes; 'f'
    // is collected then thrown away by the line-ending LF, so the rest of
    // the junk line yields no line at all.
    // Post-LF bytes start a clean line: "ok" reads normally.
    assert_eq!(reader.read_line(&mut buf), Ok(2));
    assert_eq!(
        &buf[..2],
        b"ok",
        "next line reads cleanly from post-LF bytes"
    );
}

// -------------------------------------------------------------------------
// HW-31: bounded per-call work budget — one read_line consumes at most
// MAX_BYTES_PER_CALL bytes from the source; budget exhaustion returns
// pending Ok(0) with all state preserved. Instrumented mock (byte-counting
// ByteSource) only — no MMIO.
// -------------------------------------------------------------------------

/// HW-31 test plumbing: a byte source that counts how many source-read
/// ATTEMPTS the reader made. HW-35: `attempts` counts EVERY `read_byte`
/// call — successful reads AND gap (`None`) errors alike — matching the
/// HW-31 budget rule that each loop iteration costs one attempt regardless
/// of outcome. `bytes_read` separately counts successful byte reads, so
/// tests can pin both quantities. The counts live in `Rc<Cell<_>>`s so
/// tests can read them while the `LineReader` still holds the source as
/// `&mut dyn ByteSource`.
#[derive(Clone)]
struct CountingRx {
    bytes: Rc<core::cell::RefCell<alloc::collections::VecDeque<u8>>>,
    consumed: Rc<core::cell::Cell<usize>>,
    attempts: Rc<core::cell::Cell<usize>>,
}

impl CountingRx {
    fn new(bytes: alloc::collections::VecDeque<u8>) -> Self {
        Self {
            bytes: Rc::new(core::cell::RefCell::new(bytes)),
            consumed: Rc::new(core::cell::Cell::new(0)),
            attempts: Rc::new(core::cell::Cell::new(0)),
        }
    }

    /// Append more bytes to the stream (test-side, while the reader holds
    /// the source: the queue is shared behind a `RefCell`, so this takes
    /// `&self` and stays callable through the reader's `&mut` borrow).
    fn feed(&self, bytes: &[u8]) {
        self.bytes.borrow_mut().extend(bytes.iter().copied());
    }
}

impl ByteSource for CountingRx {
    fn read_byte(&mut self) -> Result<u8, ByteSourceError> {
        // HW-35: every read ATTEMPT counts against the budget, gap errors
        // included — that is what read_line charges per loop iteration.
        self.attempts.set(self.attempts.get() + 1);
        match self.bytes.borrow_mut().pop_front() {
            Some(b) => {
                self.consumed.set(self.consumed.get() + 1);
                Ok(b)
            }
            None => Err(ByteSourceError::Rx(RxError::WouldReadNothing)),
        }
    }
}

/// HW-35 variant of `CountingRx`: the queue holds `Option<u8>` entries, so
/// a `None` slot is an explicit FIFO gap (`WouldReadNothing`) in the MIDDLE
/// of the stream — unlike plain `CountingRx`, where gaps only occur once the
/// queue drains. Same attempt/byte counters: `attempts` counts every
/// `read_byte` call, `consumed` only successful byte reads.
#[derive(Clone)]
struct GapAwareCountingRx {
    slots: Rc<core::cell::RefCell<alloc::collections::VecDeque<Option<u8>>>>,
    consumed: Rc<core::cell::Cell<usize>>,
    attempts: Rc<core::cell::Cell<usize>>,
}

impl GapAwareCountingRx {
    fn new(slots: alloc::collections::VecDeque<Option<u8>>) -> Self {
        Self {
            slots: Rc::new(core::cell::RefCell::new(slots)),
            consumed: Rc::new(core::cell::Cell::new(0)),
            attempts: Rc::new(core::cell::Cell::new(0)),
        }
    }
}

impl ByteSource for GapAwareCountingRx {
    fn read_byte(&mut self) -> Result<u8, ByteSourceError> {
        self.attempts.set(self.attempts.get() + 1);
        match self.slots.borrow_mut().pop_front() {
            Some(Some(b)) => {
                self.consumed.set(self.consumed.get() + 1);
                Ok(b)
            }
            // Explicit gap slot, or drained queue: both are FIFO gaps.
            Some(None) | None => Err(ByteSourceError::Rx(RxError::WouldReadNothing)),
        }
    }
}

/// HW-31: a continuous stream of DEL/BS control bytes can no longer make one
/// call loop forever. Feed 1000 BS bytes (0x7F) then LF: the first
/// `read_line` consumes at most `MAX_BYTES_PER_CALL` source bytes (counted
/// via the instrumented mock) and returns pending `Ok(0)`; subsequent calls
/// keep consuming in budget-sized chunks; the final line completes as an
/// empty line (`Ok(0)` with `last_completed() == true`) because every BS
/// erased nothing on an empty buffer.
#[test]
fn budget_bounds_control_stream() {
    let mut source = CountingRx::new(
        core::iter::repeat(0x7Fu8)
            .take(1000)
            .chain(core::iter::once(b'\n'))
            .collect(),
    );
    // The shared count is read through an Rc the source also holds, so it
    // stays test-accessible while the reader owns `&mut source`.
    let consumed = Rc::clone(&source.consumed);
    let mut reader = rxline::LineReader::new(&mut source);
    let mut buf = [0u8; 16];

    // First call: budget-bounded, pending — no line completed.
    assert_eq!(reader.read_line(&mut buf), Ok(0));
    assert!(!reader.last_completed(), "budget exhaustion is pending");
    assert_eq!(
        consumed.get(),
        rxline::MAX_BYTES_PER_CALL,
        "first call must consume exactly the budget when the stream is continuous"
    );

    // Subsequent calls finish the stream in budget-sized chunks, each still
    // bounded by the budget.
    let mut total = consumed.get();
    let mut completed = false;
    for _ in 0..100 {
        let before = consumed.get();
        let r = reader.read_line(&mut buf);
        assert!(consumed.get() - before <= rxline::MAX_BYTES_PER_CALL);
        total = consumed.get();
        if r == Ok(0) && reader.last_completed() {
            completed = true;
            break;
        }
    }
    assert!(completed, "the LF must eventually complete a line");
    // All 1000 BS erased nothing (empty buffer), so the completed line is
    // the empty line: Ok(0) + last_completed, and the LF was the last byte.
    assert_eq!(total, 1001, "exactly the 1000 BS + the LF consumed");
}

/// HW-31: budget exhaustion mid-line preserves the partial exactly like the
/// FIFO-gap path. Feed MAX+3 storable bytes then LF with a 300-byte `buf`:
/// the first call consumes the full budget (MAX bytes, all buffered — the
/// budget return happens on the next loop iteration *before* another source
/// read, so no byte is consumed but unprocessed) and returns pending `Ok(0)`
/// with the partial intact; the second call continues from that partial and
/// completes the line at exactly MAX+3 bytes. Because `buf.len()` (300)
/// EXCEEDS `MAX_BYTES_PER_CALL` (256), the budget runs out mid-line before
/// any `LineTooLong` could fire. HW-35: the stream is a VARIED cycling
/// printable sequence (0x21..0x7E) compared byte-exactly, so a byte lost,
/// duplicated, or reordered across the budget boundary cannot pass. The
/// tail of the test then shrinks the cap to 64 and pins the documented
/// HW-27/HW-31 interaction: with `buf.len() < MAX_BYTES_PER_CALL` the line
/// overflows caller capacity before the budget can run out, so the
/// `LineTooLong(64)` fires on the 65th storable byte, the partial is
/// discarded, and the remainder is swallowed to the LF.
#[test]
fn budget_preserves_partial() {
    // HW-35: varied cycling printable sequence, not a uniform pattern —
    // byte-exact comparison below detects drop/duplicate/reorder.
    let pattern: Vec<u8> = (0x21u8..=0x7E).collect();
    let n = rxline::MAX_BYTES_PER_CALL + 3;
    let expected: Vec<u8> = pattern.iter().copied().cycle().take(n).collect();
    let mut source = CountingRx::new(
        expected
            .iter()
            .copied()
            .chain(core::iter::once(b'\n'))
            .collect(),
    );
    let consumed = Rc::clone(&source.consumed);
    let queue = Rc::clone(&source.bytes);
    let mut reader = rxline::LineReader::new(&mut source);
    // Capacity > budget on purpose: the budget must run out mid-line with
    // the partial preserved, before any LineTooLong could fire.
    let mut buf = [0u8; 300];

    // First call: full budget consumed, pending, partial preserved. The
    // budget check runs BEFORE each source read, so exactly MAX bytes are
    // consumed and all MAX enter the line buffer — nothing is consumed but
    // unprocessed across the budget boundary.
    assert_eq!(reader.read_line(&mut buf), Ok(0));
    assert!(!reader.last_completed(), "budget exhaustion is pending");
    assert_eq!(consumed.get(), rxline::MAX_BYTES_PER_CALL);

    // Second call: continues from the preserved partial (3 bytes left, then
    // the LF) and completes the line at exactly MAX+3 bytes — no byte was
    // lost or duplicated across the budget return.
    assert_eq!(
        reader.read_line(&mut buf),
        Ok(rxline::MAX_BYTES_PER_CALL + 3)
    );
    assert!(reader.last_completed(), "the LF completed the line");
    assert_eq!(
        &buf[..n],
        &expected[..],
        "byte-exact: the varied partial survived the budget boundary"
    );
    assert_eq!(consumed.get(), rxline::MAX_BYTES_PER_CALL + 4);

    // Discard-mode streams are bounded too: overflow a fresh line and check
    // that the swallow loop stops at the budget, not at the LF. Capacity <
    // budget pins the documented LineTooLong interaction (overflow is
    // reported before any budget return).
    for _ in 0..1000 {
        queue.borrow_mut().push_back(b'b');
    }
    let mut small = [0u8; 64];
    assert_eq!(
        reader.read_line(&mut small),
        Err(rxline::RxLineError::LineTooLong(64)),
        "capacity < budget: the overflow completes this call before the budget can run out"
    );
    assert_eq!(consumed.get(), rxline::MAX_BYTES_PER_CALL + 4 + 65);

    // Discard mode then swallows junk within the per-call budget — 935 'b'
    // remain, but this call consumes at most MAX of them and returns
    // pending: no unbounded swallow loop.
    let before = consumed.get();
    assert_eq!(reader.read_line(&mut small), Ok(0));
    assert!(
        !reader.last_completed(),
        "discard-mode bytes are budget-bounded too"
    );
    assert_eq!(consumed.get() - before, rxline::MAX_BYTES_PER_CALL);

    // And the policy still completes: drain the rest in bounded calls until
    // a post-LF line comes through clean.
    queue.borrow_mut().extend(b"\nok\n");
    let mut completed = false;
    for _ in 0..20 {
        if reader.read_line(&mut small) == Ok(2) && reader.last_completed() {
            completed = true;
            break;
        }
    }
    assert!(
        completed,
        "discard state survives budget returns end-to-end"
    );
    assert_eq!(&small[..2], b"ok");
}

/// HW-35 (watch review of HW-31B): `pending_cr` must survive budget
/// exhaustion when CR is the LAST byte the call consumes. Feed 255 storable
/// bytes + CR as the 256th consumed byte: the CR completes the line (setting
/// `pending_cr`) and the call returns `Ok(255)` before the budget check can
/// fire; the NEXT call starts at the LF, which the preserved `pending_cr`
/// must swallow — ONE line total, no phantom empty line after it. If a
/// regression cleared `pending_cr` on the budget/gap return, the LF would
/// instead complete a second (empty) line and this test would fail.
#[test]
fn budget_cr_at_boundary_preserved() {
    // 255 storable bytes (varied pattern) + CR = exactly 256 consumed bytes.
    let pattern: Vec<u8> = (0x21u8..=0x7E).collect();
    let expected: Vec<u8> = pattern.iter().copied().cycle().take(255).collect();
    let mut source = CountingRx::new(
        expected
            .iter()
            .copied()
            .chain(core::iter::once(b'\r'))
            .chain(core::iter::once(b'\n'))
            .collect(),
    );
    let consumed = Rc::clone(&source.consumed);
    let attempts = Rc::clone(&source.attempts);
    let mut reader = rxline::LineReader::new(&mut source);
    // cap 300 > budget: the budget is the binding bound, not capacity.
    let mut buf = [0u8; 300];

    // First call: the CR is the 256th consumed byte and completes the line
    // before any budget return; exactly the budget is consumed.
    assert_eq!(reader.read_line(&mut buf), Ok(255));
    assert!(reader.last_completed(), "the CR completed the line");
    assert_eq!(
        &buf[..255],
        &expected[..],
        "byte-exact: all 255 varied bytes delivered"
    );
    assert_eq!(consumed.get(), rxline::MAX_BYTES_PER_CALL);
    assert_eq!(attempts.get(), rxline::MAX_BYTES_PER_CALL);

    // Second call: the LF arrives with `pending_cr` preserved across the
    // call boundary. It must be SWALLOWED (CRLF = one line), leaving the
    // source empty: the call then hits the FIFO gap and returns pending
    // `Ok(0)` with `last_completed() == false` — NOT a completed empty line.
    assert_eq!(reader.read_line(&mut buf), Ok(0));
    assert!(
        !reader.last_completed(),
        "the preserved pending_cr swallowed the LF: no phantom empty line"
    );
    assert_eq!(
        consumed.get(),
        rxline::MAX_BYTES_PER_CALL + 1,
        "the swallowed LF was consumed as the second half of the CRLF pair"
    );
}

/// HW-35 (watch review of HW-31B), header narrowed by HW-36: what this
/// body actually pins. The source holds 256 explicit gap slots followed by
/// 256 storable bytes and an LF. Each call's FIRST gap attempt (a
/// `WouldReadNothing` error) ends the call immediately via the HW-29 gap
/// return — `Ok(0)` pending — so 256 calls drain the 256 gap slots, one
/// slot per call. The instrumented source shows each call made exactly one
/// read attempt (the source-side `attempts` counter increments once per
/// call, 1..=256) and zero successful byte reads (`consumed` stays 0
/// through all 256 gap-slot calls). NOTE (HW-36 honesty): this does NOT
/// pin the reader's INTERNAL budget charging gaps — the per-call budget
/// resets at every call boundary, so an internal budget that never
/// charged gap attempts would behave identically here. What IS pinned:
/// gap returns are lossless (no byte lost, no phantom completion), and
/// after the gap run the line completes byte-exact across two budget
/// boundaries — call 257 consumes the whole fresh budget on the 256
/// storable bytes (still pending), call 258 completes at the LF with
/// every varied byte intact.
#[test]
fn gap_attempt_ends_call_losslessly() {
    let pattern: Vec<u8> = (0x41u8..=0x5Au8).collect();
    let expected: Vec<u8> = (0..256usize).map(|i| pattern[i % 26]).collect();
    // 256 explicit gap slots, then 256 storable bytes, then the LF.
    let mut q = alloc::collections::VecDeque::new();
    for _ in 0..rxline::MAX_BYTES_PER_CALL {
        q.push_back(None);
    }
    q.extend(expected.iter().map(|&b| Some(b)));
    q.push_back(Some(b'\n'));
    let mut source = GapAwareCountingRx::new(q);
    let consumed = Rc::clone(&source.consumed);
    let attempts = Rc::clone(&source.attempts);
    let mut reader = rxline::LineReader::new(&mut source);
    let mut buf = [0u8; 512];

    // Call 1: the FIRST gap attempt (a `WouldReadNothing` error) ends the
    // call via the HW-29 gap return — and the instrumented source shows
    // the gap was CHARGED as a source-side read attempt (attempts == 1,
    // consumed == 0). (HW-36 note: the source-side counter is what this
    // pins; the reader's internal per-call budget resets each call and is
    // not observable through this scenario.)
    assert_eq!(reader.read_line(&mut buf), Ok(0));
    assert!(!reader.last_completed(), "the gap return is pending");
    assert_eq!(consumed.get(), 0, "no successful byte reads");
    assert_eq!(
        attempts.get(),
        1,
        "the gap ATTEMPT was charged: a byte-only budget model would show 0"
    );

    // Drain the remaining 255 gap slots with calls that each end at their
    // first gap attempt (255 more calls, source-side attempts 2..=256).
    for i in 2..=rxline::MAX_BYTES_PER_CALL {
        assert_eq!(reader.read_line(&mut buf), Ok(0));
        assert_eq!(attempts.get(), i);
    }
    assert_eq!(consumed.get(), 0, "still zero bytes read");

    // Call 257: the gap slots are gone; the 256 storable bytes consume the
    // whole fresh budget — exhausted exactly at the last byte, before the
    // LF: still pending, nothing lost.
    assert_eq!(reader.read_line(&mut buf), Ok(0));
    assert!(!reader.last_completed());
    assert_eq!(consumed.get(), 256);

    // Call 258: the LF completes the line — all 256 varied bytes, in order,
    // byte-exact across two budget boundaries and the 256-call gap run.
    assert_eq!(reader.read_line(&mut buf), Ok(256));
    assert!(reader.last_completed());
    assert_eq!(
        &buf[..256],
        &expected[..],
        "lossless across budget AND gap boundaries: every byte, in order"
    );
}

/// HW-31: the HW-29 gap contract is untouched by the budget — the
/// independent root probe `crates/geni-uart/tests/uart_line_root_probe.rs`
/// (not modified) stays green, and its exact scenario passes here too: a
/// FIFO gap mid-line returns pending Ok(0) and the preserved partial
/// completes as one line on the next call.
#[test]
fn gap_probe_still_passes() {
    let script = ScriptedRx::shared(b"ab");
    feed_script(&script, &[None]); // FIFO gap after "ab"
    let mut rx = ScriptedRx::from_shared(Rc::clone(&script));
    let mut reader = rxline::LineReader::new(&mut rx);
    let mut buf = [0u8; 16];

    assert_eq!(
        reader.read_line(&mut buf),
        Ok(0),
        "FIFO gap must not complete partial line (root probe contract)"
    );
    assert!(!reader.last_completed());
    feed_script(&script, &[Some(b'c'), Some(b'\n')]);
    assert_eq!(reader.read_line(&mut buf), Ok(3));
    assert_eq!(&buf[..3], b"abc", "one line across the gap, never split");
}

// -------------------------------------------------------------------------
// HW-32: caller-buffer capacity contract — cap = buf.len() is the max
// COMPLETED line length deliverable in one call; cap and the HW-31 budget
// are independent bounds; cap == 0 is defined behavior; losslessness holds
// across the budget boundary. Mock seams only (ScriptedRx / CountingRx) —
// no MMIO.
// -------------------------------------------------------------------------

/// HW-32 `cap_exact_fit`: a line of exactly `cap` storable bytes followed by
/// LF completes in one call: `Ok(cap)` with all `cap` bytes copied into
/// `buf`. The overflow check fires only on the `cap + 1`-th storable byte,
/// so an exact fit never trips `LineTooLong`.
#[test]
fn cap_exact_fit() {
    let mut auto = AutoSe::new();
    feed_rx(&mut auto, b"abcdefg\n");
    let mut uart = GeniUart::new(auto);
    uart.init(CLK_DIV_115200).expect("init");
    uart.start_rx();

    let mut reader = rxline::LineReader::new(&mut uart);
    let mut buf = [0u8; 7]; // cap == line length exactly

    assert_eq!(
        reader.read_line(&mut buf),
        Ok(7),
        "a line of exactly cap bytes must complete, not overflow"
    );
    assert!(reader.last_completed());
    assert_eq!(&buf[..7], b"abcdefg", "all cap bytes delivered");
}

/// HW-32 `cap_smaller_than_line`: with `cap <` the line length, the call
/// returns `Err(LineTooLong(cap))` exactly once per over-long line (partial
/// discarded, nothing in `buf`), and swallow-to-LF stays intact: the rest of
/// the long line up to its LF produces no further output, and the NEXT line
/// completes normally. HW-35: `buf` is pre-filled with a 0xA5 sentinel and
/// asserted byte-for-byte UNCHANGED after the error — pinning the
/// documented "nothing copied into `buf`" half of the `LineTooLong`
/// contract.
#[test]
fn cap_smaller_than_line() {
    let mut auto = AutoSe::new();
    // 10-byte line "abcdefghij" against a 4-byte cap, then a clean line.
    feed_rx(&mut auto, b"abcdefghij\nhi\n");
    let mut uart = GeniUart::new(auto);
    uart.init(CLK_DIV_115200).expect("init");
    uart.start_rx();

    let mut reader = rxline::LineReader::new(&mut uart);
    let mut buf = [0xA5u8; 4]; // HW-35 sentinel fill

    assert_eq!(
        reader.read_line(&mut buf),
        Err(rxline::RxLineError::LineTooLong(4)),
        "cap 4 < line 10: LineTooLong(cap) on the 5th storable byte"
    );
    assert!(!reader.last_completed());
    assert_eq!(
        &buf[..],
        &[0xA5u8; 4][..],
        "LineTooLong copies NOTHING into buf: sentinel bytes untouched"
    );
    // Swallow-to-LF: 'e'..'j' and their LF produce NO second error and no
    // phantom line — the next completion is the post-LF line "hi".
    assert_eq!(
        reader.read_line(&mut buf),
        Ok(2),
        "swallow-to-LF intact: exactly one error per over-long line"
    );
    assert_eq!(&buf[..2], b"hi", "next line completes normally");
}

/// HW-32 `lossless_across_budget`: capacity EXCEEDS the per-call budget — a
/// line of `MAX_BYTES_PER_CALL + 40` bytes plus LF with caller cap 512. The
/// first call exhausts the budget and returns pending `Ok(0)`; the second
/// call completes with EXACTLY the full byte sequence — every byte present
/// in order, no byte lost or duplicated across the budget boundary. HW-35:
/// the line is a VARIED cycling printable sequence (0x21..0x7E), not
/// uniform 'x' bytes, so the byte-exact comparison detects a byte being
/// dropped and another duplicated, or any reordering.
#[test]
fn lossless_across_budget() {
    let n = rxline::MAX_BYTES_PER_CALL + 40;
    let pattern: Vec<u8> = (0x21u8..=0x7E).collect();
    let expected: Vec<u8> = pattern.iter().copied().cycle().take(n).collect();
    let mut source = CountingRx::new(
        expected
            .iter()
            .copied()
            .chain(core::iter::once(b'\n'))
            .collect(),
    );
    let consumed = Rc::clone(&source.consumed);
    let mut reader = rxline::LineReader::new(&mut source);
    // cap 512 > MAX_BYTES_PER_CALL: capacity is not the binding bound here.
    let mut buf = [0u8; 512];

    // First call: budget exhausted mid-line, pending — state preserved.
    assert_eq!(reader.read_line(&mut buf), Ok(0));
    assert!(!reader.last_completed(), "budget exhaustion is pending");
    assert_eq!(
        consumed.get(),
        rxline::MAX_BYTES_PER_CALL,
        "first call consumes exactly the budget"
    );

    // Second call: completes with the FULL line — cap (512) >= n so no
    // LineTooLong; every byte survives the budget boundary.
    assert_eq!(reader.read_line(&mut buf), Ok(n));
    assert!(reader.last_completed());
    assert_eq!(
        buf[..n],
        expected[..],
        "lossless: all MAX+40 varied bytes, in order, byte-exact"
    );
    assert_eq!(
        consumed.get(),
        n + 1,
        "exactly the line bytes + the LF consumed"
    );
}

/// HW-32 `zero_cap_defined`: cap 0 fed `"a\n"` — the code's actual (now
/// documented) behavior: the first storable byte immediately returns
/// `Err(LineTooLong(0))` and the rest of that line is swallowed to its LF;
/// a subsequent bare LF then completes an empty line `Ok(0)` with
/// `last_completed() == true`, proving state still advances normally under
/// a zero-cap buffer.
#[test]
fn zero_cap_defined() {
    let script = ScriptedRx::shared(b"a\n\n");
    let mut rx = ScriptedRx::from_shared(script);
    let mut reader = rxline::LineReader::new(&mut rx);
    let mut buf: [u8; 0] = [];

    // 'a' is storable but cap 0 cannot hold it: too-long at the FIRST
    // storable byte, and the line's LF is swallowed by discard mode.
    assert_eq!(
        reader.read_line(&mut buf),
        Err(rxline::RxLineError::LineTooLong(0)),
        "cap 0: any storable byte makes the line too-long at once"
    );
    assert!(!reader.last_completed());

    // State advanced normally: discard mode ended at the swallowed LF, so
    // the second LF completes an empty line (zero-cap buf never receives
    // bytes; Ok(0) + last_completed is a completed empty line).
    assert_eq!(reader.read_line(&mut buf), Ok(0));
    assert!(
        reader.last_completed(),
        "cap 0 still completes lines: the bare LF is a completed empty line"
    );
}

/// HW-35 (watch review of HW-32): pin the remaining zero-cap behaviors
/// documented on `read_line` — control bytes and a FIFO gap are handled
/// normally under cap 0, BEFORE the first storable byte trips
/// `LineTooLong(0)`. Script: BS, DEL, TAB (0x09), NUL (0x00), gap, then
/// 'a'. The control bytes must produce NO error (BS/DEL pop an empty buffer
/// harmlessly; other `< 0x20` bytes are dropped silently) and the gap must
/// return pending `Ok(0)` with `last_completed() == false` — a zero-cap call
/// with no storable byte still advances state normally. Then 'a' — the
/// first storable byte — trips `Err(LineTooLong(0))` exactly as documented.
#[test]
fn zero_cap_control_and_gap_defined() {
    let script = ScriptedRx::shared(&[0x08, 0x7F, 0x09, 0x00]);
    feed_script(&script, &[None]); // FIFO gap after the control bytes
    feed_script(&script, &[Some(b'a')]);
    let mut rx = ScriptedRx::from_shared(Rc::clone(&script));
    let mut reader = rxline::LineReader::new(&mut rx);
    let mut buf: [u8; 0] = [];

    // Control bytes then the gap: no error, no completion — the gap return
    // is pending Ok(0), identical shape to the nonzero-cap gap contract.
    assert_eq!(
        reader.read_line(&mut buf),
        Ok(0),
        "cap 0: control bytes cause no error; the FIFO gap returns Ok(0)"
    );
    assert!(
        !reader.last_completed(),
        "cap 0: a FIFO gap is pending, never a completed line"
    );

    // The first storable byte now trips the documented zero-cap error.
    assert_eq!(
        reader.read_line(&mut buf),
        Err(rxline::RxLineError::LineTooLong(0)),
        "cap 0: the first storable byte after clean state is LineTooLong(0)"
    );
    assert!(!reader.last_completed());

    // State stays clean after the discard: the swallow-to-LF ends the
    // discarded line, and the next line completes normally (Ok(2), b"ok").
    feed_script(&script, &[Some(b'\n'), Some(b'o'), Some(b'k'), Some(b'\n')]);
    let mut small = [0u8; 4];
    assert_eq!(reader.read_line(&mut small), Ok(2));
    assert!(
        reader.last_completed(),
        "swallow-to-LF ended the discarded line; the next line completes clean"
    );
    assert_eq!(&small[..2], b"ok");
}

// -------------------------------------------------------------------------
// HW-34: shrinking caller cap — a smaller buf on a later call must yield an
// explicit LineTooLong at completion, NEVER a silent min+clear truncation
// (root probe `smaller_completion_buffer_must_not_silently_drop_pending_bytes`).
// -------------------------------------------------------------------------

/// HW-34 `shrinking_cap_completion_errors`: the exact root scenario —
/// source `a b c d GAP LF`; first call with cap 8 returns pending `Ok(0)`;
/// second call with cap 2 at the LF must return
/// `Err(LineTooLong(2))`, NOT `Ok(2)` with "ab" while silently losing "cd".
#[test]
fn shrinking_cap_completion_errors() {
    let script = ScriptedRx::shared(b"abcd");
    let mut rx = ScriptedRx::from_shared(script.clone());
    let mut reader = rxline::LineReader::new(&mut rx);

    // First call: 4 bytes buffered, then the FIFO gap -> pending Ok(0).
    let mut large = [0u8; 8];
    assert_eq!(reader.read_line(&mut large), Ok(0));
    assert!(!reader.last_completed(), "gap mid-line is pending");

    // The LF arrives; the caller shrank cap from 8 to 2. Completion with a
    // 4-byte line into a 2-byte buffer must be an explicit capacity error.
    feed_script(&script, &[Some(b'\n')]);
    let mut small = [0u8; 2];
    assert_eq!(
        reader.read_line(&mut small),
        Err(rxline::RxLineError::LineTooLong(2)),
        "shrinking cap: completion of a longer line is LineTooLong(cap), never Ok(min) with silent drop"
    );
    assert!(!reader.last_completed());
}

/// HW-34 `shrinking_cap_next_line_ok`: after the shrinking-cap error, reader
/// state is clean — feeding `"hi\n"` with cap 4 completes `Ok(2)` with b"hi".
#[test]
fn shrinking_cap_next_line_ok() {
    let script = ScriptedRx::shared(b"abcd");
    let mut rx = ScriptedRx::from_shared(script.clone());
    let mut reader = rxline::LineReader::new(&mut rx);

    let mut large = [0u8; 8];
    assert_eq!(reader.read_line(&mut large), Ok(0));
    feed_script(&script, &[Some(b'\n')]);
    let mut small = [0u8; 2];
    assert_eq!(
        reader.read_line(&mut small),
        Err(rxline::RxLineError::LineTooLong(2)),
        "root scenario first: explicit error on the shrunk cap"
    );

    // State clean after the error (no lingering discard mode): the next line
    // completes normally with an adequate buffer.
    feed_script(&script, &[Some(b'h'), Some(b'i'), Some(b'\n')]);
    let mut buf = [0u8; 4];
    assert_eq!(reader.read_line(&mut buf), Ok(2));
    assert!(reader.last_completed());
    assert_eq!(
        &buf[..2],
        b"hi",
        "next line unaffected by the shrinking-cap error"
    );
}

/// HW-36 `cr_shrinking_cap_then_lf_no_phantom_line`: the CR completion arm
/// of the shrinking-cap rule (the review's exact scenario). Source
/// `a b c d GAP CR GAP LF h i LF`; first call with cap 8 returns pending
/// `Ok(0)`; second call with cap 2 at the CR must return
/// `Err(LineTooLong(2))` — and because the CR terminator IS consumed by
/// that error, `pending_cr` must survive so the matching LF of the CRLF
/// pair is swallowed on the third call. Before the HW-36 fix the LF was
/// treated as a bare LF and manufactured a phantom completed empty line
/// (`Ok(0)` with `last_completed() == true`); the third call must instead
/// complete `Ok(2)` with b"hi".
#[test]
fn cr_shrinking_cap_then_lf_no_phantom_line() {
    let script = ScriptedRx::shared(b"abcd");
    let mut rx = ScriptedRx::from_shared(script.clone());
    let mut reader = rxline::LineReader::new(&mut rx);

    // First call: 4 bytes buffered, then the FIFO gap -> pending Ok(0).
    let mut large = [0u8; 8];
    assert_eq!(reader.read_line(&mut large), Ok(0));
    assert!(!reader.last_completed(), "gap mid-line is pending");

    // The CR arrives; the caller shrank cap from 8 to 2. Completing a
    // 4-byte line at the CR into a 2-byte buffer is an explicit error.
    feed_script(&script, &[Some(b'\r')]);
    let mut small = [0u8; 2];
    assert_eq!(
        reader.read_line(&mut small),
        Err(rxline::RxLineError::LineTooLong(2)),
        "shrinking cap at CR: LineTooLong(2), never Ok(min) with silent drop"
    );
    assert!(!reader.last_completed());

    // The matching LF of the CRLF pair arrives next. The consumed CR must
    // have set pending_cr so this LF is SWALLOWED — never a phantom
    // completed empty line — and the following "hi" + LF completes normally.
    feed_script(&script, &[Some(b'\n'), Some(b'h'), Some(b'i'), Some(b'\n')]);
    let mut buf = [0u8; 8];
    assert_eq!(
        reader.read_line(&mut buf),
        Ok(2),
        "the LF was swallowed as the second half of the CRLF pair: the next
         real line completes, never a phantom Ok(0) completed empty line"
    );
    assert!(reader.last_completed());
    assert_eq!(&buf[..2], b"hi", "byte-exact: the line after the CRLF pair");
}

// -------------------------------------------------------------------------
// HW-33: allocation accounting for the LineReader internal buffer — the
// mechanism is a #[cfg(test)] counting global_allocator wrapper (this file)
// around the system allocator; snapshots via hw33_allocs() bound alloc/dealloc
// counts per scenario. The only Vec growth under test is rxline's `line`
// buffer: mock sources (ScriptedRx/CountingRx) pre-build their queues before
// the snapshot window and never allocate during read_line. Mock seams only —
// no MMIO.
// -------------------------------------------------------------------------

mod hw33_alloc {
    extern crate std;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// HW-33 test plumbing: a counting **pass-through wrapper** around the
    /// system allocator, installed as the `#[global_allocator]` of the test
    /// binary ONLY (the lib is `no_std`; production builds never see it).
    /// Every alloc/dealloc CALL is counted; memory itself stays glibc's.
    struct CountingAlloc;

    static ALLOCS: AtomicUsize = AtomicUsize::new(0);
    static DEALLOCS: AtomicUsize = AtomicUsize::new(0);

    // Per-thread counting gate. A scenario turns it ON only inside its
    // measured windows, so other harness threads' allocations never enter a
    // delta. glibc malloc internals run on the calling thread and pause with
    // it; scenarios additionally hold `hw33_lock()` (process-wide) across
    // each window so no other HW-33 scenario counts concurrently, and
    // non-HW-33 tests never enable counting on their threads.
    thread_local! {
        static COUNT_ME: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    pub fn set_counting(on: bool) {
        COUNT_ME.with(|c| c.set(on));
    }

    unsafe impl GlobalAlloc for CountingAlloc {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            if COUNT_ME.get() {
                ALLOCS.fetch_add(1, Ordering::Relaxed);
            }
            System.alloc(l)
        }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            if COUNT_ME.get() {
                DEALLOCS.fetch_add(1, Ordering::Relaxed);
            }
            System.dealloc(p, l)
        }
    }

    #[global_allocator]
    static ALLOCATOR: CountingAlloc = CountingAlloc;

    /// Process-wide mutex serializing the allocation-counting scenarios.
    pub fn hw33_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        &LOCK
    }

    /// Snapshot of (allocs, deallocs) for the CALLING thread's counting gate.
    /// Usage contract (followed by every HW-33 scenario): hold `hw33_lock()`,
    /// keep `set_counting(true)` on this thread across the whole window, and
    /// warm caches before enabling counting (`hw33_begin`). Deltas between two
    /// snapshots then count exactly the scenario's own alloc/dealloc calls —
    /// deterministically, under any harness thread schedule.
    pub fn hw33_allocs() -> (usize, usize) {
        (
            ALLOCS.load(Ordering::Relaxed),
            DEALLOCS.load(Ordering::Relaxed),
        )
    }

    /// Start a measured window on this thread: warm the allocator path with
    /// counting OFF (glibc per-thread arena/tcache setup and first-touch
    /// mallocs never enter a delta), then enable counting.
    pub fn hw33_begin() -> CountingGuard {
        warm_caches();
        set_counting(true);
        CountingGuard(())
    }

    /// RAII guard: counting on this thread is OFF again when the scenario
    /// exits (normally or via a failing assertion's unwind).
    pub struct CountingGuard(());

    impl Drop for CountingGuard {
        fn drop(&mut self) {
            set_counting(false);
        }
    }

    fn warm_caches() {
        thread_local! {
            static WARMED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        }
        WARMED.with(|w| {
            if w.get() {
                return;
            }
            set_counting(false);
            let mut ballast: std::vec::Vec<std::vec::Vec<u8>> = std::vec::Vec::new();
            for _ in 0..64 {
                ballast.push(std::vec::Vec::with_capacity(1 << 13));
            }
            for size in [1usize, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024] {
                let v: std::vec::Vec<u8> = std::vec::Vec::with_capacity(size);
                drop(v);
            }
            ballast.clear();
            w.set(true);
        });
    }
}

use self::hw33_alloc::{hw33_allocs, hw33_begin};

/// HW-33 `alloc_repeated_workload_zero_churn`: 1000 lines through ONE reader
/// allocate nothing once warm. The reader's single internal buffer grows to
/// its first line (one alloc, capacity 8 for a <=8-byte line), then every
/// subsequent same-length line reuses it: the 999-line window measures
/// **0 allocations / 0 deallocations**, and all 1000 lines deliver correctly.
#[test]
fn alloc_repeated_workload_zero_churn() {
    let _serial = hw33_alloc::hw33_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let _counting = hw33_begin();
    let script = ScriptedRx::shared(&[]);
    let mut rx = ScriptedRx::from_shared(Rc::clone(&script));
    let mut reader = rxline::LineReader::new(&mut rx);
    let mut buf = [0u8; 16];

    // Warmup: the very first line pays exactly one allocation (the Vec's
    // first block) — nothing else grows in this window.
    feed_bytes(&script, b"a\n");
    let (a0, d0) = hw33_allocs();
    assert_eq!(reader.read_line(&mut buf), Ok(1));
    let (a1, d1) = hw33_allocs();
    assert_eq!(
        a1 - a0,
        1,
        "first buffered line allocates exactly one block"
    );
    assert_eq!(d1 - d0, 0, "no dealloc mid-workload (capacity persists)");

    // Repeated workload: 999 more lines through the SAME reader — zero churn.
    feed_bytes(&script, b"abcdefgh\n");
    for _ in 0..999 {
        feed_bytes(&script, b"abcdefgh\n");
    }
    let (a2, d2) = hw33_allocs();
    for i in 0..999u32 {
        assert_eq!(reader.read_line(&mut buf), Ok(8), "line {i} must complete");
        assert_eq!(&buf[..8], b"abcdefgh");
    }
    let (a3, d3) = hw33_allocs();
    assert_eq!(a3 - a2, 0, "warm repeated workload: ZERO allocations");
    assert_eq!(d3 - d2, 0, "warm repeated workload: ZERO deallocations");
}

/// HW-33B `alloc_growth_bound_pow2_on_reader`: the growth bound is measured
/// on the **ACTUAL LineReader**, not a plain-`Vec` proxy (root advisory: a
/// Vec proxy cannot establish the reader's real capacity). Feeds lines of
/// length `n` — every `n` in 1..=300 — through ONE fresh reader and pins,
/// per line, BOTH the internal buffer's capacity (via the `#[cfg(test)]`-only
/// `LineReader::line_capacity()` accessor) AND the counting-allocator deltas
/// across that line. Measured reader bound: capacity after buffering a line
/// of length `n` is exactly the smallest power of two `>= max(n, 8)` (the
/// 8-byte floor is Vec's first block), never more than 2x the longest line;
/// and reaching 300 costs the reader exactly **7 allocations** across ALL
/// 300 lines (blocks 8,16,32,...,512) — amortized doubling, not per-line.
#[test]
fn alloc_growth_bound_pow2_on_reader() {
    let _serial = hw33_alloc::hw33_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let _counting = hw33_begin();
    let bound = |n: usize| n.max(8).next_power_of_two();

    // ONE fresh reader for the whole ramp; caller buf is a fixed stack
    // array (never heap-allocated by this layer) large enough for the
    // longest line.
    let script = ScriptedRx::shared(&[]);
    let mut rx = ScriptedRx::from_shared(Rc::clone(&script));
    let mut reader = rxline::LineReader::new(&mut rx);
    let mut buf = [0u8; 512];

    // Stage ALL 300 lines (plus LFs) in the script BEFORE the measured
    // window — script-building Vec churn is harness scaffolding, never in
    // a reader window.
    let staged: Vec<u8> = (1..=300usize)
        .flat_map(|len| {
            (0..len)
                .map(|i| b'a' + (i % 26) as u8)
                .chain(core::iter::once(b'\n'))
        })
        .collect();
    feed_bytes(&script, &staged);

    let (a0, _d0) = hw33_allocs();
    for len in 1..=300usize {
        // Measured window: poll the REAL reader until line {len} completes.
        // A single read_line call consumes at most MAX_BYTES_PER_CALL
        // source reads (HW-31), so a >255-byte line spans calls — state is
        // lossless across the budget boundary and the buffer keeps growing.
        let (pa0, _pd0) = hw33_allocs();
        loop {
            match reader.read_line(&mut buf) {
                Ok(n) if reader.last_completed() => {
                    assert_eq!(n, len, "line {len} completes");
                    break;
                }
                Ok(_) => continue, // budget/gap: pending, state preserved
                Err(e) => panic!("line {len} must not error: {e:?}"),
            }
        }
        let (pa1, _pd1) = hw33_allocs();

        // The reader's ACTUAL internal capacity, read off the reader itself.
        let cap = reader.line_capacity();
        assert_eq!(cap, bound(len), "reader capacity after buffering {len}");
        assert!(cap <= 2 * len.max(8), "amortized doubling: cap <= 2x");
        // Growth is a step function: at most one realloc per line.
        assert!(
            pa1 - pa0 <= 1,
            "one line grows the reader buffer by at most one block"
        );
    }
    let (a1, _d1) = hw33_allocs();
    // Blocks 8,16,32,...,512: the READER pays exactly 7 allocations across
    // ALL 300 lines — matching the naive Vec guess's count, but now measured
    // on the actual reader.
    assert_eq!(
        a1 - a0,
        7,
        "reader growth to 300 costs exactly 7 blocks (8..512)"
    );
}

/// HW-33 `alloc_gap_and_budget_allocate_nothing` (scope qualified by
/// HW-33B): the FIFO-gap and HW-31 budget-exhaustion paths allocate nothing
/// **in the measured WARM windows only** — the reader's buffer is already
/// grown (capacity 8 from line "a") and each window moves control/BS bytes
/// or a single byte that never forces growth. This is NOT a blanket claim:
/// cold accumulation of printable bytes into a not-yet-grown buffer DOES
/// allocate growth blocks — pinned by `alloc_cold_gap_accumulation_allocates`
/// (same reader, same gap mechanism, cold window).
#[test]
fn alloc_gap_and_budget_allocate_nothing() {
    let _serial = hw33_alloc::hw33_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let _counting = hw33_begin();
    // Gap path: reader is warm (capacity 8 from line "a"), FIFO drains mid-
    // second-line -> pending Ok(0). No growth, no allocation.
    let script = ScriptedRx::shared(b"a\nb");
    let mut rx = ScriptedRx::from_shared(Rc::clone(&script));
    let mut reader = rxline::LineReader::new(&mut rx);
    let mut buf = [0u8; 16];
    assert_eq!(reader.read_line(&mut buf), Ok(1)); // warm the buffer

    let (a0, d0) = hw33_allocs();
    assert_eq!(reader.read_line(&mut buf), Ok(0)); // 'b' buffered, then gap
    assert!(!reader.last_completed(), "gap return is pending");
    let (a1, d1) = hw33_allocs();
    assert_eq!(a1 - a0, 0, "FIFO-gap path allocates nothing");
    assert_eq!(d1 - d0, 0, "FIFO-gap path deallocates nothing");

    // Budget path: warm reader (cap 8), then a call that consumes the full
    // MAX_BYTES_PER_CALL budget of control bytes — every iteration drops or
    // pops, so the buffer never grows and the window measures zero.
    let filler: Vec<u8> = core::iter::repeat(0x7Fu8)
        .take(rxline::MAX_BYTES_PER_CALL)
        .collect();
    feed_bytes(&script, &filler);
    let (a2, d2) = hw33_allocs();
    assert_eq!(reader.read_line(&mut buf), Ok(0));
    assert!(!reader.last_completed(), "budget exhaustion is pending");
    let (a3, d3) = hw33_allocs();
    assert_eq!(a3 - a2, 0, "budget-exhaustion path allocates nothing");
    assert_eq!(d3 - d2, 0, "budget-exhaustion path deallocates nothing");
}

/// HW-33B `alloc_cold_gap_accumulation_allocates`: the counterexample that
/// qualifies the warm-window claim — a COLD reader (buffer never grown) hit
/// by a FIFO gap mid-line DOES allocate: accumulating printable bytes into
/// the un-grown buffer pays growth blocks. Same reader, same gap mechanism
/// as `alloc_gap_and_budget_allocate_nothing`, cold window: 10 printable
/// bytes then the gap measure exactly **2 allocations** (the capacity-8
/// first block plus one realloc to 16), with the reader's real capacity at
/// 16 afterwards. So "gap paths allocate nothing" is true only of WARM
/// windows; cold printable accumulation allocates growth blocks.
#[test]
fn alloc_cold_gap_accumulation_allocates() {
    let _serial = hw33_alloc::hw33_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let _counting = hw33_begin();
    // Fresh reader: no line has ever entered the buffer (capacity 0).
    let script = ScriptedRx::shared(b"abcdefghij");
    let mut rx = ScriptedRx::from_shared(Rc::clone(&script));
    let mut reader = rxline::LineReader::new(&mut rx);
    let mut buf = [0u8; 16];

    let (a0, _d0) = hw33_allocs();
    assert_eq!(reader.read_line(&mut buf), Ok(0)); // 10 bytes buffered, then gap
    assert!(!reader.last_completed(), "gap return is pending");
    let (a1, _d1) = hw33_allocs();
    // Cold accumulation of 10 printable bytes: first block (cap 8) + one
    // realloc to 16 — growth blocks, NOT zero.
    assert_eq!(
        a1 - a0,
        2,
        "cold gap mid-line allocates growth blocks (8 then 16)"
    );
    assert_eq!(
        reader.line_capacity(),
        16,
        "reader's real capacity after cold accumulation of 10 bytes"
    );
}

/// HW-33 `alloc_line_too_long_keeps_capacity`: LineTooLong DISCARDS the
/// partial line (empties the buffer) but KEEPS its capacity — measured zero
/// allocations for a same-length follow-up line after the error, proving no
/// shrink-and-regrow churn.
#[test]
fn alloc_line_too_long_keeps_capacity() {
    let _serial = hw33_alloc::hw33_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let _counting = hw33_begin();
    // 10-byte line against cap 4: LineTooLong on the 5th storable byte. The
    // buffer already holds 4 bytes (warm from that same line), so even the
    // error path itself allocates nothing beyond what the line build cost.
    let script = ScriptedRx::shared(b"abcdefghij\nxy\n");
    let mut rx = ScriptedRx::from_shared(script);
    let mut reader = rxline::LineReader::new(&mut rx);
    let mut buf = [0u8; 4];

    assert_eq!(
        reader.read_line(&mut buf),
        Err(rxline::RxLineError::LineTooLong(4)),
        "cap 4 < line 10: overflow as documented"
    );

    // The next line (same length class, within the kept capacity) must cost
    // ZERO allocations — capacity survived the discard.
    let (a0, d0) = hw33_allocs();
    assert_eq!(reader.read_line(&mut buf), Ok(2));
    assert_eq!(&buf[..2], b"xy");
    let (a1, d1) = hw33_allocs();
    assert_eq!(
        a1 - a0,
        0,
        "post-LineTooLong reuse allocates nothing: capacity kept"
    );
    assert_eq!(
        d1 - d0,
        0,
        "LineTooLong discards the partial but never shrinks"
    );
}
