# USBGADGETD — wiring design (Phase A, daemon layer)

Root-authored design, 2026-10-29 (root watch). Status: DESIGN, no code yet.
Scope: the userspace daemon that turns the already-proven `dwc3-gadget`
core (K11, on main, 36 host tests PASS) into the agent control channel:
a USB serial gadget on the OnePlus 6 with the Redox RAM boot shell
attached, so an agent on the 3090 host can drive the on-device shell
over the cable. Register map, sequences and license path live in
`specs/dwc3-gadget.md` and are NOT repeated here; this doc only wires
the core to real Redox daemon APIs, every one of which was read in the
sources cited below on 2026-10-29.

## 1. Verified API surface (ground truth, not assumptions)

| Need | API | Source read |
|---|---|---|
| Open memory scheme root | `common::init()` opens `/scheme/memory/scheme-root`; then `memory_root_fd()` | `forks/redox/base/drivers/common/src/lib.rs` |
| Map MMIO windows | `common::physmap(base, len, Prot::RW, MemoryType)` / safe `PhysBorrowed::map(...)` (Drop unmaps). MMIO precedent: `MemoryType::DeviceMemory` | `drivers/common/src/lib.rs`; precedent `drivers/storage/bcm2835-sdhcid/src/*.rs` uses `common::init()` + `physmap(..., DeviceMemory)` |
| DMA allocation | `common::dma::Dma::zeroed()` → physically contiguous, zeroed; on aarch64 the DMA memory type is `Uncacheable` (`zeroed@uc?phys_contiguous`); physical address via `Dma::physical()` after virt→phys translation (corrected 2026-10-29 V1b: current forks dma.rs has `physical()`, not `phys()`) | `drivers/common/src/dma.rs` |
| Core's hardware trait | `dwc3-gadget` is `no_std`, `forbid(unsafe_code)`, and takes two `mmio_replay::Mmio` windows: `R` = registers by ABSOLUTE address (core + QSCRATCH in one trait object), `D` = DMA by BUS address. `Config::sdm845_usb1(dma_base, delay_us)` fixes core `0x0a600000`, QSCRATCH `0x0a6f8800`, `select_utmi_as_pipe_clk: true`, `poll_limit: 10_000` | `crates/dwc3-gadget/src/{lib,controller}.rs`, `crates/mmio-replay/src/mmio.rs` |
| DMA layout | `layout::DMA_SIZE` = 22 KiB, must be 4 KiB-aligned; event ring 4 KiB, 32 TRBs, EP0 1 KiB, 32×512 B endpoint buffers | `crates/dwc3-gadget/src/layout.rs` |
| USB stack config (host-proven) | `embassy_usb::Builder` with VID `0x1209` / PID `0x0d06` (pid.codes TEST ids, internal use only), composite-with-IAD, class `0xef/0x02/0x01`, MPS0 64, buffers 256/64/0/128, CDC-NCM (`State`, MAC `02:6f:70:36:00:01`, 512) + CDC-ACM (`State`, 512) | `crates/dwc3-gadget/tests/common/device.rs` (the exact device the 36 tests enumerate) |
| Daemon lifecycle | Redox daemons: `daemon::Daemon::new(daemon)`, `daemon.ready()` after setup; scheme daemons use `redox_scheme::Socket` + `register_sync_scheme` | `drivers/usb/xhcid/src/main.rs`, `ptyd/src/main.rs` (in `forks/redox/base`) |
| Init service units | TOML units in `init.initfs.d/`, started by `init`; targets pull services via `requires_weak`. RAM-boot variant: `90_ramboot.target` requires only `30_ramsh.service` | `forks/redox/base` branch `op6/ramboot`: `init.initfs.d/{30_ramsh,90_ramboot}*` |
| The shell | `ramsh` (base `op6/ramboot` branch) UNCONDITIONALLY dup2s `/scheme/debug/no-preserve` onto fds 0/1/2, then runs a line shell (`help echo pwd ls cat exit`) on stdio. It makes NO termios/pty calls | `forks/redox/base` branch `op6/ramboot`: `ramsh/src/main.rs` |
| Initfs staging precedent | Copy binary into a worktree-local ramboot initfs tree, add a `lib/init.d/*.service` unit, add it to `90_ramboot.target`, repack with `redox-initfs-ar`, rebuild the ramboot EFI | `scripts/qemu-demo/stage.sh` (sandbox-demo) |

## 2. Daemon structure (crate `crates/usbgadgetd`, std, Redox target)

```
main()
  daemon::Daemon::new(daemon_main)

daemon_main(daemon):
  common::init()
  regs = RegsMmio::map()?          // §3, DeviceMemory, two windows
  dma  = DmaMmio::alloc()?         // §3, Dma<[u8; DMA_SIZE]>, bus = phys
  core = RefCell::new(Dwc3::new(regs, dma, Config::sdm845_usb1(bus, delay_us)))
  core.init()?                     // any Err -> log + exit, see §7
  register usbcon scheme (§5)      // BEFORE embassy run, so ramsh can open it
  daemon.ready()
  block_on(join(usb.run(), acm_bridge(), ncm_sink()))
```

- Executor: NO separate executor crate in v1. The daemon is
  single-threaded and runs `embassy_futures::block_on` over a `join`
  of the USB runner and the bridge futures — the same cooperative
  polling the host test bench uses, and the core's driver futures
  already re-wake themselves (`wake_by_ref`) in polled mode. Polling
  only; SPI 133 IRQ path stays out of scope (spec §5).
- `critical-section`: the host tests link the `std` implementation
  (dev-dependency). v1 tries that exact impl first; a custom
  single-thread impl is written ONLY if the std impl fails to link or
  run on `aarch64-unknown-redox`, and its soundness argument
  (single thread, no signal handlers, no second core) must be written
  in the code at that point. Gate G-W1 below.
- `delay_us: fn(u32)`: a plain `fn` pointer, so no captured state —
  implement as a busy-wait on `std::time::Instant` (the glue sequence
  needs ~100 µs steps; `thread::sleep` granularity on Redox is not
  trusted for that).

## 3. The two Mmio impls (the only unsafe in the project)

Both live in the daemon crate (the core stays `forbid(unsafe_code)`).

- `RegsMmio { core: PhysBorrowed, qscratch: PhysBorrowed }`
  maps `0x0a600000 / 0xcd00` and `0x0a6f8800 / 0x400` with
  `Prot::RW, MemoryType::DeviceMemory`. `read/write` dispatch on the
  absolute address: inside core range → core ptr + (addr − base);
  inside QSCRATCH range → qscratch ptr + (addr − base); anything else
  is a programming error — return 0 on read, drop the write, and set a
  sticky fault flag the daemon loop checks (never panic across the
  trait boundary; the core bounds every poll, a panic would not).
  Accesses are volatile, at the trait's width, in program order —
  exactly the `Mmio` contract in `mmio-replay`.
- `DmaMmio { dma: Dma<[u8; DMA_SIZE]>, bus: u64 }`
  `Dma::zeroed()` rounds the allocation up to 24 KiB (6 pages);
  `bus = dma.physical() as u64` is page-aligned (subject to the SMMU gate in section 4 / V3: translated, so this identity holds only in bypass), satisfying the layout's
  4 KiB rule. `read/write` translate `addr − bus`, bounds-check
  against `DMA_SIZE`, and copy through the volatile pointer. The DMA
  region is uncached on aarch64 by construction (`dma.rs`).

## 4. SMMU gate (unchanged, hard)

`bus = phys` is true ONLY if `apps_smmu` SID `0x740` is in bypass.
That is still UNPROVEN (spec §§5/7). The daemon therefore does not
get enabled in any payload until the pmOS oracle capture (spec §7:
`devmem` register dumps + `/sys/kernel/debug` SMMU dump, read-only
SSH) has settled it. This design does not weaken that gate; the
daemon logs its `dma_base` at startup so the oracle and the device
run can be compared value-for-value.

## 5. Shell attachment — decision

Facts that decide this: (a) `ramsh` hardcodes `/scheme/debug` — ANY
attachment needs a small `ramsh` change; (b) `ramsh` uses only stdio
line I/O, no termios — a plain byte stream is enough; (c) `ptyd` is a
rootfs daemon (`BASE_BINS`, `init.d/00_ptyd.service`) and is NOT in
the ramboot initfs (`INITFS_BINS` has no `ptyd`, `init.initfs.d/` has
no pty unit) — the pty route would need staging `ptyd` too.

DECISION (root, v1): **scheme route.**
- `usbgadgetd` registers a byte-stream scheme `usbcon` via
  `redox_scheme` (precedents: `ptyd`, `fbcond` scheme services),
  backed by two queues joined to the CDC-ACM endpoints.
- `ramsh` patch (in `forks/redox/base`, branch `op6/ramboot` — the
  only base change in v1): take the console path from `argv[1]`,
  defaulting to `/scheme/debug/no-preserve`. Default behaviour is
  bit-for-bit the current behaviour.
- A second ramsh instance is started for the gadget:
  `35_ramsh_usb.service` (cmd `ramsh`, args `["/scheme/usbcon"]`),
  required by `90_ramboot.target` only — never by the normal initfs
  target. The debug-console ramsh stays up regardless, so a dead
  daemon never costs the operator the panel shell.
- Fallback if the scheme route fights back: pty route (stage `ptyd`,
  bridge ACM ↔ `/dev/ptmx`, point the second ramsh at the pts).
  Recorded here so it is a fallback, not a redesign.

## 6. CDC functions in v1

Keep the composite descriptor set EXACTLY as host-tested (§1) —
changing it would invalidate the enumeration evidence. CDC-ACM is
the control channel (§5). CDC-NCM is included for parity but its
data path is a sink/source stub inside the daemon (accept, log
lengths, drop) until the netstack bridge lands as Phase A2; do NOT
advertise NCM to the operator as networking. Host side for the
control channel is a plain serial device (`cdc_acm` on the 3090);
no host driver work is part of v1.

## 7. Failure behaviour

Daemon init or runtime fault ⇒ log the core `Error` (it is a typed
enum: `UnsupportedCore(raw)`, `BadHwParams(raw)`, timeouts, …) to
the log scheme, exit non-zero, and leave everything else running.
There is no `Drop` cleanup in the core, so on exit the daemon makes
one best-effort `disconnect` attempt first. A payload whose daemon
fails must still reach the debug ramsh — that is a staging test
requirement (§8 V2), not a hope.

## 8. Verification ladder

- V0 (host, already true): `cargo test --offline` in
  `crates/dwc3-gadget` — 36 PASS. The daemon work must not touch the
  core crate; any core change restarts this ladder.
- V1 (compile spike): `crates/usbgadgetd` skeleton with the §3 Mmio
  impls behind `cfg(target_os = "redox")` and a `MemModel`-backed
  host path; `cargo check` host + a redoxer cross-check for
  `aarch64-unknown-redox` using the forks toolchain. Settles G-W1.
- V2 (QEMU negative path): stage per the sandbox-demo precedent and
  boot the QEMU ramboot image. QEMU has NO peripheral DWC3
  (root-verified B0 probe), so the expected result is: daemon fails
  cleanly at physmap/init, debug ramsh still reaches `ram:~$`.
  That failure path is what V2 proves — not enumeration.
- V3 (oracle): pmOS read-only capture per spec §7 → SMMU + GSNPSID
  settled with real values.
- V4 (device): payload v2 + gate-lite review + attempt #2 — a
  separate operator-gated work order, not part of this design.

## 9. Open gates

- G-W1: executor/critical-section linking on `aarch64-unknown-redox`
  (settled by V1).
- G-W2: `ramsh` argv patch + `35_ramsh_usb.service` +
  `90_ramboot.target` edit land in the base `op6/ramboot` branch /
  staging tree (settled by V1/V2 work).
- G-W3: SMMU SID 0x740 bypass-vs-translated (settled by V3; HARD gate
  for V4).
- G-W4: real GSNPSID / GHWPARAMS3 on SDM845 (settled by V3; the model
  values are placeholders until then).
- G-W5: scheme name `usbcon` collision check against base scheme
  names at V1 (no `usbcon` exists in base as of this design; `usb.*`
  is xhcid's host-side namespace, which this does not touch).
