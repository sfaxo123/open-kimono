# INTx net feasibility — E1 QEMU gate (read-only feasibility report)

Status: checkpoint 2026-10-25, DSH spark session `session-2f7841f0` (read-only lane).
Scope kept read-only: no code edits, no QEMU image build, no guest boot, no edits to
`scripts/` or `forks/redox`. Every command below is a **proposal** for the next
executor (orchestrator or a fresh bounded task), not something executed here.
Fact labels: **[F]** verified against source/probe this session; **[I]** inference;
**[U]** unknown.

## 0. Bottom line

**E1-EXP-1 (one minimal experiment):** re-run the E1 demo boot chain with the emulated
NIC swapped from `virtio-net-pci` (whose Redox-side IRQ allocation goes through the
non-x86 `Msix` `unimplemented!()` stub → the known `virtio: MSI-X is not implemented`
panic) to a **pure-INTx PCI NIC — primary `rtl8139`** (binary + `pcid.d` match-toml
already present in the E1 staging/ISO), then read the serial console to check whether
the NIC daemon (a) spawns, (b) opens its IRQ handle **without** panicking, and (c)
serves traffic (dhcpd lease). No Redox source change is needed to *try* the gate: the
whole INTx chain — pcid's DTB `interrupt-map` parser, the `/scheme/irq/phandle-*/<spec>`
channel, kernel `open_phandle_irq` → GICv3 SPI translate — **exists in the E1 baseline
and has never been executed** on this or any arch in-tree. [F]

**Premise correction:** `e1000-sysbus` **does not exist** in host QEMU 8.2.2
(`-M virt -device help`: `Device 'e1000-sysbus' not found`) — the task framing's
"e1000-sysbus vs PCI slot availability" resolves to *no sysbus path exists at all*.
Usable INTx NICs on `-M virt` are PCI-bus devices: `rtl8139`, `e1000` (=82540EM),
`e1000-82544gc`, `e1000-82545em` (and `e1000e`=82574L which has **no** Redox driver
here — ixgbed targets 82598/82599/X520 only; exclude). [F]

## 1. Existing QEMU gates vs this plan (kept distinct)

- **Existing gate 1 — `scripts/integrate.sh --qemu`:** builds `virt.dtb` via
  `qemu-system-aarch64 -M virt,acpi=off,gic-version=3,dumpdtb=…`, overlays
  `docs/workbench/K8B-SIMPLEFB.dts` (`dtc` → `fdtoverlay` → `simplefb-virt.dtb`), then
  boots `timeout 45` with `-nic user,model=virtio-net-pci,bootfile=bootaa64.efi` +
  `-dtb simplefb-virt.dtb`, and greps markers `ram:~$`, `timeline-demo-publish`,
  `spmid: no SPMI arbiter`, `vesad: simplefb 800x600`, `device tree simple-framebuffer`.
  **This gate never runs NIC daemons** — the latest run (`out/integration/qemu.log`,
  2026-10-25 04:11) ends `init: RAM boot shell on /scheme/debug; no rootfs mounted`
  (killed by timeout). The two `e1000`-looking grep hits in that log (`BB2E0000 size
  E10000`) are memory-range strings, not devices — do not misread them. [F]
- **Existing chain 2 — `scripts/e1-qemu.sh` (demo/live-ISO boot chain):** full-OS
  chain (`e1-live.iso` on virtio-blk `bootindex=2`, NIC `bootindex=1` with
  `tftp=…/tftp-live, bootfile=bootaa64.efi`, `-dtb simplefb-virt.dtb`); it greps
  `init: switchroot to /usr`, `vesad: simplefb 800x600`, `timeline-demo-publish`.
  The **only in-tree evidence of NIC daemons ever running** is the K14-era serial log
  `docs/workbench/E1-REDOX-SERIAL.txt`: `pcid-spawner: spawn
  "/scheme/initfs/lib/drivers/virtio-blkd"` (:476) and `spawn
  "/usr/lib/drivers/virtio-netd"` (:558), each followed by the `MSI-X is not
  implemented on this architecture` panic + `FATAL: Not an SVC` dumps (:480-481,
  :562-566), and `… virtio-blkd exited without notifying readiness` (:554). So today:
  **no network scheme exists at Redox runtime in any recorded run**, and where a NIC
  daemon ran, it died via the MSI-X stub — i.e. via a path that bypasses pcid's INTx
  handling, not via it. [F]
- **PXE ≠ Redox networking:** fetching `bootaa64.efi` via TFTP / UEFI PXE (host-side
  `10.0.2.0/24` user-net; the "10.0.2.15" claim in task text is **not verified in
  provided files** — treat as premise [U]) proves nothing about Redox's TCP/IP stack.
  Redox-runtime evidence markers are defined in §5.
- **This feasibility report is the input to a future, independent net gate.** It does
  **not** modify either gate above. E1-EXP-1 must run from a **worktree-local copy**
  (`scripts/qemu-demo/e1-intx-swap.sh` style, per the ledger's worktree-local
  `scripts/qemu-demo/` precedent), never by editing `scripts/e1-qemu.sh` or
  `scripts/integrate.sh`.

## 2. The INTx chain as it exists in base-e1 / kernel-e1

- **[F] pcid-interface helper:** `drivers/pcid/src/driver_interface/irq_helpers.rs` —
  on non-x86 there are exactly two sources of interrupt vectors: the
  `#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))] impl Msix` stubs
  (`enable`/`alloc` → `unimplemented!("… MSI-X is not implemented on this
  architecture")`, ~lines 307-319 — this is the exact `:313` line the virtio daemons
  panicked at), and `pci_try_allocate_interrupt_vector` (non-x86 arm ~413-431) which
  checks **no** MSI/MSI-X at all and returns `InterruptMethod::Intx` directly from
  `legacy_interrupt_line` (comment: `// FIXME support MSI on non-x86 systems`).
  `pci_allocate_interrupt_vector` (called by rtl8139d) aborts with
  `{driver}: no interrupts supported at all` when that line is `None`.
- **[F] Driver-side paths (the two INTx candidates):**
  - `drivers/net/e1000d/src/main.rs`: requires `legacy_interrupt_line`
    (`.expect("e1000d: no legacy interrupts supported")` when absent) → `irq.
    irq_handle("e1000d")` → EventQueue **subscribes the IRQ fd (READ)**; on event it
    reads the fd, checks `adapter().irq()` (`device.rs::fn irq()` = **ICR MMIO read**,
    e1000 read-to-clear INTx ack), writes the fd back, and `scheme.tick()`s the
    network stack. No MSI-X code anywhere in the driver.
  - `drivers/net/rtl8139d/src/main.rs:7,64`: `use pcid_interface::irq_helpers::
    pci_allocate_interrupt_vector;` → `pci_allocate_interrupt_vector(&mut
    pcid_handle, "rtl8139d")` → on aarch64 that helper takes the **INTx Legacy arm**.
    Event loop + driver-network wrapper strings present in the ISO binary
    (`rtl8139d: failed to get next event`, `failed to create network scheme`,
    `failed to regitster network scheme`).
  - `drivers/net/virtio-netd/src`: **no** `Msix` hits by grep — its panic string
    (`virtio: …`) carries the `{driver}` prefix, i.e. `Msix::enable(handle,
    "virtio")`-style direct stub call through pcid-interface. [I on call site]
- **[F] pcid DTB side:** `drivers/pcid/src/cfg_access/mod.rs` parses the host-bridge
  `interrupt-map` with `chunks_exact(4)` + a 5-u32 `next_chunk` window
  `[addr3, pin, phandle]`, then consumes `#address-cells`-worth of zero cells from the
  **interrupt-parent** and the parent `#interrupt-cells`-long specifier (match arms for
  1/2/3) into `InterruptMap`. `main.rs enable_function` sets IRQ line 9 default,
  validates pin 1-4, and `find`s the map entry with key
  `[dt_address & mask[0..3], pin & mask[3]]` (`mask = <0x1800 0 0 7>`) → `Some(
  LegacyInterruptLine { irq, phandled })`. `LegacyInterruptLine::irq_handle`: when
  `phandled` is `Some`, opens `File::create("/scheme/irq/phandle-<ph>/<spec cells>")`;
  otherwise `File::open("/scheme/irq/<n>")` (x86 PIC-style raw number; the non-x86
  `open_irq` arm creates it with `reservation: None`).
- **[F] kernel-e1 IrqScheme (GICv3 SPI routing via `open_phandle_irq`):**
  `src/scheme/irq.rs:224` `#[cfg(dtb)] open_phandle_irq`: splits the path spec,
  `IRQ_CHIP.phandle_to_ic_idx(phandle)` (`src/dtb/irqchip.rs:646`; ENOENT if
  unresolvable), `irq_xlate` → `arch/aarch64/device/irqchip/gic.rs:283-285`:
  `IrqCell::L3(0, irq, _) => irq + 32` (SPI INTID math; `:277` same decode for
  dispatch; IROUTER/ITARGETSR code paths ~40-53, SPI level/enable loops ~333-345),
  `scheme_irq_number` (`:129` → `Option<u16>` — the K6 irq-width work landed this
  u16/224-virq redesign in the main line, `docs/workbench/K6-IRQ-WIDTH.md`), and on
  O_CREAT: `IRQ_CHIP.irq_configure_route(...)` (`dtb/irqchip.rs:495`) +
  `set_reserved`. → **The kernel can translate and route a PCI INTx SPI given a hit in
  pcid's map; the phandle path is the designed one on aarch64.** Whether the
  pcid-key↔map-entry arithmetic above actually *hits* for a hot-plugged slot, and
  whether e1000d's fd-write equals a correct level-IRQ EOI under this scheme — both
  never executed → **[U]**, resolved by E1-EXP-1.
- **[F] DTB ground truth (generated, live `out/integration/simplefb-virt.dtb`; same
  lineage in `virt.dtb`):** `pcie@10000000` node carries `interrupt-map` **160
  cells** = 16 devfn-groups × 4 pins, each entry
  `addr3 · pin · phandle 0x8002 · <2 zero addr cells> · <spec3>` — e.g.
  dev0 INTA → spec `<0 3 4>` (SPI 3 → INTID 35, level-high). Parent `intc@8000000`:
  `#address-cells=<2>`, `#interrupt-cells=<3>`; root `interrupt-parent=<0x8002>`;
  `msi-map` → ITS `0x8003` exists but is **unused by Redox** (no ITS consumer; MSI/MSI-X
  skipped on non-x86 — see helper above). A python replay of pcid's parser logic over
  these real bytes consumed all 16 entries cleanly with the 5-cell+2+3 layout — the
  byte layout **matches** what pcid's match arms expect, **provided**
  `find_phandle(0x8002)`'s node yields `#address-cells` (it exists here; whether
  pcid's key arithmetic then *hits* stays [U]).
- **[F] QEMU host probe (this machine, QEMU 8.2.2):** `e1000`, `e1000-82544gc`,
  `e1000-82545em`, `e1000e`, `rtl8139`, `virtio-net-pci`, `pci-bridge`,
  `pcie-root-port`, `pcie-pci-bridge` all listed by `-M virt -device help` (bus PCI);
  `e1000-sysbus` **not found**; `e1000` and `rtl8139` instantiate fine on `-M virt`
  (dumpdtb probes exited 0). **No `pci-bridge`/`plxi3340` node** appears in the
  auto-dumped DTB (0 matches) → no automatic 32-bit PCI bridge slot set on this
  machine model; NICs attach to the pcie host bus with the static per-slot
  `interrupt-map` shown. No `iommu-map` seen → default no-IOMMU path [F-probe].
- **[F] Image contents (E1 staging + ISO):** `e1-staging/usr/lib/drivers/` contains
  `e1000d`, `rtl8139d`, `ixgbed`, `rtl8168d`, `virtio-netd`, … and
  `usr/lib/pcid.d/{e1000d,rtl8139d}.toml`: e1000d `class 0x02, ids 8086=[1004,100e,
  100f,109a,1503]`; rtl8139d `class 0x02, ids 10ec=[8139]` → command = driver name.
  `pcid-spawner/src/main.rs` enumerates `/scheme/pci`, matches `full_device_id`
  against `pcid.d/*.toml` (`match_function`) and spawns the named driver. **QEMU's
  `rtl8139` emulates the exact `10ec:8139` id** → spawner match is the safe path
  [F/U: whether the ISO's own toml set still matches each *emulated* device's id must
  be checked at run; e1000 variants need the toml id-list checked per model].
- **[F] AAVMF risk:** host `QEMU_EFI.fd` (Debian `2024.02-2ubuntu0.10` build) contains
  no `E1000/8254/8139/Ne2000/VirtioNet/Network` driver strings → firmware NIC-boot
  (PXE) with this AAVMF is unlikely to exist; swapping the NIC model should therefore
  not alter the Redox boot chain if it boots from ISO/TFTP as configured. **[I]**

## 3. E1-EXP-1 — the one minimal next experiment (proposal; do-not-execute here)

**Host-side pre-check (≤60 s; tool query, not a guest):** one `dumpdtb` with the
swapped NIC (and one with an explicit bridge) run in `out/integration/`, `fdtdump`
and confirm (a) no `e1000`/`rtl8139`/ethernet fdt node appears (expected — QEMU
doesn't describe PCI NIC endpoints in fdt) and (b) the `pcie@10000000` node with its
16×4 `interrupt-map` is still present → this de-risks the "slot wiring" unknown before
spending a boot. [F design: endpoint fdt absence was already confirmed in the probes]

**Gate (one bounded run, primary attempt):** in a **worktree copy** of the
`scripts/e1-qemu.sh` chain (new file, e.g. `scripts/qemu-demo/e1-intx-swap.sh`;
shared scripts untouched), change exactly the NIC device, keeping machine flags,
`-dtb simplefb-virt.dtb`, ISO and user-net:

```bash
# diff vs the existing e1-qemu chain — the ONLY intended change:
-    -device virtio-net-pci,netdev=e1net,bootindex=1 \
+    -device rtl8139,netdev=e1net,bootindex=1 \
# exact full command (from the worktree root; $fork/$tftp/$dtb/$image = worktree-local
# out/ paths per the ledger rule; bootfile kept identical to the current chain):
set -euo pipefail
fork=$HOME/Developer/forks/redox            # read-only source of image/tftp
out="$PWD/out/integration"; mkdir -p "$out"
test -f "$fork/redox/build/aarch64/desktop-minimal/e1-live.iso"
timeout --signal=TERM --kill-after=5 90 qemu-system-aarch64 \
    -machine virt,acpi=off,gic-version=3 -cpu max -smp 1 -m 2048 \
    -bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd \
    -drive "if=none,id=e1disk,file=$fork/redox/build/aarch64/desktop-minimal/e1-live.iso,format=raw" \
    -device virtio-blk-pci,drive=e1disk,bootindex=2 \
    -device ramfb -dtb "$out/simplefb-virt.dtb" \
    -netdev "user,id=e1net,tftp=$fork/bootloader-e1/build/tftp-live,bootfile=bootaa64.efi" \
    -device "rtl8139,netdev=e1net,bootindex=1" \
    -display none -serial file:"$out/intx-swap-serial.txt" \
    -monitor "unix:$out/intx-swap-monitor.sock,server,nowait"
grep -q 'spawn ".*\(rtl8139d\|e1000d\)"' "$out/intx-swap-serial.txt"   # spawner fired
grep -vq 'MSI-X is not implemented' "$out/intx-swap-serial.txt"        # old panic gone
# optional (only if dhcpd is enabled in that chain's service set — [U] verify at run):
grep -E 'dhcpd:.*(10\.0\.2\.[0-9]+|yiaddr)' "$out/intx-swap-serial.txt" || true
```

- **Why rtl8139 as primary:** INTx-only (no MSI-X capability at all → cannot reach the
  `Msix` stub even accidentally); driver binary **and** its `pcid.d` toml id `8139`
  already in the image/staging (exact QEMU-emulated id); `100 Mb/s` speed irrelevant to
  a scheme/lease gate. **e1000-82544gc/82545em/82540EM = fallback ladder** — same
  INTx design (verified: `e1000d` IRQ-fd subscription + ICR ack) but its toml ids
  `1004/100e/100f/109a/1503` must be checked to actually cover the *emulated* model's
  id, otherwise pcid-spawner will not spawn anything and the gate proves nothing.
  `e1000-sysbus`: excluded (doesn't exist); `e1000e`: excluded (82574L, no Redox
  driver). If a variant must hang off a bridge, use explicit
  `-device pcie-pci-bridge -device rtl8139,bus=pci-bridge.0` — default `virt` has no
  auto PCI-bridge here [F].
- **Expected Redox-runtime evidence (PASS markers, all on the serial log):**
  1. `pcid-spawner: spawn "…rtl8139d"` (or `…e1000d`) — the spawn-by-scan chain works;
  2. driver init banner (` + RTL8139 ` / `E1000 …`) and **no**
     `not implemented: …MSI-X…`, **no** `FATAL: Not an SVC`, **no**
     `no interrupts supported at all` / `no legacy interrupts supported` /
     `failed to open IRQ file` panic (their presence = the chain's failure signature,
     see below);
  3. `network` scheme registered for the NIC: absence of
     `failed to create network scheme` / `failed to regitster network scheme`; the
     task premise "`/scheme/tcp` exists" does **not** apply to this stack — this tree
     exposes `network.<name>` scheme(s) per daemon (driver-network wrapper asserts
     `scheme_name.starts_with("network")`), so `/scheme/network.*` is the right check;
  4. strongest (optional): a dhcpd lease — dhcpd source defaults to `iface = "eth0"`
     and prints `dhcpd: {err}` on failure; success would show an acquired
     `10.0.2.16`-style yiaddr on the QEMU user-net. Only required if dhcpd is in that
     chain's service set [U]; if not present, markers 1-3 are the acceptance.
- **Hard timeout:** `timeout --signal=TERM --kill-after=5 90` (90 s wall per attempt —
  2× the current 45 s gate budget: boot + spawner + lease; `e1-qemu.sh`'s own marker
  loops cap ≈160 s+40 s — keep ≤90 s here). Total experiment budget ≤4 attempts ×
  90 s + ≤2×60 s dumpdtb pre-checks; **stop on repeated identical failures** (same
  signature twice = chain limitation, not tuning; escalate to plan revision instead).
- **Fail signatures (read straight off the serial; each names the next action):**
  `no interrupts supported at all` / `no legacy interrupts supported` → pcid map
  lookup miss → try explicit bridge/bus variants; `failed to open IRQ file` → kernel
  xlate/route gap → plan revision (kernel edit, new task); `failed to get next
  event` storm = IRQ delivered but wrong SPI/polarity → plan revision;
  `no driver for 0:xx.0` (spawner debug) → toml id ≠ emulated id → toml/id decision
  (orchestrator; **not** an inline edit here — pcid.d tomls are generated image
  content).
- **Rollback:** the run happens only in the worktree-local script copy +
  worktree-local `out/integration/` scratch (`e1-live.iso`, `virt.dtb`,
  `simplefb-virt.dtb` under `out/` are generated artifacts — safe to recreate; do not
  delete `docs/workbench` evidence). Nothing in `scripts/` or `forks/` is edited by
  this plan; the gate script itself stays worktree-local until the orchestrator
  decides otherwise. Killing QEMU = write `quit` to the monitor socket (as today's
  scripts do) or rely on the 90 s timeout kill. **No rollback = "restore edited
  script", because there is no edited script.**

## 4. Fact vs inference vs unknown (summary)

- **[F]** Everything cited in §1-§2 with file:line/byte evidence: the `:313` panic and
  non-x86 INTx-only helper; both candidate drivers' INTx paths; the `8086/10ec`
  tomls; the `pcie@10000000` 16×4 `interrupt-map` + GIC cells; QEMU catalog &
  instantiation probes; `e1000-sysbus` absence; no auto PCI-bridge; the daemon-spawn +
  panic history; the current gate's RAM-shell state and its non-net markers.
- **[I]** "Firmware PXE with this AAVMF probably can't drive any NIC" (strings scan
  only); the `10.0.2.15` legacy claim (premise, not in provided logs); that the
  pcid key-arithmetic + kernel route *would* work end-to-end (each piece exists; the
  chain never ran); that rtl8139d's event loop is IRQ-fd-driven (inferred from
  helper + common/daemon strings, file not fully read).
- **[U → resolved only by E1-EXP-1]** whether QEMU wires pcie.0 INTx to the correct
  SPI for the attached slot; whether pcid's `find` hits for a real device; whether
  e1000d/rtl8139d run correctly once spawned (both untested upstream); whether the
  live chain's boot path depends on NIC-firmware PXE (AAVMF likely rules this out);
  whether dhcpd is even in the service set.

## 5. Explicit non-claims

Emulator-only scope (SDM845 has no PCI — the phone question is explicitly out of
scope); no claim that QEMU *will* work (three links above are unproven — the gate is
cheap precisely because failure is instantly readable from serial); no QEMU or image
build was performed while writing this; nothing outside this file was edited; the
`e1-live.iso` was **not** rebuilt (primary attempt consumes the existing image; if the
run later shows missing binaries, that becomes an image-content task, not an
assumption).
