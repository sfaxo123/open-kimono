# Redox on the real OnePlus 6: device runs (excerpt from the private run ledger)

RAM-only `fastboot boot`. Nothing is flashed, and postmarketOS stays installed. Each cycle boots a Redox image built here. The kernel arms its own timer, then resets the phone, which returns to postmarketOS without anyone touching it. The op6-cycle tool on the USB host records a receipt for every cycle. These are the original entries, with host paths and identifiers redacted.

## 2026-10-31 ~21:00 PT - F-1 COMPLETE: the SCM line is photographable; burst capture live (Remy, root)

Closes REVIEW-20261031-1930 F-1. (a) Kernel patch 0039 (c47bec5):
kreset() holds ~5 s (busy-wait on cntvct_el0) after printing the SCM
result, behind the same qcom-dload-mode-clear feature; emergency_reset
unchanged. (b) Tooling: op6-cycle bursts frames from TTL-10 until
classification ends, keeps the last 3, OCRs them, and records
webcam_scm_frame (9a9efc3); ATTENDED-ONLY allowlist entries now refuse
without --attended (476e726; tests 38/38 on the Mac).

Image: out/r1-v4-t60h/op6-embedded-efi-redox-v4-reset.img sha256
95317c8941913b16efabf736234c64ef7088fafb3a97852c051d7161bd734393
(manifest match; kernel top ac9ec17a = 0038+0039; binary contains
"download-mode cleared" and "holding 5 s"). Gate at 8 GiB: reboot
scenario PASS (out/emu-op6/root-p2-t60h-reboot/RESULT.json, serial
e38cae48 - serial shows the SCM WARN and the hold line, then the PSCI
reset). The ttl scenario hit the runner's 90 s wall bound twice
(serials 8845ef61, 00342f78; shell + RAMBOOT line present, guest clock
crawling under host load average ~9) - recorded as emulator-bound, NOT
a pass; the reset path it would exercise is the one the reboot
scenario proved, and the TTL trigger itself is proven on hardware
below.

Device verification, 3/3 unattended cycles PASS (driver verify3 on
the Mac; receipts in out/device-receipts/): T034349Z sha 42270e2f
boot->pmOS 101 s; T034859Z sha f7ea204e 102 s; T035411Z sha 25b6c545
102 s. Against the t60 baseline of 97-98 s, the +5 s hold is visible
in every cycle. Webcam fields record not-authorized: the burst
machinery ran, but the operator's one-time camera Allow on the Mac
(GUI-session TCC prompt) is still outstanding, so webcam_scm_frame
on a real frame is the one F-1 proof still open. Claude review of
0039 + the burst/attended tooling requested (docs/reviews).

## 2026-10-31 ~20:00 PT — P2 COMPLETE: SCM success print + TTL-60 image 8 GiB-gated + R-3 webcam receipts + 5/5 consecutive unattended cycles PASS (Remy, root)

P2 of `docs/reviews/PLAN-20261031-agent-control.md` is COMPLETE.

- **SCM observability:** kernel patch 0038 prints `qcom SCM: download-mode cleared` on
  clear success at both reset entrypoints (790ae67; revised 368458b after the pinned
  toolchain's `semicolon_in_expressions_from_macros` lint rejected macro calls in
  match-expression position — the arms are brace-wrapped into statement position;
  first build failure preserved in the build worktree's `out/kernel-build-p2-0038.log`).
  QEMU refuses the SCM clear, so both gate scenarios show the pre-existing WARN line:
  the 0038 path is proven to execute, but the SUCCESS line itself is still unobserved
  until a panel read exists (webcam permission, below).
- **TTL-60 image:** `out/r1-v4-t60/op6-embedded-efi-redox-v4-reset.img` sha256
  `eed4d1e56f9702ab1ebb587d4d260bdbcdf0c92eb6a3583cc5536045e5b3b064`, built from main
  368458b. **8 GiB oracle gate, root-verified against the files:** ttl PASS
  (`out/emu-op6/root-p2-t60-ttl/RESULT.json`, serial sha `8acb0de4…`, 84.0 s) and
  reboot PASS (`out/emu-op6/root-p2-t60-reboot/RESULT.json`, serial sha `e73bdc08…`,
  23.5 s), both `qemu_exited_after_reset=true`, and `input_image_sha256` == image sha
  == manifest `android_image_sha256`. Allowlist pin committed (3d1823d).
- **R-3 webcam:** `op6-cycle --webcam` + `op6-webcam.sh` / `op6-cam.swift` /
  `op6-ocr.swift` (ebde6d0); unit tests **27/27** on the Mac. Every P2 receipt carries
  `webcam_status` / `webcam_detail`. **Limitation:** capture from SSH-launched
  processes returns "camera not authorized" (macOS TCC), so receipts record
  `not-authorized`; one operator Allow click at the Mac is queued. Fail-soft held:
  no cycle outcome was affected.
- **Reliability run** (driver `p2_run5.sh` on the Mac, 2026-11-01T02:06Z–02:33Z):
  **5/5 consecutive PASS**, no retries needed. Receipts copied to
  `out/device-receipts/` (originals on the Mac): T020837Z sha `1821ba23…`
  (boot→pmOS 98 s, total 231 s), T021347Z sha `1d6ba687…` (97 s / 229 s),
  T021857Z sha `ba272bec…` (97 s / 227 s), T022403Z sha `11492122…`
  (97 s / 226 s), T022907Z sha `7162511d…` (97 s / 229 s); battery 100% before
