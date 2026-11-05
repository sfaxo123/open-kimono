# EVIDENCE — host test re-run + device/emulator receipts

- Date (UTC): 2026-11-04 ~23:10 UTC (Nov 4, ~3:10 PM PT)
- Base commit: `2d38f16` (`git rev-parse --short HEAD` at run time)
- Host: `x86_64` (`uname -m`)
- Command per suite: `timeout 500 cargo test --offline --manifest-path <path>` (cargo 1.99.1, offline)
- Totals: **31 suites, 1327 passed, 3 failed, 1 ignored.** 29 suites fully green, 1 failed, 1 did not build offline.
- Code was not fixed. Failures recorded as-is.

## 1. Host test suites (all real re-runs, tonight)

Result per suite = sum over that suite's test targets. Only failures reproduced verbatim below.

| suite | result (summed `test result` lines) | run time |
|---|---|---|
| apps/agentd | ok — 153 passed, 0 failed (22 targets) | 16s |
| apps/kimono-ui | ok — 7 passed, 0 failed (2 targets) | 0s |
| apps/net-probe | ok — 0 passed, 0 failed (1 target, no tests) | 0s |
| apps/sandbox-demo | ok — 3 passed, 0 failed (1 target) | 0s |
| apps/timeline | ok — 33 passed, 0 failed (9 targets) | 0s |
| apps/voice-bridge | ok — 22 passed, 0 failed (4 targets) | 0s |
| apps/voiced | ok — 59 passed, 0 failed (7 targets) | 13s |
| apps/wyoming-probe | ok — 16 passed, 0 failed (2 targets) | 2s |
| crates/a2ui-render | ok — 118 passed, 0 failed (12 targets) | 0s |
| crates/caps-check | ok — 3 passed, 0 failed (2 targets) | 0s |
| crates/device-control | ok — 166 passed, 0 failed (19 targets) | 0s |
| crates/device-control-host | ok — 30 passed, 0 failed (6 targets) | 1s |
| crates/dwc3-gadget | ok — 38 passed, 0 failed, 1 ignored (6 targets) | 0s |
| crates/fb-uefi-boot | **did not build offline** — `error: unwinding panics are not supported without std` + `error[E0152]: duplicate lang item` (no-std UEFI binary, expected) | 0s |
| crates/fb-write | ok — 17 passed, 0 failed (4 targets) | 0s |
| crates/fdt-diff | **FAILED — 39 passed, 3 failed** (8 targets) | 0s |
| crates/geni-i2c | ok — 4 passed, 0 failed (3 targets) | 0s |
| crates/geni-uart | ok — 46 passed, 0 failed (3 targets) | 0s |
| crates/lfm3-brain | ok — 102 passed, 0 failed (11 targets) | 4s |
| crates/mmio-replay | ok — 38 passed, 0 failed (3 targets) | 0s |
| crates/op6-boot | ok — 21 passed, 0 failed (5 targets) | 0s |
| crates/qcom-rproc | ok — 8 passed, 0 failed (3 targets) | 0s |
| crates/rmi4-touch | ok — 10 passed, 0 failed (5 targets) | 0s |
| crates/sandbox | ok — 39 passed, 0 failed (8 targets) | 0s |
| crates/spmid | ok — 8 passed, 0 failed (3 targets) | 0s |
| crates/svcd | ok — 76 passed, 0 failed (9 targets) | 1s |
| crates/trace-events | ok — 36 passed, 0 failed (8 targets) | 0s |
| crates/tracejournal | ok — 7 passed, 0 failed (5 targets) | 0s |
| crates/ufs | ok — 47 passed, 0 failed (6 targets) | 1s |
| crates/usbgadgetd | ok — 94 passed, 0 failed (1 target) | 4s |
| crates/wyoming | ok — 87 passed, 0 failed (8 targets) | 15s |

Two non-green suites, verbatim:

- `crates/fdt-diff` — 3 failures, all in `--test fdt_real_pair`, all missing-external-artifact panics, not code bugs:
  - `test result: FAILED. 0 passed; 3 failed; 0 ignored; 0 measured; 0 filtered out`
  - `error: test failed, to rerun pass '--test fdt_real_pair'`
  - `HW-22 evidence artifact B missing/unreadable: $HOME/artifacts/handoff/oneplus-redox-20261024/phone-boot/u-boot-…` (`crates/fdt-diff/tests/fdt_real_pair.rs:178`)
  - The other 7 targets in this suite pass (39 passed).
- `crates/fb-uefi-boot` — no-std UEFI crate, cannot link the test harness offline. Did not fix (brief forbids it).

## 2. Device evidence — what is PHONE, what is EMULATOR

Rule used: PHONE only if the file itself names the phone (hostname `oneplus-enchilada`, `aarch64` device probe, `sdm845`). Filename claims without in-content markers are marked unverified.

### docs/track-a/evidence/ (all 11 files)

- `A0-phone-smoke.trace` — **PHONE** (proven). Content holds `"arch":"aarch64", "hostname":"oneplus-enchilada", "libc":"musl", "os_release":"7.1.0-rc1-sdm845", "target":"aarch64-unknown-linux-musl"`. The only trace with a device probe.
- `A0-phone-countdown.trace` — filename says phone (countdown-timer service trace). No hostname/arch markers in content. Unverified.
- `A1-phone-real-request.trace` — agentd `RUN_STARTED`, `threadId":"phone"`. No device markers. Unverified.
- `A2-real-agentd-sse.trace` — agentd trace, `threadId":"phone"`. No device markers. Unverified.
- `a3-water-reminder.trace` — water-reminder run. The only `virtual*` hits are virtual-timer test names (`virtual_end_ms`), not emulator markers. No device markers. Unverified, host-side.
- `A4-phone-agentd.trace` — agentd run, `threadId":"phone"`. No device markers. Unverified.
- `A4-phone-quiet-test.trace` — voiced quiet test ("What is the battery level right now?"). No device markers. Unverified.
- `A4-voiced-smoke.trace` — voiced water-reminder smoke. No device markers. Unverified.
- `A5-host-power.trace` — **HOST** (filename says host; agentd power run, no device markers).
- `a5-phone-power.trace` — filename says phone. No device markers. Unverified.
- `final-demo-water-reminder.trace` — demo water-reminder trace. No device markers. Unverified.
- No file in this dir mentions qemu/emulator/x86_64 (grep-verified).
- Context: `docs/track-a/A3-PHONE-DEMO.md` says the phone demo is "not yet run" and "the phone left the mesh", so filename-only phone claims above stay unverified.

### docs/workbench/ — EMULATOR (files say so themselves)

- `K1-EVIDENCE.md` — host + QEMU evidence (title says it).
- `K3-RAMBOOT.md` — QEMU boot without a drive.
- `K5-QEMU-REDOX.txt`, `K5-QEMU-DIAG.txt` — QEMU boot logs (U-Boot 2026.10, 2 GiB DRAM).
- `K8-QEMU.txt`, `K8B-QEMU.txt`, `K8C-QEMU.txt`, `K14-QEMU.txt` — QEMU run logs.
- `E1-REDOX-SERIAL.txt` — QEMU serial (`Start PXE over IPv4`, station IP 10.0.2.15 = QEMU user-net).
- `QEMU-SANDBOX-DEMO.txt` + `QEMU-SANDBOX-DEMO-runs.tsv` — QEMU sandbox demo.
- `EMU-OP6-HARNESS.md`, `EMU-TOUCH-E2E.md` (explicit: "not a physical phone test"), `EMU-BUTTONS-E2E.md`, `EMU-UFS-RO.md`, `EMU-TOUCH-TESTS.txt`, `SPMID-EMU-CRASH.md` — emulator harness/tests.
- `evidence/20261025T052814Z-*-serial.txt` + `-receipt.tsv`, `evidence/20261025T054222Z-*-serial.txt` — aarch64 UEFI/Redox boot serials (QEMU-style PXE/UEFI lines); receipts note they were back-filled, not written at boot.
- `K14-FBCON.md`, `E1-REDOX-DEMO.md` — state "no phone access" outright.
- `E1-WYOMING-PROBE.md` — host interop + offline aarch64 cross-build. HOST.
- `E1-NET-EXCHANGE.txt`, `E1-NET-NATIVE-PROBE.txt`, `K13-CARGO-TEST.txt`, `K9-TESTS.txt`, `K10-UFS-TESTS.txt`, `K11-TESTS.txt`, `K12-TRACE-TESTS.md`, `B4-SVCD-TESTS.txt`, `B5-SANDBOX-TESTS.txt` — host test/probe logs. HOST.
- `user-simulations/key-tests/KEY-TEST-1-RESULTS.md` — HOST user-sim: Muse Wonder as phone brain via API-side RUST shim, 16 scenarios / 39 turns, mechanical 11/16, behavioural 14/16.
- `user-simulations/key-tests/KEY-TEST-1-PHONE-RUN.md` — phone run sheet (plan: "phone in hand"), not a completed phone trace.

## 3. Meta models in this project (verified)

- Lane C contributor workers run `muse-spark-1.3-contributor` via the Meta Model API, one packet each, one commit, budget-capped (`docs/hackathon/LANE-C.md`, "Lane C — cloud contributors" amendment 2026-10-28).
- `muse-wonder-1.0` reviewed every new diff as independent reviewer and acted as the phone-brain under test in KEY-TEST-1 (`docs/hackathon/LANE-C.md` provenance line; `docs/workbench/user-simulations/key-tests/KEY-TEST-1-RESULTS.md`).
- Making Muse Spark the on-phone agent brain via the Meta Model API: in progress (`docs/hackathon/MUSE-BRAIN.md` does not exist yet).
