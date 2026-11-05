# Stage 3 design note — bridging CDC-ACM to the shell

Status: design note for root's review (stage 3's first
deliverable per the staged plan). The pure bridge core is
implemented and host-tested in
`crates/usbgadgetd/src/acm_shell.rs`; the Redox glue is
`crates/usbgadgetd/src/redox_stage3.rs`. This note records
the one real decision and the policies around it.

## The decision: which byte stream is "the shell"?

The bridge needs a byte stream on the OS side. Options:

1. **The console scheme node the RAM-boot image's shell is
   already attached to** — `/scheme/debug`. The image boots
   to a shell on the debug console; ACM bytes written there
   are the shell's input and its output is what reads
   return. No new OS component, works with the image as
   built. Downsides: the debug console is also the kernel
   log's home (log noise can interleave with the session),
   and console semantics (echo, line handling) are whatever
   the console driver does.
2. **A dedicated pty-style scheme.** Cleaner session
   semantics, isolates the shell from kernel logs. Redox's
   base system has pty work, but the RAM image for this port
   does not currently expose one to the daemon, so this is
   image work, not daemon work.
3. **Spawn a private shell process and wire its stdio.**
   The daemon would need process/spawn plumbing it does not
   have, and the "shell the RAM-boot image exposes" (the
   plan's wording) is the console shell, not a private one.

**Recommendation: option 1 for stage 3's first silicon
run**, behind the single constant `SHELL_STREAM_PATH` in
`redox_stage3.rs`. The bridge core (`acm_shell.rs`) is
transport-agnostic — it speaks the `ByteStream` trait — so
moving to option 2 later changes the glue's stream type and
the constant, and none of the tested logic. Decide after
the first attended run shows how noisy the console stream
actually is in a session.

## Policies the core implements (all host-tested)

- **Disconnect:** pending host→shell bytes are dropped at
  disconnect (stale input must never execute in the next
  session); shell→host bytes already accepted are retained
  and delivered first on reconnect; the shell stream is
  never closed by the bridge; while no host is attached the
  bridge does not read the shell, so fresh output (e.g. a
  new prompt) waits upstream instead of filling a queue
  nobody drains.
- **Backpressure:** fixed queues, honest would-block. A
  full queue stops reads from its source; nothing is
  dropped silently while connected.
- **Line coding:** parsed and validated (CDC 7-byte
  struct); a refused SET leaves the running settings
  untouched and GET answers what is in force. For a virtual
  port the settings change nothing electrical — they are
  tracked because a host terminal sets them on open and
  tooling reads them back.
- **Identity:** the ACM device carries the bench device's
  VID/PID/strings (0x1209/0x0d06, "redox-oneplus6" /
  "OnePlus 6 gadget" / "K11"), so host-side behaviour
  matches what the model bench already proved, and the
  Mac sees the same device it saw in emulation runs.

## Threading shape (glue)

Two threads, `std::sync::mpsc` between them: the embassy
thread (`usb.run()` joined with the ACM packet task) and a
bridge thread pumping at ~1 ms. Blocking scheme reads live
on a dedicated reader thread inside the shell stream
adapter, so the bridge's streams never block. If the shell
stream cannot be opened, the daemon logs STAGE3 ABORT and
keeps USB enumeration alive — a missing shell must not look
like a dead device.

## What silicon still has to answer

Sustained bulk throughput without drops under polling; the
interface appearing as a usable `/dev/cu.usbmodem*` on the
Mac; session survival across re-enumeration; interactive
latency; and the option-1 noise question above.
