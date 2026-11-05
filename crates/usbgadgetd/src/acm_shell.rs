//! P5 stage 3 (pure core): CDC-ACM serial shell plumbing.
//!
//! Three pieces, all host-tested, none target-gated:
//!
//! 1. The descriptor plan — the identity the stage-2 device
//!    presents when the ACM function is registered. The values
//!    deliberately match the dwc3-gadget bench device
//!    (`tests/common/device.rs`) so host behaviour observed
//!    against the model carries over: same VID/PID the Mac
//!    already knows, same strings, same EP0 packet size.
//! 2. Line coding — the CDC 7-byte wire struct, validated into
//!    [`ShellLineSettings`]. A shell endpoint has no real UART
//!    behind it, but the settings are tracked honestly: a
//!    host's SET_LINE_CODING is parsed and checked, an invalid
//!    one is refused WITHOUT disturbing the running settings,
//!    and GET_LINE_CODING answers what is actually in force.
//! 3. [`ShellBridge`] — the byte pump between the host ACM
//!    stream and the shell stream, over the [`ByteStream`]
//!    trait so the Redox glue (a scheme file on one side, the
//!    embassy class on the other) is the only target-specific
//!    part. Disconnect policy, pinned by tests: when the host
//!    goes away, pending host→shell bytes are DROPPED (stale
//!    input must never leak into the next session), shell→host
//!    bytes already accepted are RETAINED and delivered first
//!    on reconnect, the shell side is never closed by the
//!    bridge, and shell output produced while no host is
//!    attached waits upstream (the bridge does not read the
//!    shell while disconnected).
//!
//! The Redox-side endpoint choice (which scheme node is "the
//! shell") is the stage-3 design note's open point; this core
//! is transport-agnostic precisely so that decision can change
//! without touching tested logic.

use embassy_usb::class::cdc_acm::{LineCoding, ParityType, StopBits};

// ----------------------------------------------------------------
// Descriptor plan

/// USB vendor id (pid.codes, as used by the bench device).
pub const VID: u16 = 0x1209;
/// USB product id (bench device).
pub const PID: u16 = 0x0d06;
/// Manufacturer string.
pub const MANUFACTURER: &str = "redox-oneplus6";
/// Product string.
pub const PRODUCT: &str = "OnePlus 6 gadget";
/// Serial string.
pub const SERIAL: &str = "K11";
/// EP0 max packet size.
pub const MAX_PACKET_SIZE_0: u8 = 64;
/// Bulk packet size passed to `CdcAcmClass::new` (high-speed).
pub const ACM_MAX_PACKET_SIZE: u16 = 512;

// ----------------------------------------------------------------
// Line coding

/// Stop-bit format of a line coding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopFormat {
    /// 1 stop bit (CDC char-format 0).
    One,
    /// 1.5 stop bits (CDC char-format 1).
    OnePointFive,
    /// 2 stop bits (CDC char-format 2).
    Two,
}

/// Parity format of a line coding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parity {
    /// CDC parity 0.
    None,
    /// CDC parity 1.
    Odd,
    /// CDC parity 2.
    Even,
    /// CDC parity 3.
    Mark,
    /// CDC parity 4.
    Space,
}

/// A validated shell line configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShellLineSettings {
    /// Baud rate (bits/second). Informational for a virtual
    /// port, but a zero rate is meaningless and refused.
    pub data_rate: u32,
    /// Data bits per character, 5..=8 for this shell.
    pub data_bits: u8,
    /// Stop-bit format.
    pub stop: StopFormat,
    /// Parity format.
    pub parity: Parity,
}

impl Default for ShellLineSettings {
    /// 115200 8N1 — the conventional serial-console default.
    fn default() -> Self {
        Self {
            data_rate: 115_200,
            data_bits: 8,
            stop: StopFormat::One,
            parity: Parity::None,
        }
    }
}

/// Why a line-coding struct was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineCodingError {
    /// data_rate was zero.
    ZeroDataRate,
    /// data_bits outside 5..=8 (raw value).
    BadDataBits(u8),
    /// char-format byte outside 0..=2 (raw value).
    BadCharFormat(u8),
    /// parity byte outside 0..=4 (raw value).
    BadParity(u8),
}

impl core::fmt::Display for LineCodingError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LineCodingError::ZeroDataRate => write!(f, "line coding data_rate is zero"),
            LineCodingError::BadDataBits(b) => write!(f, "line coding data_bits {b} not in 5..=8"),
            LineCodingError::BadCharFormat(b) => write!(f, "line coding char_format {b} not in 0..=2"),
            LineCodingError::BadParity(b) => write!(f, "line coding parity {b} not in 0..=4"),
        }
    }
}

impl std::error::Error for LineCodingError {}

/// Serialise settings to the CDC 7-byte line-coding struct
/// (data_rate LE, char_format, parity, data_bits).
pub fn settings_to_bytes(s: &ShellLineSettings) -> [u8; 7] {
    let char_format = match s.stop {
        StopFormat::One => 0,
        StopFormat::OnePointFive => 1,
        StopFormat::Two => 2,
    };
    let parity = match s.parity {
        Parity::None => 0,
        Parity::Odd => 1,
        Parity::Even => 2,
        Parity::Mark => 3,
        Parity::Space => 4,
    };
    let mut b = [0u8; 7];
    b[0..4].copy_from_slice(&s.data_rate.to_le_bytes());
    b[4] = char_format;
    b[5] = parity;
    b[6] = s.data_bits;
    b
}

/// Parse and validate a CDC 7-byte line-coding struct.
pub fn settings_from_bytes(b: &[u8; 7]) -> Result<ShellLineSettings, LineCodingError> {
    let data_rate = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    if data_rate == 0 {
        return Err(LineCodingError::ZeroDataRate);
    }
    let stop = match b[4] {
        0 => StopFormat::One,
        1 => StopFormat::OnePointFive,
        2 => StopFormat::Two,
        other => return Err(LineCodingError::BadCharFormat(other)),
    };
    let parity = match b[5] {
        0 => Parity::None,
        1 => Parity::Odd,
        2 => Parity::Even,
        3 => Parity::Mark,
        4 => Parity::Space,
        other => return Err(LineCodingError::BadParity(other)),
    };
    if !(5..=8).contains(&b[6]) {
        return Err(LineCodingError::BadDataBits(b[6]));
    }
    Ok(ShellLineSettings {
        data_rate,
        data_bits: b[6],
        stop,
        parity,
    })
}

/// Convert the embassy class's parsed line coding into the
/// wire struct (the class exposes accessors, not the bytes).
pub fn line_coding_to_bytes(lc: &LineCoding) -> [u8; 7] {
    let stop = match lc.stop_bits() {
        StopBits::One => StopFormat::One,
        StopBits::OnePointFive => StopFormat::OnePointFive,
        StopBits::Two => StopFormat::Two,
    };
    let parity = match lc.parity_type() {
        ParityType::None => Parity::None,
        ParityType::Odd => Parity::Odd,
        ParityType::Even => Parity::Even,
        ParityType::Mark => Parity::Mark,
        ParityType::Space => Parity::Space,
    };
    settings_to_bytes(&ShellLineSettings {
        data_rate: lc.data_rate(),
        data_bits: lc.data_bits(),
        stop,
        parity,
    })
}

/// Holds the line settings actually in force. A refused
/// SET_LINE_CODING leaves the previous settings untouched.
#[derive(Clone, Copy, Debug, Default)]
pub struct LineCodingState {
    current: ShellLineSettings,
}

impl LineCodingState {
    /// Settings currently in force.
    pub fn current(&self) -> ShellLineSettings {
        self.current
    }

    /// Apply a host SET_LINE_CODING (wire bytes). On success
    /// returns the new settings; on refusal the previous
    /// settings remain in force.
    pub fn apply(&mut self, bytes: &[u8; 7]) -> Result<ShellLineSettings, LineCodingError> {
        let next = settings_from_bytes(bytes)?;
        self.current = next;
        Ok(next)
    }

    /// The wire bytes GET_LINE_CODING must answer with.
    pub fn get_bytes(&self) -> [u8; 7] {
        settings_to_bytes(&self.current)
    }
}

// ----------------------------------------------------------------
// Byte streams and the bridge

/// The one stream failure: the peer is gone (host unplugged,
/// scheme closed). A 0-length Ok is NOT an error — it is the
/// non-blocking "nothing right now" signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamError {
    /// Peer closed / disconnected.
    Closed,
}

/// A non-blocking byte stream endpoint for the bridge.
/// Implementations: the embassy ACM class (via the glue's
/// channel adapter) and the shell scheme file on Redox; fake
/// scripted streams in tests.
pub trait ByteStream {
    /// Read available bytes. `Ok(0)` = would block.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, StreamError>;
    /// Write bytes, possibly fewer than given (backpressure).
    /// `Ok(0)` = would block.
    fn write(&mut self, buf: &[u8]) -> Result<usize, StreamError>;
}

/// Fixed-capacity FIFO byte ring.
#[derive(Clone, Debug)]
struct Ring<const N: usize> {
    buf: [u8; N],
    head: usize,
    len: usize,
}

impl<const N: usize> Ring<N> {
    fn new() -> Self {
        Self {
            buf: [0; N],
            head: 0,
            len: 0,
        }
    }
    fn is_empty(&self) -> bool {
        self.len == 0
    }
    fn space(&self) -> usize {
        N - self.len
    }
    fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }
    /// Append as much of `data` as fits; returns bytes taken.
    fn push(&mut self, data: &[u8]) -> usize {
        let take = data.len().min(self.space());
        for (i, &b) in data[..take].iter().enumerate() {
            self.buf[(self.head + self.len + i) % N] = b;
        }
        self.len += take;
        take
    }
    /// Copy up to `out.len()` bytes out without removing them.
    fn peek(&self, out: &mut [u8]) -> usize {
        let n = out.len().min(self.len);
        for i in 0..n {
            out[i] = self.buf[(self.head + i) % N];
        }
        n
    }
    /// Drop `n` bytes from the front (n <= len).
    fn consume(&mut self, n: usize) {
        debug_assert!(n <= self.len);
        self.head = (self.head + n) % N;
        self.len -= n;
    }
}

/// What one [`ShellBridge::pump`] did, for the daemon log and
/// for tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PumpReport {
    /// Bytes moved host → shell this pump.
    pub host_to_shell: usize,
    /// Bytes moved shell → host this pump.
    pub shell_to_host: usize,
    /// The host stream reported Closed during this pump.
    pub host_closed: bool,
    /// The shell stream reported Closed during this pump
    /// (sticky failure — the bridge keeps the flag in stats;
    /// restarting the shell is the daemon's decision).
    pub shell_closed: bool,
}

/// Cumulative bridge counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BridgeStats {
    /// Total bytes delivered to the shell.
    pub to_shell: u64,
    /// Total bytes delivered to the host.
    pub to_host: u64,
    /// Host disconnects observed.
    pub disconnects: u32,
    /// Host→shell bytes dropped at disconnects.
    pub dropped_at_disconnect: u64,
}

/// The shell byte pump. `N` is the per-direction queue
/// capacity; backpressure is honest — a full queue stops reads
/// from its source, nothing is ever dropped silently while
/// connected.
pub struct ShellBridge<H: ByteStream, S: ByteStream, const N: usize> {
    host: H,
    shell: S,
    h2s: Ring<N>,
    s2h: Ring<N>,
    connected: bool,
    shell_failed: bool,
    stats: BridgeStats,
}

/// The daemon's bridge size: 4 KiB per direction.
pub type ShellBridgeStd<H, S> = ShellBridge<H, S, 4096>;

impl<H: ByteStream, S: ByteStream, const N: usize> ShellBridge<H, S, N> {
    /// A new bridge; starts disconnected (the host must be
    /// announced with [`ShellBridge::set_host_connected`]).
    pub fn new(host: H, shell: S) -> Self {
        Self {
            host,
            shell,
            h2s: Ring::new(),
            s2h: Ring::new(),
            connected: false,
            shell_failed: false,
            stats: BridgeStats::default(),
        }
    }

    /// Whether a host session is currently announced.
    pub fn host_connected(&self) -> bool {
        self.connected
    }

    /// Whether the shell stream has ever reported Closed.
    pub fn shell_failed(&self) -> bool {
        self.shell_failed
    }

    /// Cumulative counters.
    pub fn stats(&self) -> BridgeStats {
        self.stats
    }

    /// Announce a host session start/end. On end: pending
    /// host→shell bytes are dropped (counted); shell→host
    /// bytes already accepted are retained for the next
    /// session.
    pub fn set_host_connected(&mut self, connected: bool) {
        if self.connected && !connected {
            self.stats.dropped_at_disconnect += self.h2s.len as u64;
            self.h2s.clear();
            self.stats.disconnects += 1;
        }
        self.connected = connected;
    }

    /// Swap the host stream (a new session usually means a new
    /// endpoint object); returns the old one.
    pub fn replace_host(&mut self, host: H) -> H {
        core::mem::replace(&mut self.host, host)
    }

    /// Access the shell stream (glue bookkeeping).
    pub fn shell_mut(&mut self) -> &mut S {
        &mut self.shell
    }

    /// Move bytes in both directions until both sides would
    /// block (bounded by a work budget so the caller keeps
    /// control of its loop cadence).
    pub fn pump(&mut self) -> PumpReport {
        let mut report = PumpReport::default();
        if !self.connected {
            return report;
        }
        let mut chunk = [0u8; 256];
        // host -> shell
        for _ in 0..16 {
            let mut progressed = false;
            if self.h2s.space() > 0 {
                let want = self.h2s.space().min(chunk.len());
                match self.host.read(&mut chunk[..want]) {
                    Ok(0) => {}
                    Ok(n) => {
                        let took = self.h2s.push(&chunk[..n]);
                        debug_assert_eq!(took, n, "read was bounded by ring space");
                        progressed = true;
                    }
                    Err(StreamError::Closed) => {
                        report.host_closed = true;
                        self.set_host_connected(false);
                        return report;
                    }
                }
            }
            if !self.h2s.is_empty() {
                let n = self.h2s.peek(&mut chunk);
                match self.shell.write(&chunk[..n]) {
                    Ok(0) => {}
                    Ok(w) => {
                        self.h2s.consume(w);
                        self.stats.to_shell += w as u64;
                        report.host_to_shell += w;
                        progressed = true;
                    }
                    Err(StreamError::Closed) => {
                        report.shell_closed = true;
                        self.shell_failed = true;
                    }
                }
            }
            if !progressed {
                break;
            }
        }
        // shell -> host
        for _ in 0..16 {
            let mut progressed = false;
            if !self.shell_failed && self.s2h.space() > 0 {
                let want = self.s2h.space().min(chunk.len());
                match self.shell.read(&mut chunk[..want]) {
                    Ok(0) => {}
                    Ok(n) => {
                        let took = self.s2h.push(&chunk[..n]);
                        debug_assert_eq!(took, n, "read was bounded by ring space");
                        progressed = true;
                    }
                    Err(StreamError::Closed) => {
                        report.shell_closed = true;
                        self.shell_failed = true;
                    }
                }
            }
            if !self.s2h.is_empty() {
                let n = self.s2h.peek(&mut chunk);
                match self.host.write(&chunk[..n]) {
                    Ok(0) => {}
                    Ok(w) => {
                        self.s2h.consume(w);
                        self.stats.to_host += w as u64;
                        report.shell_to_host += w;
                        progressed = true;
                    }
                    Err(StreamError::Closed) => {
                        report.host_closed = true;
                        self.set_host_connected(false);
                        return report;
                    }
                }
            }
            if !progressed {
                break;
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    // ---- line coding ----

    #[test]
    fn default_line_coding_is_115200_8n1_on_the_wire() {
        let s = ShellLineSettings::default();
        assert_eq!(settings_to_bytes(&s), [0x00, 0xc2, 0x01, 0x00, 0, 0, 8]);
        assert_eq!(settings_from_bytes(&settings_to_bytes(&s)), Ok(s));
        let mut state = LineCodingState::default();
        assert_eq!(state.get_bytes(), [0x00, 0xc2, 0x01, 0x00, 0, 0, 8]);
        // A new rate applies and is then what GET reports.
        let b = settings_to_bytes(&ShellLineSettings {
            data_rate: 9600,
            data_bits: 7,
            stop: StopFormat::Two,
            parity: Parity::Even,
        });
        assert_eq!(state.apply(&b).unwrap().data_rate, 9600);
        assert_eq!(state.get_bytes(), b);
    }

    #[test]
    fn invalid_line_codings_refuse_and_keep_previous() {
        let mut state = LineCodingState::default();
        let before = state.get_bytes();
        let mut zero_rate = before;
        zero_rate[0..4].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(
            state.apply(&zero_rate),
            Err(LineCodingError::ZeroDataRate)
        );
        let mut bad_bits = before;
        bad_bits[6] = 16;
        assert_eq!(
            state.apply(&bad_bits),
            Err(LineCodingError::BadDataBits(16))
        );
        let mut bad_fmt = before;
        bad_fmt[4] = 3;
        assert_eq!(
            state.apply(&bad_fmt),
            Err(LineCodingError::BadCharFormat(3))
        );
        let mut bad_parity = before;
        bad_parity[5] = 5;
        assert_eq!(
            state.apply(&bad_parity),
            Err(LineCodingError::BadParity(5))
        );
        assert_eq!(state.get_bytes(), before, "refusals never disturb state");
    }

    #[test]
    fn embassy_line_coding_converts() {
        // embassy's own Default (8000 baud 8N1) must convert
        // through the accessor path without loss.
        let lc = LineCoding::default();
        let bytes = line_coding_to_bytes(&lc);
        assert_eq!(bytes, [0x40, 0x1f, 0x00, 0x00, 0, 0, 8]);
        let parsed = settings_from_bytes(&bytes).unwrap();
        assert_eq!(parsed.data_rate, 8000);
        assert_eq!(parsed.data_bits, 8);
    }

    // ---- fake streams ----

    #[derive(Default)]
    struct FakeStream {
        /// Bytes the peer will produce, in order.
        incoming: VecDeque<u8>,
        /// Bytes written by the bridge, in order.
        outgoing: Vec<u8>,
        /// Cap on bytes accepted per write call (backpressure).
        write_cap: Option<usize>,
        /// Reads/writes report Closed.
        closed: bool,
        /// Reads report Closed (peer vanished mid-session).
        read_closed: bool,
    }

    impl ByteStream for FakeStream {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize, StreamError> {
            if self.closed || self.read_closed {
                return Err(StreamError::Closed);
            }
            let n = buf.len().min(self.incoming.len());
            for slot in &mut buf[..n] {
                *slot = self.incoming.pop_front().unwrap();
            }
            Ok(n)
        }
        fn write(&mut self, buf: &[u8]) -> Result<usize, StreamError> {
            if self.closed {
                return Err(StreamError::Closed);
            }
            let n = self.write_cap.map_or(buf.len(), |c| c.min(buf.len()));
            self.outgoing.extend_from_slice(&buf[..n]);
            Ok(n)
        }
    }

    /// A scripted shell: emits a prompt at start; answers a
    /// line `echo X\n` with `X\n$ `; anything else with
    /// `? \n$ ` (never used by the transcript test).
    struct FakeShell {
        stream: FakeStream,
        line: Vec<u8>,
    }

    impl FakeShell {
        fn new() -> Self {
            let mut stream = FakeStream::default();
            stream.incoming.extend(b"$ ".iter());
            Self {
                stream,
                line: Vec::new(),
            }
        }
    }

    impl ByteStream for FakeShell {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize, StreamError> {
            self.stream.read(buf)
        }
        fn write(&mut self, buf: &[u8]) -> Result<usize, StreamError> {
            let n = self.stream.write(buf)?;
            for &b in &self.stream.outgoing[self.stream.outgoing.len() - n..] {
                if b == b'\n' {
                    let line = core::mem::take(&mut self.line);
                    if let Some(rest) = line.strip_prefix(b"echo ".as_slice()) {
                        self.stream.incoming.extend(rest.iter());
                        self.stream.incoming.push_back(b'\n');
                    } else {
                        self.stream.incoming.extend(b"? ".iter());
                    }
                    self.stream.incoming.extend(b"$ ".iter());
                } else {
                    self.line.push(b);
                }
            }
            Ok(n)
        }
    }

    fn pump_until_idle<H: ByteStream, S: ByteStream, const N: usize>(
        bridge: &mut ShellBridge<H, S, N>,
    ) {
        for _ in 0..64 {
            let r = bridge.pump();
            if r.host_to_shell == 0 && r.shell_to_host == 0 {
                return;
            }
        }
        panic!("bridge never went idle");
    }

    #[test]
    fn transcript_echo_round_trip_is_byte_exact() {
        let host = FakeStream {
            incoming: b"echo hi\n".iter().copied().collect(),
            ..Default::default()
        };
        let mut bridge = ShellBridge::<_, _, 256>::new(host, FakeShell::new());
        bridge.set_host_connected(true);
        pump_until_idle(&mut bridge);
        // The shell received exactly the typed line...
        assert_eq!(bridge.shell_mut().stream.outgoing, b"echo hi\n");
        // ...and the host received prompt, answer, prompt.
        assert_eq!(bridge.host.outgoing, b"$ hi\n$ ");
        assert_eq!(bridge.stats().to_shell, 8);
        assert_eq!(bridge.stats().to_host, 7);
    }

    #[test]
    fn backpressure_loses_nothing_in_either_direction() {
        // Host accepts only 3 bytes per write; shell emits 100.
        let mut shell = FakeStream::default();
        shell.incoming.extend((0..100u8).map(|i| i ^ 0x5a));
        let host = FakeStream {
            write_cap: Some(3),
            ..Default::default()
        };
        let mut bridge = ShellBridge::<_, _, 64>::new(host, shell);
        bridge.set_host_connected(true);
        pump_until_idle(&mut bridge);
        let expected: Vec<u8> = (0..100u8).map(|i| i ^ 0x5a).collect();
        assert_eq!(bridge.host.outgoing, expected);

        // And the other way: shell accepts 2 bytes per write.
        let host = FakeStream {
            incoming: b"0123456789".iter().copied().collect(),
            ..Default::default()
        };
        let shell = FakeStream {
            write_cap: Some(2),
            ..Default::default()
        };
        let mut bridge = ShellBridge::<_, _, 64>::new(host, shell);
        bridge.set_host_connected(true);
        pump_until_idle(&mut bridge);
        assert_eq!(bridge.shell.outgoing, b"0123456789");
    }

    #[test]
    fn full_queue_pauses_source_reads_without_loss() {
        // Shell emits 200 bytes; host accepts nothing (write
        // returns 0 = would block forever). The bridge must
        // stop reading the shell once its queue is full.
        let mut shell = FakeStream::default();
        shell.incoming.extend(std::iter::repeat_n(b'x', 200));
        let host = FakeStream {
            write_cap: Some(0),
            ..Default::default()
        };
        let mut bridge = ShellBridge::<_, _, 64>::new(host, shell);
        bridge.set_host_connected(true);
        bridge.pump();
        let remaining = bridge.shell.incoming.len();
        assert!(
            remaining >= 200 - 64,
            "bridge read past its queue capacity: {remaining} left"
        );
        assert_eq!(bridge.stats().to_host, 0);
    }

    #[test]
    fn disconnect_drops_stale_input_and_keeps_shell_alive() {
        // Host typed bytes but the shell end is capped to 0 for
        // now, so input sits in the bridge queue; then the host
        // vanishes (read reports Closed).
        let host = FakeStream {
            incoming: b"rm -rf /\n".iter().copied().collect(),
            read_closed: false,
            ..Default::default()
        };
        let shell = FakeStream {
            write_cap: Some(0),
            ..Default::default()
        };
        let mut bridge = ShellBridge::<_, _, 256>::new(host, shell);
        bridge.set_host_connected(true);
        bridge.pump(); // input queued in the bridge, shell capped
        bridge.host.read_closed = true;
        let r = bridge.pump();
        assert!(r.host_closed);
        assert!(!bridge.host_connected());
        assert!(!bridge.shell_failed(), "shell side must survive");
        assert_eq!(bridge.stats().disconnects, 1);
        assert_eq!(bridge.stats().dropped_at_disconnect, 9);
        assert!(bridge.shell.outgoing.is_empty(), "stale input never delivered");

        // A new session starts clean: nothing stale is flushed
        // to the shell, and the shell's waiting output flows.
        let mut bridge = bridge;
        bridge.shell_mut().write_cap = None;
        bridge.shell_mut().incoming.extend(b"rebooted\n$ ".iter());
        let new_host = FakeStream::default();
        let _old = bridge.replace_host(new_host);
        bridge.set_host_connected(true);
        pump_until_idle(&mut bridge);
        assert!(bridge.shell.outgoing.is_empty());
        assert_eq!(bridge.host.outgoing, b"rebooted\n$ ");
    }

    #[test]
    fn disconnected_bridge_does_not_touch_either_stream() {
        let mut shell = FakeStream::default();
        shell.incoming.extend(b"early\n".iter());
        let host = FakeStream {
            incoming: b"typed\n".iter().copied().collect(),
            ..Default::default()
        };
        let mut bridge = ShellBridge::<_, _, 256>::new(host, shell);
        // Never connected: pump is a no-op.
        let r = bridge.pump();
        assert_eq!(r, PumpReport::default());
        assert_eq!(bridge.shell.incoming.len(), 6);
        assert_eq!(bridge.host.incoming.len(), 6);
    }
}
