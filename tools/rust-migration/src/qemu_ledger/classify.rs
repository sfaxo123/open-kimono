//! Path-only category rules for the QEMU source ledger.
//!
//! The category set is fixed by the brief: `machine`, `device`, `core`,
//! `build`, `config`, `doc`, `test`, `qtest-harness`, `other-known`.
//! There is deliberately NO catch-all: a path no rule covers is an
//! UNMAPPED failure that makes the run exit nonzero and name the
//! offender — the same contract as the INV-01 repo inventory and the
//! INV-UB-01 U-Boot ledger. (`other-known` is reachable only through
//! explicit rules naming known non-source or undecided surfaces, never
//! by falling off the end of the ladder.)
//!
//! Every rule is a path-prefix, basename, or exact-path rule over the
//! git tree listing; no file content is read. The ladder runs
//! top-to-bottom and the FIRST match wins. Judgement calls are
//! documented at their rule.

/// The category a QEMU tree path belongs to, or `None` when no rule
/// covers it — which makes "unmapped" loud, never silent.
pub fn classify(path: &str) -> Option<&'static str> {
    // ---- machine: the OnePlus 6 machine itself (fork-only surfaces) ----
    // First rule so the machine and its docs are never swept into
    // device/doc. Covers README.oneplus6.md, docs/oneplus6/** (the
    // fork's own reports, evidence logs and runner scripts) and every
    // hw/ + include/hw/ path carrying an op6/qcom custom name.
    if is_oneplus6_machine(path) {
        return Some("machine");
    }

    // ---- qtest-harness: the python harness that drives qtests ----
    // `python/qemu/**` is the QEMUMachine/qtest protocol library the
    // functional + qtest suites import. The rest of python/ is
    // packaging machinery (falls through to other-known below).
    if path.starts_with("python/qemu/") {
        return Some("qtest-harness");
    }

    // ---- test: the dedicated test trees, wholesale ----
    if path == "tests" || path.starts_with("tests/") {
        return Some("test");
    }

    // ---- config: build-time configuration trees ----
    if path.starts_with("configs/") {
        return Some("config");
    }

    // ---- doc: prose trees ----
    if path == "docs" || path.starts_with("docs/") {
        return Some("doc");
    }
    if path == "Licenses" || path.starts_with("Licenses/") {
        return Some("doc");
    }

    // ---- device: hw/ + its headers (machine files already taken) ----
    if path.starts_with("include/hw/") {
        return Some("device");
    }
    if path == "hw" || path.starts_with("hw/") {
        return Some("device");
    }

    // ---- core: the emulator core (CPU/memory/exec machinery) ----
    if in_core_tree(path) {
        return Some("core");
    }

    // ---- build: root build machinery and option docs ----
    // Exact-path rules only — never a prefix that could swallow source.
    if matches!(
        path,
        "Makefile"
            | "meson.build"
            | "meson_options.txt"
            | "configure"
            | "run.in"
            | "Kconfig"
            | "Kconfig.host"
            | "qemu-options.hx"
            | "qemu-img-cmds.hx"
            | "hmp-commands.hx"
            | "hmp-commands-info.hx"
            | "trace-events"
            | "version.rc"
            | "qemu.nsi"
            | "pythondeps.toml"
            | "clippy.toml"
            | "VERSION"
    ) {
        return Some("build");
    }

    // ---- doc: root prose ----
    if matches!(
        path,
        "README.rst"
            | "MAINTAINERS"
            | ".mailmap"
            | "gitdm.config"
            | "COPYING"
            | "COPYING.LIB"
            | "LICENSE"
    ) {
        return Some("doc");
    }

    // ---- other-known: explicit rules for known, undecided surfaces ----
    // The guest agent (qga): a separate in-guest daemon, not emulator
    // core and not a device.
    if starts_dir(path, "qga") {
        return Some("other-known");
    }
    // NLS message catalogs.
    if starts_dir(path, "po") {
        return Some("other-known");
    }
    // The TCG plugin API surface (plugins/ + include/plugins/).
    if starts_dir(path, "plugins") || starts_dir(path, "include/plugins") {
        return Some("other-known");
    }
    // Authorization backends (SASL).
    if starts_dir(path, "authz") || starts_dir(path, "include/authz") {
        return Some("other-known");
    }
    // Imported kernel headers (virtio/kvm ABIs).
    if starts_dir(path, "include/standard-headers") {
        return Some("other-known");
    }
    // python venv build glue.
    if starts_dir(path, "pyvenv") {
        return Some("other-known");
    }
    // The storage stack behind the devices (block layer, NBD, SCSI
    // helpers, 9p, fsdev, storage daemons). Judgement call: the op6
    // boot path drives hw/ufs + hw/scsi, which are `device`; the block
    // layer behind them is its own surface, decided later. `job.c` /
    // `job-qmp.c` are the block-job machinery those headers declare.
    if in_block_stack(path) {
        return Some("other-known");
    }
    // Firmware blobs + their build glue.
    if path == "pc-bios" || path.starts_with("pc-bios/") {
        return Some("other-known");
    }
    if path == "roms" || path.starts_with("roms/") {
        return Some("other-known");
    }
    // Imported kernel headers.
    if path == "linux-headers" || path.starts_with("linux-headers/") {
        return Some("other-known");
    }
    // Vendored/external imports and wrappers.
    if path.starts_with("subprojects/") {
        return Some("other-known");
    }
    if path == "libdecnumber" || path.starts_with("libdecnumber/") {
        return Some("other-known");
    }
    if path == "include/libdecnumber" || path.starts_with("include/libdecnumber/") {
        return Some("other-known");
    }
    // Host-side tooling, helper programs, contributed tools.
    if path == "scripts" || path.starts_with("scripts/") {
        return Some("other-known");
    }
    if path == "tools" || path.starts_with("tools/") {
        return Some("other-known");
    }
    if path == "contrib" || path.starts_with("contrib/") {
        return Some("other-known");
    }
    if path == "ebpf" || path.starts_with("ebpf/") {
        return Some("other-known");
    }
    if path == "python" || path.starts_with("python/") {
        return Some("other-known");
    }
    // Root QEMU tools (qemu-img, qemu-io, qemu-nbd, edid, keymap, the
    // bridge helper, SASL config).
    if matches!(
        path,
        "qemu-img.c"
            | "qemu-io.c"
            | "qemu-io-cmds.c"
            | "qemu-nbd.c"
            | "qemu-edid.c"
            | "qemu-keymap.c"
            | "qemu-bridge-helper.c"
            | "qemu.sasl"
    ) {
        return Some("other-known");
    }
    // CI pipelines, editor/launcher config, git bookkeeping: every
    // dot-prefixed top-level entry (.gitlab-ci.d/, .github/, .gitignore,
    // .b4-config, .exrc, .gdbinit, …). Tracked repo surfaces.
    if path.starts_with('.') {
        return Some("other-known");
    }

    None
}

/// The OnePlus 6 machine surfaces (fork-only): the machine source, its
/// support files and its docs. Explicit name rules — every entry is a
/// fork addition on top of upstream QEMU (verified at the pin: no
/// upstream QEMU file carries these names; `vmgenid`/`physmem` were
/// checked against this name set and do not collide).
fn is_oneplus6_machine(path: &str) -> bool {
    // The machine's own docs and runner scripts.
    if path == "README.oneplus6.md" || path.starts_with("docs/oneplus6/") {
        return true;
    }
    // hw/ machine + custom device sources and their headers.
    if path.starts_with("hw/") || path.starts_with("include/hw/") {
        return basename_has_op6_name(path);
    }
    false
}

/// The custom-name fragments identifying the fork's device set, matched
/// against the file's BASENAME only (so no directory name can drag an
/// unrelated file into `machine`). Matching is case-insensitive so
/// upstream files cannot collide by substring: at the pin this catches
/// exactly the fork's op6/qcom additions and NOT the upstream ACPI
/// vmgenid device (`vmgenid` contains `geni` case-sensitively).
fn basename_has_op6_name(path: &str) -> bool {
    let name = match path.rsplit_once('/') {
        Some((_, n)) => n,
        None => path,
    };
    const NAMES: [&str; 9] = [
        "oneplus6", "qcom", "rmi4", "geni", "smem", "pdc", "tlmm", "ufshc", "s3706",
    ];
    let lower = name.to_lowercase();
    NAMES.iter().any(|n| lower.contains(n))
}

/// The emulator-core trees (whole-tree rules). Judgement calls, each a
/// whole-tree rule:
/// - `target/` `tcg/` `fpu/` `disas/` — CPU emulation, code generation,
///   softfloat, disassembly: the CPU half of the brief's core bucket.
/// - `system/` `accel/` `qom/` `monitor/` `qapi/` `stubs/` `trace/`
///   `stats/` `dump/` `gdbstub/` `semihosting/` `replay/` `migration/`
///   `plugin/` — the system-emulation runtime around exec.
/// - `util/` `io/` `crypto/` `qobject/` `chardev/` `net/` `audio/`
///   `backends/` — support services the core links.
/// - `rust/` — the in-tree Rust bindings of the core.
/// - `ui/` — the display/input host surface (`ui/input.c` is a T1 file).
/// - `linux-user/` `bsd-user/` `common-user/` — user-mode emulation
///   entry, CPU/OS-ABI surfaces of the core.
/// - `include/` subdirectories pairing with the above, plus the four
///   root headers (`elf.h`, `glib-compat.h`, `qemu-io.h`, `qemu-main.h`).
/// - the root core sources (`cpu-*.c`, `iothread.c`, `page-vary-*.c`,
///   `os-*.c`, `module-common.c`, `event-loop-base.c`, `target-info*.c`,
///   `replication.c`).
fn in_core_tree(path: &str) -> bool {
    if starts_dir(path, "accel")
        || starts_dir(path, "audio")
        || starts_dir(path, "backends")
        || starts_dir(path, "chardev")
        || starts_dir(path, "crypto")
        || starts_dir(path, "disas")
        || starts_dir(path, "dump")
        || starts_dir(path, "gdbstub")
        || starts_dir(path, "io")
        || starts_dir(path, "migration")
        || starts_dir(path, "monitor")
        || starts_dir(path, "net")
        || starts_dir(path, "plugin")
        || starts_dir(path, "qapi")
        || starts_dir(path, "qobject")
        || starts_dir(path, "qom")
        || starts_dir(path, "replay")
        || starts_dir(path, "rust")
        || starts_dir(path, "semihosting")
        || starts_dir(path, "stats")
        || starts_dir(path, "stubs")
        || starts_dir(path, "system")
        || starts_dir(path, "target")
        || starts_dir(path, "tcg")
        || starts_dir(path, "trace")
        || starts_dir(path, "ui")
        || starts_dir(path, "util")
        || starts_dir(path, "fpu")
        || starts_dir(path, "host")
        || starts_dir(path, "linux-user")
        || starts_dir(path, "bsd-user")
        || starts_dir(path, "common-user")
    {
        return true;
    }
    if let Some(rest) = path.strip_prefix("include/") {
        // include/hw/ is device (taken earlier); include/block and
        // include/scsi are the block stack (below); include/standard-headers
        // is an imported-kernel-header surface (below).
        if starts_dir(rest, "accel")
            || starts_dir(rest, "audio")
            || starts_dir(rest, "chardev")
            || starts_dir(rest, "crypto")
            || starts_dir(rest, "disas")
            || starts_dir(rest, "exec")
            || starts_dir(rest, "fpu")
            || starts_dir(rest, "gdbstub")
            || starts_dir(rest, "io")
            || starts_dir(rest, "migration")
            || starts_dir(rest, "monitor")
            || starts_dir(rest, "net")
            || starts_dir(rest, "plugin")
            || starts_dir(rest, "qapi")
            || starts_dir(rest, "qobject")
            || starts_dir(rest, "qemu")
            || starts_dir(rest, "qom")
            || starts_dir(rest, "semihosting")
            || starts_dir(rest, "stats")
            || starts_dir(rest, "system")
            || starts_dir(rest, "tcg")
            || starts_dir(rest, "trace")
            || starts_dir(rest, "ui")
            || starts_dir(rest, "user")
        {
            return true;
        }
        // The four root headers in include/.
        return matches!(
            rest,
            "elf.h" | "glib-compat.h" | "qemu-io.h" | "qemu-main.h"
        );
    }
    // Root core sources.
    matches!(
        path,
        "cpu-common.c"
            | "cpu-target.c"
            | "iothread.c"
            | "event-loop-base.c"
            | "module-common.c"
            | "os-posix.c"
            | "os-win32.c"
            | "os-wasm.c"
            | "page-vary-common.c"
            | "page-vary-system.c"
            | "target-info.c"
            | "target-info-qom.c"
            | "target-info-stub.c"
            | "replication.c"
    )
}

/// The storage stack: block layer, NBD, SCSI helpers, 9p, fsdev, the
/// storage daemons — a known surface outside the emulator core (the
/// devices that use it live under hw/ and are `device`).
fn in_block_stack(path: &str) -> bool {
    starts_dir(path, "block")
        || starts_dir(path, "nbd")
        || starts_dir(path, "scsi")
        || starts_dir(path, "fsdev")
        || starts_dir(path, "storage-daemon")
        || starts_dir(path, "9pfs")
        || matches!(path, "job.c" | "job-qmp.c")
        || starts_dir(path, "include/block")
        || starts_dir(path, "include/scsi")
        || matches!(
            path,
            "block.c" | "blockdev.c" | "blockdev-nbd.c" | "blockjob.c"
        )
}

/// True when `path` is `dir` itself or lives under `dir/`.
fn starts_dir(path: &str, dir: &str) -> bool {
    path == dir || path.starts_with(&format!("{dir}/"))
}
