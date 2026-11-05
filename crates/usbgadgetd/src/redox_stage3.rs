//! P5 stage 3 — Redox glue (`cfg(target_os = "redox")`,
//! features `dwc3-bringup` + `cdc-acm-shell`): bridge the
//! stage-2 device's CDC-ACM function to the shell byte stream
//! so a host terminal gets the shell prompt.
//!
//! Structure (two threads, all state crossing between them
//! over `std::sync::mpsc`):
//!
//! - The calling thread runs the embassy side: `usb.run()`
//!   joined with the ACM task, which moves packets between
//!   the class and the channels, announces connect/disconnect
//!   and line-coding changes as [`HostEvent`]s.
//! - A bridge thread owns the pure `acm_shell::ShellBridge`
//!   and pumps it at a ~1 ms cadence: host events drive
//!   `set_host_connected` / the line-coding state, the pump
//!   moves the bytes.
//!
//! THE SHELL ENDPOINT — the stage-3 design point. Redox has
//! no pty layer in the base image this targets, and the
//! RAM-boot image's shell already speaks over a byte-stream
//! scheme node; the bridge therefore opens
//! [`SHELL_STREAM_PATH`] (`/scheme/debug`, the console stream
//! the RAM image's shell is attached to). The bridge core is
//! transport-agnostic, so re-pointing this one constant is
//! the whole change if root settles on a different node (a
//! pty scheme, a dedicated console scheme). The path is
//! opened twice — once per direction — because the reader
//! thread owns its handle.
//!
//! Blocking discipline: scheme reads block, so they live on
//! their own thread and reach the bridge as channel messages;
//! the bridge's `ByteStream` impl never blocks. ACM writes
//! are awaited inside the embassy task. Nothing here panics
//! on a closed channel or file: failures degrade to the
//! bridge's `Closed` semantics and a log line.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::mpsc::{channel, Receiver, Sender};

use embassy_futures::join::join;
use embassy_futures::select::{select, Either};
use embassy_usb::class::cdc_acm::CdcAcmClass;
use embassy_usb::driver::Driver;
use embassy_usb::UsbDevice;

use crate::acm_shell::{
    line_coding_to_bytes, ByteStream, LineCodingState, ShellBridgeStd, StreamError,
};

/// Scheme node the shell byte stream is opened on. See the
/// module header: this constant is the design point.
pub const SHELL_STREAM_PATH: &str = "/scheme/debug";

/// Events from the embassy/ACM thread to the bridge thread.
enum HostEvent {
    /// Host opened the port (wait_connection returned).
    Connected,
    /// Host went away (packet error / disconnect).
    Disconnected,
    /// Host set a new line coding (CDC wire bytes).
    LineCoding([u8; 7]),
}

/// Host side of the bridge: channel-backed, never blocks.
struct MpscHost {
    data_rx: Receiver<Vec<u8>>,
    data_tx: Sender<Vec<u8>>,
    pending: VecDeque<u8>,
}

impl ByteStream for MpscHost {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, StreamError> {
        while self.pending.is_empty() {
            match self.data_rx.try_recv() {
                Ok(chunk) => self.pending.extend(chunk),
                Err(_) => break,
            }
        }
        let n = buf.len().min(self.pending.len());
        for slot in &mut buf[..n] {
            *slot = self.pending.pop_front().expect("length checked");
        }
        Ok(n)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize, StreamError> {
        self.data_tx
            .send(buf.to_vec())
            .map_err(|_| StreamError::Closed)?;
        Ok(buf.len())
    }
}

/// Shell side of the bridge: a reader thread turns blocking
/// scheme reads into channel chunks; writes go to a second
/// handle on the calling (bridge) thread.
struct SchemeStream {
    rx: Receiver<Vec<u8>>,
    pending: VecDeque<u8>,
    writer: std::fs::File,
    read_failed: bool,
}

impl SchemeStream {
    fn open(path: &str) -> std::io::Result<Self> {
        let mut reader = std::fs::File::open(path)?;
        let writer = std::fs::OpenOptions::new().write(true).open(path)?;
        let (tx, rx) = channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut buf = [0u8; 256];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,          // EOF: shell stream closed
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            rx,
            pending: VecDeque::new(),
            writer,
            read_failed: false,
        })
    }
}

impl ByteStream for SchemeStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, StreamError> {
        while self.pending.is_empty() {
            match self.rx.try_recv() {
                Ok(chunk) => self.pending.extend(chunk),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.read_failed = true;
                    break;
                }
            }
        }
        if self.pending.is_empty() && self.read_failed {
            return Err(StreamError::Closed);
        }
        let n = buf.len().min(self.pending.len());
        for slot in &mut buf[..n] {
            *slot = self.pending.pop_front().expect("length checked");
        }
        Ok(n)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize, StreamError> {
        match self.writer.write(buf) {
            Ok(n) => {
                let _ = self.writer.flush();
                Ok(n)
            }
            Err(_) => Err(StreamError::Closed),
        }
    }
}

/// The ACM task (embassy thread): packet pump + event source.
async fn acm_task<'d, D: Driver<'d>>(
    mut class: CdcAcmClass<'d, D>,
    data_tx: Sender<Vec<u8>>,
    data_rx: Receiver<Vec<u8>>,
    events: Sender<HostEvent>,
) {
    let mut buf = [0u8; 512];
    let mut last_coding: Option<[u8; 7]> = None;
    'sessions: loop {
        class.wait_connection().await;
        if events.send(HostEvent::Connected).is_err() {
            return;
        }
        'connected: loop {
            // Line coding is polled on every wake: the class
            // exposes no change waker in embassy-usb 0.6, and
            // for a virtual port the value matters only as
            // tracked state.
            let coding = line_coding_to_bytes(&class.line_coding());
            if last_coding != Some(coding) {
                last_coding = Some(coding);
                if events.send(HostEvent::LineCoding(coding)).is_err() {
                    return;
                }
            }
            // A future that yields the next queued shell→host
            // chunk, polling the channel cooperatively.
            let next_out = async {
                loop {
                    match data_rx.try_recv() {
                        Ok(chunk) => break chunk,
                        Err(_) => embassy_futures::yield_now().await,
                    }
                }
            };
            match select(class.read_packet(&mut buf), next_out).await {
                Either::First(Ok(n)) => {
                    if data_tx.send(buf[..n].to_vec()).is_err() {
                        return;
                    }
                }
                Either::First(Err(_)) => {
                    let _ = events.send(HostEvent::Disconnected);
                    continue 'sessions;
                }
                Either::Second(chunk) => {
                    if class.write_packet(&chunk).await.is_err() {
                        let _ = events.send(HostEvent::Disconnected);
                        continue 'sessions;
                    }
                }
            }
        }
    }
}

/// Run the stage-3 session. Never returns in practice
/// (`UsbDevice::run` diverges); called from
/// `redox_stage2::run_bringup` with the built device and the
/// registered ACM class.
pub fn run_session<'d, D: Driver<'d>>(mut usb: UsbDevice<'d, D>, class: CdcAcmClass<'d, D>) {
    let (h2s_tx, h2s_rx) = channel::<Vec<u8>>(); // host -> shell data
    let (s2h_tx, s2h_rx) = channel::<Vec<u8>>(); // shell -> host data
    let (events_tx, events_rx) = channel::<HostEvent>();

    let shell = match SchemeStream::open(SHELL_STREAM_PATH) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "usbgadgetd: STAGE3 ABORT: cannot open shell stream {SHELL_STREAM_PATH}: {e}; \
                 USB device still enumerates, shell bridge down"
            );
            // Keep enumeration alive even with the bridge down.
            embassy_futures::block_on(usb.run());
            return;
        }
    };
    let host = MpscHost {
        data_rx: h2s_rx,
        data_tx: s2h_tx,
        pending: VecDeque::new(),
    };
    let bridge_thread = std::thread::spawn(move || {
        let mut bridge: ShellBridgeStd<MpscHost, SchemeStream> =
            ShellBridgeStd::new(host, shell);
        let mut line = LineCodingState::default();
        eprintln!("usbgadgetd: STAGE3 SHELL BRIDGE UP (host stream pending)");
        loop {
            while let Ok(ev) = events_rx.try_recv() {
                match ev {
                    HostEvent::Connected => {
                        bridge.set_host_connected(true);
                        eprintln!("usbgadgetd: STAGE3 host connected");
                    }
                    HostEvent::Disconnected => {
                        bridge.set_host_connected(false);
                        eprintln!("usbgadgetd: STAGE3 host disconnected");
                    }
                    HostEvent::LineCoding(bytes) => match line.apply(&bytes) {
                        Ok(s) => eprintln!(
                            "usbgadgetd: STAGE3 line coding: {} baud, {} data bits",
                            s.data_rate, s.data_bits
                        ),
                        Err(e) => eprintln!(
                            "usbgadgetd: STAGE3 line coding refused: {e}; keeping {:?}",
                            line.current()
                        ),
                    },
                }
            }
            let report = bridge.pump();
            if report.shell_closed {
                eprintln!("usbgadgetd: STAGE3 SHELL STREAM CLOSED; bridge idling");
            }
            if bridge.shell_failed() {
                // Nothing more the bridge can do; keep the
                // thread alive so events keep draining and
                // the failure stays visible in one place.
                std::thread::sleep(std::time::Duration::from_millis(50));
            } else {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    });

    let acm = acm_task(class, h2s_tx, s2h_rx, events_tx);
    embassy_futures::block_on(join(usb.run(), acm));
    let _ = bridge_thread.join();
}
