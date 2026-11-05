//! Rust port of `scripts/phone/build-embedded-efi-v4-wdt.py` - build a
//! RAM-bootable Android image with an EFI app embedded in U-Boot.
//!
//! The port mirrors the Python step for step, in the same order, with
//! the same fail-closed checks:
//!   - CMD_C / ENV overlay pins, DTB sha256 pin (48518e1d...259383)
//!   - payload sha256 verification against --payload-sha256
//!   - payload-receipt freshness guard on --out (a different payload's
//!     receipt refuses; an existing image file refuses - never
//!     overwrite)
//!   - canonical DTS round-trip through dtc
//!   - U-Boot at the pinned commit 3e15c281... in an isolated worktree,
//!     overlay files written only inside that worktree, dirty-path
//!     allowlist enforced
//!   - SOURCE_DATE_EPOCH pinned from the U-Boot commit (reproducible)
//!   - gzip level 9 / mtime 0 byte-identical to Python's gzip module
//!     (see gzipz.rs), DTB appended, mkbootimg v0 packaging
//!   - unpack + symbol + .rodata verification of the embedded payload
//!   - manifest.json receipt (schema identical to the Python one; the
//!     single deliberate divergence: `packager_sha256` hashes this
//!     binary instead of a .py source file)
//!
//! Pinned host tools (mkbootimg, unpack_bootimg, bison/flex/m4) are
//! tools this program RUNS, not code it authors; they stay external,
//! exactly as in the Python packager.

mod gzipz;
mod jsonx;
mod sha256;

use jsonx::Json;
use sha256::{sha256_file, sha256_hex};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const UBOOT_COMMIT: &str = "3e15c2819caa7e14e1e4b2ff758a41cbc58a7311";
const CMD_C_SHA256_PIN: &str = "d882d4f62477fad6613ad2973438a77086566697b3244a62995f0675838fd5d7";
const DTB_SHA256_PIN: &str =
    "48518e1de2b2024aa078f4ba2ce2dfc1e3f9e8923c4a212b320413dbb1259383";
const ENV_SHA256_PIN: &str = "78548f293be3d5071a6a176343dc256efeca53399addb5e23a6842ea953780f4";

const UPSTREAM_UBOOT_REPO: &str = "$HOME/Developer/forks/u-boot";
const PHONE_BOOT: &str = "$HOME/artifacts/handoff/oneplus-redox-20261024/phone-boot";

// Extracted byte-exactly from the Python packager (its CMD_C / ENV
// literals); the pin checks below and the unit tests prove the bytes.
const CMD_C: &str = include_str!("../assets/cmd_c.txt");
const ENV_TEXT: &str = include_str!("../assets/env.txt");

fn die(msg: &str) -> ! {
    eprintln!("{}", msg);
    std::process::exit(1);
}

fn run(argv: &[String], cwd: Option<&Path>, env: Option<&HashMap<String, String>>) {
    println!("+ {}", argv.join(" "));
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    if let Some(e) = env {
        cmd.env_clear();
        cmd.envs(e.iter());
    }
    match cmd.status() {
        Ok(s) if s.success() => {}
        Ok(s) => std::process::exit(s.code().unwrap_or(1)),
        Err(e) => die(&format!("failed to spawn {}: {}", argv[0], e)),
    }
}

struct Captured {
    rc: i32,
    stdout: String,
    stderr: String,
}

fn capture(argv: &[String], cwd: Option<&Path>) -> Captured {
    let out = Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(cwd.map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from(".")))
        .stdin(Stdio::null())
        .output();
    match out {
        Ok(o) => Captured {
            rc: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        },
        Err(e) => die(&format!("failed to spawn {}: {}", argv[0], e)),
    }
}

fn capture_ok(argv: &[String], cwd: Option<&Path>) -> String {
    let c = capture(argv, cwd);
    if c.rc != 0 {
        std::process::exit(if c.rc > 0 { c.rc } else { 1 });
    }
    c.stdout
}

fn which(prog: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let cand = dir.join(prog);
        if cand.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(m) = cand.metadata() {
                    if m.permissions().mode() & 0o111 != 0 {
                        return Some(cand);
                    }
                }
            }
        }
    }
    None
}

/// Measured /config/bootdelay probe (fail-closed; mirrors the Python).
fn dtb_config_bootdelay_override(fdtget: &Path, dtb: &Path, label: &str) -> bool {
    let c = capture(
        &[
            fdtget.to_string_lossy().into_owned(),
            "-p".into(),
            dtb.to_string_lossy().into_owned(),
            "/config".into(),
        ],
        None,
    );
    if c.rc == 0 {
        return c.stdout.lines().any(|l| l == "bootdelay");
    }
    if c.stderr.contains("FDT_ERR_NOTFOUND") {
        return false;
    }
    die(&format!(
        "{}: fdtget /config probe failed (rc={}): {} - cannot verify, aborting",
        label,
        c.rc,
        c.stderr.trim()
    ));
}

/// The payload .incbin assembly, byte-identical to the Python f-string.
fn payload_asm(payload_abs: &str) -> String {
    let esc = payload_abs.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "/* SPDX-License-Identifier: GPL-2.0+ */\n.section .rodata\n.balign 16\n.global embedded_efi_start\n.global embedded_efi_end\nembedded_efi_start:\n.incbin \"{}\"\nembedded_efi_end:\n",
        esc
    )
}

/// Environment sanity (mirrors the Python generated-env check).
fn env_sanity_check(env_text: &str) -> bool {
    let mut values: HashMap<&str, &str> = HashMap::new();
    for line in env_text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            values.insert(k, v);
        }
    }
    values.get("bootdelay") == Some(&"-2")
        && values.get("bootcmd")
            == Some(&"wdt dev watchdog@17980000; wdt start 300000; op6efi")
        && values
            .get("preboot")
            .map(|p| p.starts_with("echo OP6_EFI_DIAG_NOABORT_V2; printenv"))
            .unwrap_or(false)
}

/// Payload-receipt freshness decision (pure): given the prior
/// manifest's (payload_kind, payload_sha256) and the current payload
/// sha, refuse when the receipt names a different payload.
fn freshness_refusal(prior: Option<(&str, &str)>, payload_sha: &str) -> Option<String> {
    if let Some((kind, sha)) = prior {
        if kind != "redox-v4-reset" || sha != payload_sha {
            return Some("different payload receipt".into());
        }
    }
    None
}

/// Remove the gki certificate import line from mkbootimg (the Python
/// used a MULTILINE regex sub; line-drop is equivalent for this file).
fn strip_gki_import(source: &str) -> String {
    let target = "from gki.generate_gki_certificate import generate_gki_certificate";
    let mut out = String::with_capacity(source.len());
    for line in source.split_inclusive('\n') {
        if line.trim_end() == target {
            continue;
        }
        out.push_str(line);
    }
    out
}

fn mkbootimg_args(mkbootimg: &Path, kernel: &Path, image: &Path) -> Vec<String> {
    vec![
        mkbootimg.to_string_lossy().into_owned(),
        "--kernel".into(),
        kernel.to_string_lossy().into_owned(),
        "--output".into(),
        image.to_string_lossy().into_owned(),
        "--pagesize".into(),
        "4096".into(),
        "--base".into(),
        "0".into(),
        "--kernel_offset".into(),
        "0x8000".into(),
        "--ramdisk_offset".into(),
        "0x01000000".into(),
        "--second_offset".into(),
        "0x00f00000".into(),
        "--tags_offset".into(),
        "0x100".into(),
        "--header_version".into(),
        "0".into(),
    ]
}

fn parse_nm_symbols(nm_out: &str) -> BTreeMap<String, u64> {
    let want: HashSet<&str> = [
        "embedded_efi_start",
        "embedded_efi_end",
        "__image_copy_start",
        "__image_copy_end",
    ]
    .into_iter()
    .collect();
    let mut map = BTreeMap::new();
    for line in nm_out.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() >= 3 && want.contains(fields[fields.len() - 1]) {
            if let Ok(v) = u64::from_str_radix(fields[0], 16) {
                map.insert(fields[fields.len() - 1].to_string(), v);
            }
        }
    }
    map
}

fn parse_rodata_range(map_text: &str) -> Option<(u64, u64)> {
    for line in map_text.lines() {
        if let Some(rest) = line.strip_prefix(".rodata") {
            if !rest.chars().next().map(|c| c.is_whitespace()).unwrap_or(false) {
                continue;
            }
            let toks: Vec<&str> = rest.split_whitespace().collect();
            if toks.len() >= 2 {
                let a = toks[0].strip_prefix("0x")?;
                let b = toks[1].strip_prefix("0x")?;
                if a.chars().all(|c| c.is_ascii_hexdigit())
                    && b.chars().all(|c| c.is_ascii_hexdigit())
                {
                    return Some((
                        u64::from_str_radix(a, 16).ok()?,
                        u64::from_str_radix(b, 16).ok()?,
                    ));
                }
            }
        }
    }
    None
}

/// Find the first offset of needle in haystack (prefix-filtered scan).
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    let probe = &needle[..std::cmp::min(64, needle.len())];
    haystack
        .windows(probe.len())
        .position(|w| w == probe)
        .and_then(|base| {
            let mut pos = base;
            loop {
                if haystack[pos..].starts_with(needle) {
                    return Some(pos);
                }
                // next candidate after pos
                let next = haystack[pos + 1..]
                    .windows(probe.len())
                    .position(|w| w == probe)?;
                pos = pos + 1 + next;
            }
        })
}

fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    let mut count = 0;
    let mut from = 0;
    while let Some(off) = find_subslice(&haystack[from..], needle) {
        count += 1;
        from += off + 1;
        if count > 1 {
            break;
        }
    }
    count
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

struct Args {
    payload_path: PathBuf,
    payload_sha256: String,
    out: Option<PathBuf>,
    project: Option<PathBuf>,
}

fn parse_args() -> Args {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut payload_path: Option<PathBuf> = None;
    let mut payload_sha256: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut project: Option<PathBuf> = None;
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--payload-path" => {
                i += 1;
                payload_path = Some(PathBuf::from(argv.get(i).unwrap_or_else(|| die("--payload-path needs a value"))));
            }
            "--payload-sha256" => {
                i += 1;
                payload_sha256 = Some(argv.get(i).unwrap_or_else(|| die("--payload-sha256 needs a value")).clone());
            }
            "--out" => {
                i += 1;
                out = Some(PathBuf::from(argv.get(i).unwrap_or_else(|| die("--out needs a value"))));
            }
            "--project" => {
                i += 1;
                project = Some(PathBuf::from(argv.get(i).unwrap_or_else(|| die("--project needs a value"))));
            }
            "-h" | "--help" => {
                println!("usage: packager --payload-path <efi> --payload-sha256 <hex> [--out <dir>] [--project <repo root>]");
                std::process::exit(0);
            }
            other => die(&format!("unknown argument: {}", other)),
        }
        i += 1;
    }
    Args {
        payload_path: payload_path.unwrap_or_else(|| die("--payload-path is required")),
        payload_sha256: payload_sha256.unwrap_or_else(|| die("--payload-sha256 is required")),
        out,
        project,
    }
}

fn main() {
    let args = parse_args();
    let project = match &args.project {
        Some(p) => p.clone(),
        None => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| die("cannot read cwd"));
            if cwd.join("scripts/phone/embedded-efi/baseline.config").is_file() {
                cwd
            } else {
                die("--project not given and the cwd is not the repo root (no scripts/phone/embedded-efi/baseline.config)");
            }
        }
    };
    let project = project
        .canonicalize()
        .unwrap_or_else(|_| die("project path cannot be resolved"));
    let artifact = project.join("out/r1-reset-v4");
    let source = artifact.join("src");
    let source_repo = artifact.join("u-boot-repo");
    let inputs = project.join("scripts/phone/embedded-efi");
    let base_config = inputs.join("baseline.config");
    let dtb_source = inputs.join("sdm845-oneplus-enchilada.dts");
    let dtb = inputs.join("sdm845-oneplus-enchilada.dtb");
    let host_tools = Path::new(PHONE_BOOT).join("host-tools/usr/bin");
    let phone_boot = Path::new(PHONE_BOOT);

    // --- pins on the generated overlay literals ---
    if sha256_hex(CMD_C.as_bytes()) != CMD_C_SHA256_PIN {
        die("op6efi C glue drifted from the pinned Attempt-#1 overlay (CMD_C_SHA256_PIN)");
    }
    if sha256_hex(ENV_TEXT.as_bytes()) != ENV_SHA256_PIN {
        die("boot environment drifted from the pinned Attempt-#1 overlay (ENV_SHA256_PIN)");
    }

    // --- payload verification ---
    let payload = args
        .payload_path
        .canonicalize()
        .unwrap_or_else(|_| die(&format!("missing EFI payload: {}", args.payload_path.display())));
    if !payload.is_file() {
        die(&format!("missing EFI payload: {}", payload.display()));
    }
    let payload_sha = sha256_file(&payload).unwrap_or_else(|e| die(&format!("cannot hash payload: {}", e)));
    if payload_sha != args.payload_sha256 {
        die(&format!(
            "EFI payload SHA-256 mismatch: expected {}, got {}",
            args.payload_sha256, payload_sha
        ));
    }

    // --- required inputs + DTB pin ---
    for p in [
        &base_config,
        &dtb,
        &dtb_source,
        &host_tools.join("mkbootimg"),
        &host_tools.join("unpack_bootimg"),
    ] {
        if !p.exists() {
            die(&format!("required input missing: {}", p.display()));
        }
    }
    let dtb_sha = sha256_file(&dtb).unwrap_or_else(|e| die(&format!("cannot hash DTB: {}", e)));
    if dtb_sha != DTB_SHA256_PIN {
        die(&format!("phone DTB drifted from DTB_SHA256_PIN: {}", dtb_sha));
    }
    let dtc = which("dtc").unwrap_or_else(|| die("dtc is required to verify the reconstructed phone-DTB source"));
    let fdtget = match which("fdtget") {
        Some(f) => f,
        None => die(&format!("phone DTB is not confirmed as OnePlus 6: {}", dtb.display())),
    };
    let model = capture(
        &[s(&fdtget), s(&dtb), "/".into(), "model".into()],
        None,
    );
    if model.rc != 0 || model.stdout.trim() != "OnePlus 6" {
        die(&format!("phone DTB is not confirmed as OnePlus 6: {}", dtb.display()));
    }
    let phone_dtb_override = dtb_config_bootdelay_override(&fdtget, &dtb, "phone DTB");
    if phone_dtb_override {
        die("phone DTB /config/bootdelay overrides the no-abort autoboot setting");
    }

    // --- U-Boot source worktree at the pinned commit ---
    std::fs::create_dir_all(&artifact).unwrap_or_else(|e| die(&format!("cannot create artifact dir: {}", e)));
    if !source_repo.exists() {
        run(
            &[
                "git".into(),
                "clone".into(),
                "--shared".into(),
                "--no-checkout".into(),
                UPSTREAM_UBOOT_REPO.into(),
                s(&source_repo),
            ],
            None,
            None,
        );
    }
    if !source.exists() {
        run(
            &[
                "git".into(),
                "-C".into(),
                s(&source_repo),
                "worktree".into(),
                "add".into(),
                "--detach".into(),
                s(&source),
                UBOOT_COMMIT.into(),
            ],
            None,
            None,
        );
    }
    if !source.join(".git").exists() {
        die(&format!("refusing non-worktree source path: {}", source.display()));
    }
    let actual = capture_ok(
        &["git".into(), "-C".into(), s(&source), "rev-parse".into(), "HEAD".into()],
        None,
    )
    .trim()
    .to_string();
    if actual != UBOOT_COMMIT {
        die(&format!("U-Boot worktree must stay at {}; found {}", UBOOT_COMMIT, actual));
    }
    let allowed_dirty: HashSet<&str> = [
        "cmd/Makefile",
        "board/qualcomm/qcom-phone-embedded-efi.env",
        "cmd/op6efi-payload.S",
        "cmd/op6efi.c",
    ]
    .into_iter()
    .collect();
    let status = capture_ok(
        &[
            "git".into(),
            "-C".into(),
            s(&source),
            "status".into(),
            "--porcelain".into(),
            "--untracked-files=all".into(),
        ],
        None,
    );
    let dirty: HashSet<String> = status
        .lines()
        .map(|l| l.chars().skip(3).collect::<String>())
        .collect();
    let extra: Vec<&String> = dirty
        .iter()
        .filter(|p| !allowed_dirty.contains(p.as_str()))
        .collect();
    if !extra.is_empty() {
        let mut names: Vec<String> = extra.iter().map(|p| format!("'{}'", p)).collect();
        names.sort();
        die(&format!("refusing unrelated worktree changes: [{}]", names.join(", ")));
    }
    let original_makefile = capture_ok(
        &[
            "git".into(),
            "-C".into(),
            s(&source),
            "show".into(),
            format!("{}:cmd/Makefile", UBOOT_COMMIT),
        ],
        None,
    );
    let anchor = "obj-$(CONFIG_CMD_BOOTEFI) += bootefi.o\n";
    let expected_makefile = original_makefile.replacen(
        anchor,
        "obj-$(CONFIG_CMD_BOOTEFI) += bootefi.o\nobj-$(CONFIG_CMD_BOOTEFI) += op6efi.o op6efi-payload.o\n",
        1,
    );
    let makefile_path = source.join("cmd/Makefile");
    let current_makefile =
        std::fs::read_to_string(&makefile_path).unwrap_or_else(|e| die(&format!("cannot read cmd/Makefile: {}", e)));
    if current_makefile != original_makefile && current_makefile != expected_makefile {
        die("refusing an unexpected cmd/Makefile edit in U-Boot worktree");
    }

    // --- out dir + payload-receipt freshness guard ---
    let out = match &args.out {
        Some(o) => {
            if o.is_absolute() {
                o.clone()
            } else {
                std::env::current_dir().unwrap_or_else(|_| die("cannot read cwd")).join(o)
            }
        }
        None => artifact.join("package"),
    };
    let out = out
        .canonicalize()
        .unwrap_or_else(|_| {
            std::fs::create_dir_all(&out).unwrap_or_else(|e| die(&format!("cannot create --out: {}", e)));
            out.canonicalize().unwrap_or_else(|_| die("cannot resolve --out"))
        });
    std::fs::create_dir_all(&out).unwrap_or_else(|e| die(&format!("cannot create --out: {}", e)));
    let image_path = out.join("op6-embedded-efi-redox-v4-reset.img");
    let prior_manifest = out.join("manifest.json");
    if prior_manifest.is_file() {
        let prior_text = std::fs::read_to_string(&prior_manifest)
            .unwrap_or_else(|e| die(&format!("cannot read prior manifest: {}", e)));
        let kind = jsonx::extract_string_field(&prior_text, "payload_kind").unwrap_or_default();
        let sha = jsonx::extract_string_field(&prior_text, "payload_sha256").unwrap_or_default();
        if freshness_refusal(Some((&kind, &sha)), &payload_sha).is_some() {
            die(&format!(
                "output contains a different payload receipt; choose a fresh --out: {}",
                out.display()
            ));
        }
    }
    if image_path.exists() {
        die(&format!(
            "refusing to overwrite an existing image; choose a fresh --out: {}",
            image_path.display()
        ));
    }

    // --- canonical DTS round-trip ---
    let roundtrip_dtb = out.join("phone-dtb-source-roundtrip.dtb");
    let roundtrip_dts = out.join("phone-dtb-source-roundtrip.dts");
    run(
        &[s(&dtc), "-I".into(), "dts".into(), "-O".into(), "dtb".into(), "-o".into(), s(&roundtrip_dtb), s(&dtb_source)],
        None,
        None,
    );
    run(
        &[s(&dtc), "-I".into(), "dtb".into(), "-O".into(), "dts".into(), "-o".into(), s(&roundtrip_dts), s(&roundtrip_dtb)],
        None,
        None,
    );
    let rt = std::fs::read(&roundtrip_dts).unwrap_or_else(|e| die(&format!("cannot read roundtrip dts: {}", e)));
    let dts_bytes = std::fs::read(&dtb_source).unwrap_or_else(|e| die(&format!("cannot read DTB source: {}", e)));
    if rt != dts_bytes {
        die("reconstructed phone DTS does not round-trip canonically through dtc");
    }

    // --- overlay writes inside the U-Boot worktree ---
    std::fs::write(source.join("cmd/op6efi.c"), CMD_C.as_bytes())
        .unwrap_or_else(|e| die(&format!("cannot write op6efi.c: {}", e)));
    let asm_text = payload_asm(&s(&payload));
    let asm_path = source.join("cmd/op6efi-payload.S");
    std::fs::write(&asm_path, asm_text.as_bytes())
        .unwrap_or_else(|e| die(&format!("cannot write op6efi-payload.S: {}", e)));
    let mktext = std::fs::read_to_string(&makefile_path)
        .unwrap_or_else(|e| die(&format!("cannot read cmd/Makefile: {}", e)));
    if !mktext.contains("obj-$(CONFIG_CMD_BOOTEFI) += op6efi.o op6efi-payload.o") {
        if !mktext.contains(anchor) {
            die("cannot locate cmd/Makefile integration point");
        }
        std::fs::write(
            &makefile_path,
            mktext.replacen(
                anchor,
                "obj-$(CONFIG_CMD_BOOTEFI) += bootefi.o\nobj-$(CONFIG_CMD_BOOTEFI) += op6efi.o op6efi-payload.o\n",
                1,
            ),
        )
        .unwrap_or_else(|e| die(&format!("cannot write cmd/Makefile: {}", e)));
    }
    let envfile = source.join("board/qualcomm/qcom-phone-embedded-efi.env");
    std::fs::write(&envfile, ENV_TEXT.as_bytes())
        .unwrap_or_else(|e| die(&format!("cannot write env file: {}", e)));
    if !env_sanity_check(ENV_TEXT) {
        die("generated environment must autoboot op6efi with bootdelay=-2");
    }

    // --- configure ---
    let config = out.join(".config");
    std::fs::copy(&base_config, &config)
        .unwrap_or_else(|e| die(&format!("cannot copy baseline config: {}", e)));
    let scripts_config = source.join("scripts/config");
    run(
        &[
            s(&scripts_config),
            "--file".into(),
            s(&config),
            "--set-str".into(),
            "ENV_DEFAULT_ENV_TEXT_FILE".into(),
            "board/qualcomm/qcom-phone-embedded-efi.env".into(),
        ],
        None,
        None,
    );
    run(
        &[
            s(&scripts_config),
            "--file".into(),
            s(&config),
            "--set-val".into(),
            "BOOTDELAY".into(),
            "-2".into(),
        ],
        None,
        None,
    );

    // --- host environment (pinned tools + reproducibility epoch) ---
    let mut hostenv: HashMap<String, String> = std::env::vars().collect();
    let old_path = hostenv.get("PATH").cloned().unwrap_or_default();
    hostenv.insert("PATH".into(), format!("{}:{}", s(&host_tools), old_path));
    hostenv.insert(
        "BISON_PKGDATADIR".into(),
        s(&phone_boot.join("host-tools/usr/share/bison")),
    );
    hostenv.insert("M4".into(), s(&host_tools.join("m4")));
    let epoch = capture_ok(
        &[
            "git".into(),
            "-C".into(),
            s(&source),
            "show".into(),
            "-s".into(),
            "--format=%ct".into(),
            UBOOT_COMMIT.into(),
        ],
        None,
    )
    .trim()
    .to_string();
    hostenv.insert("SOURCE_DATE_EPOCH".into(), epoch.clone());

    run(
        &[
            "make".into(),
            format!("O={}", s(&out)),
            "ARCH=arm".into(),
            "CROSS_COMPILE=aarch64-linux-gnu-".into(),
            "olddefconfig".into(),
        ],
        Some(&source),
        Some(&hostenv),
    );
    let cfg = std::fs::read_to_string(&config)
        .unwrap_or_else(|e| die(&format!("cannot read built .config: {}", e)));
    for required in [
        "CONFIG_BOOTDELAY=-2",
        "CONFIG_CMD_BOOTEFI=y",
        "CONFIG_ENV_DEFAULT_ENV_TEXT_FILE=\"board/qualcomm/qcom-phone-embedded-efi.env\"",
    ] {
        if !cfg.contains(required) {
            die(&format!("configuration did not retain {:?}", required));
        }
    }
    run(
        &[
            "make".into(),
            format!("O={}", s(&out)),
            "ARCH=arm".into(),
            "CROSS_COMPILE=aarch64-linux-gnu-".into(),
            "-j8".into(),
        ],
        Some(&source),
        Some(&hostenv),
    );

    // --- built DTB bootdelay probe ---
    let built_dtb = out.join("u-boot.dtb");
    if !built_dtb.is_file() {
        die(&format!(
            "built U-Boot DTB missing; /config/bootdelay check cannot run: {}",
            built_dtb.display()
        ));
    }
    let built_dtb_override = dtb_config_bootdelay_override(&fdtget, &built_dtb, "built U-Boot DTB");
    if built_dtb_override {
        die("built U-Boot DTB /config/bootdelay overrides the no-abort setting");
    }

    // --- gzip + DTB payload assembly ---
    let nodtb_path = out.join("u-boot-nodtb.bin");
    let nodtb = std::fs::read(&nodtb_path)
        .unwrap_or_else(|e| die(&format!("cannot read u-boot-nodtb.bin: {}", e)));
    let gz = gzipz::gzip_level9_mtime0(&nodtb)
        .unwrap_or_else(|e| die(&format!("gzip failed: {}", e)));
    let gzip_path = out.join("u-boot-nodtb.bin.gz");
    std::fs::write(&gzip_path, &gz).unwrap_or_else(|e| die(&format!("cannot write gzip: {}", e)));
    let dtb_bytes = std::fs::read(&dtb).unwrap_or_else(|e| die(&format!("cannot read DTB: {}", e)));
    let mut payload_with_dtb = gz.clone();
    payload_with_dtb.extend_from_slice(&dtb_bytes);
    let payload_with_dtb_path = out.join("u-boot-nodtb.bin.gz-dtb");
    std::fs::write(&payload_with_dtb_path, &payload_with_dtb)
        .unwrap_or_else(|e| die(&format!("cannot write gz-dtb: {}", e)));

    // --- mkbootimg packaging ---
    let mkbootimg_src = std::fs::read_to_string(host_tools.join("mkbootimg"))
        .unwrap_or_else(|e| die(&format!("cannot read pinned mkbootimg: {}", e)));
    let mkbootimg_path = out.join("mkbootimg.py");
    std::fs::write(&mkbootimg_path, strip_gki_import(&mkbootimg_src))
        .unwrap_or_else(|e| die(&format!("cannot write mkbootimg.py: {}", e)));
    let python3 = if Path::new("/usr/bin/python3").exists() {
        "/usr/bin/python3".to_string()
    } else {
        s(&which("python3").unwrap_or_else(|| die("python3 not found")))
    };
    let mut mk_args = vec![python3];
    mk_args.extend(mkbootimg_args(&mkbootimg_path, &payload_with_dtb_path, &image_path));
    run(&mk_args, None, Some(&hostenv));

    // --- unpack verification ---
    let unpack = out.join("unpacked");
    std::fs::create_dir_all(&unpack).unwrap_or_else(|e| die(&format!("cannot create unpacked dir: {}", e)));
    let info = capture_ok(
        &[
            s(&host_tools.join("unpack_bootimg")),
            "--boot_img".into(),
            s(&image_path),
            "--out".into(),
            s(&unpack),
        ],
        None,
    );
    let extracted = unpack.join("kernel");
    if !extracted.is_file()
        || std::fs::read(&extracted).unwrap_or_default() != payload_with_dtb
    {
        die("unpacked Android kernel payload differs from packaged gzip+DTB");
    }

    // --- embedded payload verification ---
    let blob = std::fs::read(&payload).unwrap_or_else(|e| die(&format!("cannot read payload: {}", e)));
    let offset = match find_subslice(&nodtb, &blob) {
        Some(o) if count_occurrences(&nodtb, &blob) == 1 => o,
        _ => die("embedded EFI blob not found exactly once in U-Boot image"),
    };
    let syms_out = capture_ok(
        &["aarch64-linux-gnu-nm".into(), "-n".into(), s(&out.join("u-boot"))],
        None,
    );
    let symmap = parse_nm_symbols(&syms_out);
    let (start, end) = (
        symmap.get("embedded_efi_start").copied(),
        symmap.get("embedded_efi_end").copied(),
    );
    let (start, end) = match (start, end) {
        (Some(a), Some(b)) if b - a == blob.len() as u64 => (a, b),
        _ => die(&format!("bad embedded payload symbols: {:?}", symmap)),
    };
    let (image_start, image_end) = (
        symmap.get("__image_copy_start").copied(),
        symmap.get("__image_copy_end").copied(),
    );
    let (image_start, image_end) = match (image_start, image_end) {
        (Some(a), Some(b)) if start >= a && end <= b => (a, b),
        _ => die(&format!(
            "EFI symbols lie outside the linked U-Boot image: {:?}",
            symmap
        )),
    };
    if start - image_start != offset as u64 || end - image_start > nodtb.len() as u64 {
        die("linked EFI symbol offsets do not match embedded bytes in u-boot-nodtb.bin");
    }
    let map_text = std::fs::read_to_string(out.join("u-boot.map"))
        .unwrap_or_else(|e| die(&format!("cannot read u-boot.map: {}", e)));
    let (ro_start, ro_size) = parse_rodata_range(&map_text)
        .unwrap_or_else(|| die("could not find .rodata bounds in U-Boot linker map"));
    if start < ro_start || end > ro_start + ro_size {
        die("embedded EFI symbols lie outside .rodata");
    }
    if nodtb[offset..offset + blob.len()] != blob[..] {
        die("embedded EFI differs from source");
    }

    // --- manifest receipt ---
    let asm_bytes = std::fs::read(&asm_path).unwrap_or_default();
    let env_bytes = std::fs::read(&envfile).unwrap_or_default();
    let makefile_bytes = std::fs::read(&makefile_path).unwrap_or_default();
    let mut overlay = Vec::new();
    overlay.extend_from_slice(CMD_C.as_bytes());
    overlay.extend_from_slice(&asm_bytes);
    overlay.extend_from_slice(&env_bytes);
    overlay.extend_from_slice(&makefile_bytes);
    let exe = std::env::current_exe().unwrap_or_else(|_| die("cannot locate own executable"));
    let packager_sha = sha256_file(&exe).unwrap_or_else(|e| die(&format!("cannot hash own binary: {}", e)));
    let manifest = Json::Obj(vec![
        ("u_boot_commit".into(), Json::Str(actual.clone())),
        (
            "source_date_epoch".into(),
            Json::Int(epoch.parse::<i64>().unwrap_or_else(|_| die("bad SOURCE_DATE_EPOCH"))),
        ),
        ("u_boot_source_repo".into(), Json::Str(s(&source_repo))),
        ("packager_sha256".into(), Json::Str(packager_sha)),
        (
            "project_inputs".into(),
            Json::Obj(vec![
                (
                    "baseline_config_sha256".into(),
                    Json::Str(sha256_file(&base_config).unwrap_or_default()),
                ),
                ("phone_dtb_sha256".into(), Json::Str(dtb_sha.clone())),
                (
                    "phone_dtb_dts_sha256".into(),
                    Json::Str(sha256_file(&dtb_source).unwrap_or_default()),
                ),
                ("phone_dtb_source_canonical_roundtrip".into(), Json::Bool(true)),
                (
                    "command_source_sha256".into(),
                    Json::Str(sha256_hex(CMD_C.as_bytes())),
                ),
                (
                    "payload_assembly_sha256".into(),
                    Json::Str(sha256_hex(&asm_bytes)),
                ),
                (
                    "boot_environment_sha256".into(),
                    Json::Str(sha256_hex(&env_bytes)),
                ),
            ]),
        ),
        ("efi_overlay_sha256".into(), Json::Str(sha256_hex(&overlay))),
        (
            "base_config_sha256".into(),
            Json::Str(sha256_file(&base_config).unwrap_or_default()),
        ),
        ("phone_dtb_sha256".into(), Json::Str(dtb_sha.clone())),
        ("phone_dtb_model".into(), Json::Str("OnePlus 6".into())),
        ("phone_dtb_config_bootdelay_override".into(), Json::Bool(phone_dtb_override)),
        ("built_dtb_config_bootdelay_override".into(), Json::Bool(built_dtb_override)),
        ("bootdelay_environment".into(), Json::Str("-2".into())),
        ("bootdelay_config".into(), Json::Str("CONFIG_BOOTDELAY=-2".into())),
        (
            "phone_dtb_rebuild_provenance".into(),
            Json::Str("source/build recipe unavailable; pinned checked-in DTB input".into()),
        ),
        (
            "built_config_sha256".into(),
            Json::Str(sha256_file(&config).unwrap_or_default()),
        ),
        ("payload_kind".into(), Json::Str("redox-v4-reset".into())),
        ("payload_path".into(), Json::Str(s(&payload))),
        ("payload_sha256".into(), Json::Str(payload_sha.clone())),
        ("payload_size".into(), Json::Int(blob.len() as i64)),
        ("embedded_symbol_start".into(), Json::Str(format!("{:#x}", start))),
        ("embedded_symbol_end".into(), Json::Str(format!("{:#x}", end))),
        (
            "linked_image_range".into(),
            Json::Arr(vec![
                Json::Str(format!("{:#x}", image_start)),
                Json::Str(format!("{:#x}", image_end)),
            ]),
        ),
        (
            "rodata_range".into(),
            Json::Arr(vec![
                Json::Str(format!("{:#x}", ro_start)),
                Json::Str(format!("{:#x}", ro_start + ro_size)),
            ]),
        ),
        ("embedded_file_offset_nodtb".into(), Json::Int(offset as i64)),
        ("dtb_sha256".into(), Json::Str(dtb_sha.clone())),
        (
            "android_image_sha256".into(),
            Json::Str(sha256_file(&image_path).unwrap_or_default()),
        ),
        (
            "android_header_inspection".into(),
            Json::Str(info.trim().to_string()),
        ),
        ("build_directory".into(), Json::Str(s(&out))),
        ("physical_boot_or_flash_performed".into(), Json::Bool(false)),
    ]);
    let manifest_text = jsonx::dumps_indent2(&manifest);
    std::fs::write(out.join("manifest.json"), format!("{}\n", manifest_text))
        .unwrap_or_else(|e| die(&format!("cannot write manifest: {}", e)));
    println!("{}", manifest_text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_pins_match_extracted_assets() {
        assert_eq!(sha256_hex(CMD_C.as_bytes()), CMD_C_SHA256_PIN);
        assert_eq!(sha256_hex(ENV_TEXT.as_bytes()), ENV_SHA256_PIN);
    }

    #[test]
    fn payload_asm_shape_and_escaping() {
        let asm = payload_asm("/tmp/a b/efi.efi");
        assert!(asm.contains(".incbin \"/tmp/a b/efi.efi\""));
        assert!(asm.starts_with("/* SPDX-License-Identifier: GPL-2.0+ */\n.section .rodata\n"));
        assert!(asm.ends_with("embedded_efi_end:\n"));
        let esc = payload_asm("/we\"ird\\path.efi");
        assert!(esc.contains(".incbin \"/we\\\"ird\\\\path.efi\""));
    }

    #[test]
    fn env_sanity_accepts_the_pinned_env_and_rejects_drift() {
        assert!(env_sanity_check(ENV_TEXT));
        let tampered = ENV_TEXT.replace("bootdelay=-2", "bootdelay=3");
        assert!(!env_sanity_check(&tampered));
        let tampered2 = ENV_TEXT.replace("wdt start 300000; ", "");
        assert!(!env_sanity_check(&tampered2));
    }

    #[test]
    fn freshness_decision() {
        assert!(freshness_refusal(None, "aa").is_none());
        assert!(freshness_refusal(Some(("redox-v4-reset", "aa")), "aa").is_none());
        assert!(freshness_refusal(Some(("redox-v4-reset", "bb")), "aa").is_some());
        assert!(freshness_refusal(Some(("other-kind", "aa")), "aa").is_some());
    }

    #[test]
    fn mkbootimg_argument_assembly() {
        let args = mkbootimg_args(
            Path::new("/out/mkbootimg.py"),
            Path::new("/out/gz-dtb"),
            Path::new("/out/img"),
        );
        assert_eq!(
            args,
            vec![
                "/out/mkbootimg.py", "--kernel", "/out/gz-dtb", "--output", "/out/img",
                "--pagesize", "4096", "--base", "0", "--kernel_offset", "0x8000",
                "--ramdisk_offset", "0x01000000", "--second_offset", "0x00f00000",
                "--tags_offset", "0x100", "--header_version", "0"
            ]
        );
    }

    #[test]
    fn strip_gki_import_line() {
        let src = "import os\nfrom gki.generate_gki_certificate import generate_gki_certificate\nimport sys\n";
        assert_eq!(strip_gki_import(src), "import os\nimport sys\n");
        let untouched = "import os\nimport sys\n";
        assert_eq!(strip_gki_import(untouched), untouched);
    }

    #[test]
    fn nm_and_rodata_parsing() {
        let nm = "00000000000bd000 R embedded_efi_start\n0000000000144be00 R embedded_efi_end\n0000000000000000 T __image_copy_start\n";
        let m = parse_nm_symbols(nm);
        assert_eq!(m["embedded_efi_start"], 0xbd000);
        assert_eq!(m["embedded_efi_end"], 0x144be00);
        let map = ".rodata         0x00000000000bd000   0x13d9bb2\n .rodata.foo    0x1 0x2\n";
        // first line does not start at column 0 in real maps for the
        // bare section in this synthetic sample; use a col-0 sample:
        let map2 = ".rodata 0x00000000000bd000 0x13d9bb2\n";
        assert_eq!(parse_rodata_range(map2), Some((0xbd000, 0x13d9bb2)));
        let _ = map;
    }

    #[test]
    fn subslice_search() {
        let h = b"xxabcdefyyabcdefzz";
        assert_eq!(find_subslice(h, b"abcdef"), Some(2));
        assert_eq!(count_occurrences(h, b"abcdef"), 2);
        assert_eq!(find_subslice(h, b"nope"), None);
    }
}
