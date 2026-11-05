# E1-NET-FROM-IMAGE — one boot of verified armB.iso (checkpoint receipt)

Phase: `net-from-image-20261025T1137` (Codex-fixed id, atomic mkdir claim, one boot total).
Approved: Codex 4:24/4:37 AM PT, 2026-10-25. Branch `boot-from-image` off main `2484af5`.

## Checkpoint (written BEFORE the boot)
- Wrapper: `scripts/qemu-demo/e1-net-native-from-image.sh` (new, focused, <=300 lines).
- Input (READ-ONLY, `readonly=on`): `out/e1-staging-check/20261025T113022Z/armB.iso`
  sha256 `c91ab5338659fa6eef6a553c612ecac998a0046473292272328351e5b2f11e7c` (orch-verified, re-hashed pre-run: matches).
- Preserved read-only: original ISO `c02012e…361bc`, old-bad `out/e1net-native/e1net-probe.iso` `87ae0aa8…942f`,
  ALL `out/e1-staging-check/**` ARM images — hash guards before AND after, byte-identity diff enforced.
- Native-probe DEFAULT boot path NOT run (it restages/overwrites oldbad). Only `lib` functions reused
  (`evaluate_run`, `verify_image`, `reap_group`, `start_peer`, `classify`); `out` overridden to
  `out/e1-net-from-image/net-from-image-20261025T1137/`. Drive path guard: must be staging-check armB, never e1net-native.
- Flow: mkdir claim → hash guards → fresh `verify_image` (redoxfs-clone tree walk to disposable scratch; abort if fail)
  → unique FIFO + nonce `e1fi-<epoch>-<rand>` + free peer port 4781..4810 → host peer started FIRST,
  `ss` LISTEN proof written BEFORE QEMU → ONE boot <=90 s (RUN_TIMEOUT validated 40..90) → serial-driven
  `/usr/bin/net-probe <nonce> 10.0.2.2:<port>` at t=25 s → own-PGID TERM/KILL reap → `evaluate_run` verdict →
  hash guards after. No retries, no second boot even if wrapper later needs a fix.
- Gate (unchanged, same `evaluate_run` as merged tests): FULL = native WRITE_OK on serial + exact nonce
  line at host peer (`grep -qFx`) + exact nonce in directional guest→host TCP payload via
  `e1-net-native-probe-pcap.py`. Handshake alone = PARTIAL, never success. REJECT_* preserved.
- Existing 25/27 host-probe tests already green — NOT rerun (per Codex).

## Result (appended after the run)
Ran once at 2026-10-25 ~11:41–11:44 UTC (4:41–4:44 AM PT). Wrapper rc=1 (NOT FULL — honest fail-closed).
Phase claimed once via `mkdir`; claim kept; NO second boot, NO retries.

| Item | Value |
|---|---|
| Phase id | `net-from-image-20261025T1137` (claimed once, atomic mkdir) |
| Nonce / port / window | `e1fi-1792928584-5721` / 4781 / 90 s |
| verify_image (fresh tree walk) | **PASS rc=0** before boot |
| Peer listening proof | `2026-10-25T11:43:08Z` `LISTEN 127.0.0.1:4781` — verified BEFORE QEMU |
| Boot rc | `ran-full-window-reaped` (QEMU alive to window end, TERM'd by own PGID reap; qemu.log: `terminating on signal 15`) |
| Signature | `crash` — serial: `kernel::arch::aarch64::interrupt::exception:ERROR -- FATAL: Not an SVC induced synchronous exception (ty=111100)` + `SP_EL0` frames |
| **Verdict (evaluate_run, same as merged tests)** | **`REJECT_APP_CRASH`** — NOT success |
| Serial tail observation | serial reached `redox login:` and ended there — typed net-probe line met the login prompt, not a shell (observation only, no cause claimed) |
| Host peer recv | file absent — guest never connected |
| pcap payload extract | empty (sha256 `e3b0c442…b855` = empty string) |

Artifact hashes (raw, preserved in `out/e1-net-from-image/net-from-image-20261025T1137/`):
- serial.log `6f0035018f8fb24525e641303a83583f8682b9b2afa2d84a1a641be4408d0133`
- pcap.pcap `0b85423e6d4b562e1e009aab3b25131e598059c27a10edb32ee09414e032849c`
- pcap-payload.txt `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` (empty)
- peer.log `62a481edd89ee8373d2a2d358c262962809640336391ab895a8b5e470cd38074`
- host-recv.txt: none (no connection)

Hash guards BEFORE and AFTER (`hashes-before.tsv` = `hashes-after.tsv`, diff clean):
armB.iso `c91ab533…1e7c`, original ISO `c02012e…361bc`, old-bad `87ae0aa8…942f`, and ALL
`out/e1-staging-check/**` ARM images byte-identical. No mounts, no copies, no restaging.

Exit note: `phase net-from-image-20261025T1137 exited rc=1 at 2026-10-25T11:44:32Z (claim kept; ask Codex before any re-claim)`.

**Bottom line for Codex:** the boot budget is SPENT. armB.iso boots to login but the guest never
ran net-probe against the peer (login prompt met the typed input; a kernel EL0 exception ERROR
appears in the serial and drives the REJECT_APP_CRASH classification). Payload gate: 0/3 proofs.
Any next step needs a NEW Codex-approved phase id — this claim is closed.

## Codex review correction (4:44 AM PT) — classification amended, boot still spent
Observed from the actual serial: the wrapper's blind `sleep 25` sent the probe command into the
**bootloader ENV EDITOR**, not a shell. Root sent the documented Enter twice at 4:44:11 within the
SAME VM; the guest then booted **cleanly to `redox login:` — no hash panic**. The root login
attempt arrived only after the VM closed (no FIFO reader), so **no native app ever executed**.

- Correct classification: **`BOOTED_LOGIN_ONLY` / `HARNESS_HANDSHAKE_MISSING`** — a harness
  defect, NOT a transport failure and NOT an app/image failure. The raw `REJECT_APP_CRASH`
  signature above is preserved byte-for-byte; this section amends its meaning, not the evidence.
- ONE boot spent; no rerun. No source kernel patch. No new boot until Codex reviews the fix
  and grants a new budget/phase id.
- Wrapper status: **PARTIAL** — image-verify/guards/peer-proof/evaluate plumbing good; the
  fixed-sleep serial injection is defective.

### Required fix for the next phase (design only, NOT run here)
Replace blind fixed sleeps with a durable prompt-driven serial state machine, following the
login precedent already in `scripts/qemu-demo/e1-net-exchange.sh` (Enter nudge loop polling
serial for `redox login:`, then `INTX_PROBE_CMDS` default credentials `root|password`, then
shell prompt `ion:/$ ` before any probe line):
1. Early Enter ×2 (bootloader menu Enter — the manual 4:44:11 fix, automated).
2. Poll serial until `redox login:` (bounded timeout; abort as BOOTED_LOGIN_ONLY if never seen).
3. Send documented guest credentials `root` / `password` (same as e1-net-exchange.sh).
4. Poll until shell prompt (`ion:/$ `), THEN send `/usr/bin/net-probe <nonce> 10.0.2.2:<port>`.
Root will review the existing `e1-net-exchange.sh` login precedent when approving the next phase.

## Root follow-up (4:50 AM PT) — raw misclassification preserved, interpretation separate
The shared crash classifier counted **pre-probe boot-service FATALs** before app start; the
actual serial has **no `net-probe:` start line**, so the reviewed classification is
**BOOTED_LOGIN_ONLY / HARNESS_HANDSHAKE_MISSING**, not app crash. The raw `receipt.tsv` /
`verdict.txt` (`REJECT_APP_CRASH`) are kept byte-identical as the raw misclassification;
the reviewed interpretation lives separately in
`out/e1-net-from-image/net-from-image-20261025T1137/INTERPRETATION.md` — no history rewrite.
Shared `e1-net-native-probe.sh` `classify` + shared tests on MAIN are Root-owned and being
regression-tested by Root; this worker never edited them (lib-source only). Wrapper/doc
banked **PARTIAL**; no further boot without a new Codex phase id + budget.



## Phase CONTINUE-SERIAL-FIX-20261025T1149 (WIP checkpoint, host-only)
Prompt-driven serial handshake wrapper + fake-QEMU/FIFO host tests in progress. No boot, no image writes, no shared-file edits (Root owns classify/tests on main f129592). 10-min bound.

### CONTINUE-SERIAL-FIX-20261025T1149 — banked (host-only, no boot)
New: `scripts/qemu-demo/e1-net-serial-handshake.sh` (prompt-driven state machine: menu marker -> Enter x2 -> login -> root -> password prompt -> password -> shell prompt `ion:/$ ` -> ONLY THEN net-probe; every wait bounded, offset-scanned so stale markers cannot satisfy later waits; markers/creds from the e1-net-exchange.sh precedent). New: `scripts/qemu-demo/e1-net-serial-handshake-tests.sh` (FAKE QEMU/FIFO only). Actual run: `TESTS_RC=0`, **pass=6 fail=0** — T1 delayed login OK, T2 stale marker -> LOGIN_PROMPT_TIMEOUT, T3 -> PASSWORD_PROMPT_TIMEOUT, T4 -> SHELL_PROMPT_TIMEOUT, T5 no probe before authenticated shell, T6 claim one-time / re-claim refused. Abort classes: BOOTLOADER_WAIT_TIMEOUT / LOGIN_PROMPT_TIMEOUT / PASSWORD_PROMPT_TIMEOUT / SHELL_PROMPT_TIMEOUT, all inside the 90 s deadline design. Shared classify/tests on main untouched (diff 0). No boot performed.

## Codex real-prompt correction and reviewed wiring — 2026-10-25

Luna found the original six fixtures modeled the wrong prompt: ion:/$ appears
only in an older synthetic fixture. Actual E1-NET-1 serial at integration/
e1net-attempt-1-raw.txt:603 shows root:~# with CSI colors and an OSC title.
Codex changed the helper and fake VM to those captured bytes, normalized CSI/OSC
in bounded snapshot reads, and removed the tail|grep SIGPIPE race. Host suite:
6 passed,0 failed (serial-real-prompt-tests-20261025T1221.log). These are host-only.

Existing wrapper now requires an explicit boot argument and calls the handshake
immediately after QEMU starts, with8 seconds reserved for the probe response.
No fixed25-second command injection. Existing disk/hash/peer/evaluator guards
remain. Luna reviewed the final wiring without finding a blocker.

Codex approves exactly ONE new readonly90-second run for phase
net-from-image-20261025T1223. The old1137 claim and raw evidence remain intact.
No copies, mounts, builds, downloads, phone operations or model changes. A guest
payload conclusion still requires native WRITE_OK, exact host nonce and matching
directional TCP payload. This is a new bounded experiment, not renewal of1137.

## Codex independent acceptance — 2026-10-25 5:27 AM PT

Phase net-from-image-20261025T1223 executed once via visible5090 session
c44138d3, goal05360565. Actual wrapper rc0; HANDSHAKE_OK; signature read_ack;
FULL. The worker's prose time was wrong: packet timestamps establish5:25AM PT.

Raw serial independently read by Codex:
- CONNECT_OK, WRITE_OK22 bytes, FLUSH_OK.
- READ_ACK match25 bytes: ACK-e1fi-1792931093-20893; CLOSED; done.
Host receive file contains exactly e1fi-1792931093-20893 plus newline.
Independent tcpdump decoding shows guest10.0.2.15:49152 -> host10.0.2.2:4781
TCP payload22 bytes at12:25:12.469720UTC and matching25-byte host ACK payload.
This is real native guest-originated bidirectional TCP, not console echo or only
SYN/SYNACK. Both WRITE_OK and READ_ACK exist; acceptance does not rely solely on
the generic read_ack shortcut.

Receipt: out/e1-net-from-image/net-from-image-20261025T1223/receipt.tsv.
Serial sha2569ec7601c066eef1aaed6de36bdd7ffcf156556ba7ec4b1f64470fdd07a7eded8.
PCAP sha256809665741149b2813e09157979fe550d5be650349e5014a1d4935ecb85e9ef38.
Peer and extracted payload sha256 both
9fae9d92685be946e83d744153fa1386ca4298de5ce3d1c4420582da356c571f.
Protected image hash manifests before/after are byte-identical. QEMU and peer
are terminal, phase claim remains consumed. No new image writes or phone action.

Boundary: one static-IP QEMU workload only. No DHCP/internet/phone proof, no
blanket driver exoneration, no proof of the earlier ion close-before-flush theory.
Known unrelated early userspace faults remain separate. Keep original corruption
cause unknown even though clean staging now passes and the staged binary ran.
