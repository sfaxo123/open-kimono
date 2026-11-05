//! Python-reference interoperability gate (WAVE2 §1.1 acceptance gap),
//! hardened per the Cursor review (`CURSOR-WY-REVIEW-20261025.md`).
//!
//! Spawns the real upstream Python `wyoming` package (reference checkout
//! from `WYOMING_PYREF`, else `$HOME/Developer/forks/op6-refs/wyoming`) as a
//! loopback server (`tests/pyinterop_server.py`, using the reference
//! `read_event`/`write_event`/`Pong`), then drives it with the Rust `Client`:
//!
//! * all 59 golden-fixture events are sent over the socket and must come
//!   back logically identical (a `ping` is answered with a `pong` copying
//!   `text`, like the reference `PingHandler`),
//! * >=59 events are exchanged in each direction,
//! * `Client::close()` shuts the transport down, the Python server observes
//!   clean EOF, prints `SERVED 59`, and exits 0,
//! * the imported `wyoming` module is *verified* to come from the reference
//!   checkout (path assertion + recorded git commit), never assumed (W-6).
//!
//! Gate modes (`WYOMING_PYINTEROP` — accepted values from actual callers):
//!
//! * unset / empty (default): portable — visible SKIP when no reference
//!   checkout exists at the default path (the 0dd4111 behavior, preserved);
//!   this is what developer machines and CI without a reference checkout use;
//! * `required`: the 59-event exchange MUST actually run — a missing
//!   python3 or reference checkout is a hard failure, never a skip. The
//!   orchestrator's strict gate sets this so a green suite can never hide a
//!   silently skipped interop gate;
//! * any other nonempty value (typo, garbage, wrong case): rejected with an
//!   explicit failure listing the accepted values, so a misconfigured strict
//!   gate can never silently degrade into the optional skip.
//!
//! Harness lifetime rules (review W-1/W-3/W-4/W-5): the child is wrapped in
//! an RAII guard that kills+reaps on any early exit including panics; every
//! stdout read (PORT, SERVED) and the exit wait are bounded by explicit
//! deadlines; the child's stderr is drained on a helper thread and included
//! in failure messages. The fault-injection tests below induce each failure
//! and assert the harness fails *bounded* with no leftover child.

use std::fs;
use std::io::{BufRead, BufReader, Cursor, Read};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use wyoming::client::{Client, ClientState};
use wyoming::{decode, Event, TransitionKind};

/// How long to wait for the server to report its identity + listening port.
const PORT_TIMEOUT: Duration = Duration::from_secs(15);
/// Startup budget for the *real* reference gate: python cold starts under
/// heavy fleet load have been observed near the 15 s mark, so the gate that
/// must never flake gets a more generous budget. The fault-injection tests
/// keep the tight 15 s deadline — they are the bounded-failure proof.
const READY_TIMEOUT: Duration = Duration::from_secs(30);
/// How long to wait for the SERVED line after close() delivers EOF.
const SERVED_TIMEOUT: Duration = Duration::from_secs(15);
/// How long to wait for the child to exit after it printed SERVED.
const EXIT_TIMEOUT: Duration = Duration::from_secs(5);
/// Socket read timeout for the event exchange itself.
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(20);

// ---------------------------------------------------------------------------
// Gate modes (0dd4111 portable default preserved; `required` strict mode;
// unknown nonempty values rejected explicitly)
// ---------------------------------------------------------------------------

/// Parsed `WYOMING_PYINTEROP` gate mode.
#[derive(Debug, PartialEq, Eq)]
enum GateMode {
    /// Portable default: visible skip when the reference is absent.
    Optional,
    /// Strict gate: the 59-event exchange must actually run.
    Required,
}

/// Pure parse of the raw `WYOMING_PYINTEROP` value (`None` = unset).
/// Accepted values come from the actual callers: unset/empty (developer
/// machines, CI without a reference) and `required` (the orchestrator's
/// strict gate). Any other nonempty value is a typo or garbage and is
/// rejected with a diagnostic naming the offending value — it must never
/// silently degrade into the optional skip.
fn parse_gate_mode(raw: Option<&str>) -> Result<GateMode, String> {
    match raw {
        None | Some("") => Ok(GateMode::Optional),
        Some("required") => Ok(GateMode::Required),
        Some(other) => Err(format!(
            "WYOMING_PYINTEROP={other:?} is not a recognized gate mode \
             (accepted: unset for the optional default, \"required\" for \
             the strict gate)"
        )),
    }
}

/// Read `WYOMING_PYINTEROP` and parse it; an unknown mode fails the test
/// immediately with the parse diagnostic. Pure parsing is unit-tested
/// separately (`gate_mode_parsing`) without mutating the process env.
fn gate_mode() -> GateMode {
    let raw = match std::env::var("WYOMING_PYINTEROP") {
        Ok(v) => Some(v),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => panic!(
            "WYOMING_PYINTEROP is not valid UTF-8 — cannot determine the \
             gate mode (accepted: unset, \"required\")"
        ),
    };
    parse_gate_mode(raw.as_deref()).unwrap_or_else(|msg| panic!("{msg}"))
}

/// Reference checkout resolution, preserving the 0dd4111 semantics:
/// `WYOMING_PYREF` pins the path (missing checkout = hard fail); otherwise
/// the default path is used and a missing checkout is a *visible* skip —
/// unless `WYOMING_PYINTEROP=required` turns that skip into a failure.
/// The mode is parsed first, so an unknown value fails even when the
/// reference happens to exist.
fn ref_dir() -> Option<PathBuf> {
    let required = gate_mode() == GateMode::Required;
    if let Ok(p) = std::env::var("WYOMING_PYREF") {
        let p = PathBuf::from(p);
        assert!(
            p.join("wyoming").join("event.py").exists(),
            "python reference checkout missing at {} (WYOMING_PYREF is an explicit pin)",
            p.display()
        );
        return Some(p);
    }
    let p = PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join("Developer/forks/op6-refs/wyoming");
    if p.join("wyoming").join("event.py").exists() {
        Some(p)
    } else if required {
        panic!(
            "WYOMING_PYINTEROP=required but no python reference at {} — the \
             59-event gate must not be skipped (set WYOMING_PYREF)",
            p.display()
        );
    } else {
        eprintln!(
            "pyinterop: SKIPPED — no reference at {} (set WYOMING_PYREF)",
            p.display()
        );
        None
    }
}

/// Minimal committed package metadata so the reference's
/// `wyoming/version.py` (`importlib.metadata.version("wyoming")`) resolves
/// under a plain PYTHONPATH import — the fork itself is read-only, so we
/// do not `pip install -e` into it. The version string it supplies is
/// *reported* by the gate, never asserted: the recorded git commit of the
/// checkout is the ground truth (review W-6).
fn pyref_pythonpath(reference: &Path) -> String {
    let meta = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/pyref_meta");
    format!("{}:{}", reference.display(), meta.display())
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn test_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join(name)
}

fn golden_events() -> Vec<Event> {
    let bytes = fs::read(fixtures_dir().join("golden.wyoming")).expect("golden.wyoming");
    let mut cursor = Cursor::new(&bytes[..]);
    let mut events = Vec::new();
    while let Some(event) = decode(&mut cursor).expect("decode golden fixture") {
        events.push(event);
    }
    assert_eq!(events.len(), 59, "golden fixture event count");
    events
}

// ---------------------------------------------------------------------------
// Child lifetime: RAII kill+wait guard, bounded line reads, stderr drain
// ---------------------------------------------------------------------------

/// RAII guard: unless disarmed, the child is killed and reaped when the
/// guard drops — including on panic unwind — so a failing test never leaves
/// an orphan python blocked in `accept()` (review W-3).
struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        ChildGuard { child: Some(child) }
    }

    fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("child already reaped")
    }

    /// Reap the child within `timeout`; `Some(status)` if it exited on its
    /// own, `None` on timeout (child still running, caller decides).
    fn wait_bounded(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child_mut().try_wait().expect("try_wait") {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn kill_and_reap(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.child = None;
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.kill_and_reap();
    }
}

/// Reads the child's stdout line-by-line on a helper thread; every read is
/// bounded by an explicit deadline (review W-1/W-5: an untimed `read_line`
/// turned a close() regression into an infinite `cargo test` hang instead
/// of a red test).
struct LineReader {
    rx: mpsc::Receiver<String>,
}

impl LineReader {
    fn new(stdout: ChildStdout) -> Self {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        LineReader { rx }
    }

    /// Next line within `timeout`: `Some(Some(line))`, `Some(None)` on EOF,
    /// `None` on timeout.
    fn next_line(&self, timeout: Duration) -> Option<Option<String>> {
        match self.rx.recv_timeout(timeout) {
            Ok(line) => Some(Some(line)),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => Some(None),
        }
    }
}

/// Collects the child's stderr on a helper thread so Python tracebacks are
/// never swallowed and the pipe can never deadlock the child (review W-4).
struct StderrLog {
    buf: Arc<Mutex<Vec<u8>>>,
}

impl StderrLog {
    fn new(stderr: ChildStderr) -> Self {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&buf);
        thread::spawn(move || {
            let mut drained = Vec::new();
            let mut stderr = stderr;
            let _ = stderr.read_to_end(&mut drained);
            *shared.lock().expect("stderr lock") = drained;
        });
        StderrLog { buf }
    }

    fn text(&self) -> String {
        let bytes = self.buf.lock().expect("stderr lock").clone();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// A spawned python test server with bounded, guarded lifetime plumbing.
struct Harness {
    guard: ChildGuard,
    out: LineReader,
    err: StderrLog,
}

impl Harness {
    fn spawn(script: &Path, args: &[&str], pythonpath: Option<&str>) -> Self {
        let mut cmd = Command::new("python3");
        cmd.arg(script)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(pp) = pythonpath {
            cmd.env("PYTHONPATH", pp);
        }
        let mut child = cmd.spawn().expect("spawn python3 test server");
        let stdout = child.stdout.take().expect("stdout pipe");
        let stderr = child.stderr.take().expect("stderr pipe");
        Harness {
            guard: ChildGuard::new(child),
            out: LineReader::new(stdout),
            err: StderrLog::new(stderr),
        }
    }

    fn stderr_text(&self) -> String {
        self.err.text()
    }

    /// Fail with the child's stderr diagnostics attached (review W-4),
    /// after killing+reaping the child (review W-3).
    fn fail(&mut self, msg: &str) -> ! {
        let stderr = self.stderr_text();
        self.guard.kill_and_reap();
        panic!("{msg}\n--- child stderr ---\n{stderr}\n--- end child stderr ---");
    }

    /// Read the next stdout line starting with `prefix`, within `timeout`
    /// total; other lines are ignored. Kills the child and fails with
    /// stderr diagnostics on timeout or EOF.
    fn wait_line(&mut self, prefix: &str, timeout: Duration) -> String {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.out.next_line(remaining) {
                Some(Some(line)) => {
                    if let Some(rest) = line.trim().strip_prefix(prefix) {
                        return rest.trim().to_string();
                    }
                }
                Some(None) => self.fail(&format!(
                    "python server exited before printing {prefix:?} line"
                )),
                None => self.fail(&format!(
                    "python server did not print {prefix:?} line within {}s",
                    timeout.as_secs()
                )),
            }
        }
    }

    fn wait_port_in(&mut self, timeout: Duration) -> u16 {
        let port = self.wait_line("PORT ", timeout);
        port.parse().expect("numeric PORT line")
    }

    fn wait_port(&mut self) -> u16 {
        self.wait_port_in(PORT_TIMEOUT)
    }
}

fn require_python3() {
    let ok = Command::new("python3")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        return;
    }
    if gate_mode() == GateMode::Required {
        panic!("WYOMING_PYINTEROP=required but python3 is unavailable");
    }
    eprintln!("pyinterop: SKIPPED — python3 unavailable");
}

/// Assert no process matching `pattern` survives (unix only; visible note
/// where pgrep is unavailable). Review W-3: failing harnesses must leave no
/// orphan python behind.
#[cfg(unix)]
fn assert_no_leftover(pattern: &str) {
    match Command::new("pgrep").arg("-f").arg(pattern).output() {
        Ok(out) if out.status.success() => panic!(
            "leftover child processes matching {pattern:?}:\n{}",
            String::from_utf8_lossy(&out.stdout)
        ),
        Ok(_) => {}
        Err(_) => eprintln!("pyinterop: pgrep unavailable, leftover check skipped"),
    }
}

#[cfg(not(unix))]
fn assert_no_leftover(_pattern: &str) {}

/// Pure regression for gate-mode parsing — no process-env mutation, so it
/// is safe alongside the env-sensitive tests. Unset/empty stay optional
/// (0dd4111 portable default), `required` stays strict, and every other
/// nonempty value (typo, wrong case, garbage) is rejected with a
/// diagnostic naming the offending value instead of silently degrading
/// into the optional skip.
#[test]
fn gate_mode_parsing_rejects_unknown_values() {
    assert_eq!(parse_gate_mode(None), Ok(GateMode::Optional));
    assert_eq!(parse_gate_mode(Some("")), Ok(GateMode::Optional));
    assert_eq!(parse_gate_mode(Some("required")), Ok(GateMode::Required));
    for bad in [
        "Required", "REQUIRED", "reqired", "strict", "skip", "true", "1", "required ",
    ] {
        let err = parse_gate_mode(Some(bad))
            .expect_err("unknown nonempty mode must be rejected, not treated as optional");
        assert!(
            err.contains(bad) && err.contains("not a recognized gate mode"),
            "diagnostic must name the offending value {bad:?}: {err}"
        );
    }
}

// ---------------------------------------------------------------------------
// The real gate: 59 events against the Python reference
// ---------------------------------------------------------------------------

#[test]
fn python_reference_interop_roundtrip() {
    let Some(reference) = ref_dir() else { return };
    let ver = Command::new("python3").arg("--version").output().expect("spawn python3");
    assert!(ver.status.success(), "system python3 must run");
    println!(
        "pyinterop: python3 {}",
        String::from_utf8_lossy(&ver.stdout).trim()
    );

    let script = test_path("pyinterop_server.py");
    let pythonpath = pyref_pythonpath(&reference);
    println!("pyinterop: server cmd: python3 {script:?} (PYTHONPATH={pythonpath})");
    let mut h = Harness::spawn(&script, &[], Some(&pythonpath));

    // Identity (review W-6): the imported module must be the reference
    // checkout, and its actual git commit is recorded — the stub metadata
    // version is reported, never trusted.
    let ref_line = h.wait_line("REF ", READY_TIMEOUT);
    let mut fields = ref_line.split_whitespace();
    let module = fields.next().expect("REF module path");
    let version = fields.next().unwrap_or("unknown");
    let module_parent = Path::new(module).parent().and_then(|d| d.canonicalize().ok());
    assert_eq!(
        module_parent.as_deref(),
        reference.join("wyoming").canonicalize().ok().as_deref(),
        "imported wyoming package must live in the reference checkout, got {module}"
    );
    let head = Command::new("git")
        .args(["-C", &reference.display().to_string(), "rev-parse", "HEAD"])
        .output();
    match head {
        Ok(o) if o.status.success() => println!(
            "pyinterop: reference commit {}",
            String::from_utf8_lossy(&o.stdout).trim()
        ),
        _ => println!("pyinterop: reference commit UNKNOWN (checkout is not a git repo)"),
    }
    println!("pyinterop: reference module {module} (metadata version {version})");

    let port = h.wait_port_in(READY_TIMEOUT);

    // Rust client over TCP against the Python server.
    let mut framed =
        wyoming::transport::connect(("127.0.0.1", port)).expect("connect python server");
    framed
        .get_mut()
        .set_read_timeout(Some(EXCHANGE_TIMEOUT))
        .expect("set read timeout");
    let mut client = Client::new(framed.into_inner());

    let events = golden_events();
    let mut sent = 0usize;
    let mut received = 0usize;
    for event in &events {
        client.send(event).expect("send frame to python");
        sent += 1;
        let transitions = client.recv().expect("recv frame from python");
        let got = transitions
            .iter()
            .find_map(|t| match &t.kind {
                TransitionKind::Received(ev) => Some(ev.clone()),
                _ => None,
            })
            .expect("a Received transition");
        received += 1;
        if event.type_ == "ping" {
            assert_eq!(got.type_, "pong", "python answers ping with pong");
            assert_eq!(got.get("text"), event.get("text"), "pong copies ping text");
        } else {
            assert_eq!(
                got, *event,
                "event {sent} ({}) round-trip through python",
                event.type_
            );
        }
        println!("pyinterop: sent {:<22} -> got {}", event.type_, got.type_);
    }
    assert!(
        sent >= 59 && received >= 59,
        "must exchange >=59 events per direction, got {sent} sent / {received} received"
    );

    // close() shuts the transport down: the Python read_event sees EOF,
    // prints SERVED, and exits cleanly — all reads bounded, so a close()
    // regression fails red instead of hanging (review W-1).
    let t = client.close();
    assert_eq!(t.to, ClientState::Closed);
    let served = h.wait_line("SERVED ", SERVED_TIMEOUT);
    assert_eq!(served, format!("{sent}"), "python served every event");
    let status = h
        .guard
        .wait_bounded(EXIT_TIMEOUT)
        .expect("python server exited after SERVED line");
    assert!(status.success(), "python server exited cleanly");
    println!("pyinterop: exchanged {sent} sent / {received} received; python EOF + exit 0");
}

// ---------------------------------------------------------------------------
// Fault injection: induced failures must be bounded and leave no child
// ---------------------------------------------------------------------------

/// W-5: a server that stalls before printing PORT must time out red, not
/// hang the suite; the child must be killed.
#[test]
fn fault_stalled_port_fails_bounded() {
    require_python3();
    let mut h = Harness::spawn(
        &test_path("pyinterop_fault_server.py"),
        &["stall-port", "--tag", "fault-stall-port"],
        None,
    );
    let start = Instant::now();
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| h.wait_port()));
    let elapsed = start.elapsed();
    assert!(outcome.is_err(), "stalled PORT must fail the harness, not hang");
    // 100 ms slack: `recv_timeout` may wake a hair early under load.
    assert!(
        elapsed + Duration::from_millis(100) >= PORT_TIMEOUT
            && elapsed < PORT_TIMEOUT + Duration::from_secs(5),
        "PORT timeout must be bounded by {PORT_TIMEOUT:?}, took {elapsed:?}"
    );
    assert_no_leftover("pyinterop_fault_server.py stall-port");
}

/// W-1: a close() that never delivers EOF (server stuck in read_event)
/// must fail the SERVED wait red within the deadline, not hang forever.
#[test]
fn fault_stuck_served_fails_bounded() {
    require_python3();
    let mut h = Harness::spawn(
        &test_path("pyinterop_fault_server.py"),
        &["stall-served", "--tag", "fault-stall-served"],
        None,
    );
    let port = h.wait_port();
    let mut framed =
        wyoming::transport::connect(("127.0.0.1", port)).expect("connect fault server");
    framed
        .get_mut()
        .set_read_timeout(Some(EXCHANGE_TIMEOUT))
        .expect("set read timeout");
    let mut client = Client::new(framed.into_inner());

    // Byte-level echo: the full 59-event exchange completes.
    for event in &golden_events() {
        client.send(event).expect("send frame to fault server");
        client.recv().expect("recv echo from fault server");
    }
    client.close();

    // The fault server never prints SERVED (stands in for a close() that
    // lost its EOF): the wait must go red within the deadline.
    let start = Instant::now();
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
        h.wait_line("SERVED ", SERVED_TIMEOUT)
    }));
    let elapsed = start.elapsed();
    assert!(outcome.is_err(), "missing SERVED must fail red, not hang");
    // 100 ms slack: `recv_timeout` may wake a hair early under load.
    assert!(
        elapsed + Duration::from_millis(100) >= SERVED_TIMEOUT
            && elapsed < SERVED_TIMEOUT + Duration::from_secs(5),
        "SERVED timeout must be bounded by {SERVED_TIMEOUT:?}, took {elapsed:?}"
    );
    assert_no_leftover("pyinterop_fault_server.py stall-served");
}

/// W-4: a child that crashes before readiness must have its stderr
/// surfaced in the failure message, not swallowed.
#[test]
fn fault_crash_stderr_preserved() {
    require_python3();
    let mut h = Harness::spawn(
        &test_path("pyinterop_fault_server.py"),
        &["crash-stderr", "--tag", "fault-crash-stderr"],
        None,
    );
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| h.wait_port()));
    let err = outcome.expect_err("crash before PORT must fail the harness");
    let msg = err
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| err.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .unwrap_or_default();
    assert!(
        msg.contains("PYINTEROP-CRASH-MARKER"),
        "child stderr diagnostics must be preserved in the failure:\n{msg}"
    );
    assert_no_leftover("pyinterop_fault_server.py crash-stderr");
}

/// W-3: dropping the guard (the panic-unwind path) kills and reaps the
/// child; nothing survives.
#[test]
fn guard_kills_and_reaps_child_on_drop() {
    require_python3();
    let child = Command::new("python3")
        .args(["-c", "import time; time.sleep(300)", "guard-drop-probe"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn sleep probe");
    drop(ChildGuard::new(child));
    assert_no_leftover("guard-drop-probe");
}
