# E1-NET-2P — code-path study: where the TCP payload write dies (checkpoint)

Read-only source study, 2026-10-25 (~02:45–03:00 AM PT). No boots, no builds, no QEMU, no phone.
Branch `codex/e1-net-exchange` (rebased on main `4e64abc`). Author: 5090 lane, same session.

> **Epistemic status (Codex 3:35 AM PT review, applied by orch):** this is a STATIC source study.
> Nothing here is runtime proof. There is no syscall trace and no ion source, so no claim below
> proves that a `write()` syscall returned Ok, that a specific parked retry was answered, or that
> the drop actually occurred at hop 8 in a running system. "Closed" hop rows mean *the local source
> was read end-to-end*, not *runtime behavior verified*. The close-removes-buffer path is a strong
> SOURCE CANDIDATE (ranked #1), not a verified runtime diagnosis. Runtime confirmation requires the
> native probe and/or P1 trace evidence.

## Ownership / reconciliation (do not duplicate)
- Spark source report ALREADY DONE: `$HOME/artifacts/handoff/redox-perplexity/E1-TCP-WRITE-ANALYSIS.md`
  (8 files: tcp.rs, mod.rs, socket.rs, smoltcp-0.13.1 grep, recipe.toml, script, E1-NET-EXCHANGE.txt).
  This doc REUSES its F3/F4/I5/I6/I7 and does not re-derive them.
- Native probe (std client + owned ISO copy, max 2 NEW 90 s boots): owned by FRESH 5090
  session-9196dd7d-1757-43df-b36c-f25c0bb50dba, goal 5e0071d5, same qemu worktree.
  OLD writer b0738453 is STOPPED — do NOT resume it. This study spends ZERO boots.
- Children (read-only tracers, legitimate): ion hop `fdd7c3ce…`, kernel hop `68e4c4be…`.
  Their findings are folded in below when present; kernel/ion rows are marked pending.

## Hop map for `echo X > /scheme/tcp/10.0.2.2:47011`
1. **ion** — builtin echo + shell-level `>` redirect. Source NOT present locally
   (`redox/recipes/core/ion/` holds only recipe.toml pointing at gitlab; Spark F8 confirmed by
   independent find here; child `fdd7c3ce` searched exhaustively and found no ion source tree —
   interrupted as bounded, fetching is out of budget). No ion source ⇒ its conduct is only
   INFERRED from runtime constraints: the socket was alive ~1 s (N3), consistent with ion being
   blocked in the syscall (kernel blocks, hop 3), and ion proceeded to E1NET-DONE (consistent
   with the parked retry having answered — but there is NO syscall trace proving `write()`
   returned Ok, and no ion source proving how it checks errors). Under that inference no errno
   reached ion, so an ion-side swallow is UNLIKELY — not excluded. Ion source/trace evidence
   would be needed to close this hop.
2. **relibc (local: `forks/redox/relibc`)** — ion's `write()` lands at
   `src/header/unistd/mod.rs:1481-1486` → `Sys::write` →
   `src/platform/redox/mod.rs:1857-1860` → `redox_rt::sys::posix_write`
   (`redox-rt/src/sys.rs:57-59` = `wrapper(true, false, || syscall::write(...))`).
   The wrapper ONLY restarts on signals (`sys.rs:43-45`); there is **no EWOULDBLOCK retry in
   userspace** — any errno returned by the kernel reaches ion verbatim as `write() == -1`.
   So whether ion ever sees an error depends entirely on the kernel hop below.
3. **kernel (kernel-e1)** — source read end-to-end via read-only child `68e4c4be` (11-hop trace);
   runtime unobserved. Kernel is a thin
   proxy: relibc resolves `/scheme/tcp` to the SchemeId; kernel forwards path bytes VERBATIM
   (`src/syscall/fs.rs:93-100` `kopenat`; no name-stripping code exists despite the mod.rs doc
   comment). Each `write()` = ONE `Opcode::Write` SQE, caller pages mapped zero-copy
   (`user.rs:1808-1841`; `capture_inner` 474-725 copies only sub-page edges), and the caller
   context BLOCKS until the scheme answers (`call_inner` 225-390, `.block("UserInner::call")` 237).
   **Zero EWOULDBLOCK handling in kernel scheme/fs** (grep: no hits) — kernel never retries nor
   propagates it; it just waits for the scheme's delayed response. `close()` = fire-and-forget
   unacknowledged `Opcode::CloseMsg` SQE, `tag: 0` (`user.rs:1676-1697`) + EVENT_HUP.
   O_CREAT/O_TRUNC opaque pass-through (`fs.rs:104-110`, re-sent as call_flags `fs.rs:737`).
   E1 branch modified NOTHING on this path (git log: only `2780adc1` touches src/scheme/, irq.rs).
4. **netstack open** — `base-e1/netstack/src/scheme/tcp.rs:66-138`: `new_socket` parses endpoint,
   allocates 64 KiB rx/tx buffers (82-85), and calls `connect()` AT OPEN (121-127); failure panics
   ("Can't connect tcp socket" — absent in E1-NET-1 serial ⇒ connect succeeded).
5. **netstack write** — `tcp.rs:157-173` `write_buf`: EPIPE arm unreachable (Spark F4);
   `!is_active()` ⇒ ENOTCONN (164-165); `can_send()` ⇒ `send_slice` Ok(n) (166-167);
   blocking + !can_send ⇒ **EWOULDBLOCK** (168-172).
6. **netstack park** — `scheme/mod.rs:421-455`: a blocking EWOULDBLOCK op is pushed to `wait_queue`
   (no timeout when `write_timeout: None`, Spark I7). `notify_sockets` (471-576) RETRIES parked ops
   every cycle (493-530) and answers ECANCELED/ETIMEDOUT only on cancel/timeout.
7. **netstack poll ordering** — `scheme/mod.rs:232-268`: per cycle, `iface.poll(...)` (which is what
   actually emits queued tx octets) runs FIRST; `notify_sockets()` runs AFTER (262). ⇒ a parked write
   that succeeds on retry lands in the smoltcp tx buffer AFTER that cycle's last `iface.poll`; it
   waits for the NEXT cycle (10–500 ms later, 264-267) to hit the wire.
8. **netstack close** — `scheme/socket.rs:640-689` `on_close`: last-ref drop calls
   `socket_set.remove(socket_handle)` (687) — the smoltcp socket is DELETED immediately. No
   `Socket::close()`, no final `iface.poll`, no tx flush. Queued tx octets vanish; no FIN is sent.
9. **driver send (source-clean, NOT runtime-exonerated)** — smoltcp emits frames via `link/ethernet.rs:85-102`:
   `network_file.write_all(&output_buffer)` (97) into the rtl8139d scheme; kernel routes that write
   to `drivers/net/driver-network/src/lib.rs:213-220` (`write` → `adapter.write_packet`, synchronous,
   no queue) → `drivers/net/rtl8139d/src/device.rs:168-197` `write_packet`: SPINS until a free TX
   descriptor (`TSD_OWN` clear), copies the frame, sets `tsad`/`tsd` (hands ownership to the NIC),
   returns Ok. Only error arm is EMSGSIZE. **No silent-drop arm exists in the driver SOURCE** — but
   absence of a drop arm in source is not runtime exoneration (descriptor/NIC behavior under load is
   unobserved). IF candidate 1 is the cause, E1-NET-1's payload never reached this hop; that ordering
   is itself part of the unproven hypothesis.

## NEW findings beyond the Spark report
- **N1 (ordering):** `iface.poll` before `notify_sockets` (mod.rs:232-268/262) ⇒ retry-accepted
  payload always waits ≥1 poll cycle (10–500 ms) for actual transmission.
- **N2 (drop point):** `on_close` removes the socket with unflushed tx (socket.rs:687) ⇒ payload
  accepted into the tx buffer but closed before the next poll is SILENTLY DISCARDED, and no FIN/RST
  is emitted. This is a true silent-drop arm — Spark F3 ("no path silently swallows a write") holds
  for `write_buf` itself but NOT for the close path.
- **N3 (runtime constraint from E1-NET-1 pcap, weakest link):** the guest ACK at 09:22:08.017 shows
  the connection was still open ~1 s after handshake — consistent with ion's fd still being open.
  ion's echo does not take 1 s, so a plausible reading is that ion was BLOCKED in write() and the
  write syscall reached the scheme and parked (this WEAKENS Spark H1 "ion never wrote" — it does not
  disprove it; the late close is also consistent with ion waiting on something else, and there is no
  syscall trace). The chain "retried → send_slice Ok → ion unblocked → ion closed ⇒ N2 discarded the
  nonce" is a CONSISTENT INFERENCE, not an observed sequence.
- **N4 (no-FIN corroboration):** zero packets after the ACK — no data, no FIN — is CONSISTENT with
  N2 (remove-without-close); a normal smoltcp close would have FIN'd. Corroborating, not conclusive
  (other paths could also yield no FIN).
- **N5 (timing argument, NOT a determinism proof):** kernel close is fire-and-forget in source
  (`user.rs:1676-1697`). IF ion wakes promptly when the parked-write retry answers, its CloseMsg
  would likely be processed before the NEXT `iface.poll` (10–500 ms later, mod.rs:264-267), so
  `socket_set.remove` would discard the queued tx. The apparent window is wide, but OS scheduling
  is NOT guaranteed to be microsecond-fast or ordered; "effectively guaranteed" overstates the
  evidence. The identical signature across both E1-NET-1 attempts is consistent with a repeatable
  drop, but repeatability ≠ proven mechanism.

## Ranked candidate drop points
1. **Close-vs-flush race (N1+N2+N3):** payload accepted into tx buffer by the parked-write retry,
   discarded by `socket_set.remove` before the next `iface.poll`. Explains ALL observations
   (SYN/SYN-ACK/ACK, no data, no FIN, ion uneventful, peer empty). Static inference, runtime unobserved.
2. **ion write-error swallow (Spark H1/H2 variant):** WEAKENED (not falsified) at the kernel hop —
   the kernel source has no EWOULDBLOCK propagation (blocks until the scheme answers,
   `user.rs:225-390`). IF the parked write was answered Ok, ion never saw an errno. But ion source
   is absent and there is no syscall trace, so an ion-side swallow cannot be formally excluded.
   Remaining ion unknowns (error-checking style, exact open flags) stay open pending source/trace.
3. **Unbounded park (Spark H3, `write_timeout: None`):** contradicted by ion continuing — the park
   WAS answered (retry success), so not the drop itself.
4. ENOTCONN arm: contradicted — `is_active()` is true in IsConnecting (Spark I5).

## Probes (described only — NOT run; boots owned by session-9196dd7d, goal 5e0071d5)
- **P0 (zero boot):** finish ion-source hop — clone ion read-only (or read kernel-side errno
  propagation) to close F8; fold child results into this doc.
- **P1 (one boot, next gate):** Spark R2's two trace lines (socket.rs write entry, tcp.rs:161
  branch) PLUS a third at socket.rs:687 printing `tx_buffer.len()` at removal. Outcome table:
  write-arrived+Ok & remove-len>0 ⇒ confirms candidate 1; no-write-line ⇒ H1; error-line ⇒ H2.
- **P2 (zero boot, host-side):** none needed — existing pcaps already show no retransmits/no FIN
  (9 packets); re-scored `synack-no-data / peer-empty / clean`.

## Status
Source-level narrowing only — NOT a runtime diagnosis. The strongest source candidate is:
**a write that parks (mod.rs:421-455) and is retry-accepted into the smoltcp tx buffer after that
cycle's iface.poll (mod.rs:232-268) would be silently discarded if ion's close removes the socket
before the next poll (socket.rs:687 + kernel user.rs:1676-1697).** Every link in that chain rests on
static reading plus consistent-but-unproven runtime inference (no syscall trace, no ion source, no
trace lines executed). Kernel/relibc/driver hops: local source read end-to-end (driver source shows
no drop arm — source-clean, not runtime-exonerated). ion hop: source absent; conduct inferred.
No boots spent; no source edited. Runtime confirmation = native probe (session 9196dd7d lane) and/or
P1 trace lines. NO kernel/netstack patch may be justified from this study alone.
