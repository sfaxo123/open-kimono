# E1-WYOMING-STAGE-PREP — Wyoming staging/boot wrapper (host-only authoring)

- Date: 2026-10-25 (Codex-authorized prep window, 6:28 AM PT). Hard bound: 20 min, host-only.
  Review pass (same day, ~7:30 AM PT) applied the Codex/Luna review fixes below.
- Branch: `wyoming-stage-prep`, forked from current `origin/main` @ `595c184`
  (`git -c core.excludesFile=/dev/null fetch origin; checkout -b wyoming-stage-prep origin/main`).
  Note: the brief named `a7e7ecb`; at fetch time `origin/main` had advanced one docs commit to
  `595c184` ("docs: accept reviewed Wyoming host probe and set next preparation gate"), a direct
  descendant of `a7e7ecb`. Branching from current origin/main as instructed.
- New files ONLY (no existing script/image touched):
  - `scripts/qemu-demo/e1-wyoming-stage.sh` — wyoming-specific staging/boot wrapper
  - `scripts/qemu-demo/e1-wyoming-stage-tests.sh` — FAKE-ONLY host tests (stubs, no qemu/mounts/images)
  - this doc
  - scratch: `out/e1-wyoming-stage-prep/` (gitignored; test logs `tests-run1.log`, `tests-run2.log`)

## What this goal did NOT do (explicit)

NO staging, NO mounts, NO copies to any image, NO image writes, NO VM/boot, NO downloads, NO phone.
The `stage` and `boot` subcommands exist as gated code paths but were NEVER executed against any
real image or toolchain in this goal — only against `/tmp` stubs inside the test suite.

## Design

Subcommand-gated: `dry-run | stage | boot`.

```
e1-wyoming-stage.sh <dry-run|stage|boot> --phase wy-<UTCstamp>[-tag] --image <path> \
    [--manifest <tsv>] [--binary <path>] [--approve] [--boot-cmd <cmd>]
```

- Guest artifact: `usr/bin/wyoming-probe` (aarch64-unknown-redox build). Pinned evidence sha256
  `a0ddc407ef8e85aff0260ccea2788b429ce5d22f44158e639e38555817167fcb` — verified against repo
  evidence (`docs/workbench/E1-WYOMING-PROBE.md:236`, `docs/HANDOFF.md:22`), NOT copied blindly
  from the brief (the brief's copy was truncated one hex digit short). The ELF was never consumed:
  it EXISTS in the sibling worktree at
  `redox-oneplus6-wy/out/wyoming-probe/redox/aarch64-unknown-redox/release/wyoming-probe`
  (verified present, sha matches the pin). No rebuild needed; a future dry-run passes it via
  explicit `--binary` under separate review.
- Manifest (`out/e1-wyoming-stage-prep/manifest.tsv`, TSV `role\tpath\tsha256`) is the ledger the
  image and binary hashes are checked against BEFORE any use; a manifest that contradicts the
  pinned evidence sha fails closed.
- Phase-ID namespace: NEW `wy-<10-digit epoch-seconds>[-tag]` — a Unix epoch timestamp, NOT a
  UTC calendar date — claimed ONCE by atomic `mkdir` of
  `out/e1-wyoming-stage-prep/<phase>/` (retry → exit 4). The existing `e1-staging-check.sh` stays
  pinned to the net-probe binary + its consumed phase-ID and is untouched.
- `dry-run`: validates args numerically (exit 2 before any side effect), verifies inputs exist,
  manifest hashes match, pinned evidence agrees, protected images match their ledgers, claim is
  fresh — then claims the phase and prints the exact plan. Zero writes outside
  `out/e1-wyoming-stage-prep/`.
- `stage`: requires explicit `--approve` (Codex budget). Input image is NEVER written: a
  byte-identical disposable copy (`cp -a --reflink=auto`, inode-distinct + sha guard) is made,
  baseline read-only walk via the hash-recomputing `redoxfs-clone` must PASS before any mount
  (stop-before-mount), then RW mount in an owned `setsid` PGID, insert ONLY `usr/bin/wyoming-probe`
  (sha-verified, chmod 755, cmp + hash readback), bounded unmount + writer reap, post-walk verify,
  protected-image after-hash verify.
- `boot`: DISABLED this phase — fails closed with `BOOT_NOT_IMPLEMENTED` BEFORE any side effect
  (before claim, copy, or mount) and requires a separately reviewed dedicated Wyoming QEMU
  harness (594dbda prompt-driven lineage: bootloader → login → `root:~#` → shell). `--boot-cmd`
  is still parsed so tests can prove it NEVER runs; the wrapper executes no boot command and
  makes no rc0-as-payload claim. No fixed-sleep pattern anywhere — the fixed-25s-sleep wrapper
  was rejected and its pattern is not reused.

## Guard list (inherited from staging-check lineage 8a824e5, wyoming-specific)

1. Explicit approved phase-ID argument required for every subcommand; format `wy-<UTCstamp>[-tag]`.
2. Phase claimed ONCE via atomic mkdir; any retry refuses (exit 4).
3. `stage`/`boot` refuse without `--approve` (exit 2) — Codex budget is not implied.
4. Numeric/format validation of all args/timeouts before any side effect (exit 2).
5. Image + binary sha256 must match the manifest before ANY use; manifest vs pinned evidence
   contradiction fails closed (exit 1).
6. Protected images verified against pinned ledgers before AND after (and by the EXIT-trap
   cleanup receipt on every failure path, so after-hashes are never silently skipped): original
   ISO `c02012e…`, old bad image `out/e1net-native/e1net-probe.iso` `87ae0aa8…`, and armB.iso
   `out/e1-staging-check/20261025T113022Z/armB.iso` `c91ab533…` (pinned from
   `e1-net-native-from-image.sh:49-50`); mismatch → exit 1. Only an ABSENT file skips (recorded).
7. Input image never written: disposable copy only (inode-distinct + byte-identical guards);
   the INPUT image is additionally re-hashed AFTER staging and a mutation fails the verdict.
8. Read-only walks via `redoxfs-clone` (hash-recomputing) before and after the mount; baseline
   failure stops BEFORE mount (stop-before-B).
9. Clone scratch: exclusive create (`noclobber`), path-alias AND dev:inode alias guards vs source,
   original ISO, old bad image, and armB.iso.
10. Source image re-hashed across each walk (unchanged or guard trip).
11. Guest-path escape guard BEFORE any copy: `$mnt/usr/bin/wyoming-probe` must resolve inside
    the canonical mount; a symlink (or symlinked parent) escaping the mount → `INSERT_PATH_ESCAPE`,
    no write.
12. Bounded TERM→KILL reap of owned `setsid` PGIDs only (writer group kept registered after
    `wait` so its children are still reaped); bounded unmount with one retry; no global pkill;
    INT/TERM traps cleanup then hard-exit (130/143).
13. `WS_*` env seams exist ONLY for the fake host tests; they redirect PATHS/commands only —
    pinned shas stay enforced against whatever file exists there — and are NOT an admission
    mechanism; never set in real runs. The production binary pin has no bypass: the pin-
    contradiction test deliberately does NOT override `WS_WY_BINARY_SHA`.

## Test suite — actual output (review pass, recorded in `out/e1-wyoming-stage-prep/tests-run3.log`)

`bash scripts/qemu-demo/e1-wyoming-stage-tests.sh` → **rc 0**, 39/39 PASS, FAKE-ONLY
(stub `redoxfs`/`redoxfs-clone`/`mountpoint`/`umount` + evil-symlink and input-tamper stubs,
4 KiB fake image, no qemu, no mounts, no real images, scratch under `mktemp -d`):

```
PASS: T1 missing phase-ID -> 2
PASS: T2 bad phase-ID ns -> 2
PASS: T3 stage no --approve -> 2
PASS: T3b no claim consumed by refusal
PASS: T4 boot no --approve -> 2
PASS: T5 boot --approve + --boot-cmd -> 2
PASS: T5b BOOT_NOT_IMPLEMENTED disclosed
PASS: T5c --boot-cmd never executed
PASS: T5d no claim/copy on boot refusal
PASS: T6 hash mismatch -> 1
PASS: T6b no claim on fail-closed
PASS: T7 dry-run -> 0
PASS: T7b claim created
PASS: T7c dry-run creates NO arm image
PASS: T7d dry-run verdict recorded
PASS: T8 consumed claim -> 4
PASS: T9 baseline fail -> 1
PASS: T9b stopped before mount (no mnt dir)
PASS: T9c stop-before-mount verdict
PASS: T9d cleanup receipt discloses after-hashes on failure
PASS: T10 stage --approve (stubs) -> 0
PASS: T10b STAGE_PASS verdict
PASS: T10c guest path inserted w/ sha
PASS: T10d input image untouched
PASS: T11 trap reaped stub child
PASS: T12 no orphans at end
PASS: T13 manifest vs production pin -> 1
PASS: T13b pin contradiction disclosed
PASS: T13c no claim on pin contradiction
PASS: T14 default run (armB absent) -> 0
PASS: T14b armB mismatch -> 1
PASS: T14c armB mismatch verdict
PASS: T15 input-mutation stage -> 1
PASS: T15b input after-hash MUTATED disclosed
PASS: T15c STAGE_FAIL verdict
PASS: T16 guest-path symlink escape -> 1
PASS: T16b INSERT_PATH_ESCAPE verdict
PASS: T16c host sentinel untouched
PASS: T16d no bytes copied through the escape

suite: 39 passed, 0 failed
```

Earlier runs: `tests-run1.log` (13/22) caught three real bugs, fixed then; `tests-run2.log`
(22/22, pre-review). The review pass added T5b–T5d, T9d, T13–T16 and fixed the post-stage
`prot` call that still passed the removed override flags (unbound-variable crash under `set -u`).

## NOT yet authorized (next gates)

- NO real staging: copying any image, running `redoxfs`/mount, inserting into any image — needs
  Codex budget + a fresh `wy-<UTCstamp>` phase-ID + `--approve`.
- NO boot/VM: `boot` is hard-disabled (`BOOT_NOT_IMPLEMENTED`) until a separately reviewed
  dedicated Wyoming QEMU harness exists; `--boot-cmd` is never executed by this wrapper.
- The aarch64 `wyoming-probe` binary EXISTS (unconsumed) in the sibling `redox-oneplus6-wy`
  worktree at the pinned sha — no rebuild needed; pass it via explicit `--binary` in a future
  dry-run under separate review. The manifest sha must equal the pinned evidence sha or the
  script fails closed.
- Physical phone: out of scope, untouched.

## Codex final review — 8:10 AM PT

Luna found cp -a preserves an image input symlink, which could mount/write its
target. Codex rejects symlink image inputs before phase claims and requires the
copy to be a regular non-symlink. Protected presence is remembered: disappearance
after a successful pre-check fails the stage verdict instead of becoming "absent".
Focused fake suite44/44 passes, including no-claim symlink rejection, protected
disappearance failure and all prior boot/escape/input-mutation guards. Root log:
$HOME/artifacts/handoff/redox-perplexity/stage-root-fixes-tests-20261025T1508.log.
No real image/copy/mount/VM operation was done by these tests. Future stage or
Wyoming guest run still requires a separately recorded budget and raw receipts.
Old68001fa7 woke from an8:00hourly report and banked8bc6ec4 before being paused.
Its4-line input failure receipt fix is preserved and included, not discarded.
