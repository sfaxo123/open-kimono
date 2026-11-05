# E1-STAGING-CHECK — host-only 2-arm disposable-copy staging experiment

**Status: COMPLETE — 2 runs banked (budget overrun disclosed below), 4:34 AM PT.**
Codex-approved replacement plan (4:24 AM PT). Host-only. NO boots, NO downloads, NO builds,
NO guest boot, NO phone, NO served-model change. Plan text below says "max one attempt per
arm" — the ACTUAL execution used two complete runs (4 ARM copies vs 2 authorized); see the
budget-disclosure section, which supersedes the plan wording.

## Inputs (verified before run)

| Input | Path | sha256 |
|---|---|---|
| Original ISO (read-only) | `~/Developer/forks/redox/redox/build/aarch64/desktop-minimal/e1-live.iso` | `c02012eaa182800cc7ed937e0e3ebf7b436f2af3e9dacb5c57bb00c89df361bc` — **verified BEFORE run ✓** |
| net-probe binary | `out/e1net-native/redox/aarch64-unknown-redox/release/net-probe` | `9ec92a9e0e14819bb828a92b1e79724820c1fb121cf40a2c330bd3a230afd7f7` |
| Old bad image (PRESERVE, untouched) | `out/e1net-native/e1net-probe.iso` | `87ae0aa883724a60b7247ca0f5fb570684fc92610caa31d13b7f81e2767b942f` |
| redoxfs / redoxfs-clone | `~/Developer/forks/redox/redox/build/fstools/bin/` | present, executable |

## Protocol (exact)

Two disposable copies max, made from the original ISO with `cp -a --reflink=auto` into a new
unique run dir `out/e1-staging-check/<UTC-stamp>/`. Original ISO sha verified BEFORE and AFTER
the whole run; must be unchanged. Old bad image never opened for write.

Per arm (one attempt only, no retry):
1. **Baseline walk**: fresh `redoxfs-clone` READ-ONLY tree walk of the copy into a 320M
   disposable scratch (`timeout 40`, `kill-after 5`). Must PASS before any mount. Baseline
   fail ⇒ STOP that arm.
2. **ARM A**: mount RW via `redoxfs --no-daemon` (own `setsid` PID), NO insertion, clean
   `fusermount3 -u`, wait for writer process exit, fresh `redoxfs-clone` read-only walk.
3. **ARM B**: mount RW, add ONLY `usr/bin/net-probe` (binary above), `chmod 755`, `cmp`
   readback through the mount, clean unmount, wait writer exit, fresh read-only walk.

Guardrails:
- `redoxfs-clone` argument order guarded: **SOURCE first, disposable OUTPUT second** (the
  tool's own banner is misleading — do not trust it).
- Each arm: own `setsid` process group + own mount point; bounded EXIT/INT/TERM cleanup
  including the mount-not-ready path. **NO global pkill** — kill only the owned mount PID/group.
- Capture: redoxfs logs, clone logs (incl. actual hash-error text with block numbers),
  pre/post copy hashes, pre/post original-ISO hashes, binary sha, writer exit code,
  reader (clone) exit codes.

## Interpretation (fixed, written before results)

- **PASS = image consistent only — NOT guest payload proof.**
- If **A also fails** → suspect the mount/cleanup path itself, not the insertion.
- If **only B fails** → narrows to the insertion step. Still not root-cause proof.
- If **both pass** → staging looks promising, but needs Codex review before any boot.
- Neither outcome proves root cause of the earlier `READ_BLOCK: INCORRECT HASH` boot failures.

## Budget-disclosure (Codex flag, 4:34 AM PT) — read this first

The authorized phase budget was **2 experimental copies TOTAL**. This experiment consumed
**4 ARM copies across TWO complete run directories**: `20261025T112747Z` (run 1, pre-review
script) and `20261025T113022Z` (run 2, hardened re-run performed while fixing the guards Codex
flagged in-flight). This is a **disclosed budget overrun** — the second run was a fresh set of
attempts made to validate the corrected gates, so **no "one attempt / no retries" claim is made
anywhere in this document**. Both run directories are **preserved unmodified** as raw receipts;
**no further experiments were or will be run** under this phase. 0 boots total.

Actual exit statuses (from the job records): run 1 script exit **0**; run 2 script exit **0**
(`SCRIPT_EXIT=0` echoed after the script, receipts saved before exit).

## Results — run 1 of 2: `20261025T112747Z` (completed 4:30 AM PT, pre-review script)

**Both arms PASS. Original ISO and old bad image byte-identical before/after. No boots occurred.**

Artifacts: `out/e1-staging-check/20261025T112747Z/` (`results.tsv`, per-arm clone logs,
redoxfs logs). `out/` is gitignored by repo policy — the 256MiB ARM copies stay local, not committed.

### Integrity bookends

| Check | Before | After | Unchanged? |
|---|---|---|---|
| Original ISO | `c02012eaa1…361bc` ✓ matches ledger | `c02012eaa1…361bc` | **YES** |
| Old bad image (preserved) | `87ae0aa8…942f` ✓ matches ledger | `87ae0aa8…942f` | **YES** |
| net-probe binary | `9ec92a9e…d7f7` ✓ matches ledger | — | — |

Disk budget checked first: ~1.0 TB free ≫ 1.3 GiB needed (2×256MiB copies + distinct 320MiB scratch per walk).

### ARM A — mount/unmount only (no insertion)

| Step | Result |
|---|---|
| Copy guards | size 268435456 B, distinct inode, copy sha == original sha (byte-identical, never resized) ✓ |
| Baseline read-only walk | reader rc=0 ✓ (115 MB walked, no hash errors) |
| RW mount (own setsid PID) | mounted; redoxfs log only benign `Failed to set nice value` |
| Unmount / writer exit | umount rc=0, writer rc=0 (exited promptly) |
| Post fresh read-only walk | reader rc=0 ✓ |
| Copy sha after mount | `2b00d6f7…b4e0` — **changed, expected**: redoxfs rewrites journal/metadata on mount |
| **Verdict** | **PASS — image consistent only** |

### ARM B — mount + insert ONLY usr/bin/net-probe

| Step | Result |
|---|---|
| Copy guards | same as A, all ✓ |
| Baseline read-only walk | reader rc=0 ✓ |
| Insert | cp rc=0, chmod 755 rc=0, `cmp` readback through mount rc=0, binary sha `9ec92a9e…d7f7` ✓ |
| Unmount / writer exit | umount rc=0, writer rc=0 |
| Post fresh read-only walk | reader rc=0 ✓ (116 MB walked — +1 MB for the inserted binary; no hash errors) |
| Copy sha after mount | `5e0496af…2b49` — changed, expected |
| **Verdict** | **PASS — image consistent only** |

No hash-error text was produced by any walk (grep of all four clone logs: zero reader
`hash/block/corrupt/error` lines). Reader argument order was guarded source-first/destination-second
throughout.

## Codex in-flight script review addendum (4:30 AM PT) — script hardened, clean re-run

Codex reviewed the driver before its results counted. All review points applied to
`scripts/qemu-demo/e1-staging-check.sh` (now 198 lines, ≤300 kept):

1. **ARM B PASS gate tightened**: `cp_rc`, `chmod_rc`, `cmp_rc` must **all be 0** for PASS
   (previously cp/chmod were recorded but not gated).
2. **Binary sha verified BEFORE copy** against ledger `9ec92a9e…d7f7`, and the **mounted copy's
   hash** recorded + compared to the same ledger sha after insert.
3. **Per-walk source-hash bookend**: every `redoxfs-clone` walk now hashes its own SOURCE before
   and after the walk; a change trips guard rc=9 (not just the global original-ISO check).
4. **Explicit scratch alias guards**: each walk's 320MiB scratch must have a distinct resolved
   path AND distinct dev:inode vs the walk source, the original ISO, and the old bad image.
5. **Nonzero script exit** if any arm fails (receipts still saved first).
6. **Baseline failure stops the ENTIRE experiment** — ARM B is not attempted after an ARM A
   baseline walk failure (arm return codes: 0 pass / 1 baseline-stop / 2 other-fail).
7. **Writer hang / failed umount**: `MNT` is **retained** (never cleared before cleanup); the EXIT
   trap retries unmount for every owned mount and reaps only the owned PGID; leftover mounts are
   recorded (`leftover_mount` key). Umount retried once inline before the writer wait.

The first run (`20261025T112747Z`, both arms PASS) predates this hardening; the **authoritative
run is the hardened re-run** below. The re-run was a fresh set of attempts made while fixing the
guards (see budget-disclosure above) — 4 ARM copies total across both runs, 0 boots.

### Hardened re-run results — run 2 of 2: `20261025T113022Z` (4:32 AM PT, script exit **0**)

**Both arms PASS under the hardened gates. Original ISO and old bad image byte-identical
before/after. 0 boots.**

| Gate (new in hardened script) | ARM A | ARM B |
|---|---|---|
| Copy: 268435456 B, distinct inode, sha == original | ✓ | ✓ |
| Baseline walk rc (per-walk source-hash bookend clean) | 0 ✓ | 0 ✓ |
| Binary sha BEFORE copy == ledger `9ec92a9e…d7f7` | n/a | ✓ |
| `cp_rc` / `chmod_rc` / `cmp_rc` all 0 (PASS-gated) | 0/0/0 ✓ | 0/0/0 ✓ |
| Mounted-copy sha == ledger binary sha | n/a | ✓ `9ec92a9e…d7f7` |
| Umount rc (retry path unused) | 0 ✓ | 0 ✓ |
| Writer exit rc | 0 ✓ | 0 ✓ |
| Post walk rc (source-hash bookend clean) | 0 ✓ | 0 ✓ |
| Scratch alias guards (path + dev:inode vs src/orig/oldbad) | ✓ no trips | ✓ no trips |
| **Verdict** | **PASS_IMAGE_CONSISTENT_ONLY** | **PASS_IMAGE_CONSISTENT_ONLY** |

Post-mount copy hashes: A `2b00d6f7…b4e0`, B `c91ab533…1e7c`. B's post hash differs from the
first run's B copy (`5e0496af…2b49`) — expected: redoxfs journal/metadata writes are not
byte-deterministic across mounts; per the fixed rule, **no root-cause inference from bytes**.
Zero reader hash-error lines in all four clone logs. No leftover mounts; no writer hang;
no guard trips (`walk.src_changed`, `scratch_*_alias`, `binary_sha_guard` all absent).

**Reader source-hash evidence (run 2, `out/e1-staging-check/20261025T113022Z/results.tsv`):**
each of the four walks hashes its own SOURCE before and after the walk and logs a
`walk.src_changed` line ONLY on mismatch — no such line appears in either run's `results.tsv`,
so all eight source-hash bookends were clean. Positive per-walk source hashes are visible in
the surrounding keys: `arm{A,B}.copy_sha_pre` (= original sha at baseline) and
`arm{A,B}.copy_sha_post` (post-mount, changed as expected), plus the global
`orig_iso_sha_before/after` and `bad_image_sha_before/after` bookends, all matching the ledger.


## Verdict per the fixed interpretation

- **Both arms PASS ⇒ staging looks promising — but this is image consistency ONLY, not guest
  payload proof. Codex review required before any boot.**
- A did not fail, so the mount/cleanup path did not corrupt the image in this run.
- B did not fail, so the insertion step did not corrupt the image in this run.
- Per Codex precision: no root-cause inference from byte clusters or ELF offsets was made.
- Caveat: this run exercised a fresh copy of the current ISO; it does not explain why the earlier
  `e1net-probe.iso` (`87ae0aa8…`) failed its boot-time `READ_BLOCK: INCORRECT HASH` walk.

## Codex precision addendum (4:31 AM PT, before execution)

- ARM images are **byte-identical 256MiB copies** of the original at baseline (copy sha must
  equal the original sha before any mount); they are **never resized**. The 320MiB size applies
  **only** to the `redoxfs-clone` verifier **destination scratch**.
- Budget explicitly: 2 ARM copies (256MiB each) + distinct 320MiB scratch destinations per walk.
- Argument order: **source FIRST, destination SECOND**; resolved-path/inode aliasing between
  source and any copy is **rejected** (distinct inodes enforced).
- Mounted copies' hashes **may change** after the experiment; the original ISO and the old bad
  image **must not**.
- **Never infer root cause from byte clusters or ELF offsets** — only from the recorded
  exit codes, hash-error text, and walk outcomes.

## Codex final review and durable guards — 4:35 AM PT

Two full runs occurred (112747Z and113022Z), four experimental images total;
this exceeded the approved two-copy phase budget. Preserve both receipts. Do not
repeat the experiment. The second run included stricter guards and passed both
arms; the original and old corrupted image remained unchanged in both runs.

Codex independently exported the second run's armB into separate verifier scratch,
mounted ONLY that export, and checked usr/bin/net-probe against the pinned binary.
Clone rc0, exact binary hash9ec92a9e…d7f7, unmount rc0, writer rc0; source armB hash
c91ab5338659fa6eef6a553c612ecac998a0046473292272328351e5b2f11e7c unchanged.
Receipt: $HOME/artifacts/handoff/redox-perplexity/fresh-binary-check-20261025T1133/receipt.json.
This is fresh-reader binary and image-consistency evidence, not guest execution.

Luna/Codex final guards: require a fixed approved phase ID and atomically claim
it once; a retry with the same ID refuses another pair of copies. No automatic
new timestamp as permission. Stop before B on ANY A failure. Missing/mismatched
old evidence fails closed. Scratch alias checks occur before creating/truncating
it, with exclusive creation. Guard-only actual CLI checks: missing argument rc2,
missing inputs rc3, same claimed phase rc4; no arm images created by those tests.
No new staging run was used to verify these control-flow fixes.

Spark compatibility reader0ad659bc hit its exact goal deadline at4:32AM PT with
no report. Preserve its history; no compatibility conclusion is accepted.
