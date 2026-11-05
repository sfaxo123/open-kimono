# K11 — DWC3 device mode + CDC-NCM/ACM (host evidence)

Branch `drivers/usb-gadget`. Spec: `specs/dwc3-gadget.md`. Crate: `crates/dwc3-gadget` (no_std, MIT).
Test log: `K11-TESTS.txt` (debug + release, clippy, aarch64 no_std build).

## What exists

- `Dwc3<R: Mmio, D: Mmio>`: polled core — GSNPSID/GHWPARAMS3 checks, bounded soft reset, device mode,
  QSCRATCH UTMI-as-pipe-clock + VBUS override, GUSB2PHYCFG quirks, event ring, EP0 control state
  machine (SETUP → DATA → STATUS_2/STATUS_3, ZLP, stall + re-arm), bulk/interrupt endpoints (one TRB
  per packet, bounce buffers), DEPSTARTCFG(2) at configuration, bounded endpoint commands.
- `Driver`/`Bus`/`ControlPipe`/`Endpoint`: `embassy-usb-driver` 0.2.2 traits over `&RefCell<Dwc3>`.
- DMA layout: one 22 KiB region (`layout.rs`): event ring, TRBs, EP0 buffer, 512 B per endpoint.

## Tests (36 + 1 ignored doc example)

| File | Count | What |
|---|---|---|
| `src/*` unit | 8 | TRB/event encode/decode, DMA layout |
| `tests/init.rs` | 10 | strict mmio-replay golden trace init+connect (literal spec values), disconnect; negatives: unknown core, bad GHWPARAMS3 (4 values), unaligned DMA, soft-reset timeout, command timeout, command status error, RUN/STOP halt timeout, connect before init |
| `tests/enumerate.rs` | 3 | real `embassy-usb` 0.6.0 on the scripted model: reset → GET_DESCRIPTOR(dev, 64) → SET_ADDRESS (DEVADDR written before status) → GET_DESCRIPTOR(dev/config/string) → SET_CONFIGURATION (DEPSTARTCFG(2) before user DEPCFG); CDC-ACM SET/GET_LINE_CODING + bulk echo (11 B, 300 B); CDC-NCM GET_NTB_PARAMETERS, SET_NTB_INPUT_SIZE, SET_INTERFACE alt 1, NETWORK_CONNECTION notification, NTB16 echo of a 60 B and a 1000 B (3-packet) frame |
| `tests/negative.rs` | 15 | stall on unknown request + recovery, EP0 OUT overflow stall, IN data aborted by reset, bulk read overflow, oversize write, disable ends transfer with resource index, bulk stall/clear, reset disables endpoints, suspend/wakeup/disconnect, SuperSpeed refused, corrupt GEVNTCOUNT, allocation limits, command failure/timeout (no retry storm), status NAKed until accept |

Negative/error-path tests: 8 of the 10 in `init.rs` (all but the init+connect and disconnect
traces) plus all 15 in `negative.rs` → 23.

The scripted model (`tests/common/model.rs`) executes DEPCMDs, packetizes host traffic through the
armed TRBs, posts events into the ring, and records driver protocol violations (status TRB type vs
two/three-stage, START on a busy/unconfigured endpoint, END_TRANSFER with a wrong resource index,
GEVNTCOUNT over-ack, babble). Mutation check: 5 deliberate driver bugs (STATUS_2 after OUT data,
no DEPSTARTCFG(2), no EP0 MPS update, no DEVADDR write, over-ack) each fail the enumeration tests;
a one-bit DEVTEN change fails the golden trace.

## Not proven here (hardware / integration gates)

- Real register behaviour: model values (GSNPSID `0x5533310a`, GHWPARAMS3 16/8 endpoints) are
  placeholders until the oracle capture in spec §7.
- **SMMU:** the core is behind `apps_smmu` SID 0x740; DMA bus addresses equal physical only if the
  stream is bypassed. No Redox SMMU driver exists.
- No Redox daemon yet (`usbgadgetd`: map windows, allocate uncached DMA, run embassy-usb, bridge NCM
  to the network stack and ACM to a pty). Needs a `critical-section` implementation (CDC-ACM).
- No IRQ path (SPI 133; needs K6 IRQ width); polling only.
- Not handled: SETUP arriving during a data stage (controller "setup pending" status), isochronous,
  TX FIFO resizing, LPM/U1/U2, remote wakeup, multi-TRB/scatter transfers (throughput).
