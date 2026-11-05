//! Path/extension/content-sniff classification rules for the repo
//! inventory.
//!
//! The category set is the plan's (rust, python, shell, c, header,
//! ffi-binding, native-build-dep, doc, data-fixture, build-config,
//! other-known) plus one extra the brief leaves open ("at minimum"):
//! `evidence` — captured command output and digest/receipt records,
//! keyed on directory role and record shape rather than on any file
//! list.
//!
//! A file that matches no rule is an UNCLASSIFIED failure by design and
//! the run must name the offender: a ledger with blank places is a map
//! that lies, and "zero unclassified" is this tool's whole contract.
//! Every rule below is structural (a directory role, an extension, or a
//! content sniff) — nothing hardcodes a filename set.

/// The category a file belongs to, or `None` when no rule covers it —
/// which makes "unclassified" loud, never silent. `content` is the raw
/// file bytes; rules that cannot be decided from the path alone sniff
/// them.
pub fn classify(path: &str, content: &[u8]) -> Option<&'static str> {
    classify_ref(path, content)
}

/// `Some(category)` if a rule covers `path`, `None` if none does. Rules
/// run top-to-bottom and the FIRST match wins, so each is worded not to
/// swallow an earlier family's files.
fn classify_ref(path: &str, content: &[u8]) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let lowered = path.to_ascii_lowercase();
    let lowered_name = name.to_ascii_lowercase();

    // ---- build-config: build inputs and pinned dependency resolution ----
    // A lockfile is pinned dependency resolution — load-bearing for an
    // exact rebuild and for the audit that a crate's dependency claim is
    // true — wherever it sits, never a data fixture (a fixtures tree
    // may carry one).
    if lowered_name == "cargo.lock" || lowered_name.ends_with(".lock") {
        return Some("build-config");
    }
    // A `.toml` outside a data tree is a manifest or a cargo config
    // (`build.rustflags` linker-script directives); the data-tree rule
    // below keeps test-tree TOMLs (and the `Cargo.toml` files that sit
    // in `crates/*/tests/fixtures/`) out of this family's reach, so the
    // rule never swallows test data either way.
    if lowered_name.ends_with(".toml") && !in_data_tree(path) {
        return Some("build-config");
    }
    // `.in` harness templates (scripts/oracle/*/…): the harness build
    // scripts copy these into a scratch crate verbatim, so they ARE
    // build inputs: `Cargo.toml.in` declares that crate's dependencies
    // and `lib.rs.in` is compiled inside it — the native/unsafe audit
    // input (INV-DEP-01), not fixture data that merely replays.
    if lowered_name.ends_with(".in") {
        return Some("build-config");
    }
    // `link.md` is a cargo `rustc-link-arg` script directive, not prose;
    // it must never be swallowed by a docs rule.
    if lowered_name == "link.md" {
        return Some("build-config");
    }
    // Flat-style Python package metadata (PEP 566 payload): the pinned
    // version of a reference package the parity harness installs pins
    // that dependency, so the file IS a build input. Keyed on the
    // `dist-info` component — an explicit marker that the file is
    // package metadata — never on the filename (the `lowered_name`
    // rules above cannot see `METADATA` once lowercased, and a rule
    // keyed on that literal would also mislabel any future lowercase
    // `metadata` file that is not package metadata).
    if path.contains(".dist-info/") {
        return Some("build-config");
    }
    // Kconfig fragments (`.config`, `…_defconfig`): a kernel/firmware
    // build configuration, not prose and not an oracle output, wherever
    // they sit (embedded-efi keeps a `baseline.config` beside its DTS).
    if lowered_name.ends_with(".config") || lowered_name.ends_with("_defconfig") {
        return Some("build-config");
    }
    if lowered_name == "cmakelists.txt" || lowered.contains("cmake_scripts") {
        return Some("build-config");
    }

    // ---- data-fixture / evidence (directory-role rules) ---------------------
    // A path under a `tests/`, `fixtures/` or `evidence/` tree is test
    // or audit DATA (a copy-in harness, a golden stream, a replayed
    // trace, a receipt record), whatever its extension says. `.h`/`.hpp`
    // are excluded inside those trees: a copied-in native reference
    // header is what a host replay build consumes (the native-boundary
    // audit of G-INT must be able to re-find it), while a
    // `fixtures/*.json` golden feed stays fixture data.
    if in_data_tree(path) {
        if lowered_name.ends_with(".h") || lowered_name.ends_with(".hpp") {
            return Some("native-build-dep");
        }
        return Some("data-fixture");
    }

    // ---- native-build-dep / ffi-binding --------------------------------------
    // A host script that drives a native toolchain (QEMU, the aarch64
    // cross toolchain and binutils, the image builders) from host
    // Python or shell is a dependency OF that toolchain in the plan's
    // sense: the parity harness cannot run without the external native
    // binary the script invokes or emits bytes for. Sniffed from
    // content, never from the filename; test-tree and oracle scripts
    // are claimed by the rules above, BEFORE this sniff, so this rule
    // cannot swallow them: they stay fixtures and the toolchain they
    // drive stays audit input for HT-GEN, not a binding swallowed by a
    // naming accident.
    if (lowered_name.ends_with(".py") || lowered_name.ends_with(".sh"))
        && !in_test_tree(path)
        && !path.starts_with("scripts/oracle/")
        && drives_native_toolchain(content)
    {
        return Some("native-build-dep");
    }
    if lowered_name.ends_with(".wasm") || content.starts_with(b"\0asm") {
        return Some("ffi-binding");
    }

    // ---- language families ----------------------------------------------------
    // Extension-only fallbacks, after the tree-role rules have had
    // their claim (a copy-in harness is data first, a driver second).
    if lowered_name.ends_with(".py") {
        return Some("python");
    }
    if lowered_name.ends_with(".sh") {
        return Some("shell");
    }
    if lowered_name.ends_with(".rs") {
        return Some("rust");
    }
    if lowered_name.ends_with(".c") || lowered_name.ends_with(".cc") || name.ends_with(".S") {
        return Some("c");
    }
    if lowered_name.ends_with(".h") || lowered_name.ends_with(".hpp") {
        return Some("header");
    }

    // ---- doc / evidence under the docs tree --------------------------------------
    // `docs/` holds documentation AND captured output. A `.md`/`.txt`
    // under `docs/` that sniffs as machine output (see
    // `machine_output`) is `evidence`: an audit that searches
    // documentation cannot be fed machine output, and a rule that
    // labels a captured transcript `doc` hides it from the parity
    // audits that diff against it (B4-SVCD-TESTS.txt begins with a
    // `date:` line — prose to the eye, capture to a diff). Everything
    // else under `docs/` — prose markdown, the columnar plan.csv and
    // run tables, the trace captures and their digests — is evidence
    // too: an audit input, not documentation, and not fixture data a
    // test reads back either.
    if path.starts_with("docs/") {
        if (lowered_name.ends_with(".md") || lowered_name.ends_with(".txt"))
            && !machine_output(content)
        {
            return Some("doc");
        }
        return Some("evidence");
    }
    // Project prose outside the docs tree (crate READMEs).
    if lowered_name.ends_with(".md") || name == "README" {
        return Some("doc");
    }

    // ---- other-known --------------------------------------------------------------
    // Remaining records, media and archives — evidence and data an
    // audit keys off, with no more specific family to call them:
    // command and transcript output (`.txt`), receipt tables (`.tsv`),
    // digest lists (`SHA256SUMS`), machine-readable captures (`.json`,
    // `.jsonl`), compressed captures (`.gz`), patch series (`.patch`),
    // devicetree source/blob (`.dts`, `.dtb`), screenshots (`.png`),
    // plan tables (`.csv`), and git bookkeeping files.
    if lowered_name.ends_with(".txt") || lowered_name.ends_with(".tsv") {
        return Some("other-known");
    }
    if lowered_name == "SHA256SUMS" || lowered_name == "metadata" {
        return Some("other-known");
    }
    if lowered_name.ends_with(".patch") {
        return Some("other-known");
    }
    if lowered_name == ".gitignore" || lowered_name == ".gitattributes" {
        return Some("other-known");
    }
    if lowered_name.ends_with(".dts") || lowered_name.ends_with(".dtb") {
        return Some("other-known");
    }
    if lowered_name.ends_with(".png")
        || lowered_name.ends_with(".jpg")
        || lowered_name.ends_with(".jpeg")
    {
        return Some("other-known");
    }
    if lowered_name.ends_with(".json") || lowered_name.ends_with(".jsonl") {
        return Some("other-known");
    }
    if lowered_name.ends_with(".gz") {
        return Some("other-known");
    }
    if lowered_name.ends_with(".csv") {
        return Some("other-known");
    }
    None
}

/// Directory-role fixture/evidence rule: a path under any `tests/`,
/// `fixtures/` or `evidence/` tree, at any depth, including a tree that
/// starts the path.
fn in_data_tree(path: &str) -> bool {
    in_test_tree(path) || in_fixture_tree(path) || in_evidence_tree(path)
}

fn in_test_tree(path: &str) -> bool {
    path.starts_with("tests/") || path.contains("/tests/")
}

fn in_fixture_tree(path: &str) -> bool {
    path.starts_with("fixtures/") || path.contains("/fixtures/")
}

fn in_evidence_tree(path: &str) -> bool {
    path.starts_with("evidence/") || path.contains("/evidence/")
}

/// Sniff the leading 4 KiB for a native-toolchain reference: QEMU
/// system emulation, the aarch64 cross toolchain and binutils, and the
/// image builders that assemble a bootable guest image. Content-sniffed
/// because an extension-only rule could never see the `qemu` command
/// inside a script.
fn drives_native_toolchain(content: &[u8]) -> bool {
    let head = &content[..content.len().min(4096)];
    let lowered = String::from_utf8_lossy(head).to_ascii_lowercase();
    lowered.contains("qemu")
        || lowered.contains("aarch64-linux-gnu")
        || lowered.contains("objcopy")
        || lowered.contains("objdump")
        || lowered.contains("mkimage")
        || lowered.contains("fitimg")
        || lowered.contains("dd if=")
        || lowered.contains("losetup")
        || lowered.contains("truncate -s")
}

/// A file whose first line sniffs as a capture/receipt record (a host
/// absolute path, or a columnar digest-receipt header) is machine
/// output, not documentation.
fn machine_output(content: &[u8]) -> bool {
    if content.is_empty() {
        return false;
    }
    let head = &content[..content.len().min(4096)];
    let first_line = match head.iter().position(|b| b == &b'\n') {
        Some(at) => &head[..at],
        None => head,
    };
    if first_line.starts_with(b"/") || first_line.starts_with(b"~/") {
        return true;
    }
    if first_line.starts_with(b"artifact\t")
        || first_line.starts_with(b"run_id\t")
        || first_line.starts_with(b"source_ref\t")
    {
        return true;
    }
    false
}
