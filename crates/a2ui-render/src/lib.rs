//! `a2ui-render` — the A2UI subset renderer (task C6, docs/TASKS-WAVE1.md).
//!
//! A2UI (op6-refs/a2ui @ 409ded2, Apache-2.0) is a declarative JSON format:
//! an agent describes the *intent* of a UI as a flat list of components with
//! id references plus a data model, and each client renders it in its own
//! native style. This crate is the phone's client: one column, black + orange,
//! large touch targets — the same taste as `apps/timeline` (VISION §8).
//!
//! Supported components: `Text` (body/caption and the `h1`–`h5` heading
//! variants), `Button`, `List`, `Progress` (see the note in [`scene`]),
//! `Image` (placeholder box — no network on the phone), `TextField`, plus the
//! containers `Column`, `Row`, `Card`, `Divider`. Anything else renders a
//! visible `unsupported: X` chip and is named in [`scene::Scene::unsupported`]
//! — never silence (VISION §5).
//!
//! The pipeline is split so every interesting half is host-testable:
//!
//! - [`doc`] — the four v0.9 messages, from an object, an array, the gallery
//!   wrapper, or JSONL. Bad messages are counted and named, never fatal.
//! - [`surface`] — component table + data model + typed-in field values.
//! - [`value`] — dynamic values: literals, JSON-Pointer bindings, and the
//!   `formatString` / `formatNumber` client functions.
//! - [`scene`] — layout into positioned [`scene::Item`]s, with hit-testing.
//! - [`view`] — draws a scene onto any `orbclient::Renderer`: the live SDL
//!   window and the off-screen `Surface` used by the golden-image tests run
//!   the identical draw path.
//! - [`events`] — a tap or a typed character becomes a `trace-events` record
//!   (AG-UI `CUSTOM` carrying the A2UI client→server action schema).
//! - [`png`] — minimal PNG writer + decoder, so the goldens are real images.
//!
//! (A worked end-to-end example lives in `tests/golden.rs`: parse a document,
//! lay it out, draw it, compare the pixels.)

pub mod doc;
pub mod events;
pub mod png;
pub mod pointer;
pub mod scene;
pub mod surface;
pub mod value;
pub mod view;
