# TLMM — pinctrl/GPIO and per-GPIO interrupt controller (`qcom,sdm845-pinctrl`)

Spec-only. No code in this file is copied from any reference; §8 records what was
consulted and under which licence. Sibling spec: `specs/pdc.md` (the PDC interrupt
controller that TLMM hands wake-capable GPIOs to).

**Block.** TLMM = Top-Level Mode Mapper. One hardware block that is *both* the
SDM845 pin controller (function select, direction, drive strength, pull) and the
interrupt generator for every general-purpose GPIO. It is the first of the two
Qualcomm interrupt controllers that sit in front of the GIC (K1 finding; see §5).

**DT node** (`L:arch/arm64/boot/dts/qcom/sdm845.dtsi:2801-2810`):

```dts
tlmm: pinctrl@3400000 {
    compatible = "qcom,sdm845-pinctrl";
    reg = <0 0x03400000 0 0xc00000>;              /* one window, three tiles */
    interrupts = <GIC_SPI 208 IRQ_TYPE_LEVEL_HIGH>; /* summary line -> INTID 240 */
    gpio-controller;  #gpio-cells = <2>;
    interrupt-controller; #interrupt-cells = <2>;   /* <pin, type> */
    gpio-ranges = <&tlmm 0 0 151>;
    wakeup-parent = <&pdc_intc>;
};
```

**Why this port needs it.** Three separate things go wrong without a TLMM story:

1. Every GPIO-routed interrupt on enchilada is *generated* here: touch INT on
   GPIO 125 (`L:sdm845-oneplus-common.dtsi:526`), hall sensor on GPIO 124
   (`:93`), NFC INT on GPIO 63 (`:496`), alert slider on GPIOs 126/52/24 (`:40,49,58`).
2. The pins the other specs depend on *are* TLMM pins: the debug console is
   GPIO 4 (TX) / GPIO 5 (RX) with `function = "qup9"` (`L:sdm845.dtsi:1863-1877`
   node, `:3240-3252` state), and I2C12 (touch bus) is GPIO 49/50 with
   `function = "qup12"` (`L:sdm845.dtsi:2994-2997`). `specs/geni-debug-uart.md` and
   `specs/geni-i2c.md` inherit their pin state from this block.
3. Spare GPIO *outputs* are used as chip resets and rail enables: touch reset
   GPIO 99, `ts_1p8_supply` enable GPIO 88 (`L:sdm845-oneplus-common.dtsi:202`),
   panel VCI/POC 26/25 (`:213,227`), NFC enable/firmware 12/62 (`:498-499`),
   display TE on GPIO 10 (`function = "mdp_vsync"`), display reset 6, codec reset
   64, speaker reset 69, and UFS reset on **GPIO 150 = the `UFS_RESET` special
   pin** (`L:sdm845-oneplus-common.dtsi:911`, `L:pinctrl-sdm845.c:249-257`).

**Reference revisions.** Shortened as `L:`/`U:`/`O:`/`R:` below; every hardware
claim must keep its `file:line` so a later reader can re-check it.

| Tag | Checkout @ commit | Files consulted |
| --- | --- | --- |
| `L:` | `~/Developer/forks/linux` @ `5b0764a` | `drivers/pinctrl/qcom/{pinctrl-msm.c,pinctrl-msm.h,pinctrl-sdm845.c}`, `arch/arm64/boot/dts/qcom/{sdm845.dtsi,sdm845-oneplus-common.dtsi,sdm845-oneplus-enchilada.dts}`, `include/dt-bindings/interrupt-controller/irq.h` |
| `U:` | `~/Developer/forks/u-boot` @ `8649a54d` | `drivers/pinctrl/qcom/{pinctrl-qcom.c,pinctrl-qcom.h,pinctrl-sdm845.c}`, `drivers/gpio/msm_gpio.c`, `arch/arm/mach-snapdragon/include/mach/gpio.h`, `dts/upstream/Bindings/pinctrl/{qcom,sdm845-pinctrl.yaml,qcom,tlmm-common.yaml}`, `configs/qcom_defconfig` |
| `O:` | `~/Developer/forks/openbsd-src` @ `20cb344` | `sys/dev/fdt/qcgpio_fdt.c` (v1.10) |
| `R:` | `~/Developer/forks/redox/kernel` @ `8186c379` (`op6/dt-acpi-off`), base @ `d2ffc4ca` | `patches/kernel-irqchips/0002-*`, `src/dtb/irqchip.rs`, `src/scheme/irq.rs`, `docs/workbench/K1-EVIDENCE.md` |

The Linux checkout is sparse/blobless and has **no `Documentation/`**, so the DT
binding is cited from the U-Boot mirror of the kernel bindings.

---

## 1. What the block is

* **Addressing.** One DT `reg` window of `0xc00000` (12 MiB) starting at
  `0x03400000`. Inside it the pins live in three *tiles* at fixed offsets —
  `EAST 0x100000`, `NORTH 0x500000`, `SOUTH 0x900000` — and each pin occupies a
  `0x1000` stride inside its tile (`L:pinctrl-sdm845.c:13-16` for the tile
  constants, `:17-54` for the `PINGROUP` macro that computes
  `ctl_reg = base + REG_SIZE * id`, `io_reg = +0x4`, `intr_cfg_reg = +0x8`,
  `intr_status_reg = +0xc`). U-Boot encodes the identical table
  (`U:drivers/pinctrl/qcom/pinctrl-sdm845.c:26-60`, e.g. GPIO 4/5/6 = NORTH,
  GPIO 12/24/25/26 = SOUTH, GPIO 69/88/124/125/126 = EAST) and the same
  arithmetic in `qcom_pin_offset(offs, sel) = sel * 0x1000 + offs[sel]`
  (`U:arch/arm/mach-snapdragon/include/mach/gpio.h:34`).
  Linux maps the whole window as **one** `ioremap` because `sdm845_pinctrl`
  sets no `.tiles`/`.ntiles` (`L:pinctrl-msm.c:1613-1622`,
  `L:pinctrl-msm.h:179-180`), so the tile offset is folded into the register
  offset: `addr = 0x03400000 + tile + 0x1000 * pin + reg`.
  `O:sys/dev/fdt/qcgpio_fdt.c:38,49` writes the stride as `0x1000 * pin` with
  **no tile term**, and `qcgpio_fdt_match()` only matches
  `qcom,sc7280-pinctrl` / `qcom,sc8280xp-tlmm` / `qcom,x1e80100-tlmm`
  (`O:qcgpio_fdt.c:112-115`) — it is an architecture reference for SDM845, not
  a porting donor (see §8 G-1).
* **Pin count, and why three numbers circulate.** The binding's pin-name pattern
  is `^gpio([0-9]|[1-9][0-9]|1[0-4][0-9])$` with `gpio-line-names` `maxItems: 150`
  (`U:dts/upstream/Bindings/pinctrl/qcom,sdm845-pinctrl.yaml:61`); U-Boot's data
  is `pin_count = 154` with `special_pins_start = 150`
  (`U:drivers/pinctrl/qcom/pinctrl-sdm845.c:91-93`); Linux lists 154 pins
  (`PINCTRL_PIN(149,"GPIO_149")`, `(150,"UFS_RESET")`, `(151,"SDC2_CLK")`,
  `(152,"SDC2_CMD")`, `(153,"SDC2_DATA")`, `L:pinctrl-sdm845.c:249-257`) and sets
  `.ngpios = 151` (`L:pinctrl-sdm845.c:1299-1302`), while the ACPI variant uses
  `.ngpios = 150` plus `reserved_gpios` (`:1310-1311`).
  **Use this instead of arguing about the number:** pins 0..149 are ordinary
  GPIO-capable pins with a full four-register set; 150..153 are *special* pins
  built by the `SDC_QDSD_PINGROUP`/`UFS_RESET` macros, which set every
  `intr_*`, `mux_*`, `oe`, `in`, `out` bit index to `-1`
  (`L:pinctrl-sdm845.c:56-101`) — they have no interrupt configuration and no
  function-select field. GPIO 150/`UFS_RESET` is usable as a *data* pin (that is
  what `reset-gpios = <&tlmm 150 GPIO_ACTIVE_LOW>` on `&ufs_mem_hc` does) but
  must never be indexed as an interrupt source.
* **TLMM as interrupt controller.** `#interrupt-cells = <2>`: the first cell is
  the TLMM pin number, the second is the trigger type from
  `include/dt-bindings/interrupt-controller/irq.h` — `NONE 0`, `EDGE_RISING 1`,
  `EDGE_FALLING 2`, `EDGE_BOTH 3`, `LEVEL_HIGH 4`, `LEVEL_LOW 8`
  (`L:include/dt-bindings/interrupt-controller/irq.h:13-18`). The binding
  requires `interrupts`, `interrupt-controller`, `#interrupt-cells`,
  `gpio-controller`, `#gpio-cells`, `gpio-ranges`
  (`U:dts/upstream/Bindings/pinctrl/qcom,tlmm-common.yaml:17-25,39,64-69`).
  On this phone `interrupts-extended` names TLMM four times: `<&tlmm 125
  IRQ_TYPE_EDGE_FALLING>` (touch, `L:sdm845-oneplus-common.dtsi:526`),
  `<&tlmm 63 IRQ_TYPE_EDGE_RISING>` (NFC, `:496`), `<&tlmm 124
  GPIO_ACTIVE_LOW>` (hall, `:93`, `wakeup-source` at `:98` — read as a polled
  GPIO, not an IRQ) and the 124/125 pair inside the `ts_default_pins`/`hall`
  pinctrl states.
* **Pins this phone actually uses** (all from `L:sdm845-oneplus-common.dtsi`):
  GPIO 4/5 uart9 (`qup9`, `:729-737` overrides: RX 2 mA + bias-pull-up, TX 2 mA +
  bias-disable); GPIO 10 display TE (`mdp_vsync`); GPIO 24/52/126 alert slider
  (`alert_slider_default`, 2 mA + bias-disable, `:985-990`); GPIO 25/26 panel
  POC/VCI; GPIO 12/62 NFC enable + firmware, GPIO 63 NFC INT (NFC states
  `nfc_default`/`nfc_sleep` at `:1053-1075`, pull-up in `default`, disable in
  `sleep`); GPIO 64 codec reset; GPIO 69 speaker reset
  (`L:sdm845-oneplus-enchilada.dts:55-64`); GPIO 88 `ts_1p8_supply`; GPIO 99 +
  125 `ts_default_pins` (16 mA + bias-pull-up, `:1038-1043`).
* **Never touch.** `&tlmm { gpio-reserved-ranges = <0 4>, <81 4>; }`
  (`L:sdm845-oneplus-common.dtsi:961-962`) — the same eight pins (0..3, 81..84)
  are the ACPI `reserved_gpios` list (`L:pinctrl-sdm845.c:1270`), i.e. pins
  owned by another processor. `PLAN.md` A3 and A8/A9 repeat the rule: no writes
  to `no-map` regions, `protected-clocks`, or `gpio-reserved-ranges`.

## 2. What firmware leaves configured

* The stock boot chain is ABL/LE; **no local source**, so nothing here can be
  asserted about what it leaves in TLMM. Treat "firmware leaves the console
  pins in `qup9` and never arms a GPIO interrupt" as an *assumption to probe*
  (§7.3), not as a fact.
* Mainline U-Boot does have SDM845 TLMM code, and A2/K5's "U-Boot first" route
  means it may run before Redox: `U_BOOT_DRIVER(pinctrl_sdm845)` matches
  `"qcom,sdm845-pinctrl"` (`U:drivers/pinctrl/qcom/pinctrl-sdm845.c:101-112`) and
  is compiled by `configs/qcom_defconfig` (`PINCTRL=y`, `PINCTRL_QCOM_GENERIC=y`
  at `:119-121`, which defaults `PINCTRL_QCOM_SDM845` to `y`,
  `U:drivers/pinctrl/qcom/Kconfig:127-131`; `MSM_GPIO` and `QCOM_PMIC_GPIO` at
  `:85-86`). Its ops implement `set_state`, `pinmux_set` and `pinconf_set`
  (`U:drivers/pinctrl/qcom/pinctrl-qcom.c:235-245`) — function select,
  drive strength (`argument = (argument / 2) - 1`, `:175-180`) and bias
  (`:181-183`) — so it *can* rewrite pin state if a driver asks for a function.
* **Nothing in the boot chain arms GPIO interrupts.** `grep -i 'intr\|interrupt'`
  over `U:drivers/pinctrl/qcom/*.{c,h}` and `U:drivers/gpio/msm_gpio.c` finds no
  INT_CFG/INT_STATUS handling (the only `intr` hits are a QCS615 *function*
  name, `emac_phy_intr`). So INT_CFG/INT_STATUS on the touch/hall/NFC GPIOs is
  either zero or leftover state from the previously running kernel; either way
  Redox must write it itself, and must not assume a clean zero.
* Console rule: **verify, never blanket-re-init.** GPIO 4/5 leave the boot chain
  as `qup9` (mux value 1 in their own list) because that is what the console
  needs (`L:sdm845.dtsi:3240-3252`, `L:sdm845-oneplus-common.dtsi:729-737`).
  Any "reset all pins to GPIO mode" loop kills the early console, and is also
  wrong on principle: Linux's `msm_pinmux_set_mux` masks the interrupt on a pin
  it muxes *away* from GPIO and, on the first mux *back* to GPIO with an output
  direction, reads IN_OUT and pre-loads OUT to avoid a glitch
  (`L:pinctrl-msm.c:195-232`). Redox has no pinctrl subsystem, so the caller
  (uart/i2c/touch driver) either trusts the firmware-left state or re-applies a
  state it verified.

## 3. Register map

Per pin, `base = tile + 0x1000 * pin` inside the `0x03400000 + 0xc00000` window.

| Off | Name | Fields | Evidence |
| --- | --- | --- | --- |
| `+0x00` | `GPIO_CTRL` | `[1:0]` pull: 0 none, 1 down, 2 keeper/up-no-keeper, 3 up (`pull_bit = 0`, `MSM_NO_PULL/MSM_PULL_DOWN/MSM_KEEPER/MSM_PULL_UP`, `L:pinctrl-msm.c:326-330`); `[5:2]` function select, 4 bits because `nfuncs = 11` — `GENMASK(mux_bit + order_base_2(nfuncs) - 1, mux_bit)` with `mux_bit = 2` (`L:pinctrl-msm.c:185`, `L:pinctrl-sdm845.c:44`); `[8:6]` drive strength, mA = `(regval + 1) * 2` → 2…16 mA (`drv_bit = 6`, `msm_regval_to_drive`, `L:pinctrl-msm.c:333-336`); bit 9 output-enable (`oe_bit = 9`, `L:pinctrl-sdm845.c:41`; `U:drivers/pinctrl/qcom/pinctrl-qcom.c` `TLMM_GPIO_OE_MASK BIT(9)`; `O:qcgpio_fdt.c:34` `TLMM_GPIO_CFG_OUT_EN (1 << 9)`) | macro `L:pinctrl-sdm845.c:17-54` |
| `+0x04` | `GPIO_IN_OUT` | bit 0 = pin input value (`in_bit = 0`), bit 1 = output latch (`out_bit = 1`) | `L:pinctrl-sdm845.c:42-43`, `L:pinctrl-msm.c:617-627`, `O:qcgpio_fdt.c:228-243` |
| `+0x08` | `GPIO_INT_CFG` | bit 0 `INT_ENABLE`; bit 1 `POL`; `[3:2]` `DECT_CTL` = 0 level, 1 edge-positive, 2 edge-negative, 3 both; bit 4 `RAW_STATUS_EN`; `[7:5]` interrupt target, value **3** | `O:qcgpio_fdt.c:39-48`, `L:pinctrl-sdm845.c:44-53` (`intr_target_bit = 5`, `intr_target_kpss_val = 3`, `intr_raw_status_bit = 4`, `intr_polarity_bit = 1`, `intr_detection_bit = 2`, `intr_detection_width = 2`), `L:pinctrl-msm.c:1100-1122` |
| `+0x0c` | `GPIO_INT_STATUS` | bit 0 = latched interrupt for this pin; **clear by writing the bit low** (OpenBSD writes `stat & ~BIT(0)`, Linux writes 0 when `intr_ack_high` is false — which is the case for SDM845) | `O:qcgpio_fdt.c:49-50,404-410`, `L:pinctrl-msm.c:118-123` |

Notes that change code:

* `intr_target_reg` is **not** set for SDM845, so the target field is read and
  written *through `GPIO_INT_CFG`*: `msm_readl_intr_target`/`msm_writel_intr_target`
  fall back to `intr_cfg_reg` when `intr_target_reg` is unset
  (`L:pinctrl-msm.c:102-116`). Do not go looking for a fifth register.
* The function-select **value is positional**, not global: it is the index of
  the function inside that pin's own `funcs` list (`L:pinctrl-msm.c:184-194`,
  `:236-240`). `gpio` is always index 0 (`msm_mux_gpio` first in `PINGROUP`,
  `L:pinctrl-sdm845.c:17-30`); `qup9` is index 1 *only* on GPIO 4/5/6
  (`L:pinctrl-sdm845.c:504`, `:1117-1119`). A "function gpio = 0, peripheral = 1"
  shortcut is a bug.
* Worked addresses (all inside the one window): GPIO 125 INT_CFG =
  `0x03400000 + 0x100000 + 125*0x1000 + 8` = **`0x0357d008`**, INT_STATUS =
  `0x0357d00c`; GPIO 4 (NORTH) CTL = `0x03904000`; GPIO 63 (NORTH) INT_CFG =
  `0x0393f008`; GPIO 88 (EAST) CTL = `0x03558000`. `0x03500000` by itself is the
  **EAST tile base**, i.e. pin 0's `GPIO_CTRL` — the `devmem 0x03500000` recipe
  in `specs/pdc.md` §7 is a tile probe, not GPIO 125's INT_CFG.
* The three TLMM consumers whose state matters on this phone, in one line each:
  GPIO 125 (touch INT, EAST, `EDGE_FALLING`, PDC pin 100 → INTID 647),
  GPIO 124 (hall, EAST, PDC pin 99 → INTID 646), GPIO 63 (NFC INT, NORTH,
  `EDGE_RISING`, PDC pin 64 → INTID 576). `sdm845_pdc_map` has 78 entries and
  includes `{24,37}, {52,54}, {63,64}, {88,77}, {124,99}, {125,100}, {127,102}`
  — **no entry for 126**, which is why the alert-slider pin 126 has no wake
  route (`L:pinctrl-sdm845.c:1274-1290`).

## 4. Sequences

### 4.1 Arm a GPIO as an interrupt input (the RMI4 case: GPIO 125, edge-falling)

1. Confirm the pin is in GPIO mode: `GPIO_CTRL[5:2] == 0`. The touch driver's
   pinctrl state asks for exactly that (`L:sdm845-oneplus-common.dtsi:1038-1043`
   sets `function = "gpio"` on GPIO 99 and 125, 16 mA, bias-pull-up).
2. Release the output driver: clear `GPIO_CTRL` bit 9 (that is all
   `msm_gpio_direction_input` does, `L:pinctrl-msm.c:556-574`). The input buffer
   is not gated by any bit — `get_direction`/`gpio_get` read the IN_OUT bits.
3. If bias must be applied, write `GPIO_CTRL[1:0] = 3` (pull-up) together with
   `[8:6] = 7` (16 mA) — one read-modify-write of CTL is enough; the pull and
   drive fields are non-overlapping but adjacent (bits 1:0 and 8:6).
4. Decide the **parent** first. GPIO 125 is wake-capable (PDC pin 100), so with
   `wakeup-parent = <&pdc_intc>` present Linux deliberately leaves TLMM alone:
   `msm_gpio_irq_mask/unmask/ack/set_type` all early-return when the pin is in
   `skip_wake_irqs` (`L:pinctrl-msm.c:846-847,896-897,1075-1082`, populated in
   `msm_gpio_init` from `wakeup-parent` + `wakeirq_map`). Arm **either** the PDC
   (see `specs/pdc.md` §4.2 step 6 and §5) **or** the TLMM summary line — never
   both, or the event double-latches and the TLMM chain re-raises the level
   interrupt that was just cleared.
5. Otherwise program `GPIO_INT_CFG`: clear `[3:1]`, write the detection code and
   the polarity bit, keep the target field at 3, set `RAW_STATUS_EN` (bit 4),
   then set `INT_ENABLE` (bit 0). Linux reads CTL, sets RAW_STATUS_EN, the
   detection and polarity bits, and the target field in one go, then clears the
   pending status if anything changed (`L:pinctrl-msm.c:1124-1186`); OpenBSD
   does the same in one write (`O:qcgpio_fdt.c:303-334`) and Linux's comment
   explains why RAW_STATUS_EN matters: "toggling the RAW_STATUS could cause the
   INTR_STATUS to be set for EDGE interrupts" (`L:pinctrl-msm.c:1124-1129`).
6. Clear any pending status before unmasking (§4.3) and dispatch from the
   parent's handler (§5).

`msm_gpio_irq_reqres` (`L:pinctrl-msm.c:1219`) is the oracle for the whole
sequence: request the pin, mux it to GPIO, `gpiochip_lock_as_irq()`, and mark
the line `IRQ_DISABLE_UNLAZY`.

### 4.2 Mask / unmask

Linux's `mask` (`:835`) calls `irq_chip_mask_parent()` when the line has a
parent, then clears `INT_ENABLE`; for level types it also clears
`RAW_STATUS_EN`. `unmask` (`:885`) re-asserts `RAW_STATUS_EN` + `INT_ENABLE`.
Both skip wake-routed pins entirely.

### 4.3 Clear a pending interrupt

Write the status register so bit 0 goes **low** — `O:qcgpio_fdt.c:409-410`
writes `stat & ~BIT(0)`; Linux's `msm_ack_intr_status` writes
`BIT(intr_status_bit)` only when `intr_ack_high` is true and **0** otherwise,
and `pinctrl-sdm845.c` never sets `intr_ack_high` (`L:pinctrl-msm.c:118-123`).
See §8 C-2: the Redox K1 draft writes `1` here instead, which sets/keeps the
bit rather than clearing it.

### 4.4 Re-mux a pin away while its IRQ is armed

Mask the IRQ *before* changing `GPIO_CTRL[5:2]` and re-enable after; Linux
tracks this in `disabled_for_mux` and calls `disable_irq_nosync()` because the
input buffer keeps seeing the pin toggle even when muxed away
(`L:pinctrl-msm.c:195-207`).

### 4.5 Sleep/resume (for the record)

OpenBSD saves each pin's `INT_CFG` on suspend, disables non-wake pins, and
restores the array on resume (`O:qcgpio_fdt.c:185-210`); Linux sets
`IRQCHIP_MASK_ON_SUSPEND | IRQCHIP_SET_TYPE_MASKED |
IRQCHIP_ENABLE_WAKEUP_ON_SUSPEND` on the TLMM irq chip
(`L:pinctrl-msm.c:1444-1447`). Redox has no suspend/resume in this wave; note
only that per-pin interrupt state is *not* guaranteed to survive deep sleep, so
a resume path must re-arm, not assume.

## 5. Interrupts, DMA, power

* **One summary line.** `interrupts = <GIC_SPI 208 IRQ_TYPE_LEVEL_HIGH>` →
  INTID `208 + 32 = 240` (`L:sdm845.dtsi:2804`; the +32 math and the pass-through
  behaviour are in `specs/pdc.md` §4.2/§5). Level-high, so the dispatch loop
  must clear per-pin `INT_STATUS` before EOI or the line re-asserts.
* **Per-pin sources are not GIC sources.** Linux chains them: `msm_gpio_init`
  sets `girq->num_parents = 1`, `parents[0] = pctrl->irq` (the summary line),
  `handler = handle_bad_irq` and `default_type = IRQ_TYPE_NONE` for the
  children, and `child_to_parent_hwirq = msm_gpio_wakeirq`
  (`L:pinctrl-msm.c:1452-1532`, `:1396-1418`, `:1428-1450`). The summary handler
  walks the *enabled* pin bitmap, reads each pin's `INT_STATUS`, and dispatches
  with `generic_handle_domain_irq()`; if no bit is set it calls
  `handle_bad_irq()` (`L:pinctrl-msm.c:1364-1394`). OpenBSD's equivalent walks
  every pin with a handler and skips non-wake pins while suspended
  (`O:qcgpio_fdt.c:384-414`).
* **Wake routing.** `wakeup-parent = <&pdc_intc>` (`L:sdm845.dtsi:2810`) — for
  wake-capable GPIOs the event must be armed in the **PDC** instead of TLMM:
  GPIO 125 → PDC pin 100 → PDC SPI 615 → GIC INTID 647; GPIO 124 → 99 → INTID
  646; GPIO 63 → 64 → INTID 576 (`sdm845_pdc_map` + `specs/pdc.md` §5).
  `wakeirq_dual_edge_errata = true` on SDM845 (`L:pinctrl-sdm845.c:1299-1302`):
  the PDC cannot express both-edge, which is why Linux has the
  `dual_edge_irqs`/`msm_gpio_update_dual_edge_pos` workaround
  (`L:pinctrl-msm.c:1065-1070,1189-1190`). Not needed for the three IRQ lines
  on this phone (all single-edge), but it is why "both edges" is a dirty word
  on Qualcomm wake paths.
* **DMA.** None. TLMM is not a bus master and the DT node declares no
  `dmas`/`iommus`. The SMMU question (PLAN A11) belongs to the consumers.
* **Power/clocks.** The node declares no `clocks`, `power-domains` or `resets`
  (`L:sdm845.dtsi:2801-2810`) — unlike the GENI SEs, which sit in
  `SDM845_CX` (`L:sdm845.dtsi:1871`). What *is* board-dependent is the pad
  supply (`vdd_io`) for each tile; the touch INT pair is built for 16 mA with a
  pull-up (`L:sdm845-oneplus-common.dtsi:1038-1043`), which is a hint that
  GPIO 99/125 are not a 1.8 V weak-drive domain. Gap G-3 in §8.

## 6. Redox interface

**What K1 (`patches/kernel-irqchips`, branch `op6/dt-acpi-off`) already
implements** — read this before writing anything:

* `src/arch/aarch64/device/irqchip/tlmm.rs` (162 lines): matches
  `"qcom,sdm845-pinctrl"`; `GPIO_COUNT = 150`; `Tlmm { base, virq_base,
  enabled: [bool; 150], probe_only }`; `irq_init` demands a `reg` length of at
  least `0xc00000` and a `child_eois_root` chain; `probe_only` is driven by a
  `redox,irqchip-probe-only` DT property (host-test aid, not hardware truth).
* `qcom_irq.rs`: `TILE_RUNS` — the same 19 tile runs U-Boot encodes, e.g.
  `(0,3,0x100000) … (122,126,0x100000) (127,149,0x500000)`;
  `tlmm_pin_offset(pin, register) = tile + pin * 0x1000 + register` (returns
  `None` for pins with no tile, including pin 150); `TLMM_WAKE_PINS` mirrors
  `sdm845_pdc_map` (`tlmm_wake_pin(126) → None`);
  `tlmm_type_bits(flags)`: `1 → 0b01 << 2`, `2 → 0b10 << 2`, `3 → 0b11 << 2`,
  `4 → 1 << 1`, `8 → 0`, anything else `None`.
* Interrupt handling: `irq_handler` iterates pins, skips `!enabled[pin]`, reads
  `INT_STATUS` at `+0xc` and, if bit 0 is set, writes `1` to it and calls
  `irq_trigger(virq_base + pin)`; `irq_ack() → 0`; `irq_eoi(pin)` writes `1`;
  `irq_enable` writes `read | (3 << 5) | (1 << 4) | 1`; `irq_disable` clears bit
  0; `irq_configure(IrqCell::L2(pin, flags))` clears `0b111 << 1` and ORs
  `tlmm_type_bits(flags)`; `irq_xlate`/`irq_to_virq` map a pin to
  `virq_base + pin`; `irq_wakeup_spec(L2(gpio, flags)) → IrqCell::L2(pdc_pin,
  flags)`; `irq_handler_eois_root() → true`.
* Framework widenings in the same patch: `irq_desc: [IrqDesc; 2048]`,
  `COUNTS: [usize; IRQ_QUEUE_LEN]` (2048 on aarch64/riscv64),
  `TOTAL_IRQ_COUNT: u16 = 2048`, `irq_trigger(irq: u16)`, `Handle::Irq { irq:
  u16 }`, `IrqReservation { index: u16 }`; `dtb/irqchip.rs` gains `IrqCell`
  L1/L2/L3, `IrqDescItem { child_ic_idx, route: Option<(usize, u32)> }`,
  `child_eois_root`, `irq_wakeup_spec`, wake-parent routing in `irq_xlate`/
  `irq_to_virq`; `arch/aarch64/interrupt/irq.rs` accepts `virq < 2048` and
  honours `child_eois_root` → `irqchip::end_root(raw_iar)`.

**Redox-side blockers that are not TLMM's fault but gate it:**

* IRQ numbers are `u8` on main: `COUNTS: [usize; 224]`, `BASE_IRQ_COUNT = 16`,
  `TOTAL_IRQ_COUNT = 224`, `irq_trigger(irq: u8, …)`, `Handle::Phandle(u8,
  Vec<u8>)` (`R:src/scheme/irq.rs`); `is_reserved`/`set_reserved` still index
  `irq_desc` with a `u8` and `available_irqs_iter` is a stub returning `0..0`
  (`R:src/dtb/irqchip.rs:500,505,516`). INTID 240 (direct TLMM summary) and 647
  (touch through the PDC) are both > 255, so **today they are unreachable** —
  track under K6 (`op6/irq-width`) and K8.
* `register_irq(virq)`/`irq_desc` accept only `< 1024` on main (2048 in K1); a
  TLMM domain alone wants 150 descriptors.
* There is **no pinctrl or GPIO subsystem in the Redox kernel** — `grep` over
  `src/` finds no pinctrl/pinmux/gpio code at all. Consequences: no
  `/scheme/pinmap` equivalent, nothing parses `pinctrl-0`/`pinctrl-1`, and
  `gpio-ranges`/`gpio-reserved-ranges` have no consumer. `interrupts-extended`
  itself needs `patches/kernel/0001`+`0002` (see `docs/GOTCHAS.md`).
* Recommended shape (spec, not code): one register-level TLMM module, used by
  the kernel irqchip path and, if a daemon ever needs pin state, by that daemon
  through `common::physmap`/`PhysBorrowed::map(base, len, Prot, MemoryType::
  Uncacheable)` → `/scheme/memory/physical@uc` (`R:base/drivers/common/src/
  lib.rs:163-190`, `drivers/README.md:105-114`), with `crates/mmio-replay`'s
  `Mmio`/`MemModel`/`Recorder`/`Replayer` (`Compare::Strict`) as the seam for
  host tests. There is **no `exampled`** in the base checkout (K7 finding), so
  start from `PhysBorrowed` and `inputd::ProducerHandle`, not from a non-existent
  donor. Userspace IRQ delivery, if ever needed, follows the in-tree pattern:
  open `/scheme/irq/<virq>` then `EventQueue::subscribe(fd, …, EventFlags::READ)`
  (`R:base/drivers/audio/sb16d/src/main.rs:40-54`).
* For the RMI4 path specifically: the touch INT must be armed *somewhere*
  (PDC or TLMM) before `rmi4d` can poll or subscribe, and the RMI4 spec already
  says "no reset, rail, or pin writes belong in the initial read-only driver"
  (`specs/rmi4-touch.md`) — which is only true if the pin/IRQ state is already
  correct. That is the dependency this spec exists to settle.

## 7. Verification

1. **Host tests (do these first; no hardware needed).** Express TLMM as a
   `MemModel` behind `crates/mmio-replay`'s `Mmio` trait — a sparse window whose
   four live registers per pin sit at `tile + pin*0x1000 + {0,4,8,c}`, with the
   tile table from §1 copied from the *reference*, not guessed. Minimum set:
   * tile-addressing test: assert `tlmm_pin_offset(125, 8) == 0x17d008` and that
     a model built as `pin * 0x1000` (no tile term, the OpenBSD macro shape)
     reads a **different** address — the failure mode, made explicit;
   * type-decode test for all five `IRQ_TYPE_*` values, including the polarity
     bit for edge types (see §8 C-1);
   * negative tests that must be *refusals*, not vacuous passes: `function !=
     "gpio"` (e.g. GPIO 10 `mdp_vsync`) must not be re-muxed by a GPIO-IRQ
     helper, and `drive-strength = <24>` must be rejected as out-of-range for
     the 2…16 mA encoding;
   * pin-domain test: pin 150 (UFS_RESET) and pins 0..3/81..84 must be absent
     from the interrupt source list (special-pin and reserved-range rules).
   Replay a captured trace in strict mode once real captures exist.
2. **QEMU.** SDM845 TLMM is not modelled and QEMU has no SDM845 machine, so
   QEMU can only prove the *DT-parsing* half: extend the existing
   `docs/workbench/k1-irqchips.dtso` style fixture (`tlmm_test:
   pinctrl@3400000` with `redox,irqchip-probe-only`, `redox,probe-interrupt =
   <125 2>`, `wakeup-parent = <&pdc_test>`) and check the probe log still reports
   `GPIOs=150 virqs=288..438` and `IRQ probe route: … pin=125 flags=2`.
3. **Linux-oracle capture (deferred; no phone in this wave).** On the pmOS
   phone, before Redox runs: `devmem` `GPIO_CTRL`/`GPIO_INT_CFG`/`GPIO_INT_STATUS`
   for GPIOs 4, 5, 63, 124, 125 (addresses in §3), plus
   `cat /sys/kernel/debug/pinctrl/*/pinmux-pins | grep -i '125\|124'`,
   `cat /sys/kernel/debug/gpio`, and `grep -iE 'tlmm|pdc' /proc/interrupts`.
   Record the values verbatim into `docs/workbench/` and re-run the host tests
   against that capture: this answers both "what does firmware leave configured"
   (§2) and the §8 C-1/C-2 conflicts. The existing recipe in `specs/pdc.md` §7
   (`devmem $((0x03500000 + 0x1000*125 + 8))` → expect INT_ENABLE **cleared**)
   already exercises GPIO 125's INT_CFG and stays authoritative for the PDC side.

## 8. Licence path, gaps, conflicts

**Licence.** Redox code stays **MIT, clean-room**. There is no permissive donor
that covers SDM845 TLMM: `O:sys/dev/fdt/qcgpio_fdt.c` is ISC (`O:qcgpio_fdt.c:1-3`
"ISC"-style BSD permission header, v1.10) but matches no SDM845 part and has no
tile support; `U:drivers/pinctrl/qcom/*` and `U:arch/arm/mach-snapdragon/include/
mach/gpio.h` are `GPL-2.0+` and implement no interrupt path; `L:pinctrl-msm.c`
and `L:pinctrl-msm.h` are `GPL-2.0-only`, `L:pinctrl-sdm845.c` is `GPL-2.0`; the
binding is dual `GPL-2.0-only OR BSD-2-Clause`
(`U:dts/upstream/Bindings/pinctrl/qcom,sdm845-pinctrl.yaml:1-4`). Read those as
oracles only. If a clean-room implementation stalls, a separate GPL-2.0 crate is
the fallback (PLAN A8 rule) — decide *before* copying, per `docs/README.md`'s
"ask before using a Linux driver as a reference" guidance.

**Conflicts and gaps to resolve, in priority order.**

* **C-1 — edge polarity.** Both references OR the polarity bit for edge types
  (`O:qcgpio_fdt.c:303-316`: `EDGE_POS | POL_CTL`, `EDGE_NEG | POL_CTL`,
  `EDGE_BOTH`; `L:pinctrl-msm.c:1129-1148` with `intr_detection_width == 2` sets
  `2 << 2` *and* `BIT(intr_polarity_bit)` for EDGE_FALLING). K1's
  `tlmm_type_bits(2)` writes `0b10 << 2` with **no** polarity bit. One of the
  two reference behaviours must be proven — this is exactly the K1-EVIDENCE
  gate "confirm MMIO mapping attrs and TLMM tile/target behaviour against the
  Linux oracle". Do not "fix" K1 or copy the references without the probe.
* **C-2 — how to clear `INT_STATUS`.** OpenBSD clears bit 0; Linux writes 0 for
  SDM845 (`intr_ack_high` false); K1's `irq_handler`/`irq_eoi` write `1`. If the
  bit is "high = pending, write 0 to clear", writing 1 either does nothing or
  re-latches, which on a level-high summary line looks like an interrupt storm.
* **C-3 — interrupt target field.** `[7:5] == 3`: OpenBSD calls the field
  `TARGET_PROC_RPM`, Linux calls the same constant `intr_target_kpss_val` and
  comments "Route interrupts to application cpu". Same value, contradictory
  names: do not rename or reinterpret the field in Redox code without evidence.
* **G-1** — OpenBSD's `qcgpio_fdt.c` neither matches `qcom,sdm845-pinctrl` nor
  models tiles, and hardcodes 175/230/239 pins per SoC
  (`O:qcgpio_fdt.c:129-133`); "the ISC donor" is *shape* only.
* **G-2** — pin-domain sizes disagree (150 names / 151 GPIO numbers / 154
  pins / 151 in `gpio-ranges`); K1 uses `GPIO_COUNT = 150`, which happens to
  match the interrupt-capable pins but must be stated as such, and `TILE_RUNS`
  stops at 149 (so pins 150..153 have no `INT_CFG`/`INT_STATUS` at all).
* **G-3** — the `vdd_io` rail for each tile is not encoded in the DTS
  (geni-debug-uart gap G-4 says the same for GPIO 4/5); needs a schematic or a
  boardview before any drive-strength default is invented in code.
* **G-4** — no `Documentation/` in the local Linux checkout, so binding
  citations go through `U:dts/upstream/Bindings/…`; `include/linux/irq.h` is
  absent too (the `IRQ_TYPE_*` values came from the dt-bindings header).
* **G-5** — whether ABL/U-Boot leaves `INT_CFG` armed is unverified (§2); the
  §7.3 probe settles it. Until then treat "boot chain leaves interrupts unarmed"
  as an assumption, because the "don't re-init firmware state" rule (VISION §8,
  PLAN A6) is meaningless if there is no firmware state to keep.
* **G-6** — the Redox `u8` IRQ/virq width and the `available_irqs_iter` stub
  make both candidate routes for the touch INT unreachable today (§6).
