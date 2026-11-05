//! Repository inventory core (INV-01, EXECUTION-PLAN Packet 9).
//!
//! Enumerates every git-tracked file of the repository it runs in,
//! classifies each file by path/extension/content-sniff rules, and builds
//! a deterministic ledger (`workbench/ledger/repo-inventory.json`). A file
//! that matches no rule is a failure by design: the ledger's whole
//! purpose is to be a complete map of the repository, so the run that
//! leaves even one tracked file unclassified has failed.

pub mod classify;
pub mod ledger;
pub mod scan;
pub mod sha256;
