//! `agentd` — the Track A agentic OS layer for the OnePlus 6.
//!
//! One request in, one answer out, and every step in between written to the
//! open-kimono journal (`crates/trace-events`, JSONL). Nothing in this crate
//! invents an event format: the envelope, the `gen_ai` span vocabulary and the
//! `ag_ui` protocol vocabulary all come from `trace-events`.
//!
//! Layout, one responsibility per file:
//!
//! - [`smoke`] — A0: the cross-build proof path.
//! - [`config`] — the out-of-repo runtime config and its key file.
//! - [`journal`] — the live ring plus SSE listeners.
//! - [`http`] — the small HTTP/1.1 reader/writer (server side).
//! - [`model`] — the OpenAI-compatible chat client (client side).
//! - [`phone`] — read-only `/sys` and `/proc` probes.
//! - [`tools`] — what the model may call, and what a result means.
//! - [`kimono`] — the journal writers for runs, spans and tool calls.
//! - [`launch`] — opening apps: the `app_launch` tool engine.
//! - [`session`] — conversation sessions (CV1): turns, entities, pending slots.
//! - [`brainfix`] — BRAINFIX v4: confirmation discipline for inferred destinations.
//! - [`agent`] — the request loop and the brain fallback chain.
//! - [`server`] — routes: `POST /request`, `GET /events`, `GET /health`.

#![forbid(unsafe_code)]

pub mod agent;
pub mod brainfix;
pub mod broker;
pub mod config;
pub mod drain;
pub mod power_policy;
pub mod http;
pub mod journal;
pub mod ir_help;
pub mod kimono;
pub mod launch;
pub mod model;
pub mod modules;
pub mod repair;
pub mod phone;
pub mod power;
pub mod server;
pub mod services;
pub mod session;
pub mod smoke;
pub mod stream;
pub mod tools;
