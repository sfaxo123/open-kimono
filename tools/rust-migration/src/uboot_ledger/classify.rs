//! Path-only category rules for the U-Boot source ledger.
//!
//! The category set is fixed by the brief: `driver`, `dts`, `build`,
//! `config`, `doc`, `test`, `lib`, `other`. There is deliberately NO
//! catch-all over the source trees: a path no rule covers is an
//! UNMAPPED failure that makes the run exit nonzero and name the
//! offender — the same contract as the INV-01 repo inventory. (`other`
//! is reachable only through explicit directory rules for non-source
//! surfaces, never by falling off the end of the ladder.)
//!
//! Every rule is a path-prefix or basename/extension rule over the git
//! tree listing; no file content is read and no filename set is
//! hardcoded. The ladder runs top-to-bottom and the FIRST match wins.

/// The category a U-Boot tree path belongs to, or `None` when no rule
/// covers it — which makes "unmapped" loud, never silent.
pub fn classify(path: &str) -> Option<&'static str> {
    // ---- test: the dedicated test tree and its py/ harness ----
    if path == "test" || path.starts_with("test/") {
        return Some("test");
    }

    // ---- dts: the top-level DTS + kernel dt-bindings import ----
    if path.starts_with("dts/") {
        return Some("dts");
    }

    // ---- driver: driver-model subsystem trees, board support ----
    if path == "drivers" || path.starts_with("drivers/") {
        return Some("driver");
    }
    if path == "board" || path.starts_with("board/") {
        return Some("driver");
    }

    // ---- config: defconfig and fragment surfaces ----
    if path.starts_with("configs/") {
        return Some("config");
    }

    // ---- doc: prose trees (doc/ and Licenses/) ----
    if path == "Licenses" || path.starts_with("Licenses/") {
        return Some("doc");
    }
    if path == "doc" || path.starts_with("doc/") {
        // The doc build glue is build machinery, not prose.
        if path == "doc/Makefile" {
            return Some("build");
        }
        return Some("doc");
    }

    // ---- test: standalone POST tests live in their own tree ----
    if path == "post" || path.starts_with("post/") {
        return Some("test");
    }

    // ---- other: host-side tooling, build scripts, examples ----
    if path.starts_with("tools/") || path.starts_with("scripts/") {
        return Some("other");
    }
    if path == "examples" || path.starts_with("examples/") {
        return Some("other");
    }

    // ---- build: root build machinery and repo config dotfiles ----
    if matches!(path, "Makefile" | "Kbuild" | "config.mk") {
        return Some("build");
    }
    if path == "Kconfig" {
        return Some("config");
    }
    if path.starts_with('.') {
        // CI pipelines, checkpatch/patman/b4 config, clang-format,
        // .github/: repo-level tooling configuration. VCS bookkeeping
        // files (.gitignore/.gitattributes) are included: they are
        // tracked surfaces of the tree and must be inventoried too.
        return Some("build");
    }
    if path == "COPYING" {
        return Some("doc");
    }

    // ---- core source trees: extension rules (judgement calls below) ----
    if in_core_tree(path) {
        return classify_source_ext(path);
    }

    None
}

/// The top-level trees that hold runtime/bootloader code and its build
/// glue — the trees where an unmapped path must fail rather than be
/// swept into a catch-all.
fn in_core_tree(path: &str) -> bool {
    const CORE: [&str; 14] = [
        "arch",
        "boot",
        "cmd",
        "common",
        "disk",
        "env",
        "fs",
        "include",
        "lib",
        "net",
        "api",
        "Kconfig",
        "MAINTAINERS",
        "README",
    ];
    CORE.iter()
        .any(|top| path == *top || path.starts_with(&format!("{top}/")))
}

/// Extension/basename rules for the core source trees. Judgement calls,
/// documented:
/// - `.dts/.dtsi/.dtso` under `arch/` are board device trees -> `dts`,
///   even though they sit in `arch/`.
/// - `.yaml` under `arch/*/cpu/*/` or `arch/*/mach-*/` is the kernel
///   SoC dt-binding schema import -> `dts`; a `.yaml` elsewhere in the
///   core trees (e.g. lib content) falls through to `other`.
/// - build glue (`Makefile*`, `Kbuild`, `Kconfig*`, `.mk/.mak`,
///   `.lds/.ldsi`, `.postlink`) -> `build`.
/// - config fragments and env text (`.cfg`, `.config`, `.env`,
///   `.defconfig`, `_defconfig`) -> `config`.
/// - README/MAINTAINERS/docs/prose anywhere in the core trees -> `doc`.
/// - test fixtures (`expected_*`, `.hex`) -> `other`: data, not test
///   code, shipped in trees that are not test trees.
/// - compiled or binary blobs (`.bin/.dat/.bmp/.ttf/.ff/.png/.gif/
///   .jpg/.svg/.css/.html/.gz/.xz`) -> `other`: inventory records, not
///   migratable source.
/// - everything else in a core tree is bootloader source or its
///   signing/keying material -> `driver` (C, asm, headers, dts include
///   fragments, ACPI tables, linker-ish includes, certificate and key
///   blobs under lib/ used by image signing).
fn classify_source_ext(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = ext_of(name);

    // Device trees inside arch/ are dts surfaces.
    if matches!(ext, Some("dts") | Some("dtsi") | Some("dtso")) {
        return Some("dts");
    }
    // Kernel dt-bindings schema imports under mach-/cpu dirs.
    if ext == Some("yaml") && (path.contains("/mach-") || path.contains("/cpu/")) {
        return Some("dts");
    }

    // Build machinery by basename or extension.
    if name == "Makefile"
        || name.starts_with("Makefile.")
        || name == "Kbuild"
        || name == "Kconfig"
        || name.starts_with("Kconfig.")
        || matches!(ext, Some("mk") | Some("mak") | Some("lds") | Some("ldsi"))
    {
        return Some("build");
    }
    if path.ends_with(".postlink") {
        return Some("build");
    }

    // Config fragments and environment text.
    if name.ends_with("_defconfig") || ext == Some("defconfig") {
        return Some("config");
    }
    if matches!(ext, Some("cfg") | Some("config") | Some("env")) {
        return Some("config");
    }

    // Prose and repo records.
    if name == "README"
        || name.starts_with("README.")
        || name == "Readme"
        || name == "MAINTAINERS"
        || matches!(ext, Some("rst") | Some("md") | Some("txt"))
    {
        return Some("doc");
    }

    // Test fixtures: expected-output data shipped in non-test trees.
    if name.starts_with("expected_") || ext == Some("hex") {
        return Some("other");
    }

    // Compiled or binary blobs in core trees are neither source nor
    // build input we migrate — inventory them as `other`.
    if matches!(
        ext,
        Some("bin")
            | Some("dat")
            | Some("bmp")
            | Some("ttf")
            | Some("ff")
            | Some("png")
            | Some("gif")
            | Some("jpg")
            | Some("svg")
            | Some("css")
            | Some("html")
            | Some("shtml")
            | Some("gz")
            | Some("xz")
            | Some("pdf")
    ) {
        return Some("other");
    }

    // Everything else under a core tree is bootloader source or its
    // signing/keying material.
    Some("driver")
}

/// The extension of a basename (`.def` in `foo.defconfig` is NOT an
/// extension — only the last dot segment, and a leading-dot basename
/// like `.gitignore` has none).
fn ext_of(name: &str) -> Option<&str> {
    let dot = name.rfind('.')?;
    if dot == 0 {
        return None;
    }
    Some(&name[dot + 1..])
}
