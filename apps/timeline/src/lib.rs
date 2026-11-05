//! `timeline` — the open-kimono view (task C4, docs/TASKS-WAVE1.md).
//!
//! One screen, black + orange, no clutter (VISION §8): the mic level bar, the
//! live transcript, the cleaned text, the streaming reasoning, every tool
//! call, and the services/power list — all fed by a JSONL stream of
//! `trace-events` records (the C2 crate, already merged on main).
//!
//! The crate is split so the interesting half is host-testable without a
//! window:
//!
//! - [`model`] — the pure event→view state machine. `cargo test` covers it.
//! - [`feed`] — JSONL tailing (file or stdin), bad lines counted, never fatal.
//! - [`view`] — draws a [`model::TimelineModel`] onto anything implementing
//!   `orbclient::Renderer` — the live SDL `Window` or an off-screen `Surface`.
//! - [`png`] — writes a rendered frame out as a PNG (workbench evidence).
//!
//! The `timeline` binary wires them together: `--demo` replays the fixture
//! stream, `--stream <path|->` tails a live journal, `--screenshot <png>`
//! renders the final frame headlessly.

pub mod feed;
pub mod model;
pub mod png;
pub mod view;
