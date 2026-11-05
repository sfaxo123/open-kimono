//! QEMU source ledger (INV-QEMU-01, EXECUTION-PLAN §6.3 — mirror of
//! INV-UB-01).
//!
//! Enumerates the git TREE at one root-pinned ref in a local read-only
//! fork of QEMU carrying the OnePlus 6 machine (`qemu-oneplus6`) and
//! emits one row per file, classified and tiered. No working tree is
//! consulted and nothing is fetched: enumeration is `git ls-tree -r`
//! against the fork, classification is rule-based over paths only. The
//! fork is GPL-2.0 external code: ledger rows only — no source is ever
//! copied into this repo.
//!
//! Like the repo inventory (INV-01) and the U-Boot ledger (INV-UB-01),
//! a path that matches no category rule is a failure by design — the
//! run exits nonzero and names the offenders.

pub mod classify;
pub mod git;
pub mod ledger;
pub mod tier;
