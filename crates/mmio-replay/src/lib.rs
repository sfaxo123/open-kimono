//! `mmio-replay` — MMIO trait + recorder/replayer of register traces.
//!
//! Part of the Redox-on-OnePlus-6 "emulator first" rule (docs/PLAN.md,
//! VISION "Emulator first"): the OnePlus 6's peripherals are not modelled by
//! QEMU, so drivers are developed against **register traces replayed from the
//! real phone** (recorded later by the pmOS oracle, `scripts/oracle/`).
//!
//! A driver takes a `&mut impl [`Mmio`]` instead of touching pointers:
//!
//! - production: the real register window;
//! - host test: a [`MemModel`] register model, a [`Recorder`] wrapping one
//!   (to capture a trace), or a [`Replayer`] feeding a recorded trace back to
//!   the driver and comparing every transaction against it.
//!
//! Traces are JSONL — one flat JSON object per transaction:
//!
//! ```text
//! {"op":"w","width":32,"addr":"0xa84010","value":"0x2","delay_us":0}
//! {"op":"r","width":32,"addr":"0xa84018","value":"0x40000000","delay_us":12}
//! ```
//!
//! `op` is `"r"`/`"w"`, `width` is 8/16/32/64, `addr`/`value` are hex
//! strings (decimal numbers are also accepted), `delay_us` is the time since
//! the previous event and is optional.
//!
//! Compare modes on the [`Replayer`]:
//!
//! - [`Compare::Strict`]: every transaction must match the next recorded
//!   event exactly (op, width, address, and written value); the first
//!   deviation fails the replay.
//! - [`Compare::Lenient`]: structure must match — a driver may write values
//!   that differ from the recording (logged as warnings) and may skip
//!   recorded events, but touching a register that is not in the trace fails.
//!
//! ```
//! use mmio_replay::{Compare, MemModel, Mmio, Recorder, Replayer};
//!
//! // Record a reference run against a register model…
//! let mut model = MemModel::new();
//! model.load(0xa84e24, &[0x10, 0x00, 0x00, 0x00]); // firmware-left state
//! let mut rec = Recorder::new(&mut model);
//! rec.write_u32(0xa84010, 1);
//! assert_eq!(rec.read_u32(0xa84e24), 0x10);
//! let trace = rec.into_trace();
//!
//! // …freeze it as JSONL…
//! let text = trace.to_jsonl();
//! let trace = mmio_replay::Trace::from_jsonl(&text).unwrap();
//!
//! // …and replay it strictly against the driver.
//! let mut rep = Replayer::strict(trace);
//! rep.write_u32(0xa84010, 1);
//! assert_eq!(rep.read_u32(0xa84e24), 0x10); // recorded value comes back
//! assert!(rep.finish().ok());
//! ```

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

extern crate alloc;

mod mmio;
mod recorder;
mod replayer;
mod trace;

pub use mmio::{MemModel, Mmio, Op, Width};
pub use recorder::Recorder;
pub use replayer::{Compare, Mismatch, MismatchKind, ObservedAccess, ReplayReport, Replayer};
pub use trace::{Event, ParseError, Trace};
