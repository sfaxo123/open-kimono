//! fdt-diff — deterministic, bounds-checked DTB (Flattened Device Tree) comparator.
//!
//! HW-07 (Lane H): HW-06's capture plan will produce a byte-exact fdt blob from the
//! phone (`dd if=/sys/firmware/fdt`) after the first boot. G3 closes when that blob is
//! proven equal (or structurally equal) to the DTB embedded in full31
//! (offset 3,453,288, 155,913 B, sha 48518e1d…9383). This crate judges that evidence
//! mechanically.
//!
//! Design rules (same posture as the input_record codec):
//! - Every offset/size read from the blob is bounds-checked before use. A malformed
//!   blob yields an explicit `FdtError`, never a panic, never an unbounded allocation.
//! - `compare` tries exact byte equality first (no hashing), then a structural diff
//!   over the subtree walk: nodes/props present in one side only, value mismatches
//!   with node path + hex offsets, and node NAME differences reported as
//!   `NameMismatch` (G1 lesson: live ABL writes `chosen/framebuffer-0`, the package
//!   DTB writes `chosen/framebuffer` — same body, different spelling).
//!
//! `no_std`-friendly: the library core uses only `Vec`/`String` from `alloc`; the
//! demo/tests run on host with std.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::borrow::Cow;
use alloc::string::{String, ToString};
use alloc::{vec, vec::Vec};

// ---------------------------------------------------------------------------
// Constants (FDT spec, v17 layout)
// ---------------------------------------------------------------------------

/// Big-endian magic at offset 0 of every DTB: d00dfeed.
pub const FDT_MAGIC: u32 = 0xd00d_feed;
/// Struct-block token: begin a node (name follows, NUL-terminated, 4-aligned).
pub const FDT_BEGIN_NODE: u32 = 0x0000_0001;
/// Struct-block token: end the current node.
pub const FDT_END_NODE: u32 = 0x0000_0002;
/// Struct-block token: a property (len, nameoff, then len bytes of data).
pub const FDT_PROP: u32 = 0x0000_0003;
/// Struct-block token: end of the struct block.
pub const FDT_END: u32 = 0x0000_0009;

/// FDT v17 header: 10 big-endian u32 fields = 40 bytes
/// (magic, totalsize, off_dt_struct, off_dt_strings, off_rsv_map, version,
/// last_comp_version, boot_cpuid_phys, size_dt_struct, size_dt_strings).
/// NOTE: the last two fields follow the layout this repo's fixtures and
/// DtbBuilder emit (verified byte-for-byte against the live phone DTBs).
pub const FDT_HEADER_LEN: usize = 40;
/// Smallest plausible totalsize: header + FDT_END + root begin/end = 44 B.
/// Anything below this cannot be a walkable tree.
pub const FDT_MIN_SIZE: usize = 44;
/// Hard sanity ceiling on totalsize. The full31 embedded DTB is 155,913 B; the
/// largest real phone DTs are ~125 KB. 16 MiB is >100x headroom and still far
/// below anything that could exhaust a boot-time parser.
pub const FDT_MAX_SIZE: usize = 16 * 1024 * 1024;
/// Ceiling on a single property value length (bounded by totalsize anyway, but
/// stated explicitly so the check reads as intent, matching codec style).
pub const FDT_MAX_PROP_LEN: usize = FDT_MAX_SIZE;
/// Ceiling on nesting depth; real DTs stay < 30. 64 rejects runaway BEGIN_NODE.
/// HW-09 renames the former `FDT_MAX_DEPTH` to `MAX_NODE_DEPTH` (no in-repo
/// consumer referenced the old name; the walk check and its error are unchanged).
pub const MAX_NODE_DEPTH: usize = 64;
/// HW-09: ceiling on the number of DTB candidates `extract_dtbs` may return.
/// The full31 image carries a handful of DTBs; 4096 is orders of magnitude
/// above any real container while capping the `Vec<DtbCandidate>` growth a
/// crafted magic-studded container could otherwise force. On reaching it the
/// scan STOPS (documented, deterministic): no panic, no unbounded Vec.
pub const MAX_CANDIDATES: usize = 4096;
/// HW-09: ceiling on the number of structural `Diff`s collected by `compare`.
/// On reaching it, collection stops early and `DiffReport::Different` carries
/// `truncated: true` so consumers can tell "few diffs" from "stopped counting".
pub const MAX_DIFFS: usize = 10_000;
/// Ceiling on a single node-name length (bytes before NUL).
pub const FDT_MAX_NAME_LEN: usize = 256;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Explicit parse failures. Every one is a *decision*, not a panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FdtError {
    /// Buffer shorter than the fixed 40-byte header.
    TooShortForHeader,
    /// First u32 != 0xd00dfeed.
    BadMagic(u32),
    /// totalsize < FDT_MIN_SIZE (absurdly small).
    TotalsizeTooSmall(u32),
    /// totalsize > FDT_MAX_SIZE (absurdly large / overflow attempt).
    TotalsizeTooLarge(u32),
    /// totalsize exceeds the buffer we were handed (truncation).
    TotalsizeExceedsBuffer { totalsize: u32, len: usize },
    /// A header offset field (off_dt_struct / off_dt_strings) is not
    /// 4-aligned, is < header end, or exceeds totalsize.
    BadOffset { field: &'static str, value: u32 },
    /// off_dt_strings >= off_dt_struct (blocks swapped or overlapping).
    BlockOrder,
    /// Struct block contains no FDT_END token.
    NoEndToken,
    /// A token or string reference ran past the struct/strings block bounds.
    OutOfBounds {
        at: usize,
        need: usize,
        limit: usize,
    },
    /// BEGIN_NODE name has no NUL before the block end (or exceeds name cap).
    BadNodeName { at: usize },
    /// A property's name offset runs past the strings block.
    BadStringOffset { str_off: u32 },
    /// BEGIN_NODE/END_NODE pairing broken (extra END_NODE, or tree not closed).
    UnbalancedNodes,
    /// Nesting exceeded `MAX_NODE_DEPTH`.
    NodeDepthExceeded,
    /// Back-compat alias for `NodeDepthExceeded` (the HW-07 name). Kept so
    /// external callers compiling against the old variant keep working; the
    /// walk emits `NodeDepthExceeded`.
    TooDeep,
    /// Struct block did not start with BEGIN_NODE of the root.
    NoRootNode,
    /// Header `version` (offset 20) is not 17. This parser implements the
    /// v17 layout only: v16 would require an extra `last_check` u32 and v18+
    /// adds further header fields — both are out of scope, so reject loudly
    /// rather than mis-parse the fixed 40-byte header.
    UnsupportedVersion(u32),
    /// `size_dt_struct` (offset 36) or `size_dt_strings` (offset 32) is 0.
    /// A zero-sized declared block cannot contain a walkable tree or any
    /// name strings; accepting it would let the walk read past the block.
    EmptyDeclaredBlock { field: &'static str },
    /// `off_dt_* + size_dt_*` does not fit within totalsize: the declared
    /// block runs past the end of the blob.
    BlockOutOfBounds {
        field: &'static str,
        off: u32,
        size: u32,
        totalsize: u32,
    },
    /// Reservation map (off_rsv_map at header offset 16) is malformed. Per
    /// the libfdt oracle (scripts/dtc/libfdt/fdt.h: `struct fdt_reserve_entry
    /// { address; size; }`), the map is a 16-byte-entry (u64 address, u64
    /// size) array TERMINATED by an all-zero entry — it is NOT sized by any
    /// header field. This error fires when: off_rsv_map is nonzero but <
    /// FDT_HEADER_LEN (the map points into the header — libfdt's check_off_
    /// requires off >= hdrsize); the entry array runs to totalsize without
    /// ever reaching its zero terminator; or an entry's address+size
    /// overflows u64.
    BadReservationMap { off: u32, totalsize: u32 },
    /// Header `last_comp_version` (offset 24) exceeds `version` (offset 20).
    /// libfdt fdt_check_header(): `fdt_version < fdt_last_comp_version` is
    /// -FDT_ERR_BADVERSION. (Offset 24 is last_comp_version, NOT a
    /// reservation-map size — HW-07B mis-read it as one; HW-07D corrects
    /// that.)
    BadLastCompVersion {
        last_comp_version: u32,
        version: u32,
    },
    /// Trailing garbage after FDT_END is tolerated, but a struct block that
    /// starts past totalsize is not (defensive; kept distinct for tests).
    StructPastEnd { off_dt_struct: u32 },
    /// HW-09B: internal control-flow signal only — an emission budget was
    /// exhausted and the caller's sink asked the walk to stop. It is caught
    /// by `Fdt::subtree_bounded` and NEVER escapes through the public API;
    /// it denotes a clean early stop, not a malformed blob.
    WalkAborted,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

#[inline]
fn be_u32(buf: &[u8], at: usize) -> Option<u32> {
    let w = buf.get(at..at + 4)?;
    Some(u32::from_be_bytes([w[0], w[1], w[2], w[3]]))
}

/// A parsed, walkable view of one DTB. Holds borrowed slices — no copying.
#[derive(Debug)]
pub struct Fdt<'a> {
    /// The full validated blob (totalsize-bounded view available via fields below).
    pub blob: &'a [u8],
    /// totalsize as recorded in the header (validated <= blob.len()).
    pub totalsize: usize,
    /// Header `last_comp_version` (offset 24), validated <= version (17).
    /// Recorded per the libfdt oracle: offset 24 is a version field, not a
    /// reservation-map size (HW-07D).
    pub last_comp_version: u32,
    /// struct block slice [off_dt_struct, off_dt_strings).
    structure: &'a [u8],
    /// strings block slice [off_dt_strings, totalsize).
    strings: &'a [u8],
    off_dt_struct: usize,
}

impl<'a> Fdt<'a> {
    /// Strictly validate and index a DTB blob. Bounds-checked end to end.
    pub fn parse(blob: &'a [u8]) -> Result<Self, FdtError> {
        if blob.len() < FDT_HEADER_LEN {
            return Err(FdtError::TooShortForHeader);
        }
        let magic = be_u32(blob, 0).unwrap();
        if magic != FDT_MAGIC {
            return Err(FdtError::BadMagic(magic));
        }
        let totalsize = be_u32(blob, 4).unwrap();
        if (totalsize as usize) < FDT_MIN_SIZE {
            return Err(FdtError::TotalsizeTooSmall(totalsize));
        }
        if (totalsize as usize) > FDT_MAX_SIZE {
            return Err(FdtError::TotalsizeTooLarge(totalsize));
        }
        if (totalsize as usize) > blob.len() {
            return Err(FdtError::TotalsizeExceedsBuffer {
                totalsize,
                len: blob.len(),
            });
        }
        let off_dt_struct = be_u32(blob, 8).unwrap();
        let off_dt_strings = be_u32(blob, 12).unwrap();
        for (field, v) in [
            ("off_dt_struct", off_dt_struct),
            ("off_dt_strings", off_dt_strings),
        ] {
            if v % 4 != 0 || (v as usize) < FDT_HEADER_LEN || (v as usize) >= totalsize as usize {
                return Err(FdtError::BadOffset { field, value: v });
            }
        }
        if off_dt_strings <= off_dt_struct {
            return Err(FdtError::BlockOrder);
        }
        // Strict v17: accept ONLY version 17 (offset 20). v16 would require
        // the extra `last_check` u32 and v18+ adds header fields — parsing
        // them with the 40-byte v17 layout would mis-read the blob, so
        // reject loudly instead.
        let version = be_u32(blob, 20).unwrap();
        if version != 17 {
            return Err(FdtError::UnsupportedVersion(version));
        }
        // Header offset 24 is `last_comp_version` (libfdt fdt.h
        // fdt_header), NOT a reservation-map size. HW-07B mis-read it as
        // `size_rsv_map` and validated it as one — false strictness, now
        // removed. Per libfdt fdt_check_header, last_comp_version must be
        // <= version (we already pin version == 17 above).
        let last_comp_version = be_u32(blob, 24).unwrap();
        if last_comp_version > version {
            return Err(FdtError::BadLastCompVersion {
                last_comp_version,
                version,
            });
        }
        // Reservation map (off_rsv_map at offset 16). Per the oracle, the
        // map is a 16-byte-entry array of (be u64 address, be u64 size)
        // pairs terminated by an all-zero entry — no header field sizes it.
        // Rules (libfdt fdt.c check_off_, adapted to this strict parser):
        // - off_rsv_map == 0 is NOT a legal map location: the map must sit
        //   at or past the end of the fixed 40-byte header (libfdt fdt.c
        //   check_off_: `off >= hdrsize`). A zero offset points at the
        //   header magic — the root probe (off16=0) must be rejected.
        //   "No map" is expressed by a lone 16-byte terminator at a valid
        //   offset (e.g. 40 in the live fixture), which the scan below
        //   accepts.
        // - off_rsv_map must be 4-aligned and >= FDT_HEADER_LEN: a map
        //   pointing into the fixed 40-byte header is invalid.
        // - The entry array is scanned from off_rsv_map in 16-byte steps
        //   and MUST reach its terminator (address == 0 && size == 0)
        //   before running past totalsize; each entry's address+size must
        //   not overflow u64.
        let off_rsv_map = be_u32(blob, 16).unwrap();
        {
            if off_rsv_map % 4 != 0 || (off_rsv_map as usize) < FDT_HEADER_LEN {
                return Err(FdtError::BadReservationMap {
                    off: off_rsv_map,
                    totalsize,
                });
            }
            let mut p = off_rsv_map as usize;
            let mut terminated = false;
            while p + 16 <= totalsize as usize {
                let hi = be_u32(blob, p).unwrap() as u64;
                let lo = be_u32(blob, p + 4).unwrap() as u64;
                let addr = (hi << 32) | lo;
                let hi = be_u32(blob, p + 8).unwrap() as u64;
                let lo = be_u32(blob, p + 12).unwrap() as u64;
                let size = (hi << 32) | lo;
                if addr == 0 && size == 0 {
                    terminated = true;
                    break;
                }
                addr.checked_add(size).ok_or(FdtError::BadReservationMap {
                    off: off_rsv_map,
                    totalsize,
                })?;
                p += 16;
            }
            if !terminated {
                return Err(FdtError::BadReservationMap {
                    off: off_rsv_map,
                    totalsize,
                });
            }
        }
        // Declared block sizes. In this repo's v17 header layout the last
        // two fields are size_dt_strings at offset 32 and size_dt_struct at
        // offset 36 (verified byte-for-byte against the live fixture DTBs
        // and matching DtbBuilder::finish). Both must be > 0 and off+size
        // must fit within totalsize. The structure/strings slices below are
        // bounded by the DECLARED sizes, never by totalsize, so the walk
        // can never read past a block.
        let size_dt_strings = be_u32(blob, 32).unwrap();
        let size_dt_struct = be_u32(blob, 36).unwrap();
        for (field, off, size) in [
            ("size_dt_struct", off_dt_struct, size_dt_struct),
            ("size_dt_strings", off_dt_strings, size_dt_strings),
        ] {
            if size == 0 {
                return Err(FdtError::EmptyDeclaredBlock { field });
            }
            let fits = off
                .checked_add(size)
                .filter(|end| (*end as u64) <= totalsize as u64)
                .is_some();
            if !fits {
                return Err(FdtError::BlockOutOfBounds {
                    field,
                    off,
                    size,
                    totalsize,
                });
            }
        }
        // Borrow slices bounded by the declared block sizes (not totalsize,
        // not blob.len()) so nothing outside a declared block is ever walked.
        let ts = totalsize as usize;
        let structure = blob
            .get(off_dt_struct as usize..(off_dt_struct as usize + size_dt_struct as usize))
            .ok_or(FdtError::StructPastEnd { off_dt_struct })?;
        let strings = blob
            .get(off_dt_strings as usize..(off_dt_strings as usize + size_dt_strings as usize))
            .ok_or(FdtError::BadOffset {
                field: "off_dt_strings",
                value: off_dt_strings,
            })?;
        let fdt = Fdt {
            blob,
            totalsize: ts,
            last_comp_version,
            structure,
            strings,
            off_dt_struct: off_dt_struct as usize,
        };
        // Walk once to prove the struct block is well-formed (tokens in bounds,
        // names terminated, nodes balanced, an FDT_END present).
        fdt.walk(|_| Ok(()))?;
        Ok(fdt)
    }

    /// The raw bytes of the struct block.
    pub fn struct_block(&self) -> &[u8] {
        self.structure
    }

    /// Absolute blob offset where the struct block begins.
    pub fn struct_offset(&self) -> usize {
        self.off_dt_struct
    }

    /// Resolve a strings-block offset to a borrowed str (NUL-terminated there).
    fn string_at(&self, str_off: u32) -> Result<&'a str, FdtError> {
        let start = str_off as usize;
        if start >= self.strings.len() {
            return Err(FdtError::BadStringOffset { str_off });
        }
        let rest = &self.strings[start..];
        let end = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or(FdtError::BadStringOffset { str_off })?;
        core::str::from_utf8(&rest[..end]).map_err(|_| FdtError::BadStringOffset { str_off })
    }

    /// Walk the struct block generically. `sink` receives each event with
    /// absolute blob offsets. All token reads are bounds-checked; the walk
    /// stops at the first FDT_END.
    pub fn walk<S>(&self, mut sink: S) -> Result<(), FdtError>
    where
        S: FnMut(WalkEvent<'a>) -> Result<(), FdtError>,
    {
        let s = self.structure;
        // Struct block must open with BEGIN_NODE whose name is the empty
        // string (root): token at 0, NUL byte at offset 4. `s` may be shorter
        // than 8 bytes; be_u32 handles that.
        if s.len() < 8 || be_u32(s, 0) != Some(FDT_BEGIN_NODE) || s[4] != 0 {
            return Err(FdtError::NoRootNode);
        }
        let mut p = 0usize; // index into struct block
        let mut depth: usize = 0;
        let mut path: Vec<&'a str> = Vec::new();
        let mut saw_end = false;

        while p + 4 <= s.len() {
            let tok = be_u32(s, p).unwrap();
            match tok {
                FDT_BEGIN_NODE => {
                    let name_at = p + 4;
                    // NUL-terminated name within the struct block.
                    let rest = &s[name_at..];
                    let nul = rest
                        .iter()
                        .position(|&b| b == 0)
                        .ok_or(FdtError::BadNodeName {
                            at: self.off_dt_struct + name_at,
                        })?;
                    if nul > FDT_MAX_NAME_LEN {
                        return Err(FdtError::BadNodeName {
                            at: self.off_dt_struct + name_at,
                        });
                    }
                    let name =
                        core::str::from_utf8(&rest[..nul]).map_err(|_| FdtError::BadNodeName {
                            at: self.off_dt_struct + name_at,
                        })?;
                    // 4-aligned advance past name + NUL.
                    let adv = nul + 1;
                    let next = name_at + adv + (4 - adv % 4) % 4;
                    if next > s.len() {
                        return Err(FdtError::OutOfBounds {
                            at: self.off_dt_struct + p,
                            need: next - p,
                            limit: self.totalsize,
                        });
                    }
                    depth += 1;
                    // HW-09: hard cap on walk nesting. `depth` counts the
                    // root, so the deepest legal chain is MAX_NODE_DEPTH
                    // nodes below the root (depth == MAX_NODE_DEPTH + 1).
                    // Past that: `NodeDepthExceeded`, never a panic and
                    // never unbounded `path`/recursion growth.
                    if depth > MAX_NODE_DEPTH + 1 {
                        return Err(FdtError::NodeDepthExceeded);
                    }
                    path.push(name);
                    sink(WalkEvent::BeginNode {
                        node_path: join_path(&path),
                        at: self.off_dt_struct + p,
                    })?;
                    p = next;
                }
                FDT_END_NODE => {
                    if depth == 0 || path.is_empty() {
                        return Err(FdtError::UnbalancedNodes);
                    }
                    depth -= 1;
                    path.pop();
                    sink(WalkEvent::EndNode {
                        at: self.off_dt_struct + p,
                    })?;
                    p += 4;
                }
                FDT_PROP => {
                    // token(4) len(4) nameoff(4) data(len, padded to 4)
                    let len = be_u32(s, p + 4).ok_or(FdtError::OutOfBounds {
                        at: self.off_dt_struct + p,
                        need: 8,
                        limit: self.totalsize,
                    })? as usize;
                    let nameoff = be_u32(s, p + 8).ok_or(FdtError::OutOfBounds {
                        at: self.off_dt_struct + p,
                        need: 8,
                        limit: self.totalsize,
                    })?;
                    if len > FDT_MAX_PROP_LEN {
                        return Err(FdtError::OutOfBounds {
                            at: self.off_dt_struct + p,
                            need: len,
                            limit: self.totalsize,
                        });
                    }
                    let data_at = p + 12;
                    if data_at + len > s.len() {
                        return Err(FdtError::OutOfBounds {
                            at: self.off_dt_struct + p,
                            need: data_at + len - p,
                            limit: self.totalsize,
                        });
                    }
                    let adv = len + (4 - len % 4) % 4;
                    if data_at + adv > s.len() {
                        return Err(FdtError::OutOfBounds {
                            at: self.off_dt_struct + p,
                            need: data_at + adv - p,
                            limit: self.totalsize,
                        });
                    }
                    let name = self.string_at(nameoff)?;
                    let value = &s[data_at..data_at + len];
                    sink(WalkEvent::Prop {
                        node_path: join_path(&path),
                        name,
                        value,
                        at: self.off_dt_struct + p,
                        value_at: self.off_dt_struct + data_at,
                    })?;
                    p = data_at + adv;
                }
                FDT_END => {
                    saw_end = true;
                    sink(WalkEvent::End {
                        at: self.off_dt_struct + p,
                    })?;
                    break;
                }
                other => {
                    // Unknown token: strict rejection (v17 has no others in the
                    // struct block).
                    let _ = other;
                    return Err(FdtError::OutOfBounds {
                        at: self.off_dt_struct + p,
                        need: 4,
                        limit: self.totalsize,
                    });
                }
            }
        }
        if !saw_end {
            return Err(FdtError::NoEndToken);
        }
        if depth != 0 {
            return Err(FdtError::UnbalancedNodes);
        }
        Ok(())
    }

    /// Collect the subtree rooted at `subtree` (e.g. "/chosen",
    /// "/reserved-memory") as a sorted Vec of (node_path, prop_name, bytes).
    /// The subtree node itself contributes props with its own path; children
    /// appear as `subtree/child`. Returns `Ok(vec![])` when the subtree is
    /// absent (that is a diff fact, not a parse error).
    /// Absolute node paths of every node in the tree (root renders "/").
    pub fn node_paths(&self) -> Result<Vec<String>, FdtError> {
        let mut out: Vec<String> = Vec::new();
        self.walk(|ev| {
            if let WalkEvent::BeginNode { node_path, .. } = ev {
                out.push(node_path);
            }
            Ok(())
        })?;
        out.sort();
        out.dedup();
        Ok(out)
    }

    pub fn subtree(&self, subtree: &str) -> Result<Vec<PropEntry<'a>>, FdtError> {
        // Uncapped public behavior: the budget is `usize::MAX`, so the walk
        // never aborts early and truncation is never set (HW-09B: bounded
        // collection lives in `subtree_bounded`; HW-09C keeps that uncapped
        // semantics exactly — `compare` never yields Incomplete).
        Ok(self.subtree_bounded(subtree, usize::MAX)?.0)
    }

    /// HW-09B: `subtree` with an emission budget. Prop entries are collected
    /// only while the walk stays inside the budget window, and once the
    /// budget is spent the walk STOPS descending — no further node/prop
    /// events are emitted or compared. That bounds WORK and PEAK
    /// ALLOCATION, not just visible length. Returns
    /// `(sorted entries, truncated)`; `truncated` is true when the budget
    /// was exhausted (entries are a deterministic walk-order prefix, sorted
    /// as `subtree` sorts them).
    ///
    /// HW-09C accounting — the budget is TOTAL ENTRIES, not nodes: every
    /// prop entry pushed into `out` AND every child-node visit (BeginNode)
    /// charges one unit against `2 * limit`, and the walk aborts at the
    /// first event that would exceed it. The old node-count-only budget
    /// was a phantom: one fat root with 100 props collected all 100 entries
    /// under cap 4, so the O(limit) intermediate-allocation claim was false
    /// (root probe: a 12-child tree under cap 2 could no longer see the
    /// differing tail — hence `DiffReport::Incomplete`, not equality).
    /// A single-node tree (the root-prop probe) is capped at 2*limit prop
    /// entries plus its own root begin; blindness past the boundary event
    /// is signalled by `truncated`.
    ///
    /// The budget is `2 * limit`, deliberately: the diff pass may reach up
    /// to `limit` emissions, and each prop comparison consults the entries
    /// of both sides. A budget of `limit` alone could blind the pass to
    /// diffs sitting in an entry list that stopped early (the root-prop
    /// probe: 100 differing root props with limit 4 must emit 4, not 1).
    /// `2 * limit` keeps every collection Vec bounded by twice the diff
    /// budget, so peak allocation stays O(limit) while the pass always
    /// sees enough entries to spend its full emission budget.
    fn subtree_bounded(
        &self,
        subtree: &str,
        limit: usize,
    ) -> Result<(Vec<PropEntry<'a>>, bool), FdtError> {
        let budget = limit.saturating_mul(2);
        // Cap the Vec's growth to the budget so even a walk that never
        // aborts (uncapped `compare`: budget == usize::MAX) retains no
        // more than `budget` slots.
        let mut out: Vec<PropEntry<'a>> = Vec::with_capacity(core::cmp::min(budget, 16));
        let mut truncated = false;
        // HW-09C entry budget: one unit per prop entry pushed and one unit
        // per BeginNode (child-node visit). The walk aborts at the first
        // event that would take spent past `budget`, so `out` never grows
        // past `budget` entries and no node is ever half-collected — the
        // collected entries are a walk-order prefix, and blindness past
        // the boundary is signalled by `truncated`.
        let mut spent = 0usize;
        // Normalize the requested subtree by segments so "/chosen" matches the
        // walked path "//chosen" (root has an empty name segment).
        let ss: Vec<&str> = subtree.split('/').filter(|s| !s.is_empty()).collect();
        let r = self.walk(|ev| {
            if let WalkEvent::BeginNode { .. } = ev {
                // Every node visit charges the budget (HW-09C): nodes are
                // work and their bodies are potential entries.
                spent += 1;
                if spent > budget {
                    truncated = true;
                    return Err(FdtError::WalkAborted);
                }
            }
            if let WalkEvent::Prop {
                node_path,
                name,
                value,
                at,
                value_at,
            } = ev
            {
                let ns: Vec<&str> = node_path.split('/').filter(|s| !s.is_empty()).collect();
                if ns.len() >= ss.len() && (ss.is_empty() || ns[..ss.len()] == ss[..]) {
                    // The prop entry charges the budget at the push site:
                    // the first entry past `budget` aborts the walk before
                    // anything is appended past the boundary.
                    spent += 1;
                    if spent > budget {
                        truncated = true;
                        return Err(FdtError::WalkAborted);
                    }
                    out.push(PropEntry {
                        node_path,
                        name,
                        value,
                        at,
                        value_at,
                    });
                }
            }
            Ok(())
        });
        match r {
            Ok(()) => {}
            Err(FdtError::WalkAborted) => {}
            Err(e) => return Err(e),
        }
        out.sort_by(|a, b| {
            a.node_path
                .cmp(&b.node_path)
                .then_with(|| a.name.cmp(b.name))
        });
        Ok((out, truncated))
    }

    /// HW-09B: `node_paths` with a node budget. At most `2 * limit` nodes
    /// are visited; the walk stops descending at the first END_NODE past
    /// the budget and returns the paths collected so far (plus the flag —
    /// callers here only need the paths). Paths are scoped to `subtree`.
    fn nodes_bounded(&self, subtree: &str, limit: usize) -> Result<Vec<String>, FdtError> {
        let budget = limit.saturating_mul(2);
        let mut out: Vec<String> = Vec::new();
        let mut nodes_seen = 0usize;
        let ss: Vec<&str> = subtree.split('/').filter(|s| !s.is_empty()).collect();
        let r = self.walk(|ev| {
            if let WalkEvent::BeginNode { node_path, .. } = ev {
                nodes_seen += 1;
                let ns: Vec<&str> = node_path.split('/').filter(|s| !s.is_empty()).collect();
                if ns.len() >= ss.len() && (ss.is_empty() || ns[..ss.len()] == ss[..]) {
                    out.push(node_path);
                }
            } else if let WalkEvent::EndNode { .. } = ev {
                if nodes_seen >= budget {
                    return Err(FdtError::WalkAborted);
                }
            }
            Ok(())
        });
        match r {
            Ok(()) => {}
            Err(FdtError::WalkAborted) => {}
            Err(e) => return Err(e),
        }
        Ok(out)
    }
}

fn join_path(path: &[&str]) -> String {
    // The root BEGIN_NODE has an empty name; children must render as
    // "/chosen", not "//chosen".
    let segs: Vec<&str> = path.iter().copied().filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return String::from("/");
    }
    let mut s = String::new();
    for seg in segs {
        s.push('/');
        s.push_str(seg);
    }
    s
}

/// Events emitted by `Fdt::walk`. Paths are absolute ("/", "/chosen", …).
pub enum WalkEvent<'a> {
    BeginNode {
        node_path: String,
        at: usize,
    },
    EndNode {
        at: usize,
    },
    Prop {
        node_path: String,
        name: &'a str,
        value: &'a [u8],
        at: usize,
        value_at: usize,
    },
    End {
        at: usize,
    },
}

/// One property occurrence inside a subtree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropEntry<'a> {
    pub node_path: String,
    pub name: &'a str,
    pub value: &'a [u8],
    /// Absolute blob offset of the PROP token.
    pub at: usize,
    /// Absolute blob offset of the value bytes.
    pub value_at: usize,
}

// ---------------------------------------------------------------------------
// Diff
// ---------------------------------------------------------------------------

/// One structural difference between two blobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diff {
    /// Node path present only in `a` (first blob).
    MissingInB {
        path: String,
    },
    /// Node path present only in `b`.
    MissingInA {
        path: String,
    },
    /// Property present in one side only.
    PropMissingInB {
        path: String,
        name: String,
    },
    PropMissingInA {
        path: String,
        name: String,
    },
    /// Same path + name, different bytes. Offsets are absolute blob offsets
    /// of the value bytes on each side.
    ValueMismatch {
        path: String,
        name: String,
        a_off: usize,
        b_off: usize,
        a_len: usize,
        b_len: usize,
    },
    /// A node exists under different spellings on the two sides but its body
    /// (props + values) matches — the G1 framebuffer/framebuffer-0 lesson.
    /// `a_path`/`b_path` are the two spellings; `parent` is the shared parent.
    NameMismatch {
        parent: String,
        a_name: String,
        b_name: String,
    },
}

/// Verdict of comparing two blobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffReport {
    /// Byte-for-byte identical inputs (checked first, no hashing).
    Equal,
    /// Bytes differ AND the comparison ran to completion over every
    /// subtree of interest with no structural difference found. Only ever
    /// issued for a COMPLETE walk — never for a budget-truncated pass
    /// (HW-09C: that case is `Incomplete`).
    StructurallyEqual { byte_diff_at: usize },
    /// HW-09C: the budget was exhausted (walk truncation in `subtree_bounded`/
    /// `nodes_bounded`, or emission truncation) BEFORE any diff was emitted.
    /// The comparison is INCOMPLETE — unexamined content remains below the
    /// budget window, so equality is NOT provable and this variant, never
    /// `StructurallyEqual`/`Equal`, is the honest verdict. `byte_diff_at` is
    /// the first differing byte offset when known. Additive variant: match
    /// with a catch-all arm for forward compatibility.
    Incomplete { byte_diff_at: Option<usize> },
    /// Bytes differ and at least one structural difference was found.
    /// `truncated` (HW-09, widened by HW-09D) is true when the comparison
    /// did not examine everything it could have: diff collection stopped
    /// early at `MAX_DIFFS`, OR a side's walk was budget-truncated before
    /// its tail was examined (`subtree_bounded`/`nodes_bounded` blindness).
    /// A partial report must self-declare — it never implies completeness.
    /// The reported `diffs` are then a deterministic prefix, not the
    /// complete set. Additive field: match with `..` for forward
    /// compatibility.
    Different {
        diffs: Vec<Diff>,
        byte_diff_at: Option<usize>,
        truncated: bool,
    },
}

impl DiffReport {
    pub fn is_equal(&self) -> bool {
        matches!(self, DiffReport::Equal)
    }
    pub fn verdict(&self) -> Cow<'static, str> {
        match self {
            DiffReport::Equal => Cow::Borrowed("EQUAL (byte-exact)"),
            DiffReport::StructurallyEqual { byte_diff_at } => Cow::Owned(alloc::format!(
                "STRUCTURALLY EQUAL (first byte diff at {:#x})",
                byte_diff_at
            )),
            DiffReport::Incomplete { byte_diff_at } => match byte_diff_at {
                Some(at) => Cow::Owned(alloc::format!(
                    "INCOMPLETE (budget exhausted, no diffs seen; first byte diff at {:#x})",
                    at
                )),
                None => Cow::Borrowed("INCOMPLETE (budget exhausted, no diffs seen)"),
            },
            DiffReport::Different {
                diffs,
                truncated,
                byte_diff_at: _,
            } => {
                let t = if *truncated { " TRUNCATED" } else { "" };
                Cow::Owned(alloc::format!(
                    "DIFFERENT ({} structural diffs){}",
                    diffs.len(),
                    t
                ))
            }
        }
    }
}

/// First index at which two equal-length-or-not buffers differ.
fn first_byte_diff(a: &[u8], b: &[u8]) -> Option<usize> {
    let n = core::cmp::min(a.len(), b.len());
    for i in 0..n {
        if a[i] != b[i] {
            return Some(i);
        }
    }
    if a.len() != b.len() {
        Some(n)
    } else {
        None
    }
}

/// Compare two DTB blobs. `subtrees` lists the subtree roots to diff
/// (e.g. `["/chosen", "/reserved-memory"]`). Both blobs must parse; a parse
/// failure is returned as `Err` with the offending side (0 = a, 1 = b).
/// Diff collection is capped at `MAX_DIFFS` (HW-09): on reaching it the
/// collection stops early and `Different { truncated: true }` is reported.
pub fn compare(a: &[u8], b: &[u8], subtrees: &[&str]) -> Result<DiffReport, (usize, FdtError)> {
    compare_with_limits(a, b, subtrees, MAX_DIFFS)
}

/// Test-visible cap wrapper (HW-09): same collection as `compare`, with the
/// diff limit passed explicitly. Production code must call `compare`; this
/// exists so truncation is testable without crafting > MAX_DIFFS real diffs.
pub fn compare_capped(
    a: &[u8],
    b: &[u8],
    subtrees: &[&str],
    max_diffs: usize,
) -> Result<DiffReport, (usize, FdtError)> {
    compare_with_limits(a, b, subtrees, max_diffs)
}

/// Internal, test-visible form of `compare`: the diff cap is a parameter so
/// truncation can be exercised without crafting > MAX_DIFFS real diffs.
/// The public API always passes `MAX_DIFFS`.
fn compare_with_limits(
    a: &[u8],
    b: &[u8],
    subtrees: &[&str],
    max_diffs: usize,
) -> Result<DiffReport, (usize, FdtError)> {
    // Both blobs must parse BEFORE any verdict is issued — including the
    // byte-equality shortcut. Otherwise an invalid input pair (e.g. two empty
    // buffers) would short-circuit to Equal and be reported as a valid
    // byte-exact DTB match.
    let fa = Fdt::parse(a).map_err(|e| (0, e))?;
    let fb = Fdt::parse(b).map_err(|e| (1, e))?;
    if a == b {
        return Ok(DiffReport::Equal);
    }
    let byte_diff_at = first_byte_diff(a, b);

    // HW-09B: the budget is threaded INTO collection, not applied after it.
    // `diff_bounded` stops the walk and the diffing the moment `limit` diffs
    // exist, so `diffs` never grows past `limit` entries and its retained
    // capacity never exceeds 2*limit (the probe bar).
    // HW-09C: a budget-truncated pass with ZERO emitted diffs is an
    // INCOMPLETE comparison, never `StructurallyEqual`. Claiming structural
    // equality after walking only a prefix of the tree is false equality —
    // the unexamined tail may differ (root probe 2: two 12-child trees
    // differing only in the LAST child under cap 2). The byte-equality
    // shortcut above still issues `Equal` when the blobs are identical.
    // (diffs, walk_blind, emission_truncated): the flags come out of
    // `diff_bounded` SEPARATELY (HW-09D) so a non-empty partial report can
    // carry walk blindness without touching the exact emission-prefix flag.
    let (diffs, walk_blind, emission_truncated) = diff_bounded(&fa, &fb, subtrees, max_diffs);
    if diffs.is_empty() {
        if walk_blind || emission_truncated {
            Ok(DiffReport::Incomplete { byte_diff_at })
        } else {
            Ok(DiffReport::StructurallyEqual {
                byte_diff_at: byte_diff_at.unwrap_or(0),
            })
        }
    } else {
        // HW-09D: a NON-EMPTY partial report must self-declare its blindness.
        // `truncated` here is walk_blind OR emission_truncated: the walk was
        // budget-stopped before examining everything (root probe: root prop
        // differs AND the LAST of 12 children differs deeper down, under
        // cap 4 — the 1 root diff alone must not imply a complete walk), or
        // collection stopped at the emission cap. The exact-limit emission
        // prefix contract is untouched: `diffs` is still a deterministic
        // prefix of at most `limit`, and uncapped compare (budget ==
        // usize::MAX) can set neither flag, so it never reports truncated.
        Ok(DiffReport::Different {
            diffs,
            byte_diff_at,
            truncated: walk_blind || emission_truncated,
        })
    }
}

/// HW-09B: budgeted diff collection. The emission budget (`limit`) is
/// threaded INTO the collection recursion: `subtree_bounded` stops walking
/// descending once its entry budget (2 * limit) is spent, and
/// `diff_entries`/`diff_props` stop emitting and return as soon as
/// `out.len() == limit` (no further property or node comparisons run — that
/// is the WORK bound). Peak allocation is bounded: the result Vec starts at
/// `with_capacity(min(limit, 16))`, never grows past `limit` entries, and
/// doubling growth from a start of at most `limit` lands on a power of two
/// in `(limit, 2*limit]` — always under the probe's capacity <= 2*limit
/// bar. Design note: a `&mut dyn FnMut(Diff) -> bool` sink per push was the
/// brief's alternative; a stop-check at each emission site (count vs limit)
/// is equivalent, cheaper, and keeps the existing sorted pairing passes
/// intact.
fn diff_bounded<'x>(
    fa: &Fdt<'x>,
    fb: &Fdt<'x>,
    subtrees: &[&str],
    limit: usize,
) -> (Vec<Diff>, bool, bool) {
    let mut diffs: Vec<Diff> = Vec::with_capacity(core::cmp::min(limit, 16));
    let mut truncated = false;
    // HW-09C: true when any side's walk stopped early at the entry budget.
    let mut walk_blind = false;
    for sub in subtrees {
        if truncated {
            break;
        }
        // Bounded walks: each side collects at most 2*limit TOTAL entries
        // (prop pushes + node visits) and stops at the boundary event
        // (see subtree_bounded).
        let (ea, ta) = match fa.subtree_bounded(sub, limit) {
            Ok(v) => v,
            Err(_) => return (diffs, walk_blind, truncated),
        };
        let (eb, tb) = match fb.subtree_bounded(sub, limit) {
            Ok(v) => v,
            Err(_) => return (diffs, walk_blind, truncated),
        };
        // HW-09C: walk blindness is tracked SEPARATELY from emission
        // truncation. `trunc` (the per-subtree diff-pass flag) stays set
        // only alongside a real emission at the budget — that keeps the
        // emission prefix exact and stops further subtrees once emissions
        // hit the cap. The walk flags (`ta`/`tb`) feed `walk_blind` below:
        // a budget stop with zero diffs is an INCOMPLETE pass, never a
        // claim of equality over an unexamined tail (root probe 2).
        let mut trunc = false;
        // HW-09C: `truncated` now reflects walk blindness ALONE — a budget
        // stop is budget exhaustion even when the diff pass finds nothing,
        // so compare_with_limits reports Incomplete instead of claiming
        // equality over an unexamined tail (the false-equality root probe:
        // 12-child trees differing only in the LAST child under cap 2 must
        // NOT report equal). A diff-free blob that was walked COMPLETELY
        // still reports StructurallyEqual.
        diff_entries(sub, &ea, &eb, fa, fb, &mut diffs, limit, &mut trunc);
        if ta || tb {
            // Walk budget spent on at least one side: content below the
            // window was never examined. If no diff came out of what WAS
            // seen, this is reported as Incomplete (never StructurallyEqual).
            walk_blind = true;
        }
        if trunc {
            // Emission budget spent: stop the whole comparison. No further
            // subtree is walked or diffed.
            truncated = true;
            break;
        }
    }
    if walk_blind && diffs.is_empty() {
        truncated = true;
    }
    // HW-09D: the two blindness flags leave separately. `truncated` (the
    // emission-prefix flag) keeps its exact HW-09B meaning — set only
    // alongside a real emission at the cap — while `walk_blind` records
    // that a side's walk stopped at the entry budget. compare_with_limits
    // ORs them into Different.truncated (HW-09D) and keeps the old OR for
    // the zero-diff Incomplete decision.
    (diffs, walk_blind, truncated)
}

/// Diff two sorted prop-entry lists for one requested subtree. Node-level
/// facts (missing nodes, name mismatches) are derived from the walked node
/// paths scoped under `sub`.
///
/// HW-09B: the emission budget is threaded through here. `limit` caps how
/// many diffs may be pushed, and every emission site checks the running
/// count first: once `out.len() == limit` the pass returns immediately —
/// no further property or node comparisons run (the WORK bound) — and
/// `truncated` is set. `out` is the caller's budgeted Vec (created with
/// `with_capacity`, never grown past `limit`). Node paths come from the
/// parsed blobs' walks, computed lazily so a budget spent early (e.g. at
/// the root props) never pays for a full node-path collection.
fn diff_entries<'a>(
    sub: &str,
    a: &[PropEntry<'a>],
    b: &[PropEntry<'a>],
    fa: &Fdt<'_>,
    fb: &Fdt<'_>,
    out: &mut Vec<Diff>,
    limit: usize,
    truncated: &mut bool,
) {
    // Restrict the node universe to the subtree roots actually being diffed.
    // `sub` is the exact requested subtree root (e.g. "/chosen"); deriving
    // scope from prop paths would both over-scope (a full-tree "/" request
    // leaking into sibling-subtree calls) and blind the diff to children
    // that carry no properties.
    let norm: Vec<&str> = sub.split('/').filter(|s| !s.is_empty()).collect();
    let in_scope = |p: &str| -> bool {
        // The root node "/" is in scope for every requested subtree (it is
        // the ancestor of everything); its own properties must participate
        // in the property diff (HW-07E: excluding "/" hid root-level
        // value mismatches and produced false StructurallyEqual verdicts).
        if p == "/" {
            return true;
        }
        let ps: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
        ps.len() >= norm.len() && (norm.is_empty() || ps[..norm.len()] == norm[..])
    };

    // The node universe comes from the WALK (nodes_a/nodes_b, i.e. every
    // BeginNode path), NOT from property paths: a propertyless child node
    // must still participate in MissingInA/B. Prop entries add no paths the
    // walk did not already emit.
    // The root "/" participates in the PROPERTY diff (its own props are
    // diffed below via the explicit pairing), but it is never a node-level
    // MissingInA/B candidate — every subtree scope contains the root on
    // both sides, and the walk guarantees exactly one root per blob.
    // HW-09B: the node universe comes from BOUNDED WALKS, not a full
    // `node_paths()` collection. A walk that spends its node budget stops
    // descending — later nodes contribute no paths and are invisible to
    // MissingInA/B (blindness is signalled by `truncated`, never faked as
    // a miss). Nodes with no props still appear: the budget counts NODE
    // events, not prop entries.
    let nodes_a: Vec<String> = match fa.nodes_bounded(sub, limit) {
        Ok(v) => v,
        Err(_) => return,
    };
    let nodes_b: Vec<String> = match fb.nodes_bounded(sub, limit) {
        Ok(v) => v,
        Err(_) => return,
    };
    let mut paths_a: Vec<&str> = Vec::new();
    for p in nodes_a.iter().map(|s| s.as_str()) {
        if p != "/" && in_scope(p) {
            paths_a.push(p);
        }
    }
    let mut paths_b: Vec<&str> = Vec::new();
    for p in nodes_b.iter().map(|s| s.as_str()) {
        if p != "/" && in_scope(p) {
            paths_b.push(p);
        }
    }
    paths_a.sort_unstable();
    paths_a.dedup();
    paths_b.sort_unstable();
    paths_b.dedup();

    // Root property diff: when "/" is in scope, the root node's own props
    // participate (HW-07E: excluding them hid root-level value mismatches
    // and produced false StructurallyEqual verdicts). The root is always
    // present on both sides (parse guarantees a root), so it pairs exactly.
    if in_scope("/") {
        diff_props(a, b, "/", "/", out, limit, truncated);
        if out.len() >= limit || *truncated {
            *truncated = true;
            return;
        }
    }

    // NameMismatch pairing: for sibling node paths under the same parent that
    // differ only in name, if their prop bodies match exactly, report
    // NameMismatch instead of Missing+Missing.
    let mut consumed_a: Vec<bool> = vec![false; paths_a.len()];
    let mut consumed_b: Vec<bool> = vec![false; paths_b.len()];

    // Pass 1: NameMismatch when a same-parent sibling pair has IDENTICAL
    // bodies (the G1 framebuffer/framebuffer-0 lesson: names differ, values match).
    'nm1: for (i, pa) in paths_a.iter().enumerate() {
        if *pa == "/" || consumed_a[i] || paths_b.contains(pa) {
            continue;
        }
        for (j, pb) in paths_b.iter().enumerate() {
            if *pb == "/" || consumed_b[j] || pa == pb {
                continue;
            }
            if parents(pa) == parents(pb)
                && same_depth(pa, pb)
                && sibling_bodies_equal(a, b, pa, pb)
            {
                out.push(Diff::NameMismatch {
                    parent: parents(pa).to_string(),
                    a_name: last_seg(pa).to_string(),
                    b_name: last_seg(pb).to_string(),
                });
                consumed_a[i] = true;
                consumed_b[j] = true;
                if out.len() > limit {
                    *truncated = true;
                    break 'nm1;
                }
                break;
            }
        }
    }
    if *truncated {
        return;
    }

    // Pass 2: pair remaining same-parent siblings (bodies may differ) so their
    // props diff at value level instead of whole nodes going missing.
    'nm2: for (i, pa) in paths_a.iter().enumerate() {
        if *pa == "/" || consumed_a[i] || paths_b.contains(pa) {
            continue;
        }
        for (j, pb) in paths_b.iter().enumerate() {
            if *pb == "/" || consumed_b[j] || pa == pb {
                continue;
            }
            if parents(pa) == parents(pb) && same_depth(pa, pb) {
                out.push(Diff::NameMismatch {
                    parent: parents(pa).to_string(),
                    a_name: last_seg(pa).to_string(),
                    b_name: last_seg(pb).to_string(),
                });
                consumed_a[i] = true;
                consumed_b[j] = true;
                if out.len() > limit {
                    *truncated = true;
                    break 'nm2;
                }
                break;
            }
        }
    }
    if *truncated {
        return;
    }

    // Property-level diff: every a-path pairs with its exact b twin (or, if
    // that twin was claimed by a NameMismatch, with the claiming sibling),
    // then their props are diffed.
    for pa in paths_a.iter() {
        if *pa == "/" {
            continue;
        }
        match paired_path(pa, &paths_b, &consumed_b) {
            Some(pb) => {
                diff_props(a, b, pa, pb, out, limit, truncated);
                if let Some(j) = paths_b.iter().position(|x| *x == pb) {
                    consumed_b[j] = true;
                }
            }
            None => {}
        }
        if out.len() >= limit || *truncated {
            *truncated = true;
            return;
        }
    }
    // Node-level: paths present on exactly one side and never paired by name
    // (exact twin or NameMismatch sibling) are whole-node misses. `paths_a`/
    // `paths_b` are sorted, so the exact-twin check uses a binary search — a
    // linear scan of an unsorted list would let a b-path claimed in Pass 1 be
    // re-reported as MissingInA when its a-twin sat late in the order.
    for (i, pa) in paths_a.iter().enumerate() {
        if !consumed_a[i] && paths_b.binary_search(pa).is_err() {
            // A node path present ONLY in a is missing from b — MissingInB.
            // Semantics: MissingInB = present in a, absent in b.
            out.push(Diff::MissingInB {
                path: pa.to_string(),
            });
            consumed_a[i] = true;
            if out.len() >= limit {
                *truncated = true;
                return;
            }
        }
    }
    for (j, pb) in paths_b.iter().enumerate() {
        if !consumed_b[j] && paths_a.binary_search(pb).is_err() {
            // A node path present ONLY in b is missing from a — MissingInA.
            // Semantics: MissingInA = present in b, absent in a. (The former
            // `MissingInB` label here was inverted; HW-07E corrects it.)
            out.push(Diff::MissingInA {
                path: pb.to_string(),
            });
            if out.len() >= limit {
                *truncated = true;
                return;
            }
        }
    }
    out.sort_by(|x, y| diff_key(x).cmp(&diff_key(y)));
}

/// Diff one node path's props, emitting at most until the shared budget
/// `limit` is reached. On the emission that hits `limit`, `truncated` is set
/// and no further prop comparisons run (the WORK bound).
///
/// HW-09B: the emission budget. Emissions stop AT `limit` — the emission
/// that REACHES `limit` is reported and ends this pass with `truncated`
/// set (HW-09's contract: a capped report is a deterministic prefix of
/// exactly `cap` diffs, so `out` never grows past `limit`). `truncated` is
/// set only alongside a real emission, so a diff-free blob still reports
/// StructurallyEqual.
fn diff_props<'a>(
    a: &[PropEntry<'a>],
    b: &[PropEntry<'a>],
    pa: &str,
    pb: &str,
    out: &mut Vec<Diff>,
    limit: usize,
    truncated: &mut bool,
) {
    // HW-09B: the emission budget. Emissions stop AT `limit` — the
    // emission that REACHES `limit` ends this pass and flags `truncated`
    // (HW-09's contract: a capped report is a deterministic prefix of
    // exactly `cap` diffs, so the Vec never grows past `limit`).
    let hits = |out: &Vec<Diff>| out.len() >= limit;
    let ea: Vec<&PropEntry> = a.iter().filter(|e| e.node_path == pa).collect();
    let eb: Vec<&PropEntry> = b.iter().filter(|e| e.node_path == pb).collect();
    for x in &ea {
        match eb.iter().find(|y| y.name == x.name) {
            // A property present ONLY in a is missing from b — PropMissingInB.
            // Semantics: PropMissingInB = present in a, absent in b.
            None => out.push(Diff::PropMissingInB {
                path: pa.into(),
                name: x.name.into(),
            }),
            Some(y) => {
                if x.value != y.value {
                    out.push(Diff::ValueMismatch {
                        path: pa.into(),
                        name: x.name.into(),
                        a_off: x.value_at,
                        b_off: y.value_at,
                        a_len: x.value.len(),
                        b_len: y.value.len(),
                    });
                } else {
                    continue; // no emission: budget untouched
                }
            }
        }
        if hits(out) {
            *truncated = true;
            return;
        }
    }
    for y in &eb {
        if !ea.iter().any(|x| x.name == y.name) {
            // A property present ONLY in b is missing from a — PropMissingInA.
            // Semantics: PropMissingInA = present in b, absent in a.
            out.push(Diff::PropMissingInA {
                path: pb.into(),
                name: y.name.into(),
            });
            if hits(out) {
                *truncated = true;
                return;
            }
        }
    }
}

/// Segment-position pairing: identical string, or same depth with all-but-last
/// segments equal (sibling rename). `used` marks b-paths already claimed.
fn paired_path<'x>(p: &str, others: &'x [&'x str], used: &[bool]) -> Option<&'x str> {
    if let Some((j, x)) = others.iter().enumerate().find(|(_, x)| **x == p) {
        return if used[j] { None } else { Some(x) }; // claimed by a pairing already
    }
    let ps: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
    if ps.is_empty() {
        return None;
    }
    others
        .iter()
        .enumerate()
        .find(|(j, o)| {
            !used[*j] && {
                let os: Vec<&str> = o.split('/').filter(|s| !s.is_empty()).collect();
                os.len() == ps.len() && os[..os.len() - 1] == ps[..ps.len() - 1]
            }
        })
        .map(|(_, o)| *o)
}

fn same_depth(a: &str, b: &str) -> bool {
    a.split('/').count() == b.split('/').count()
}

fn sibling_bodies_equal<'a>(a: &[PropEntry<'a>], b: &[PropEntry<'a>], pa: &str, pb: &str) -> bool {
    let ea: Vec<(&str, &[u8])> = a
        .iter()
        .filter(|e| e.node_path == pa)
        .map(|e| (e.name, e.value))
        .collect();
    let eb: Vec<(&str, &[u8])> = b
        .iter()
        .filter(|e| e.node_path == pb)
        .map(|e| (e.name, e.value))
        .collect();
    !ea.is_empty() && ea == eb
}

fn parents(p: &str) -> &str {
    match p.rfind('/') {
        Some(0) => "/",
        Some(i) => &p[..i],
        None => "/",
    }
}

fn last_seg(p: &str) -> &str {
    match p.rfind('/') {
        Some(i) => &p[i + 1..],
        None => p,
    }
}

fn diff_key(d: &Diff) -> (u8, String, String) {
    match d {
        Diff::MissingInB { path } => (0, path.clone(), String::new()),
        Diff::MissingInA { path } => (1, path.clone(), String::new()),
        Diff::PropMissingInB { path, name } => (2, path.clone(), name.clone()),
        Diff::PropMissingInA { path, name } => (3, path.clone(), name.clone()),
        Diff::ValueMismatch { path, name, .. } => (4, path.clone(), name.clone()),
        Diff::NameMismatch {
            parent,
            a_name,
            b_name,
        } => (5, parent.clone(), alloc::format!("{}|{}", a_name, b_name)),
    }
}

// ---------------------------------------------------------------------------
// Extraction: locate DTBs embedded in a larger container (boot image)
// ---------------------------------------------------------------------------

/// One DTB located inside a bigger byte blob (e.g. the full31 boot image,
/// which carries its DTB at offset 3,453,288 — evidence, never hardcoded).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DtbCandidate {
    /// Byte offset in the container where the FDT header starts.
    pub offset: usize,
    /// `totalsize` as recorded in the validated header.
    pub totalsize: usize,
}

/// Scan `blob` for embedded DTBs: every 4-aligned offset whose u32 equals
/// `FDT_MAGIC`, whose declared totalsize FITS the remaining container bytes
/// (`totalsize <= blob.len() - off` — the explicit totalsize-fits owner,
/// stated here so the mutation probe has a single named site), and whose
/// header passes the STRICT `Fdt::parse` on the remaining bytes — kept as
/// defense in depth: it enforces magic, min/max totalsize bounds, the
/// parse-side truncation check, v17-only version, block-order and
/// reservation-map rules, and a full well-formedness walk. Never panics on
/// any input; absurd sizes are rejected through the existing `FdtError`
/// paths. At most `MAX_CANDIDATES` candidates are returned — the scan stops
/// scan `blob` for DTBs — see `extract_dtbs_with_limit` for the parameterized
/// form; the public API always passes `MAX_CANDIDATES` (HW-09). Results are
/// deterministic: ascending `offset`.
pub fn extract_dtbs(blob: &[u8]) -> Vec<DtbCandidate> {
    extract_dtbs_with_limit(blob, MAX_CANDIDATES)
}

/// Test-visible cap wrapper (HW-09): same scan as `extract_dtbs`, with the
/// candidate limit passed explicitly. Production code must call
/// `extract_dtbs`; this exists so the stop-behavior is testable without
/// packing > MAX_CANDIDATES real DTBs into one blob.
pub fn extract_dtbs_capped(blob: &[u8], max_candidates: usize) -> Vec<DtbCandidate> {
    extract_dtbs_with_limit(blob, max_candidates)
}

/// Internal, test-visible form of `extract_dtbs`: the candidate cap is a
/// parameter so the stop-behavior can be exercised without packing
/// > MAX_CANDIDATES real DTBs into one test blob. The public API always
/// passes `MAX_CANDIDATES`.
fn extract_dtbs_with_limit(blob: &[u8], max_candidates: usize) -> Vec<DtbCandidate> {
    let mut out = Vec::new();
    if blob.len() < FDT_HEADER_LEN {
        return out;
    }
    // Magic must sit on a 4-aligned boundary; step by 4. Stop where a full
    // header can no longer be read (parse would reject it anyway).
    let last = blob.len() - FDT_HEADER_LEN;
    let mut off = 0usize;
    while off <= last {
        if be_u32(blob, off) == Some(FDT_MAGIC) {
            let totalsize = be_u32(blob, off + 4).unwrap() as usize;
            // Explicit totalsize-fits owner: the declared blob must end
            // inside the container we were handed. Without it, a header
            // whose totalsize overruns the region (truncation, or a tail
            // pad counted by totalsize but owned by no block) could mint a
            // phantom candidate under a parser that clamps instead of
            // rejects. Kept ahead of parse so this rule has ONE named home.
            if totalsize <= blob.len() - off && Fdt::parse(&blob[off..]).is_ok() {
                out.push(DtbCandidate {
                    offset: off,
                    totalsize,
                });
                // HW-09: hard cap on candidate Vec growth. On reaching the
                // limit the scan STOPS — documented, deterministic (the
                // kept candidates are the lowest-offset ones), no panic.
                if out.len() >= max_candidates {
                    break;
                }
            }
        }
        off += 4;
    }
    out
}

/// Rank candidates by evidence strength against a captured blob:
///   0 = byte-equal (the G3 gold standard), 1 = structurally equal,
///   2 = different (fewest structural diffs wins inside the rank),
///   3 = the candidate did not parse or compare (last resort).
/// Ties break on lowest `offset` — the earliest copy in the image.
/// `verdicts[i]` is the compare outcome for `candidates[i]` (same order).
/// Returns the index of the best candidate, or `None` if the list is empty.
pub fn best_candidate_rank(
    candidates: &[DtbCandidate],
    verdicts: &[Result<DiffReport, (usize, FdtError)>],
) -> Option<usize> {
    let rank = |v: &Result<DiffReport, (usize, FdtError)>| -> (u8, usize) {
        match v {
            Ok(DiffReport::Equal) => (0, 0),
            // HW-09C: an Incomplete verdict proves nothing — it ranks with
            // a non-parsing candidate, never as "structurally equal".
            Ok(DiffReport::Incomplete { .. }) => (3, 0),
            Ok(DiffReport::StructurallyEqual { .. }) => (1, 0),
            Ok(DiffReport::Different { diffs, .. }) => (2, diffs.len()),
            Err(_) => (3, 0),
        }
    };
    let mut best: Option<(u8, usize, usize, usize)> = None; // (rank, ndiffs, offset, index)
    for i in 0..candidates.len().min(verdicts.len()) {
        let (r, n) = rank(&verdicts[i]);
        let key = (r, n, candidates[i].offset, i);
        match best {
            None => best = Some(key),
            Some(b) => {
                if (key.0, key.1, key.2) < (b.0, b.1, b.2) {
                    best = Some(key);
                }
            }
        }
    }
    best.map(|b| b.3)
}

// ---------------------------------------------------------------------------
// Helpers for tests/demo: minimal DTB construction (no dtc needed)
// ---------------------------------------------------------------------------

/// Tiny deterministic DTB builder used by tests (and any consumer that wants
/// a synthetic blob). Produces strictly valid v17 layout.
pub struct DtbBuilder {
    struct_buf: Vec<u8>,
    strings: Vec<u8>,
    str_map: Vec<(String, u32)>,
}

impl DtbBuilder {
    pub fn new() -> Self {
        DtbBuilder {
            struct_buf: Vec::new(),
            strings: Vec::new(),
            str_map: Vec::new(),
        }
    }

    fn str_off(&mut self, s: &str) -> u32 {
        if let Some((_, off)) = self.str_map.iter().find(|(k, _)| k == s) {
            return *off;
        }
        let off = self.strings.len() as u32;
        self.strings.extend_from_slice(s.as_bytes());
        self.strings.push(0);
        self.str_map.push((String::from(s), off));
        off
    }

    fn begin_node_raw(&mut self, name: &str) {
        // Token, then NUL-terminated name padded so the NAME+NUL run is a
        // multiple of 4 (the token itself already sits on a boundary).
        let start = self.struct_buf.len();
        self.struct_buf
            .extend_from_slice(&FDT_BEGIN_NODE.to_be_bytes());
        self.struct_buf.extend_from_slice(name.as_bytes());
        self.struct_buf.push(0);
        while self.struct_buf.len() % 4 != 0 {
            self.struct_buf.push(0);
        }
        // Invariant: the name+NUL run after the token is a multiple of 4.
        debug_assert_eq!((self.struct_buf.len() - start) % 4, 0);
    }

    fn end_node(&mut self) {
        self.struct_buf
            .extend_from_slice(&FDT_END_NODE.to_be_bytes());
    }

    pub fn prop(&mut self, name: &str, value: &[u8]) {
        let nameoff = self.str_off(name);
        self.struct_buf.extend_from_slice(&FDT_PROP.to_be_bytes());
        self.struct_buf
            .extend_from_slice(&(value.len() as u32).to_be_bytes());
        self.struct_buf.extend_from_slice(&nameoff.to_be_bytes());
        self.struct_buf.extend_from_slice(value);
        while self.struct_buf.len() % 4 != 0 {
            self.struct_buf.push(0);
        }
    }

    /// Finish the tree: root node with `root_props`, then `children` each with
    /// their own props. Returns the complete blob.
    /// Emit BEGIN_NODE("") for the root. The caller then opens children via
    /// `node(...)` guards and finally calls `finish()`, which appends the
    /// root's END_NODE and FDT_END.
    pub fn open_root(mut self) -> Self {
        // Root BEGIN_NODE: token + 4 pad bytes (empty name, NUL-terminated,
        // padded to a 4-byte run). Emitted literally so the invariant is
        // visible at the call site.
        debug_assert_eq!(self.struct_buf.len(), 0);
        self.struct_buf
            .extend_from_slice(&FDT_BEGIN_NODE.to_be_bytes());
        self.struct_buf.extend_from_slice(&[0u8; 4]); // empty name + pad
        self
    }

    /// Work inside the opened root: props + child node guards.
    pub fn root_props(&mut self) -> RootCtx<'_> {
        RootCtx { b: self }
    }

    pub fn finish(mut self) -> Vec<u8> {
        // Close the root node, then the struct block.
        self.end_node();
        let mut sb = self.struct_buf;
        sb.extend_from_slice(&FDT_END.to_be_bytes());

        let header = FDT_HEADER_LEN; // 40 bytes = 10 BE u32 fields
        let off_dt_struct = header + 16; // leave room for the rsv-map terminator
        let off_dt_strings = header + sb.len() + 16;
        let totalsize = off_dt_strings + self.strings.len();

        // FDT v17 header: 10 big-endian u32 fields = 40 bytes.
        let mut hdr = [0u8; 40];
        hdr[0..4].copy_from_slice(&FDT_MAGIC.to_be_bytes());
        hdr[4..8].copy_from_slice(&(totalsize as u32).to_be_bytes());
        hdr[8..12].copy_from_slice(&(off_dt_struct as u32).to_be_bytes());
        hdr[12..16].copy_from_slice(&(off_dt_strings as u32).to_be_bytes());
        // HW-07D: off_rsv_map must be >= FDT_HEADER_LEN (libfdt check_off_);
        // the map is a terminator-terminated array, so an empty map is one
        // all-zero 16-byte entry. We emit off 40 + a terminator.
        hdr[16..20].copy_from_slice(&(FDT_HEADER_LEN as u32).to_be_bytes()); // off_rsv_map
        hdr[20..24].copy_from_slice(&17u32.to_be_bytes()); // version
        hdr[24..28].copy_from_slice(&17u32.to_be_bytes()); // last_comp_version
        hdr[28..32].copy_from_slice(&0u32.to_be_bytes()); // boot_cpuid_phys
        hdr[32..36].copy_from_slice(&(self.strings.len() as u32).to_be_bytes()); // size_dt_strings
        hdr[36..40].copy_from_slice(&(sb.len() as u32).to_be_bytes()); // size_dt_struct
        let mut blob = Vec::with_capacity(totalsize);
        blob.extend_from_slice(&hdr);
        blob.extend_from_slice(&[0u8; 16]); // reservation-map terminator
        blob.extend_from_slice(&sb);
        blob.extend_from_slice(&self.strings);

        blob
    }

    /// Convenience: build a blob with one subtree shape:
    /// root prop `stdout-path`, child node `chosen` with props, grandchild.
    /// (Kept simple: tests compose with begin/end via `Node` guard below.)
    pub fn node(&mut self, name: &str) -> NodeGuard<'_> {
        self.begin_node_raw(name);
        NodeGuard { b: self }
    }
}

impl Default for DtbBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Borrow of the builder while inside the (already-opened) root node: props
/// and child nodes, but no END_NODE is emitted on drop (finish() does that).
pub struct RootCtx<'a> {
    b: &'a mut DtbBuilder,
}

impl<'a> RootCtx<'a> {
    pub fn prop(&mut self, name: &str, value: &[u8]) {
        self.b.prop(name, value);
    }
    pub fn node(&mut self, name: &str) -> NodeGuard<'_> {
        self.b.node(name)
    }
}

pub struct NodeGuard<'a> {
    b: &'a mut DtbBuilder,
}

impl<'a> NodeGuard<'a> {
    pub fn prop(&mut self, name: &str, value: &[u8]) {
        self.b.prop(name, value);
    }
    pub fn node(&mut self, name: &str) -> NodeGuard<'_> {
        self.b.begin_node_raw(name);
        NodeGuard { b: self.b }
    }
}

impl Drop for NodeGuard<'_> {
    fn drop(&mut self) {
        self.b.end_node();
    }
}
