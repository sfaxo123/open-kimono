//! U-Boot source ledger (INV-UB-01, EXECUTION-PLAN Packet 10).
//!
//! Enumerates the git TREES at two root-frozen pins in a local read-only
//! fork of https://source.denx.de/u-boot/u-boot.git and emits one row per
//! file for every upstream surface at the upstream pin plus every
//! downstream/custom overlay path at the downstream pin. No working tree
//! is consulted and nothing is fetched: enumeration is `git ls-tree -r`
//! against the fork, classification is rule-based over paths only.
//!
//! Like the repo inventory (INV-01), a path that matches no category rule
//! is a failure by design — the run exits nonzero and names the offenders.

pub mod classify;
pub mod git;
pub mod ledger;
pub mod tier;
