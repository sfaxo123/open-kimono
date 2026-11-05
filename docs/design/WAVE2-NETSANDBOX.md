# Wave 2 design — Wyoming client over QEMU user-net + wasmi hello on the E1 live image

Authored in commit `e31394b` at 2026-10-24 7:08:28 PM PT by the lean local-5090
orchestrator (`LOCAL_REDOX_ORCH_LEAN_20261024`). This is a design and gate plan;
it did not run the target build or contact a device.

Current integration state checked 2026-10-24 7:20 PM PT: D2 merged at
`4e07a21`; its DSH session is idle with its goal complete and its worktree is
clean at `c1fa33c`. S1 merged at `633fdf3`; its session is idle and its
worktree's untracked `specs/tlmm.md` is byte-identical to main. No owner
confirmation for the S1 merge is recorded. Preserve both worktrees; do not
remove either without its owner's confirmation.

## 0. Assets verified to exist (re-checked, not copied from handoff)

| Asset | State | Evidence |
|---|---|---|
| `crates/wyoming` | framing + typed events done, cross-checked vs Python reference | `docs/workbench/wyoming-crosscheck.md`, 59 golden typed events, `tests/framing.rs`, `tests/golden_fixtures.rs` |
| `crates/sandbox` (B5) | wasmi 2.0.0, `op6` host-fn ABI, grant gating, fuel/memory limits | merged `2f3e7c6`; `docs/workbench/B5-SANDBOX-TESTS.txt` (31 tests, branch `crates/sandbox @ 535c2d7`) |
| Guest example | `crates/sandbox/guests/hello-rs` (source only — **no checked-in `.wasm`**) | dir listing |
| E1 live image | `scripts/e1-live.sh` builds `e1-live.iso` from fork trees `base-e1/kernel-e1/bootloader-e1`, staging dir `build/e1-staging`, output `out/integration/` | script head; `out/integration/` logs incl. `qemu.log`, `initfs-pack.log` |
| QEMU net | `scripts/e1-qemu.sh` configures `-netdev user` and `virtio-net-pci`; the captured PXE log shows guest `10.0.2.15` and host `10.0.2.2` | `scripts/e1-qemu.sh:23-24`, `docs/workbench/E1-REDOX-SERIAL.txt` |

## 1. Gap analysis

### 1.1 Wyoming client (B2 over user-net)
`crates/wyoming` today is **protocol-only**: frame codec + typed events, validated
against the Python reference by golden fixtures. It has no socket transport and no
client state machine. Missing pieces, in build order:

1. **Transport adapter** — a thin `connect(addr) -> TcpStream` wrapper implementing a
   `Read`/`Write` sink for the existing `read_frame`/`write_frame` codec. Keep the codec
   pure (it is host-testable); put the socket behind a trait so the same state machine
   runs over a `UnixStream`/`Vec` mock in tests.
2. **Client state machine** — Wyoming event flow: send `Ping`/audio `Start→Continue→Stop`,
   receive `Transcript`/`AudioStart/AudioContinue/AudioStop`/`Speech`/`Wake`. Model as an
   explicit state enum; every transition emits a `tracejournal` event (B1 hook).
3. **Loopback host test** — run the Python `wyoming` reference server (fork at
   `op6-refs/wyoming @ 08a0c25`) on `127.0.0.1` against the Rust client; assert the 59
   golden events round-trip unchanged. No QEMU needed for this gate.
4. **QEMU user-net gate** — QEMU `-netdev user` gives the guest `10.0.2.x`; host services
   reachable at `10.0.2.2`. Test: guest-side client connects to a host-side fake Wyoming
   server, one `Ping`/`Pong` round-trip logged in the QEMU serial output. Add a serial
   assertion to the existing e1 QEMU harness (pattern: K-series `QEMU-PROOF.txt` asserts).

Risks: Redox TCP stack maturity under `aarch64-desktop-minimal` is unproven for outbound
user-net connections (never exercised); DNS is not needed (use literal `10.0.2.2`).
Audio payloads stay out of scope for the first gate — control-plane events only.

### 1.2 wasmi hello on the E1 live image (B5 × E1)
B5 is proven **on the host** only. The E1 image has never loaded a Wasm module. Steps:

1. **Next device-free gate: build the sandbox for Redox.** Run this in a
   visible DSH session from the repository root, after the caps-check review
   and focused tests pass:

   ```bash
   cargo build --locked --manifest-path crates/sandbox/Cargo.toml --target aarch64-unknown-redox
   ```

   This build has not yet been run. Record the DSH session ID and actual
   result. A pass proves target compilation only. If it fails, preserve the
   compiler output and stop to revise the plan before guest construction or
   QEMU staging.
2. **Build the guest to wasm32-unknown-unknown** — add a build step producing
   `hello.wasm` from `guests/hello-rs` (tiny, `default-features = false`, no WASI; the
   `op6` ABI is the only import). Commit the built `.wasm` or rebuild it in
   `e1-live.sh` staging (prefer rebuild; keep repo binary-free).
3. **Host binary** — a `sandboxd`-style Redox binary linking `crates/sandbox`.
   The actual wasmi target build above is the first compile gate and its
   result remains unknown.
4. **Stage into `e1-staging`** — add `hello.wasm` + host binary to the live ISO via
   `e1-live.sh`; run at login or via the existing timeline demo entry point.
5. **Gate** — QEMU serial shows the hello service's `TraceEmit` host event text;
   refusal path also asserted (run the guest with the Trace capability grant withheld →
   expect a refusal event). There is no `trace_without_grant_is_refused` test
   today. The nearest existing host tests are
   `timer_without_grant_is_refused` in `crates/sandbox/tests/timers.rs` and
   `rust_built_service_without_kv_set_is_refused` in
   `crates/sandbox/tests/rust_guest.rs`; a TraceEmit-specific denial test still
   needs to be implemented and does not follow from those tests.

Dependency order: perform the Redox-target build above first as a compile-only
spike. If it fails, keep that as a blocker and revise the design with evidence;
do not silently replace the on-Redox gate with a host-side sandbox run.

## 2. Nonconflict statement
- Touches: `crates/wyoming` (new transport module), `crates/sandbox` (target spike),
  `scripts/e1-live.sh`, new workbench evidence files.
- Does **not** touch: `docs/design/CAPS-SANDBOX.md`, `tools/caps-check/` (D2),
  `specs/tlmm.md` (S1), kernel/DT patches.
- Suggested lanes if dispatched later: `net/wyoming-client` and `b5/e1-wasm`.

## 3. Current execution state

- Main is `8100701`; the D2, S1, and WAVE2 merges are recorded above. Caps-check
  hardening and `scripts/integrate.sh` gate wiring are complete in the shared
  checkout; coordinator review and focused tests are pending. Preserve those
  changes.
- The Redox-target build is the next device-free gate and has not been
  dispatched yet. It requires visible DSH execution; record its session and
  result before advancing to guest build or QEMU.
- The operator has authorized commit and push. No further confirmation is
  needed for those actions; report actual Git state after checking it live.
- K14 and earlier QEMU results remain emulator evidence. They do not verify a
  phone display or Redox boot on the OnePlus 6.
