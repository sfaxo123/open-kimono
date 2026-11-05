//! End-to-end proof of the C1 contract: a mini GENI debug-UART driver
//! (uart9 @ 0x00a84000, offsets from `specs/geni-debug-uart.md`) written
//! against `&mut impl Mmio`, run through record → JSONL freeze → replay,
//! strict and lenient.

use mmio_replay::{Compare, MemModel, Mmio, Recorder, Replayer, Trace};

const UART: u64 = 0x00a8_4000;
const FORCE_DEFAULT: u64 = UART + 0x020; // GENI_FORCE_DEFAULT_REG
const SE_GENI_STATUS: u64 = UART + 0x040; // bit 0 M_CMD active, bit 12 S_CMD active
const M_CLK_CFG: u64 = UART + 0x048; // GENI_SER_M_CLK_CFG: SER_CLK_EN + CLK_DIV
const DMA_MODE_EN: u64 = UART + 0x258; // 0 = FIFO mode
const TX_TRANS_CFG: u64 = UART + 0x25c; // UART_CTS_MASK = bit 1
const TX_PACKING_CFG0: u64 = UART + 0x260; // canned 0xf
const TX_PACKING_CFG1: u64 = UART + 0x264; // canned 0
const TX_WORD_LEN: u64 = UART + 0x268; // 8 data bits
const TX_STOP_BIT_LEN: u64 = UART + 0x26c; // 0 = one stop bit
const TX_TRANS_LEN: u64 = UART + 0x270; // bytes in next TX command
const M_CMD0: u64 = UART + 0x600; // opcode [31:27], UART_START_TX = 1
const M_IRQ_STATUS: u64 = UART + 0x610; // bit 0 CMD_DONE
const M_IRQ_CLEAR: u64 = UART + 0x618; // write-1-to-clear
const TX_FIFO: u64 = UART + 0x700; // FIFO write port

const CMD_DONE: u32 = 1 << 0;
const SE_BUSY: u32 = (1 << 0) | (1 << 12);
const UART_START_TX: u32 = 1 << 27;

/// Console init, polled-TX flavour (mirrors the spec's §4 bring-up order).
fn uart_init(m: &mut impl Mmio) {
    m.write_u32(FORCE_DEFAULT, 1);
    let mut spins = 0;
    while m.read_u32(SE_GENI_STATUS) & SE_BUSY != 0 {
        spins += 1;
        assert!(spins < 100, "init watchdog");
    }
    m.write_u32(M_CLK_CFG, (0x20 << 4) | 1); // SER_CLK_EN, div 0x20
    m.write_u32(DMA_MODE_EN, 0);
    m.write_u32(TX_TRANS_CFG, 0x2); // ignore CTS
    m.write_u32(TX_PACKING_CFG0, 0xf);
    m.write_u32(TX_PACKING_CFG1, 0);
    m.write_u32(TX_WORD_LEN, 8);
    m.write_u32(TX_STOP_BIT_LEN, 0);
}

/// Polled TX of up to 4 bytes (single FIFO word, console path).
fn uart_put_short(m: &mut impl Mmio, bytes: &[u8]) {
    assert!(bytes.len() <= 4);
    m.write_u32(TX_TRANS_LEN, bytes.len() as u32);
    let mut word = 0u32;
    for (i, b) in bytes.iter().enumerate() {
        word |= (*b as u32) << (8 * i);
    }
    m.write_u32(TX_FIFO, word);
    m.write_u32(M_CMD0, UART_START_TX | bytes.len() as u32);
    let mut spins = 0;
    while m.read_u32(M_IRQ_STATUS) & CMD_DONE == 0 {
        spins += 1;
        assert!(spins < 100, "tx watchdog");
    }
    m.write_u32(M_IRQ_CLEAR, CMD_DONE);
}

fn run_driver(m: &mut impl Mmio) {
    uart_init(m);
    uart_put_short(m, b"Hi");
}

/// A register model with the state firmware leaves behind: SE idle and a
/// CMD_DONE-ready IRQ status (so the polled loops terminate).
fn firmware_left_state() -> MemModel {
    let mut model = MemModel::new();
    model.load(UART + 0x610, &[0x01, 0x00, 0x00, 0x00]); // M_IRQ_STATUS: CMD_DONE
    model
}

#[test]
fn record_freeze_replay_roundtrip() {
    let mut model = firmware_left_state();
    let mut rec = Recorder::new(&mut model);
    rec.set_delay_us(500); // power-on settle before the first poke
    run_driver(&mut rec);
    let trace = rec.into_trace();

    // 9 init + 5 TX transactions, and the TX command is what the spec says.
    assert_eq!(trace.len(), 14);
    assert_eq!(trace.total_delay_us(), 500);
    let cmd = trace.events.iter().find(|e| e.addr == M_CMD0).unwrap();
    assert_eq!(cmd.value, (UART_START_TX | 2) as u64);

    // Freeze as JSONL, re-parse, replay strictly: must match exactly.
    let frozen = trace.to_jsonl();
    let parsed = Trace::from_jsonl(&frozen).unwrap();
    assert_eq!(parsed, trace);

    let mut rep = Replayer::strict(parsed.clone());
    run_driver(&mut rep);
    let report = rep.finish();
    assert!(report.ok(), "{report}");
    assert_eq!(report.matched, 14);
}

/// Same full sequence as `run_driver`, but the packing config is shifted
/// by a nibble.
fn buggy_packing(m: &mut impl Mmio) {
    m.write_u32(FORCE_DEFAULT, 1);
    while m.read_u32(SE_GENI_STATUS) & SE_BUSY != 0 {}
    m.write_u32(M_CLK_CFG, (0x20 << 4) | 1);
    m.write_u32(DMA_MODE_EN, 0);
    m.write_u32(TX_TRANS_CFG, 0x2);
    m.write_u32(TX_PACKING_CFG0, 0xf0); // bug: 0xf0 instead of 0xf
    m.write_u32(TX_PACKING_CFG1, 0);
    m.write_u32(TX_WORD_LEN, 8);
    m.write_u32(TX_STOP_BIT_LEN, 0);
    uart_put_short(m, b"Hi");
}

/// Pokes a register the recording never touched.
fn buggy_stray(m: &mut impl Mmio) {
    m.write_u32(FORCE_DEFAULT, 1);
    m.write_u32(UART + 0x024, 0xdead); // not in the trace
}

#[test]
fn strict_catches_wrong_value_lenient_only_warns() {
    let mut model = firmware_left_state();
    let mut rec = Recorder::new(&mut model);
    run_driver(&mut rec);
    let trace = rec.into_trace();

    // Buggy driver: same full sequence, but the packing config is shifted
    // by a nibble. After the strict replay fails, unmatched reads fall back
    // to read_default — set it so the buggy driver's TX loop still ends.
    let mut rep = Replayer::strict(trace.clone()).with_read_default(CMD_DONE as u64);
    buggy_packing(&mut rep);
    let r = rep.finish();
    assert!(!r.ok(), "strict must fail on a wrong write value");
    assert_eq!(r.mismatches.len(), 1);
    assert_eq!(r.mismatches[0].got.addr, TX_PACKING_CFG0);

    let mut rep = Replayer::lenient(trace);
    buggy_packing(&mut rep);
    let r = rep.finish();
    assert!(r.ok(), "lenient tolerates value drift: {r}");
    assert_eq!(r.warnings(), 1);
}

#[test]
fn both_modes_fail_on_a_register_that_is_not_in_the_trace() {
    let mut model = firmware_left_state();
    let mut rec = Recorder::new(&mut model);
    run_driver(&mut rec);
    let trace = rec.into_trace();

    // Buggy driver pokes a register the recording never touched.
    for mode in [Compare::Strict, Compare::Lenient] {
        let mut rep = Replayer::new(trace.clone(), mode);
        buggy_stray(&mut rep);
        let r = rep.finish();
        assert!(!r.ok(), "{mode:?} must fail on an unknown register");
        assert!(r.mismatches.iter().any(|m| m.got.addr == UART + 0x024));
    }
}

#[test]
fn lenient_tolerates_extra_recorded_polling_strict_does_not() {
    // A recording that polled TX-done twice; the driver under test polls once.
    let jsonl = "\
{\"op\":\"w\",\"width\":32,\"addr\":\"0xa84270\",\"value\":\"0x1\",\"delay_us\":0}
{\"op\":\"w\",\"width\":32,\"addr\":\"0xa84700\",\"value\":\"0x41\",\"delay_us\":0}
{\"op\":\"w\",\"width\":32,\"addr\":\"0xa84600\",\"value\":\"0x08000001\",\"delay_us\":0}
{\"op\":\"r\",\"width\":32,\"addr\":\"0xa84610\",\"value\":\"0x1\",\"delay_us\":2}
{\"op\":\"r\",\"width\":32,\"addr\":\"0xa84610\",\"value\":\"0x1\",\"delay_us\":2}
{\"op\":\"w\",\"width\":32,\"addr\":\"0xa84618\",\"value\":\"0x1\",\"delay_us\":0}
";
    let trace = Trace::from_jsonl(jsonl).unwrap();

    let mut rep = Replayer::lenient(trace.clone());
    uart_put_short(&mut rep, b"A");
    let r = rep.finish();
    assert!(r.ok(), "lenient: {r}");
    assert_eq!(r.skipped.len(), 1, "the extra recorded poll is a warning");

    let mut rep = Replayer::strict(trace);
    uart_put_short(&mut rep, b"A");
    let r = rep.finish();
    assert!(!r.ok(), "strict: fewer polls than recorded must fail");
}

#[test]
fn recorded_read_values_drive_the_driver_control_flow() {
    // The recording shows the SE busy on the first status read, idle on the
    // second — a faithful replay must make the driver poll exactly twice.
    let jsonl = "\
{\"op\":\"w\",\"width\":32,\"addr\":\"0xa84020\",\"value\":\"0x1\",\"delay_us\":0}
{\"op\":\"r\",\"width\":32,\"addr\":\"0xa84040\",\"value\":\"0x1000\",\"delay_us\":0}
{\"op\":\"r\",\"width\":32,\"addr\":\"0xa84040\",\"value\":\"0x0\",\"delay_us\":4}
{\"op\":\"w\",\"width\":32,\"addr\":\"0xa84048\",\"value\":\"0x201\",\"delay_us\":0}
{\"op\":\"w\",\"width\":32,\"addr\":\"0xa84258\",\"value\":\"0x0\",\"delay_us\":0}
{\"op\":\"w\",\"width\":32,\"addr\":\"0xa8425c\",\"value\":\"0x2\",\"delay_us\":0}
{\"op\":\"w\",\"width\":32,\"addr\":\"0xa84260\",\"value\":\"0xf\",\"delay_us\":0}
{\"op\":\"w\",\"width\":32,\"addr\":\"0xa84264\",\"value\":\"0x0\",\"delay_us\":0}
{\"op\":\"w\",\"width\":32,\"addr\":\"0xa84268\",\"value\":\"0x8\",\"delay_us\":0}
{\"op\":\"w\",\"width\":32,\"addr\":\"0xa8426c\",\"value\":\"0x0\",\"delay_us\":0}
";
    let trace = Trace::from_jsonl(jsonl).unwrap();

    let mut rep = Replayer::strict(trace.clone());
    uart_init(&mut rep);
    let r = rep.finish();
    assert!(
        r.ok(),
        "recorded status values must drive the poll loop: {r}"
    );
    assert_eq!(r.matched, 10);
    assert_eq!(trace.total_delay_us(), 4);
}
