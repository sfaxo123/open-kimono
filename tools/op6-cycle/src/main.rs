//! op6-cycle - one hands-free OnePlus 6 RAM-boot cycle.
//!
//! Rust port of scripts/mac/op6-cycle.sh, drop-in: identical CLI flags,
//! exit codes (0 PASS, 1 ABORTED_NO_FASTBOOT / BOOT_CMD_FAILED,
//! 2 CRASHDUMP, 3 HUNG, 4 unattended-refusal, 5 FASTBOOT, 64 usage),
//! allowlist semantics, receipt JSON shape, and refusal behaviour, so
//! the existing Mac test suite passes against it unmodified.
//!
//! pmOS -> reboot bootloader -> ONE RAM-only `fastboot boot` ->
//! classify the USB outcome -> JSON receipt. Nothing is flashed.
//!
//! The control flow below follows the bash function-for-function;
//! comments flag each mirror. Pure logic lives in core.rs.

mod core;
mod sha256;
mod utc;

use core::{esc, find_entry, ledger_clean, parse_allowlist, receipt_json, refusal_json,
           ttl_value, FbDetector, Poll, ReceiptFields};
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SERIAL_DEFAULT: &str = "0a1b2c3d";
const CAPACITY_CMD: &str = "cat /sys/class/power_supply/*/capacity 2>/dev/null | head -1";

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn sleep_secs(s: i64) {
    std::thread::sleep(Duration::from_secs(s.max(0) as u64));
}

/// Bash `${VAR:-default}`: unset OR empty falls back.
fn env_or(key: &str, def: &str) -> String {
    match env::var(key) {
        Ok(v) if !v.is_empty() => v,
        _ => def.to_string(),
    }
}

fn env_nonempty(key: &str) -> Option<String> {
    match env::var(key) {
        Ok(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}

fn env_i64(key: &str, def: i64) -> i64 {
    env_nonempty(key)
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(def)
}

// ---------------------------------------------------------------------------
// Tee logger: the bash tool runs under `exec > >(tee -a "$LOG") 2>&1`, so
// its own lines and the inherited output of `fastboot boot` / the pmOS
// reboot land in both the terminal and the run log. say() mirrors the
// echo lines; run_tee() mirrors the inherited subprocess output.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Tee {
    file: Option<Arc<Mutex<fs::File>>>,
}

impl Tee {
    fn say(&self, line: &str) {
        println!("{}", line);
        let _ = std::io::stdout().flush();
        if let Some(f) = &self.file {
            if let Ok(mut g) = f.lock() {
                let _ = writeln!(g, "{}", line);
            }
        }
    }

    fn write_stdout(&self, bytes: &[u8]) {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(bytes);
        let _ = out.flush();
        if let Some(f) = &self.file {
            if let Ok(mut g) = f.lock() {
                let _ = g.write_all(bytes);
            }
        }
    }

    fn write_stderr(&self, bytes: &[u8]) {
        let mut out = std::io::stderr().lock();
        let _ = out.write_all(bytes);
        let _ = out.flush();
        if let Some(f) = &self.file {
            if let Ok(mut g) = f.lock() {
                let _ = g.write_all(bytes);
            }
        }
    }
}

/// Run a command with stdout/stderr streamed to the terminal AND the run
/// log (the bash tee behaviour for inherited output).
fn run_tee(tee: &Tee, program: &str, args: &[&str]) -> Option<ExitStatus> {
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let mut out_pipe = child.stdout.take()?;
    let mut err_pipe = child.stderr.take()?;
    let t_out = {
        let tee = tee.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match out_pipe.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => tee.write_stdout(&buf[..n]),
                }
            }
        })
    };
    let t_err = {
        let tee = tee.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match err_pipe.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => tee.write_stderr(&buf[..n]),
                }
            }
        })
    };
    let status = child.wait().ok();
    let _ = t_out.join();
    let _ = t_err.join();
    status
}

/// Run a command capturing stdout (stderr discarded), like the bash
/// `out=$(cmd 2>/dev/null)` captures. None if the command won't spawn.
fn run_capture(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program)
        .args(args)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Bash $() strips trailing newlines.
fn trim_capture(s: &str) -> String {
    s.trim_end_matches('\n').to_string()
}

fn is_executable(p: &str) -> bool {
    if p.is_empty() {
        return false;
    }
    let md = match fs::metadata(p) {
        Ok(m) => m,
        Err(_) => return false,
    };
    if !md.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        md.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

// ---------------------------------------------------------------------------
// Config: the bash header block (env-overridable paths and knobs).
// ---------------------------------------------------------------------------

struct Config {
    fb: String,
    ssh_pm: String,
    allowlist_disp: String,
    receipts_disp: String,
    receipts: PathBuf,
    poll: i64,
    bl_wait: i64,
    slack: i64,
    battery_floor: i64,
    serial: String,
    webcam_env: String,
    sahara_disp: String,
    sahara_dir: PathBuf,
    recovery_wait: i64,
    fb_grace: i64,
    fb_persist: i64,
    fb_recovery_wait: i64,
    usb_cmd: Option<String>,
    webcam_at: Option<i64>, // OP6_CYCLE_WEBCAM_AT (set and non-empty)
    ocr_env: String,
}

impl Config {
    fn from_env() -> Config {
        let home = env::var("HOME").unwrap_or_default();
        let receipts_disp = env_or(
            "OP6_CYCLE_RECEIPTS",
            &format!("{}/op6-r1/out/device-receipts", home),
        );
        let sahara_disp = env_or(
            "OP6_CYCLE_SAHARA_DIR",
            &format!("{}/op6-r1/out/sahara-receipts", home),
        );
        Config {
            fb: env_or("OP6_CYCLE_FB", "$HOME/tools/platform-tools/fastboot"),
            ssh_pm: env_or("OP6_CYCLE_SSH", "$HOME/.local/bin/ssh-op6cf"),
            allowlist_disp: env_or(
                "OP6_CYCLE_ALLOWLIST",
                &format!("{}/op6-r1/op6-cycle.allowlist", home),
            ),
            receipts: PathBuf::from(&receipts_disp),
            receipts_disp,
            poll: env_i64("OP6_CYCLE_POLL", 2),
            bl_wait: env_i64("OP6_CYCLE_BL_WAIT", 240),
            slack: env_i64("OP6_CYCLE_SLACK", 180),
            battery_floor: env_i64("OP6_CYCLE_BATTERY_FLOOR", 30),
            serial: env_or("OP6_CYCLE_SERIAL", SERIAL_DEFAULT),
            webcam_env: env_or("OP6_CYCLE_WEBCAM", ""),
            sahara_dir: PathBuf::from(&sahara_disp),
            sahara_disp,
            recovery_wait: env_i64("OP6_CYCLE_RECOVERY_WAIT_S", 90),
            fb_grace: env_i64("OP6_CYCLE_FASTBOOT_GRACE", 30),
            fb_persist: env_i64("OP6_CYCLE_FASTBOOT_PERSIST_S", 5),
            fb_recovery_wait: env_i64("OP6_CYCLE_FASTBOOT_RECOVERY_WAIT_S", 120),
            usb_cmd: env_nonempty("OP6_CYCLE_USB_CMD"),
            webcam_at: env_nonempty("OP6_CYCLE_WEBCAM_AT")
                .map(|v| v.parse::<i64>().unwrap_or(0)),
            ocr_env: env_or("OP6_CYCLE_OCR", ""),
        }
    }

    fn recovery_log_disp(&self) -> String {
        format!("{}/recovery.log", self.sahara_disp)
    }
}

// ---------------------------------------------------------------------------
// CLI parsing: mirrors the bash while/case loop exactly, including the
// exit-64 usage errors (which happen BEFORE any log or receipt exists).
// ---------------------------------------------------------------------------

struct Cli {
    img: String,
    ttl_expect: String,
    attended: bool,
    attended_by: String,
    webcam_flag: bool,
}

fn usage_error(msg: &str) -> i32 {
    eprintln!("{}", msg);
    64
}

fn parse_cli(argv: &[String], home: &str) -> Result<Cli, i32> {
    let mut cli = Cli {
        img: String::new(),
        ttl_expect: String::new(),
        attended: false,
        attended_by: String::new(),
        webcam_flag: false,
    };
    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];
        match arg.as_str() {
            "--ttl-expect" => {
                cli.ttl_expect = argv.get(i + 1).cloned().unwrap_or_default();
                i += 2;
            }
            "--attended" => {
                cli.attended = true;
                i += 1;
            }
            "--attended-by" => {
                if argv.len() - i < 2 {
                    return Err(usage_error(
                        "op6-cycle: --attended-by needs a note argument",
                    ));
                }
                cli.attended_by = argv[i + 1].clone();
                i += 2;
            }
            "--webcam" => {
                cli.webcam_flag = true;
                i += 1;
            }
            "--" => {
                i += 1;
                if i < argv.len() {
                    cli.img = argv[i].clone();
                    i += 1;
                }
            }
            _ => {
                if arg.starts_with('-') {
                    return Err(usage_error(&format!(
                        "op6-cycle: unknown option {}",
                        arg
                    )));
                }
                cli.img = arg.clone();
                i += 1;
            }
        }
    }
    let _ = home;
    if cli.img.is_empty() {
        return Err(usage_error(
            "usage: op6-cycle <image> [--ttl-expect N] [--attended [--attended-by NOTE]] [--webcam]",
        ));
    }
    if !Path::new(&cli.img).is_file() {
        return Err(usage_error(&format!(
            "op6-cycle: image not found: {}",
            cli.img
        )));
    }
    if !cli.attended_by.is_empty() && !cli.attended {
        return Err(usage_error(
            "op6-cycle: --attended-by requires --attended",
        ));
    }
    Ok(cli)
}

// ---------------------------------------------------------------------------
// Lock: bash `mkdir "$LOCK"` (atomic) + `trap 'rmdir ...' EXIT`.
// ---------------------------------------------------------------------------

struct LockGuard {
    path: PathBuf,
    active: bool,
}

impl LockGuard {
    fn release(&mut self) {
        if self.active {
            let _ = fs::remove_dir(&self.path);
            self.active = false;
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        self.release();
    }
}

// ---------------------------------------------------------------------------
// Webcam burst state (bash WC_* globals).
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Webcam {
    frames: Vec<String>,
    shas: Vec<String>,
    count: i64,
    fails: i64,
    stop: bool,
    status: String,
    detail: String,
}

// ---------------------------------------------------------------------------
// The cycle itself.
// ---------------------------------------------------------------------------

struct Cycle {
    cfg: Config,
    tee: Tee,
    stamp: String,
    log_disp: String,
    img: String,
    sha: String,
    ttl_raw: String,
    ttl: i64,
    gate: String,
    attended: bool,
    attended_by: String,
    webcam: String, // effective helper path; "" = disabled
    ocr: String,
    lock: LockGuard,
    // receipt fields
    started: String,
    t0: i64,
    battery_digits: String,
    product: String,
    boot_at: String,
    outcome: String,
    detail: String,
    pmos_at: String,
    crash_at: String,
    crash_epoch: i64,
    rec_fields: String,
    wc: Webcam,
}

impl Cycle {
    fn iso_now(&self) -> String {
        utc::iso(now_epoch())
    }

    /// bash refuse(): log line + refusal receipt + exit 4.
    fn refuse(&self, reason: &str) -> i32 {
        self.tee
            .say(&format!("{} REFUSED: {}", self.iso_now(), reason));
        let path = format!(
            "{}/refusals/refusal-{}.json",
            self.cfg.receipts_disp, self.stamp
        );
        let body = refusal_json(reason, &self.img, &self.iso_now());
        let _ = fs::write(&path, format!("{}\n", body));
        4
    }

    fn usb_state(&self) -> String {
        if let Some(cmd) = &self.cfg.usb_cmd {
            return run_capture(cmd, &[]).map(|s| trim_capture(&s)).unwrap_or_default();
        }
        // Mirror of the bash awk over system_profiler SPUSBDataType:
        // remember the last Product ID; at each Vendor ID line, if the
        // vendor is Google (0x18d1) or Qualcomm (0x05c6), print v:p;
        // head -1 takes the first.
        let out = run_capture("system_profiler", &["SPUSBDataType"]).unwrap_or_default();
        let mut product = "";
        for line in out.lines() {
            let toks: Vec<&str> = line.split_whitespace().collect();
            if toks.len() >= 3 && toks[0] == "Product" && toks[1] == "ID:" {
                product = toks[2];
            }
            if toks.len() >= 3 && toks[0] == "Vendor" && toks[1] == "ID:" {
                let v = toks[2];
                if v.contains("0x18d1") || v.contains("0x05c6") {
                    return format!("{}:{}", v, product);
                }
            }
        }
        String::new()
    }

    fn fb_present(&self) -> bool {
        run_capture(&self.cfg.fb, &["devices"])
            .map(|s| s.contains(&self.cfg.serial))
            .unwrap_or(false)
    }

    fn wc_file_disp(&self) -> String {
        format!("{}/webcam-{}.fields", self.cfg.receipts_disp, self.stamp)
    }

    /// bash write_receipt().
    fn write_receipt(&self) {
        let mut wc_fields = String::new();
        if !self.webcam.is_empty() {
            let wc_path = self.wc_file_disp();
            match fs::read_to_string(&wc_path) {
                Ok(content) if !content.is_empty() => {
                    wc_fields = format!(",{}", content.replace('\n', ""));
                }
                _ => {
                    wc_fields = ",\"webcam_status\":\"not-captured\"".to_string();
                }
            }
        }
        let host = run_capture("hostname", &[])
            .map(|s| trim_capture(&s))
            .unwrap_or_default();
        let image_bytes = fs::metadata(&self.img).map(|m| m.len()).unwrap_or(0);
        let fields = ReceiptFields {
            started: self.started.clone(),
            finished: self.iso_now(),
            host,
            image: self.img.clone(),
            image_sha256: self.sha.clone(),
            image_bytes,
            ttl_raw: self.ttl_raw.clone(),
            gate_evidence: self.gate.clone(),
            attended_by: if self.attended {
                Some(self.attended_by.clone())
            } else {
                None
            },
            battery_digits: self.battery_digits.clone(),
            fastboot_serial: self.cfg.serial.clone(),
            getvar_product: self.product.clone(),
            boot_sent_at: self.boot_at.clone(),
            outcome: self.outcome.clone(),
            detail: self.detail.clone(),
            pmos_returned_at: self.pmos_at.clone(),
            log: self.log_disp.clone(),
        };
        let body = receipt_json(&fields, &wc_fields, &self.rec_fields);
        let path = format!(
            "{}/op6-cycle-{}.json",
            self.cfg.receipts_disp, self.stamp
        );
        let _ = fs::write(&path, format!("{}\n", body));
        self.tee.say(&format!(
            "{} receipt: {} outcome={}",
            self.iso_now(),
            path,
            self.outcome
        ));
    }

    /// bash wc_capture(): one burst frame. Fail-soft: never affects the
    /// cycle outcome.
    fn wc_capture(&mut self) {
        let frame = format!(
            "{}/panel-{}-b{:02}.jpg",
            self.cfg.receipts_disp,
            self.stamp,
            self.wc.count + self.wc.fails + 1
        );
        if !is_executable(&self.webcam) {
            self.wc.status = "unavailable".to_string();
            self.wc.detail = format!("helper not executable: {}", self.webcam);
            self.wc.stop = true;
            return;
        }
        let out = run_capture(&self.webcam, &[&frame]).unwrap_or_default();
        let out = trim_capture(&out);
        let st = match out.find(' ') {
            Some(idx) => &out[..idx],
            None => &out[..],
        };
        match st {
            "ok" => {
                self.wc.count += 1;
                self.wc.frames.push(frame);
                // bash ${out#ok }: strip a literal "ok " prefix if present.
                let sha = out.strip_prefix("ok ").unwrap_or(&out).to_string();
                self.wc.shas.push(sha);
            }
            "not-authorized" | "no-camera" | "unavailable" => {
                self.wc.status = st.to_string();
                self.wc.stop = true;
            }
            _ => {
                self.wc.status = "capture-failed".to_string();
                self.wc.fails += 1;
                if self.wc.fails >= 3 {
                    self.wc.stop = true;
                }
            }
        }
    }

    /// bash finalize_webcam(): prune to the last 3 frames, OCR the kept
    /// ones, find the frame showing the reset, write the fields file
    /// write_receipt() merges.
    fn finalize_webcam(&self) {
        if self.webcam.is_empty() {
            return;
        }
        let wc_path = self.wc_file_disp();
        if self.wc.count == 0 {
            if !self.wc.status.is_empty() {
                let body = format!(
                    "\"webcam_status\":\"{}\",\"webcam_detail\":\"{}\"",
                    self.wc.status,
                    esc(&self.wc.detail)
                );
                let _ = fs::write(&wc_path, body);
            }
            return;
        }
        let total = self.wc.count;
        let mut kept: Vec<String> = Vec::new();
        let mut kept_shas: Vec<String> = Vec::new();
        for (i, f) in self.wc.frames.iter().enumerate() {
            let n = (i + 1) as i64;
            if total - n < 3 {
                kept.push(f.clone());
            } else {
                let _ = fs::remove_file(f);
            }
        }
        for (i, s) in self.wc.shas.iter().enumerate() {
            let n = (i + 1) as i64;
            if total - n < 3 {
                kept_shas.push(s.clone());
            }
        }
        let mut scm: Option<i64> = None;
        let mut ocr_pick = String::new();
        let mut idx: i64 = 0;
        for f in &kept {
            let mut txt = String::new();
            if is_executable(&self.ocr) {
                if let Some(raw) = run_capture(&self.ocr, &[f]) {
                    // bash: txt=$("$OCR" "$f" | tr '\n' ' ') - tr runs
                    // before the $() newline strip, so a trailing
                    // newline becomes a trailing space. Mirror exactly.
                    txt = raw.replace('\n', " ");
                }
            }
            let lower = txt.to_lowercase();
            if scm.is_none() && (lower.contains("kreset:") || lower.contains("download-mode")) {
                scm = Some(idx);
                ocr_pick = txt.clone();
            }
            if ocr_pick.is_empty() {
                ocr_pick = txt.clone();
            }
            idx += 1;
        }
        let last_sha = kept_shas.last().cloned().unwrap_or_default();
        let scm_str = scm.map(|v| v.to_string()).unwrap_or_else(|| "null".to_string());
        let ocr_cut: String = ocr_pick.chars().take(400).collect();
        let body = format!(
            "\"webcam_status\":\"ok\",\"webcam_burst_frames\":{},\"webcam_frames_kept\":{},\"webcam_frame_sha256\":\"{}\",\"webcam_scm_frame\":{},\"webcam_ocr_text\":\"{}\"",
            total,
            idx,
            last_sha,
            scm_str,
            esc(&ocr_cut)
        );
        let _ = fs::write(&wc_path, body);
        self.tee.say(&format!(
            "{} webcam burst: {} frames, kept {}, scm_frame={}",
            self.iso_now(),
            total,
            idx,
            scm_str
        ));
    }

    /// bash await_recovery() (CRASHDUMP only): release the lock so the
    /// sahara watcher can act, then wait for its RECOVERED line. Sets
    /// rec_fields; the outcome stays CRASHDUMP either way.
    fn await_recovery(&mut self) {
        self.rec_fields = ",\"recovery\":{\"watcher_recovered\":false}".to_string();
        if self.crash_at.is_empty() {
            self.crash_at = self.started.clone();
            self.crash_epoch = self.t0;
        }
        if self.lock.active && self.lock.path.is_dir() {
            self.lock.release();
            self.tee.say(&format!(
                "{} crashdump: released cycle lock so the sahara watcher can act",
                self.iso_now()
            ));
        }
        let log_path = self.cfg.recovery_log_disp();
        self.tee.say(&format!(
            "{} crashdump: waiting up to {}s for watcher recovery in {}",
            self.iso_now(),
            self.cfg.recovery_wait,
            log_path
        ));
        let deadline = now_epoch() + self.cfg.recovery_wait;
        while now_epoch() < deadline {
            let text = fs::read_to_string(&log_path).unwrap_or_default();
            let line = text
                .lines()
                .filter(|l| l.contains(" RECOVERED: "))
                .last()
                .map(|l| l.to_string());
            if let Some(line) = line {
                let ts = line.split(' ').next().unwrap_or("").to_string();
                // bash: [[ ! "$ts" < "$CRASH_AT" ]] (lexicographic)
                if ts.as_str() >= self.crash_at.as_str() {
                    // awk: $2 == "receipt" && $1 >= lo && $1 <= hi -> $3, last wins
                    let mut sahara = String::new();
                    for l in text.lines() {
                        let f: Vec<&str> = l.split_whitespace().collect();
                        if f.len() >= 2
                            && f[1] == "receipt"
                            && f[0] >= self.crash_at.as_str()
                            && f[0] <= ts.as_str()
                        {
                            sahara = f.get(2).copied().unwrap_or("").to_string();
                        }
                    }
                    if sahara.is_empty() {
                        // Fallback: newest sahara-*.json, if not older
                        // than the crash.
                        let mut best: Option<(i64, String)> = None;
                        if let Ok(rd) = fs::read_dir(&self.cfg.sahara_dir) {
                            for e in rd.flatten() {
                                let name = e.file_name().to_string_lossy().into_owned();
                                if name.starts_with("sahara-") && name.ends_with(".json") {
                                    if let Ok(md) = e.metadata() {
                                        let m = md
                                            .modified()
                                            .ok()
                                            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                                            .map(|d| d.as_secs() as i64)
                                            .unwrap_or(0);
                                        if best.as_ref().map(|(bm, _)| m >= *bm).unwrap_or(true) {
                                            best = Some((m, e.path().to_string_lossy().into_owned()));
                                        }
                                    }
                                }
                            }
                        }
                        if let Some((m, p)) = best {
                            if m >= self.crash_epoch {
                                sahara = p;
                            }
                        }
                    }
                    self.rec_fields = format!(
                        ",\"recovery\":{{\"watcher_recovered\":true,\"recovered_at\":\"{}\",\"sahara_receipt\":\"{}\"}}",
                        ts,
                        esc(&sahara)
                    );
                    let sahara_disp = if sahara.is_empty() { "unknown" } else { &sahara };
                    self.tee.say(&format!(
                        "{} crashdump: watcher recovered the phone at {} (sahara receipt: {}; usb now: {})",
                        self.iso_now(),
                        ts,
                        sahara_disp,
                        self.usb_state()
                    ));
                    return;
                }
            }
            sleep_secs(self.cfg.poll);
        }
        self.tee.say(&format!(
            "{} crashdump: no watcher recovery within {}s",
            self.iso_now(),
            self.cfg.recovery_wait
        ));
    }

    /// bash recover_fastboot() (FASTBOOT only): `fastboot reboot` and
    /// wait for pmOS USB. Sets rec_fields; the outcome stays FASTBOOT.
    fn recover_fastboot(&mut self) {
        self.tee.say(&format!(
            "{} fastboot landing: issuing fastboot reboot, waiting up to {}s for pmOS",
            self.iso_now(),
            self.cfg.fb_recovery_wait
        ));
        let _ = Command::new(&self.cfg.fb)
            .args(["-s", &self.cfg.serial, "reboot"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let deadline = now_epoch() + self.cfg.fb_recovery_wait;
        while now_epoch() < deadline {
            if self.usb_state() == "0x18d1:0xd001" {
                self.pmos_at = self.iso_now();
                self.rec_fields = format!(
                    ",\"recovery\":{{\"recovered_to_pmos\":true,\"recovered_at\":\"{}\",\"method\":\"fastboot reboot\"}}",
                    self.pmos_at
                );
                self.tee.say(&format!(
                    "{} fastboot landing: pmOS back at {}",
                    self.iso_now(),
                    self.pmos_at
                ));
                return;
            }
            sleep_secs(self.cfg.poll);
        }
        self.rec_fields =
            ",\"recovery\":{\"recovered_to_pmos\":false,\"method\":\"fastboot reboot\"}"
                .to_string();
        self.tee.say(&format!(
            "{} fastboot landing: pmOS did not return within {}s (phone left in fastboot; recover by hand)",
            self.iso_now(),
            self.cfg.fb_recovery_wait
        ));
    }
}

fn main() {
    std::process::exit(real_main());
}

fn real_main() -> i32 {
    let argv: Vec<String> = env::args().skip(1).collect();
    let home = env::var("HOME").unwrap_or_default();
    let cli = match parse_cli(&argv, &home) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let cfg = Config::from_env();

    // Mirror of the bash: mkdir -p "$RECEIPTS/refusals", STAMP, LOG, tee.
    let _ = fs::create_dir_all(cfg.receipts.join("refusals"));
    let stamp = utc::stamp(now_epoch());
    let log_disp = format!("{}/run-{}.log", cfg.receipts_disp, stamp);
    let log_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_disp)
        .ok()
        .map(|f| Arc::new(Mutex::new(f)));
    let tee = Tee { file: log_file };
    tee.say(&format!(
        "{} op6-cycle starting; image={} log={}",
        utc::iso(now_epoch()),
        cli.img,
        log_disp
    ));

    // Effective webcam helper: --webcam only supplies the default path
    // when OP6_CYCLE_WEBCAM is empty; the env var alone also enables it.
    let mut webcam = cfg.webcam_env.clone();
    if cli.webcam_flag && webcam.is_empty() {
        webcam = format!("{}/op6-r1/op6-webcam.sh", home);
    }
    // OCR default: op6-ocr next to the webcam helper.
    let mut ocr = cfg.ocr_env.clone();
    if !webcam.is_empty() && ocr.is_empty() {
        let dir = Path::new(&webcam)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        ocr = format!("{}/op6-ocr", dir);
    }

    // --- lock (mkdir is atomic) ---
    let lock_path = cfg.receipts.join(".op6-cycle.lock");
    let mut cycle = Cycle {
        cfg,
        tee,
        stamp,
        log_disp,
        img: cli.img.clone(),
        sha: String::new(),
        ttl_raw: String::new(),
        ttl: 0,
        gate: String::new(),
        attended: cli.attended,
        attended_by: cli.attended_by.clone(),
        webcam,
        ocr,
        lock: LockGuard {
            path: lock_path,
            active: false,
        },
        started: String::new(),
        t0: 0,
        battery_digits: String::new(),
        product: String::new(),
        boot_at: String::new(),
        outcome: String::new(),
        detail: String::new(),
        pmos_at: String::new(),
        crash_at: String::new(),
        crash_epoch: 0,
        rec_fields: String::new(),
        wc: Webcam::default(),
    };
    if fs::create_dir(&cycle.lock.path).is_err() {
        let reason = format!(
            "another op6-cycle is running (lock {})",
            cycle.lock.path.to_string_lossy()
        );
        return cycle.refuse(&reason);
    }
    cycle.lock.active = true;

    // --- allowlist pin ---
    cycle.sha = sha256::sha256_file_hex(Path::new(&cycle.img)).unwrap_or_default();
    let base = Path::new(&cycle.img)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let allow_text = fs::read_to_string(&cycle.cfg.allowlist_disp).unwrap_or_default();
    let entries = parse_allowlist(&allow_text);
    let entry = match find_entry(&entries, &cycle.sha, &base) {
        Some(e) => e.clone(),
        None => {
            let reason = format!(
                "image sha256 {} ({}) not in allowlist {}",
                cycle.sha, base, cycle.cfg.allowlist_disp
            );
            return cycle.refuse(&reason);
        }
    };
    if entry.mode == "ATTENDED-ONLY" && !cycle.attended {
        return cycle.refuse(
            "image is ATTENDED-ONLY (new hardware write - operator approval required); rerun with --attended",
        );
    }
    if cycle.attended {
        if !cycle.attended_by.is_empty() {
            cycle.tee.say(&format!(
                "{} attended run (--attended given; operator-approved: {})",
                cycle.iso_now(),
                cycle.attended_by
            ));
        } else {
            cycle.tee.say(&format!(
                "{} attended run (--attended given; operator-approved)",
                cycle.iso_now()
            ));
        }
    }
    cycle.ttl = match ttl_value(&entry.ttl_raw) {
        Some(v) => v,
        None => return cycle.refuse("allowlist entry has no TTL"),
    };
    cycle.ttl_raw = entry.ttl_raw.clone();
    if entry.gate.is_empty() {
        return cycle.refuse("allowlist entry has no gate-evidence reference");
    }
    cycle.gate = entry.gate.clone();
    if !cli.ttl_expect.is_empty() && cli.ttl_expect != cycle.ttl_raw {
        let reason = format!(
            "--ttl-expect {} disagrees with allowlist TTL {}",
            cli.ttl_expect, cycle.ttl_raw
        );
        return cycle.refuse(&reason);
    }
    cycle.tee.say(&format!(
        "{} pin OK: sha256={} ttl={}s gate_evidence={}",
        cycle.iso_now(),
        cycle.sha,
        cycle.ttl,
        cycle.gate
    ));

    // --- stop-on-first-non-PASS ledger check ---
    let mut newest: Option<(i64, String, PathBuf)> = None;
    if let Ok(rd) = fs::read_dir(&cycle.cfg.receipts) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("op6-cycle-") && name.ends_with(".json") {
                let m = e
                    .metadata()
                    .and_then(|md| md.modified())
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                if newest.as_ref().map(|(bm, _, _)| m >= *bm).unwrap_or(true) {
                    newest = Some((m, name, e.path()));
                }
            }
        }
    }
    if let Some((_, last_name, last_path)) = newest {
        let text = fs::read_to_string(&last_path).unwrap_or_default();
        let last_outcome = core::extract_outcome(&text);
        if !ledger_clean(&last_outcome, &text) {
            let reason = format!(
                "previous receipt {} outcome={} (stop-on-first-non-PASS)",
                last_name, last_outcome
            );
            return cycle.refuse(&reason);
        }
        cycle.tee.say(&format!(
            "{} previous receipt {} (clean): {}",
            cycle.iso_now(),
            last_outcome,
            last_name
        ));
    }

    // --- pmOS reachability + battery floor ---
    let batt_out = run_capture(&cycle.cfg.ssh_pm, &[CAPACITY_CMD]).unwrap_or_default();
    let digits: String = batt_out.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        let reason = format!(
            "phone not reachable in pmOS (battery read failed via {})",
            cycle.cfg.ssh_pm
        );
        return cycle.refuse(&reason);
    }
    let batt_val: i64 = digits.parse().unwrap_or(i64::MAX);
    if batt_val < cycle.cfg.battery_floor {
        let reason = format!(
            "battery {}% below floor {}%",
            digits, cycle.cfg.battery_floor
        );
        return cycle.refuse(&reason);
    }
    cycle.battery_digits = digits;
    cycle.tee.say(&format!(
        "{} pmOS reachable; battery {}%",
        cycle.iso_now(),
        cycle.battery_digits
    ));

    cycle.started = cycle.iso_now();
    cycle.t0 = now_epoch();

    // --- pmOS -> bootloader (wait <= BL_WAIT for the serial) ---
    if cycle.fb_present() {
        cycle.tee.say(&format!("{} phone already in fastboot", cycle.iso_now()));
    } else {
        cycle.tee.say(&format!("{} pmOS -> reboot bootloader", cycle.iso_now()));
        let ssh = cycle.cfg.ssh_pm.clone();
        let _ = run_tee(&cycle.tee, &ssh, &["sudo reboot bootloader"]);
        let mut waited = 0;
        let mut found = false;
        while waited < cycle.cfg.bl_wait {
            if cycle.fb_present() {
                found = true;
                break;
            }
            sleep_secs(cycle.cfg.poll);
            waited += cycle.cfg.poll;
        }
        if !found {
            cycle.outcome = "ABORTED_NO_FASTBOOT".to_string();
            cycle.detail = format!(
                "no fastboot serial {} after {}s; usb={}",
                cycle.cfg.serial,
                cycle.cfg.bl_wait,
                cycle.usb_state()
            );
            cycle.write_receipt();
            return 1;
        }
    }
    // getvar product: bash merges stderr into stdout and takes line 1.
    cycle.product = {
        let out = Command::new(&cycle.cfg.fb)
            .args(["-s", &cycle.cfg.serial, "getvar", "product"])
            .output();
        match out {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&o.stderr).into_owned();
                let src = if !stdout.is_empty() { stdout } else { stderr };
                src.lines().next().unwrap_or("").to_string()
            }
            Err(_) => String::new(),
        }
    };
    cycle.tee.say(&format!(
        "{} fastboot {} present; {}",
        cycle.iso_now(),
        cycle.cfg.serial,
        cycle.product
    ));

    // --- exactly ONE fastboot boot ---
    cycle.boot_at = cycle.iso_now();
    cycle.tee.say(&format!(
        "{} fastboot boot (RAM only, single attempt)",
        cycle.iso_now()
    ));
    let fb = cycle.cfg.fb.clone();
    let serial = cycle.cfg.serial.clone();
    let img = cycle.img.clone();
    let boot_status = run_tee(&cycle.tee, &fb, &["-s", &serial, "boot", &img]);
    if !boot_status.map(|s| s.success()).unwrap_or(false) {
        cycle.outcome = "BOOT_CMD_FAILED".to_string();
        cycle.detail = "fastboot boot exited non-zero".to_string();
        cycle.write_receipt();
        return 1;
    }
    let boot_epoch = now_epoch();
    let wc_at = match cycle.cfg.webcam_at {
        Some(v) => boot_epoch + v,
        None => {
            let w = boot_epoch + cycle.ttl - 10;
            if w <= boot_epoch {
                boot_epoch + 1
            } else {
                w
            }
        }
    };
    let deadline = boot_epoch + cycle.ttl + cycle.cfg.slack;
    cycle.tee.say(&format!(
        "{} boot sent; classifying until {} (TTL {}s + slack {}s)",
        cycle.iso_now(),
        utc::hms(deadline),
        cycle.ttl,
        cycle.cfg.slack
    ));

    // --- classification loop (bash case over usb_state) ---
    let mut exit_code = 3;
    let mut last = String::new();
    let mut det = FbDetector::default();
    while now_epoch() < deadline {
        let state = cycle.usb_state();
        let ep = now_epoch();
        if state != last {
            cycle.tee.say(&format!(
                "{} USB: {}",
                cycle.iso_now(),
                if state.is_empty() { "none" } else { &state }
            ));
            last = state.clone();
        }
        if !cycle.webcam.is_empty() && !cycle.wc.stop && now_epoch() >= wc_at {
            cycle.wc_capture();
        }
        match det.on_state(
            &state,
            ep,
            boot_epoch,
            cycle.ttl,
            cycle.cfg.fb_grace,
            cycle.cfg.fb_persist,
        ) {
            Poll::Pass => {
                cycle.outcome = "PASS".to_string();
                cycle.pmos_at = cycle.iso_now();
                cycle.detail = "pmOS USB gadget enumerated after self-reset".to_string();
                exit_code = 0;
                break;
            }
            Poll::Crashdump => {
                cycle.outcome = "CRASHDUMP".to_string();
                cycle.crash_at = cycle.iso_now();
                cycle.crash_epoch = now_epoch();
                cycle.detail = format!(
                    "Qualcomm crash-dump/download mode USB id {}",
                    state
                );
                exit_code = 2;
                break;
            }
            Poll::Fastboot => {
                cycle.outcome = "FASTBOOT".to_string();
                cycle.detail = "landed in fastboot (18d1:d00d) after self-reset and stayed; PON-style bootloader landing".to_string();
                exit_code = 5;
                break;
            }
            Poll::Continue => {}
        }
        sleep_secs(cycle.cfg.poll);
    }
    if cycle.outcome.is_empty() {
        let ep = now_epoch();
        if det.fastboot_at_deadline(&last, ep, cycle.cfg.fb_persist) {
            cycle.outcome = "FASTBOOT".to_string();
            cycle.detail = "landed in fastboot (18d1:d00d) after self-reset and stayed until the deadline".to_string();
            exit_code = 5;
        } else {
            cycle.outcome = "HUNG".to_string();
            cycle.detail = format!(
                "no pmOS USB after TTL+{}s; usb={}",
                cycle.cfg.slack,
                cycle.usb_state()
            );
            exit_code = 3;
        }
    }
    cycle.finalize_webcam();
    if cycle.outcome == "CRASHDUMP" {
        cycle.await_recovery();
    }
    if cycle.outcome == "FASTBOOT" {
        cycle.recover_fastboot();
    }
    cycle.write_receipt();
    exit_code
}
