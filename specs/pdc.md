# PDC — Qualcomm Power Domain Controller wake interrupt controller (`qcom,sdm845-pdc`)

The PDC sits **in front of the GICv3** in the always-on (AOSS) power domain. It is the only
controller that can see an interrupt while the application processors and the TLMM/GIC are powered
down, and it forwards the wake event to a *dedicated* GICv3 SPI. On sdm845 it runs in
**pass-through** mode: it does not latch, it only inverts polarity and gates.

Citation shorthand (repo @ commit, from `git -C <repo> log -1 --format=%h`):

| tag | repo | commit |
|---|---|---|
| `L:` | `~/Developer/forks/linux` | `5b0764a` |
| `U:` | `~/Developer/forks/u-boot` | `8649a54d` |
| `O:` | `~/Developer/forks/openbsd-src` | `20cb344` |
| `E:` | `~/Developer/forks/edk2-sdm845` | `d7fa8bd` |
| `R:` | `~/Developer/forks/redox/{kernel,base}` | `45bca4e3` |

---

## 1. Block

### 1.1 The PDC interrupt controller node

`pdc_intc: interrupt-controller@b220000` — `L:arch/arm64/boot/dts/qcom/sdm845.dtsi:5105-5112`
(byte-identical copy in `U:dts/upstream/src/arm64/qcom/sdm845.dtsi:5107-5120`):

| property | value | line |
|---|---|---|
| `compatible` | `"qcom,sdm845-pdc", "qcom,pdc"` | `L:…/sdm845.dtsi:5106` |
| `reg` | `<0 0x0b220000 0 0x10000>` | `:5107` |
| `qcom,pdc-ranges` | `<0 480 94>, <94 609 15>, <115 630 7>` | `:5108` |
| `#interrupt-cells` | `<2>` (pin, flow type) | `:5109` |
| `interrupt-parent` | `<&intc>` | `:5110` |
| `interrupt-controller` | present | `:5111` |

The node has **no clocks, no resets, no supplies and no pinctrl** — it lives in the always-on
domain and is never clocked or reset by the app processor (`L:…/sdm845.dtsi:5105-5112`, whole node).
The driver binds on the **fallback** string only: `IRQCHIP_MATCH("qcom,pdc", qcom_pdc_probe)` —
`L:drivers/irqchip/qcom-pdc.c:764`.

Binding: `U:dts/upstream/Bindings/interrupt-controller/qcom,pdc.yaml:47` (`qcom,sdm845-pdc`),
`:74` (`qcom,pdc-ranges`), `:93` (required). Semantics of the triple — *starting PDC port, GIC hwirq
number for that port, count* — `U:…/qcom,pdc.yaml:74-86`.

### 1.2 The PDC global reset block (adjacent, **out of scope**)

`pdc_reset: reset-controller@b2e0000`, `compatible = "qcom,sdm845-pdc-global"`,
`reg = <0 0x0b2e0000 0 0x20000>` — `L:…/sdm845.dtsi:5114-5118`. Its only consumer on this phone is
the modem remoteproc: `<&pdc_reset PDC_MODEM_SYNC_RESET>` / `reset-names = "mss_restart", "pdc_reset"`
— `L:…/sdm845.dtsi:3429-3430`. Reset ids: `PDC_APPS_SYNC_RESET 0` … `PDC_MODEM_SYNC_RESET 9` —
`U:dts/upstream/include/dt-bindings/reset/qcom,sdm845-pdc.h:9,18`. **The IRQ driver must never touch
this region**; it is a different `reg` window and a different driver.

### 1.3 Who consumes the PDC as an interrupt parent

1. **TLMM (GPIO) wake lines** — `tlmm: pinctrl@3400000`, `compatible = "qcom,sdm845-pinctrl"`,
   `reg = <0 0x03400000 0 0xc00000>`, `interrupts = <GIC_SPI 208 IRQ_TYPE_LEVEL_HIGH>` (the GPIO
   *summary* line, INTID 240), `#interrupt-cells = <2>`, `gpio-ranges = <&tlmm 0 0 151>`, and
   **`wakeup-parent = <&pdc_intc>`** — `L:…/sdm845.dtsi:2801-2810`.
   The GPIO→PDC-pin table is SoC data: `sdm845_pdc_map[]` — `L:drivers/pinctrl/qcom/pinctrl-sdm845.c:1274-1290`,
   wired up by `.wakeirq_map` / `.nwakeirq_map` / `.wakeirq_dual_edge_errata = true` — `:1300-1302`,
   `.ngpios = 151` — `:1299`.
2. **USB3 PHY wake lines, directly on `&pdc_intc`** — `usb30_prim`: `dp_hs_phy_irq` = pin 9,
   `dm_hs_phy_irq` = pin 8, `ss_phy_irq` = pin 6 — `L:…/sdm845.dtsi:4213-4222`; `usb30_sec`: pins 11,
   10, 7 — `:4291-4297`. These are *SPI-class* PDC pins, not GPIO-class (see §3.4).
3. **GICv3** is the PDC's parent: `intc: interrupt-controller@17a00000`, `arm,gic-v3`,
   `#interrupt-cells = <3>`, GICD `0x17a00000 0x10000`, GICR `0x17a60000 0x100000`,
   `interrupts = <GIC_PPI 9 …>` — `L:…/sdm845.dtsi:5463-5472`.

### 1.4 Pin → GIC SPI → INTID map (derived from `qcom,pdc-ranges`)

`pin_to_hwirq(region, pin) = parent_base + pin - pin_base` — `L:drivers/irqchip/qcom-pdc.c:186`.
The `parent_base` in DT is a **DT `GIC_SPI` number**, and the GIC domain adds 32:
`case 0: *hwirq = fwspec->param[1] + 32` — `L:drivers/irqchip/irq-gic-v3.c:1607` (translate entry
`gic_irq_domain_translate` at `:1588`). Redox's GICv3 driver does the same: `hwirq = irq + 32` —
`R:kernel/src/arch/aarch64/device/irqchip/gicv3.rs:138,143`.

| PDC pins | DT `GIC_SPI` | GIC INTID |
|---|---|---|
| 0–93 | 480–573 | 512–605 |
| 94–108 | 609–623 | 641–655 |
| 115–121 | 630–636 | 662–668 |
| 109–114 | *not in any range* | — (see §8 gaps) |

Worked examples for the OnePlus 6 wake sources:

| source | TLMM GPIO | PDC pin | INTID | DT reference |
|---|---|---|---|---|
| Hall sensor (`SW_LID`, `wakeup-source`) | 124 | 99 (`L:…/pinctrl-sdm845.c:1288`) | 646 | `L:…/sdm845-oneplus-common.dtsi:85-98` |
| Synaptics RMI4 touch (`EDGE_FALLING`) | 125 | 100 (`:1288`) | 647 | `L:…/sdm845-oneplus-common.dtsi:526` |
| NFC (`EDGE_RISING`) | 63 | 64 (`:1281`) | 576 | `L:…/sdm845-oneplus-common.dtsi:496` |
| USB3 prim `ss_phy_irq` / `dm` / `dp` | — | 6 / 8 / 9 | 518 / 520 / 521 | `L:…/sdm845.dtsi:4215-4222` |
| Volume keys | *not TLMM* — `&pm8998_gpios 5/6` | — | — | `L:…/sdm845-oneplus-common.dtsi:110-121` |
| Alert slider top pin | 126 | **absent from `sdm845_pdc_map`** | — | `L:…/sdm845-oneplus-common.dtsi:40` |

The alert-slider pin 126 has no PDC wake line (the map jumps `{125,100}` → `{127,102}`,
`L:…/pinctrl-sdm845.c:1288`), so it cannot wake the phone; its other pins 52 and 24 do have lines
(`{52,54}` `:1279`, `{24,37}` `:1276`).

---

## 2. What firmware leaves configured

**Unknown from local sources — treat as dirty and re-init everything.**

- ABL/edk2: `E:sdm845Pkg/Include/Configuration/DeviceMemoryMap.h:309-316` reserves only
  `PDC_DISPLAY` (0x0B290000) and `PDC_DISP_SEQ` (0x0B490000) as `EfiMemoryMappedIO`/uncacheable.
  There is **no** entry anywhere in `sdm845Pkg` for `0x0B220000` or `0x0B2E0000` (grep for those
  addresses returns nothing). So the PDC intc window is not described to the OS by the firmware's
  memory map, and whether ABL left wake pins armed is not observable from source.
- U-Boot carries the DT nodes (`U:dts/upstream/src/arm64/qcom/sdm845.dtsi:5107-5120`) but has **no
  PDC driver** — nothing in `U:drivers/` binds `qcom,pdc`. So a U-Boot boot path leaves the PDC in
  whatever state ABL/TrustZone left it.
- Consequence for the driver: the *only* safe assumption is the one both Linux and OpenBSD make —
  **mask every pin in `qcom,pdc-ranges` at init** (§4.1). Linux: `pdc_setup_pin_mapping()` calls
  `pdc->enable_intr(pin, false)` for every pin of every range —
  `L:drivers/irqchip/qcom-pdc.c:643-648`. OpenBSD: same loop, `HCLR4(sc, PDC_INTR_ENABLE(pin), bit)` —
  `O:sys/dev/fdt/qcpdc.c:131-133`.
- Mode: sdm845 is driven in **pass-through**; the SCM call that switches mode is only issued for the
  x1e/x1p compatibles — `L:drivers/irqchip/qcom-pdc.c:699` (`pdc->mode = PDC_PASS_THROUGH_MODE`),
  `:725-727` (`qcom_scm_io_writel(PDC_GPIO_INT_CTL_ENABLE, …)` → `PDC_SECONDARY_MODE`).
  `PDC_GPIO_INT_CTL_ENABLE = 0xb2045e8`, `PDC_PASS_THROUGH_MODE = 0x0`, `PDC_SECONDARY_MODE = 0x1` —
  `:31-33`. **Redox must not make that SCM call on sdm845.**

---

## 3. Register map

All offsets are from the `reg` base `0x0b220000`. Access is 32-bit; register array access is
`base + reg + i*4` — `L:drivers/irqchip/qcom-pdc.c:190-203`.

### 3.1 Layout selection by version

| offset | name | fields | citation |
|---|---|---|---|
| `0x1000` | `PDC_VERSION_REG` | `[23:16]` MAJOR, `[15:8]` MINOR, `[7:0]` STEP | `L:…/qcom-pdc.c:36-39`; read at `:683` |
| `0x100c` | `IRQ_PARAM_REG` (bank 0) | `[15:8]` NUM_GPIO, `[7:0]` NUM_SPI | `L:…/qcom-pdc.c:71-74` (layout table), `:161/173` (offset), `:734` (`num_spis = FIELD_GET(GENMASK(7,0), …)`) |

Version thresholds `3.2 / 3.0 / 2.7` — `L:…/qcom-pdc.c:45-47`; selection at `:685-697`.

### 3.2 Per-pin registers (v2.7 / v3.0 layout — the one OpenBSD hardcodes)

| offset | name | index | fields | citation |
|---|---|---|---|---|
| `0x10 + 4*b` | `IRQ_ENABLE_BANK[b]` | `b = pin / 32`, bit `pin % 32` | one enable bit per pin | `O:sys/dev/fdt/qcpdc.c:28-29`; `L:…/qcom-pdc.c:27-28` (`IRQ_ENABLE_BANK_INDEX_MASK GENMASK(31,5)`, `…_BIT_MASK GENMASK(4,0)`), `:171` (offset), `:242-259` (read-modify-write under a raw spinlock) |
| `0x110 + 4*n` | `IRQ_CFG[n]` | `n = pin` | `[2:0]` Type; v3.0 adds `[3]` GPIO_MASK, `[4]` GPIO_STATUS; v3.2 `[3]` IRQ_ENABLE, `[4]` GPIO_MASK, `[5]` GPIO_STATUS | `O:sys/dev/fdt/qcpdc.c:30`; `L:…/qcom-pdc.c:63-69` (version table), `:172` (offset), `:177` (`irq_type = GENMASK(2,0)` for v2.7), `:165-167` (v3.0 bits), `:152-155` (v3.2 bits) |

`IRQ_ENABLE_BANK` is indexed over `PDC_MAX_IRQS = 256` — `L:…/qcom-pdc.c:25-26`. Driver window is
`PDC_DRV_SIZE = 0x10000`, and the mapping is widened to at least that:
`res_size = max_t(resource_size_t, resource_size(&res), PDC_DRV_SIZE)` — `L:…/qcom-pdc.c:35,669`.
(The DT `reg` here is already `0x10000`, so no widening is needed on sdm845.)

### 3.3 The Type field — the inverter

The GIC cannot accept falling-edge or active-low. The PDC inverts falling→rising and
low→high, and the polarity bit pattern must be set per this table —
`L:drivers/irqchip/qcom-pdc.c:341-355`:

| requested | Type value | forwarded to GIC |
|---|---|---|
| level low | `0b000` `PDC_LEVEL_LOW` | level high |
| edge falling | `0b010` `PDC_EDGE_FALLING` | edge rising |
| level high | `0b100` `PDC_LEVEL_HIGH` | level high |
| edge rising | `0b110` `PDC_EDGE_RISING` | edge rising |
| edge both | `0b111` `PDC_EDGE_DUAL` | edge rising |

enum values — `L:…/qcom-pdc.c:356-362`; mapping + parent retype — `:381-409`. OpenBSD encodes the
same five values (`0x0/0x2/0x4/0x6/0x7`) — `O:sys/dev/fdt/qcpdc.c:31-35` — and writes them from the
OpenBSD level codes at `:166-186`.

### 3.4 GPIO-class vs SPI-class pins

`pdc_pin_is_gpio(pin) = pin >= pdc->num_spis` — `L:…/qcom-pdc.c:205-216`, with `num_spis` read from
`IRQ_PARAM_REG` at `:734`. Only GPIO-class pins get the latching "secondary" chip, and only when the
block is in secondary mode — `:566-579`. On sdm845 (pass-through) **all** pins use the pass-through
chip, whose ops are pure hierarchy pass-through except `irq_set_type` and enable/disable —
`:494-511`. There is **no PDC-side ack or EOI in pass-through mode**: `.irq_eoi = irq_chip_eoi_parent`
— `:496`. The v3.2-only per-pin `IRQ_ENABLE`/`GPIO_MASK`/`GPIO_STATUS` handling
(`:261-292`, selected at `:688`) and the X1E bank-remap quirk (`:218-240`) are **not applicable** to
sdm845.

---

## 4. Sequences

### 4.1 Init (probe)

1. Map `0x0b220000` for `max(reg_size, 0x10000)` bytes, uncacheable —
   `L:…/qcom-pdc.c:669`; OpenBSD equivalent `bus_space_map(fa_reg[0].addr, fa_reg[0].size, 0, &ioh)`
   after requiring `fa_nreg >= 1` — `O:sys/dev/fdt/qcpdc.c:100-129`.
2. Read `PDC_VERSION_REG` (`0x1000`) and pick the register layout — `L:…/qcom-pdc.c:683-697`.
   (OpenBSD skips this step entirely — see §8.)
3. Read `IRQ_PARAM_REG` (`0x100c`) bank 0, take `[7:0]` as `num_spis` — `L:…/qcom-pdc.c:734`.
4. Set mode = pass-through; **do not** issue the SCM mode write — `L:…/qcom-pdc.c:699,725-727`.
5. Parse `qcom,pdc-ranges` as N triples; reject if `n <= 0 || n % 3` —
   `L:…/qcom-pdc.c:618-641`; OpenBSD checks `len % (3*4)` and reads the array —
   `O:sys/dev/fdt/qcpdc.c:109-124`.
6. For **every** pin of **every** range: clear any stale GPIO_STATUS (v3.0+ secondary only — skip on
   sdm845) and **disable** it — `L:…/qcom-pdc.c:643-648`; `O:sys/dev/fdt/qcpdc.c:131-133`.
7. Register the PDC as a child domain of the GIC with a *wakeup* bus token, sized `PDC_MAX_IRQS`
   (256) — `L:…/qcom-pdc.c:750-752`, `irq_domain_update_bus_token(pdc_domain, DOMAIN_BUS_WAKEUP)`
   at `:758`.
8. TLMM side: resolve `wakeup-parent` to that wakeup domain and install the GPIO→PDC-pin translation
   — `L:drivers/pinctrl/qcom/pinctrl-msm.c:1471-1488`; every GPIO present in `wakeirq_map` is then
   marked `skip_wake_irqs` (`:1484-1487`).

### 4.2 Operate — arm one wake source (example: touch, TLMM GPIO 125)

1. Translate GPIO → PDC pin through the SoC wake map: `{125, 100}` —
   `L:drivers/pinctrl/qcom/pinctrl-sdm845.c:1288`; lookup loop `msm_gpio_wakeirq()` —
   `L:drivers/pinctrl/qcom/pinctrl-msm.c:1396-1418`. A GPIO with **no** entry gets
   `*parent = GPIO_NO_WAKE_IRQ` (`~0U`) — `pinctrl-msm.c:1406`, `L:include/linux/soc/qcom/irq.h`
   (`git show HEAD:include/linux/soc/qcom/irq.h`), and the PDC then **disconnects the hierarchy** for
   that interrupt: `if (hwirq == GPIO_NO_WAKE_IRQ) return irq_domain_disconnect_hierarchy(...)` —
   `L:…/qcom-pdc.c:558-559`.
2. Compute the parent SPI: pin 100 falls in range 2, so `609 + (100 - 94) = 615` → INTID 647 —
   `L:…/qcom-pdc.c:186` + `L:drivers/irqchip/irq-gic-v3.c:1607`.
3. Set the PDC type: write `IRQ_CFG[100]` bits `[2:0] = 0b010` (edge falling), preserving the other
   bits — `L:…/qcom-pdc.c:405-407`.
4. Retype the **GIC** side to edge-rising (the PDC already inverted) — `L:…/qcom-pdc.c:387,409`;
   same rule at allocation time — `:587-591`.
5. If the type actually changed, **clear the phantom pending state at the GIC** — the PDC emits a
   spurious interrupt when polarity is reprogrammed — `L:…/qcom-pdc.c:413-423`.
6. Leave the TLMM `INT_CFG` for GPIO 125 **disabled/untouched**: for `skip_wake_irqs` pins, mask,
   unmask and set-type all return early without writing TLMM —
   `L:drivers/pinctrl/qcom/pinctrl-msm.c:846-847,896-897,1075-1082`. Programming both TLMM and PDC
   for the same line double-latches the event.
7. Enable: set bit `100 % 32` in `IRQ_ENABLE_BANK[100/32]` = `0x10 + 4*3`, read-modify-write under a
   lock — `L:…/qcom-pdc.c:242-259`; then enable the GIC parent — `:300-304`. OpenBSD order: parent
   establish first (`O:sys/dev/fdt/qcpdc.c:196-199`), then `HSET4(PDC_INTR_ENABLE(pin), bit)` under
   `splhigh()` — `:207`.
8. Delivery: the GIC acks INTID 647; there is no PDC status to read or EOI in pass-through mode
   (`L:…/qcom-pdc.c:494-511`). Disable = clear the bank bit, then disable the parent — `:294-298`;
   OpenBSD clears the bit then disestablishes the parent — `O:sys/dev/fdt/qcpdc.c:221-223`.

### 4.3 Error recovery

- A wake line that fires but whose PDC pin is disabled is simply lost — there is no PDC pending
  status readable in v2.7 pass-through. Recovery is: disable the pin, retype it (which clears the
  phantom pending at the GIC, `L:…/qcom-pdc.c:413-423`), re-enable.
- If the GIC reports an unexpected INTID in the 512–668 band, treat it as a stale wake: EOI it and
  clear the corresponding bank bit (`L:…/qcom-pdc.c:242-259`).
- Dual-edge GPIOs on sdm845 need the TLMM-side errata workaround (`wakeirq_dual_edge_errata = true`,
  `L:drivers/pinctrl/qcom/pinctrl-sdm845.c:1302`; workaround path
  `L:drivers/pinctrl/qcom/pinctrl-msm.c:1028-1070,950+`). Simplest Redox policy: **never request
  `IRQ_TYPE_EDGE_BOTH` on a PDC line** — pick one edge and re-arm after each event.

### 4.4 Shutdown / suspend

- Linux marks the PDC chip `IRQCHIP_MASK_ON_SUSPEND | IRQCHIP_SET_TYPE_MASKED |
  IRQCHIP_SKIP_SET_WAKE | IRQCHIP_ENABLE_WAKEUP_ON_SUSPEND` — `L:…/qcom-pdc.c:505-508`: the wake
  pins are re-enabled at suspend and masked on resume; the PDC itself needs no `set_wake`.
- OpenBSD's equivalent is `qcpdc_intr_set_wakeup()` (a no-op marker) —
  `O:sys/dev/fdt/qcpdc.c:244-259` — while the TLMM driver saves/restores its own `INT_CFG` and keeps
  wakeup pins enabled across suspend — `O:sys/dev/fdt/qcgpio_fdt.c:186-196`.
- Redox shutdown: clear every bank bit for every range (same loop as §4.1 step 6), then unmap.

---

## 5. Interrupts / DMA / power

**Polling first.** The PDC is testable without any interrupt: read `PDC_VERSION_REG` and
`IRQ_PARAM_REG`, write `IRQ_CFG[n]`, toggle `IRQ_ENABLE_BANK[b]`, and read the bank back. A
polling-first driver can also *detect* a wake by polling `TLMM_GPIO_INTR_STATUS(pin)`
(`0x000c + 0x1000*pin`, bit 0) — `O:sys/dev/fdt/qcgpio_fdt.c:49-50` — with TLMM armed normally,
which exercises the touch/hall GPIOs without touching the PDC at all.

**Then the IRQ path.** Two distinct paths must not be confused:

- *Normal (non-wake) GPIO interrupts*: device → TLMM per-pin logic → **TLMM summary SPI 208
  (INTID 240)** → GIC. The TLMM driver chains on that one line and walks its enabled-pin list,
  reading each pin's `INTR_STATUS` — `L:drivers/pinctrl/qcom/pinctrl-msm.c:1364-1393`, parent wired
  at `:1492,1501` from `platform_get_irq` at `:1631`.
- *Wake GPIO interrupts*: device → TLMM pin (left disabled) → **PDC pin** → **its own dedicated GIC
  SPI** → GIC. This is the path this spec covers. The TLMM summary line is not involved.

TLMM per-pin interrupt registers (for completeness, needed to *avoid* double-arming):
`INT_CFG = 0x0008 + 0x1000*pin` with `[7:5]` target proc (`0x3<<5`), `[4]` RAW_STATUS_EN,
`[3:2]` detection (`00` level, `01` pos, `10` neg, `11` both), `[1]` polarity, `[0]` enable;
`INT_STATUS = 0x000c + 0x1000*pin` bit 0 — `O:sys/dev/fdt/qcgpio_fdt.c:38-50`; Linux field names
`.intr_target_bit 5, .intr_target_kpss_val 3, .intr_raw_status_bit 4, .intr_polarity_bit 1,
.intr_detection_bit 2, .intr_detection_width 2, .intr_enable_bit 0` —
`L:drivers/pinctrl/qcom/pinctrl-sdm845.c:17-54` (the `PINGROUP` macro; no `.intr_target_reg`, so the
target field lives inside `INT_CFG` — `L:drivers/pinctrl/qcom/pinctrl-msm.c:102-116`). RAW_STATUS_EN
is deliberately left on for edge irqs because toggling it can spuriously latch status —
`L:drivers/pinctrl/qcom/pinctrl-msm.c:855-878,1124-1128`.

**DMA:** none. The PDC is a pure MMIO interrupt controller.

**SMMU:** none — the PDC does not master memory. But note the PDC window `0x0b220000` sits in the
always-on island; it must be mapped **device/uncacheable** and must not be behind the SMMU. edk2
does not list it (§2), so Redox's own memory map must carve it out.

**Power:** the block is unconditionally powered (no clocks/resets in its node, §1.1). It is the
*only* interrupt source that survives APSS+TLMM power-down —
`U:dts/upstream/Bindings/interrupt-controller/qcom,pdc.yaml:12-23` (PDC is in the always-on domain,
detects edge/low while the GIC is non-operational, and is next in the hierarchy below the GIC).

---

## 6. Redox interface

**Verdict: the PDC must be a kernel-side irqchip, not a userspace daemon.** It sits *below* the GIC
in the DT interrupt hierarchy: every wake interrupt's EOI, masking and type live in that chain
(`L:…/qcom-pdc.c:494-511`), and Redox already resolves interrupt parents by walking DT
(`R:kernel/src/dtb/irqchip.rs:44-50`, `init()`/`travel_interrupt_ctrl` at `:476-481`). A daemon
cannot sit in the ack/EOI path.

What goes where:

- **Kernel**: a `pdc` irqchip registered alongside `gicv3`
  (`R:kernel/src/arch/aarch64/device/irqchip/mod.rs`, `gicv3.rs`). It must implement the same three
  hooks the GIC driver implements: `irq_configure` (type), `irq_xlate` (DT cells → virq),
  `irq_to_virq` — compare `R:kernel/src/arch/aarch64/device/irqchip/gicv3.rs:133-155`. The PDC's
  `irq_xlate` takes **two** cells (`#interrupt-cells = <2>`, `L:…/sdm845.dtsi:5109`) — `IrqCell::L2`
  exists in Redox (`R:kernel/src/dtb/irqchip.rs:20-24`) — and internally emits the three-cell GIC
  form `{0, spi, type}`, exactly as Linux does at `L:…/qcom-pdc.c:598-602`.
- **Userspace**: input daemons (touch, hall, USB wake) open the resulting virq through `/scheme/irq`
  and poll it via `/scheme/event`, per `R:base/drivers/README.md:114`. Daemons start from
  `exampled` (project policy: `docs/PLAN.md:85-86`).
- **MMIO**: the kernel maps `0x0b220000` + `0x10000` device-uncacheable. If any part of the policy
  is pushed to userspace, the userspace mapping goes through
  `/scheme/memory/physical@uc` (`R:base/drivers/README.md:109-113`;
  `R:base/drivers/common/src/lib.rs:163-168`; kernel side `physmap()`
  `R:kernel/src/scheme/memory.rs:135-148`, which requires page-aligned base and size).

**Known Redox blockers to fix in the same change:**

1. The userspace IRQ space is capped at 224 entries and indexed by `u8`:
   `COUNTS: [usize; 224]`, `BASE_IRQ_COUNT = 16`, `TOTAL_IRQ_COUNT = 224` —
   `R:kernel/src/scheme/irq.rs:47,53-61`; `is_reserved`/`set_reserved` take `index: u8` —
   `R:kernel/src/dtb/irqchip.rs:500-513`. sdm845's PDC INTIDs are 512–668, so the PDC domain must
   hand out **small sequential virqs** from the GIC driver's linear allocator
   (`R:…/gicv3.rs:100-118`, `irq_range = (idx, idx + cnt)`) rather than INTIDs, and the scheme must
   be widened to reach them.
2. `available_irqs_iter()` is a stub returning an empty range on aarch64 —
   `R:kernel/src/dtb/irqchip.rs:516-519` — so `/scheme/irq/<n>` enumeration is not usable yet;
   drivers must open a known virq number.
3. `register_irq(virq, handler)` is the in-kernel hook (`R:kernel/src/dtb/irqchip.rs:483-497`) and
   `IRQ_CHIP.irq_desc` is 1024 entries (`:471-474`) — enough for the 116 PDC pins, but the PDC must
   be inserted into the `irq_desc` chain (`basic.child_ic_idx`, `:461-470`) rather than bolted on.

---

## 7. Verification

**Host (no phone).** A register-model unit test over the derived map:
- `qcom,pdc-ranges` parse → expect exactly 3 regions `{0,480,94}`, `{94,609,15}`, `{115,630,7}`
  (`L:…/sdm845.dtsi:5108`).
- `pin_to_hwirq` for pins 0, 93, 94, 108, 115, 121 → 480, 573, 609, 623, 630, 636
  (`L:…/qcom-pdc.c:186`); +32 → INTIDs 512, 605, 641, 655, 662, 668 (`L:drivers/irqchip/irq-gic-v3.c:1607`).
- Type encoding: falling→`0b010` and GIC retype to rising (`L:…/qcom-pdc.c:385-388`); dual→`0b111`
  (`:389-392`); level low→`0b000`→level high (`:396-399`).
- Bank/bit split: pin 100 → bank index 3, bit 4 (`L:…/qcom-pdc.c:27-28,247-248`;
  `O:sys/dev/fdt/qcpdc.c:28-29`).
- Model must reject: pin 109–114 (no region → hierarchy disconnect, `L:…/qcom-pdc.c:594-596`) and
  `GPIO_NO_WAKE_IRQ` (`:558-559`).

**Linux oracle on the running pmOS phone** (capture before writing Redox code):

```sh
cat /proc/interrupts | grep -E 'pdc|tlmm'      # which INTIDs Linux armed
devmem 0x0b221000                              # PDC_VERSION_REG  -> settles §3.1 version question
devmem 0x0b22100c                              # IRQ_PARAM_REG: [15:8] NUM_GPIO, [7:0] NUM_SPI
for b in 0 1 2 3 4 5 6 7; do devmem $((0x0b220010 + 4*b)); done   # IRQ_ENABLE_BANK[b]
devmem 0x0b221110                              # IRQ_CFG[0]
devmem $((0x0b221110 + 4*100))                 # IRQ_CFG[100] (touch) -> expect 0b010 | high bits
devmem 0x03500000                              # TLMM GPIO 125 INT_CFG (EAST tile, 0x03400000+0x00100000)
devmem $((0x03500000 + 0x1000*125 + 8))        # INT_CFG of GPIO 125 -> expect enable bit 0 CLEARED
devmem $((0x03500000 + 0x1000*125 + 12))       # INT_STATUS of GPIO 125
cat /sys/kernel/debug/irq/domains/PDC 2>/dev/null
cat /sys/kernel/debug/pinctrl/*/pinmux-pins | grep -i '125\|124'
```
Tile bases: `tlmm` reg base `0x03400000` (`L:…/sdm845.dtsi:2803`) plus `EAST 0x00100000`,
`NORTH 0x00500000`, `SOUTH 0x00900000`, `REG_SIZE 0x1000` — `L:drivers/pinctrl/qcom/pinctrl-sdm845.c:13-16`.
Then trace: `echo 1 > /sys/kernel/debug/tracing/events/irq/enable` and wave a hand over the hall
sensor / tap the screen; the expected handler is the PDC/`pdc` domain, not the `tlmm` chained
handler.

**On-phone acceptance (`fastboot boot`)**: boot a Redox image that (a) prints
`PDC_VERSION_REG` and `IRQ_PARAM_REG`, (b) masks all 116 mapped pins and reads the banks back as 0,
(c) arms PDC pin 100 (touch) edge-falling and prints one line per event on tap, (d) arms pin 99
(hall) and prints on lid close/open, (e) confirms `/proc`-equivalent: no TLMM `INT_CFG` enable bit
was set for those pins. Negative test: with the PDC pin disabled, a tap must produce **no** event.

---

## 8. License path

**Decision: MIT clean-room.** The hardware facts come from the device tree and the DT binding, and
the permissive reference implementation is OpenBSD.

References and their licenses:

| reference | license | use |
|---|---|---|
| `O:sys/dev/fdt/qcpdc.c` (Patrick Wildt, 2022; `$OpenBSD: qcpdc.c,v 1.3 2022-12-21 23:26:54 patrick Exp $`) | **ISC** (`:1-16`) | primary clean-room base: register offsets `:28-30`, type values `:31-35`, region struct `:52-55`, mask-all-at-attach `:131-133`, establish sequence `:148-209` |
| `O:sys/dev/fdt/qcgpio_fdt.c` (Mark Kettenis, 2022) | **ISC** (`:1-9`) | TLMM `INT_CFG`/`INT_STATUS` bit layout `:33-50`, suspend/resume save-restore `:186-196` |
| `O:sys/dev/fdt/files.fdt:794-797` | ISC | how an FDT irqchip is declared/attached |
| `U:dts/upstream/src/arm64/qcom/sdm845.dtsi:5107-5120` | ISC/BSD-side DT copy | DT facts, citable without touching Linux's GPL DT |
| `U:dts/upstream/Bindings/interrupt-controller/qcom,pdc.yaml` | ISC | binding semantics of `qcom,pdc-ranges` `:74-87` |
| `L:drivers/irqchip/qcom-pdc.c`, `L:drivers/pinctrl/qcom/pinctrl-msm.c`, `L:drivers/pinctrl/qcom/pinctrl-sdm845.c`, `L:include/linux/soc/qcom/irq.h`, `L:Documentation/devicetree/bindings/interrupt-controller/qcom,pdc.yaml` | **GPL-2.0** | read as *behavioural* evidence only (the inverter table, phantom-pending clear, `skip_wake_irqs` semantics, `GPIO_NO_WAKE_IRQ`, dual-edge errata). No code is copied. |
| `E:sdm845Pkg/Include/Configuration/DeviceMemoryMap.h` | BSD-3-Clause (edk2) | firmware MMIO reservations `:309-316` |

If the phantom-pending clear, the dual-edge errata, or the `GPIO_NO_WAKE_IRQ` disconnect turn out to
be hardware behaviours we cannot reproduce from the OpenBSD model, those *specific* behaviours switch
to **GPL-2.0 in the separate GPL crate** per `docs/PLAN.md:98-102` — the rest stays MIT.

### Gaps and conflicts (must be resolved before implementation)

1. **`qcom,pdc-ranges` numbering conflict.** The binding *example* uses
   `<0 512 94>, <94 641 15>, <115 662 7>` — `U:dts/upstream/Bindings/interrupt-controller/qcom,pdc.yaml:105`
   (identical at `L:Documentation/devicetree/bindings/interrupt-controller/qcom,pdc.yaml:105`) —
   i.e. **INTIDs**, while `sdm845.dtsi:5108` uses `480/609/630`, i.e. **DT `GIC_SPI` numbers**. The
   driver feeds the DT value straight into the GIC domain, which adds 32
   (`L:drivers/irqchip/irq-gic-v3.c:1607`), so the **dtsi numbers are authoritative** and the binding
   example is written in already-offset form. Redox must add 32.
1b. **`reg` size conflict.** The same binding example says `reg = <0xb220000 0x30000>`
   (`U:…/qcom,pdc.yaml:104`) but `sdm845.dtsi:5107` says `0x10000`. Both cover every register in
   §3.2 (highest offset touched for a full 256-pin block is `0x110 + 4*255 = 0x50c`), and Linux only
   clamps the window to `PDC_DRV_SIZE = 0x10000` — `L:…/qcom-pdc.c:35,669` — because the X1E quirk
   maps the *previous* DRV region at `res.start - PDC_DRV_SIZE` (`:710`), which sdm845 does not use.
   Map `0x10000`.
2. **PDC hardware version on sdm845 is unverified.** Nothing in the local sources states whether the
   sdm845 PDC reports v2.7 or v3.0. Linux selects the v2.7 layout for anything below 3.0.0
   (`L:…/qcom-pdc.c:685-697`); OpenBSD hardcodes the `0x10`/`0x110` bank layout and **never reads
   `PDC_VERSION_REG`** (`O:sys/dev/fdt/qcpdc.c:28-30`). Resolve with `devmem 0x0b221000` on the
   phone (§7) before committing to whether `IRQ_CFG` bits [3:4] exist.
3. **Firmware state unknown.** edk2-sdm845 has no MMIO reservation for `0x0b220000`/`0x0b2e0000`
   (`E:…/DeviceMemoryMap.h:309-316` lists only the display PDC blocks), and U-Boot has no PDC driver,
   so the reset state of the enable banks after ABL is not knowable from source. Mitigation: mask
   everything at init (§4.1) and dump the banks before touching them.
4. **OpenBSD's model is incomplete for wake semantics.** `qcpdc.c` writes only the 3-bit type field
   and never touches `GPIO_STATUS`/`GPIO_MASK`/`IRQ_ENABLE` (v3.0/v3.2 fields), and it has no
   equivalent of `GPIO_NO_WAKE_IRQ` / `irq_domain_disconnect_hierarchy`
   (`L:…/qcom-pdc.c:558-559`) — a GPIO with no PDC line in OpenBSD simply has no wake path, which is
   correct but implicit. It also has no phantom-pending clear.
5. **OpenBSD GIC parent not in this checkout.** `openbsd-src` is a sparse checkout; `git ls-files
   sys/ | grep -i gic` returns only `mvgicp.c`, so the parent-side SPI handling could only be cited
   from Linux (`L:drivers/irqchip/irq-gic-v3.c:1588-1613`) and from Redox's own GICv3
   (`R:…/gicv3.rs:133-155`).
6. **Pins 109–114 are unmapped.** `qcom,pdc-ranges` skips them (`L:…/sdm845.dtsi:5108`) and
   `sdm845_pdc_map[]` never references them, yet they are inside the 0–121 pin space. Any Redox code
   that assumes a contiguous 0..121 pin→SPI map will produce a wrong INTID for pins ≥ 109; the region
   lookup must fail closed (`L:…/qcom-pdc.c:533-542,594-596`).
7. **TLMM target-field naming conflict.** OpenBSD calls `INT_CFG[7:5] == 0x3<<5`
   `TARGET_PROC_RPM` (`O:sys/dev/fdt/qcgpio_fdt.c:39-40`); Linux calls the same value
   `intr_target_kpss_val = 3` and writes it to route to the application processor
   (`L:drivers/pinctrl/qcom/pinctrl-sdm845.c:17-54`, `L:drivers/pinctrl/qcom/pinctrl-msm.c:1118-1122`).
   Linux's reading is the one consistent with the wake design (PDC-wake pins must *not* be routed to
   KPSS). Verify with `devmem` on a known-good Linux boot (§7) rather than trusting either name.
8. **Redox irq space too narrow for these INTIDs** — see §6 blockers 1–3. This is a Redox-side gap,
   not a hardware one, and it gates any userspace wake daemon.
9. **`pdc_reset` (0x0b2e0000) must stay untouched** by the IRQ driver (§1.2); it belongs to the modem
   remoteproc consumer (`L:…/sdm845.dtsi:3429-3430`).
