# GENI Serial Engine — Debug UART (uart9)

**Block**: Qualcomm GENI SE (Serial Engine) UART, used as the *debug/console* UART on the
OnePlus 6 (codename *enchilada*, SoC SDM845 / Snapdragon 845).
**DT `compatible`**: `qcom,geni-debug-uart` (distinct from `qcom,geni-uart`, which is the
BT/GPS-style non-console GENI UART).
**Role on this phone**: mainline treats uart9 as the wired console — `serial0 = &uart9`
and `stdout-path = "serial0:115200n8"`.

Reference sources and revisions (line citations below are against these checkouts):

| Repo | Commit | Files used |
|---|---|---|
| `~/Developer/forks/linux` | `5b0764a` | `arch/arm64/boot/dts/qcom/sdm845.dtsi`, `sdm845-oneplus-common.dtsi`, `sdm845-oneplus-enchilada.dts`, `drivers/tty/serial/qcom_geni_serial.c`, `drivers/soc/qcom/qcom-geni-se.c`, `drivers/clk/qcom/gcc-sdm845.c` |
| `~/Developer/forks/u-boot` | `8649a54d` | `drivers/serial/serial_msm_geni.c`, `include/soc/qcom/geni-se.h` |
| `~/Developer/forks/openbsd-src` | `20cb344` | `sys/dev/ic/qcuart.c`, `sys/dev/ic/qcuartvar.h`, `sys/dev/fdt/qcuart_fdt.c` |
| `~/Developer/forks/redox/kernel` | `a0a8ff96` | `src/dtb/serial.rs`, `src/dtb/mod.rs`, `src/dtb/irqchip.rs`, `src/devices/serial.rs`, `src/arch/aarch64/device/serial.rs`, `src/arch/aarch64/debug.rs`, `src/scheme/debug.rs`, `src/scheme/irq.rs` |
| `~/Developer/forks/pmaports` | `e6ff10d` | `device/community/device-oneplus-enchilada/*` |

> Note: the local `linux` checkout is a sparse/blobless tree — `include/linux/soc/qcom/geni-se.h`
> (mainline) is *not* present, so common SE register offsets are cited from the U-Boot header and
> the OpenBSD driver instead.

---

## 1. Block

- **DT node**: `serial@a84000`, label `uart9`, child of the QUP wrapper
  `qupv3_id_1: geniqup@ac0000` (`compatible = "qcom,geni-se-qup"`).
  Node: `linux/arch/arm64/boot/dts/qcom/sdm845.dtsi:1863-1877`; wrapper node: `sdm845.dtsi:1751-1762`.
- **compatible**: `qcom,geni-debug-uart` (`sdm845.dtsi:1864`). Linux matches this string in
  `qcom_geni_serial.c:2191-2204` (console-class device data: `qcom_geni_console_data`, FIFO transfer
  mode, `qcom_geni_serial.c:2140-2146`).
- **reg / address**: `reg = <0 0x00a84000 0 0x4000>` (`sdm845.dtsi:1865`). The parent wrapper node
  has an empty `ranges;` (`sdm845.dtsi:1760`), i.e. identity address translation, so the physical
  base of the SE register window equals `0x00a84000` (16 KiB). OpenBSD maps only `0x1000`
  (`openbsd-src sys/dev/ic/qcuart.c:58` `GENI_SPACE`, :560 `bus_space_map`) — enough for FIFO/IRQ
  registers, not for SE_HW_PARAM_* at `0xe24` (see §3, §7).
- **Interrupts**: `interrupts = <GIC_SPI 354 IRQ_TYPE_LEVEL_HIGH>` (`sdm845.dtsi:1870`).
  IRQ parent is the SoC `intc` = `arm,gic-v3` at `0x17a00000` (`sdm845.dtsi:34`, :5463-5464).
  Level-high SPI, wired through the GIC distributor — no PDC/SPI-woffset indirection for this node.
- **Clocks**:
  - SE clock: `clock-names = "se"`, `clocks = <&gcc GCC_QUPV3_WRAP1_S1_CLK>` (`sdm845.dtsi:1866-1867`).
  - That clock is a branch of the RCG `gcc_qupv3_wrap1_s1_clk` (`halt_reg 0x18144`, `enable_reg
    0x5200c` bit 23, `CLK_SET_RATE_PARENT`): `linux/drivers/clk/qcom/gcc-sdm845.c:2314-2330`.
  - Parent RCG `gcc_qupv3_wrap1_s1_clk_src`: `cmd_rcgr = 0x18148`, `mnd_width = 16`, `hid_width = 5`,
    freq table shared with wrap0 (`ftbl_gcc_qupv3_wrap0_s0_clk_src`: 7.3728/14.7456/19.2/29.4912/32/48
    MHz incl. 19.2 MHz `P_BI_TCXO` and 48 MHz from GPLL0): `gcc-sdm845.c:592-599`, :429-435.
  - Wrapper AHB clocks: `m-ahb`/`s-ahb` = `GCC_QUPV3_WRAP_1_M_AHB_CLK`/`_S_AHB_CLK` (`sdm845.dtsi:1754-1756`).
- **Resets**: none declared for this node (no `resets` property in `sdm845.dtsi:1863-1877`).
  SE-level "reset" is done in software via `GENI_FORCE_DEFAULT_REG` (§3, §4).
- **Supplies**: no `supply`-type GPIO/regulator phandles on the node. Power rail relation:
  `power-domains = <&rpmhpd SDM845_CX>` (`sdm845.dtsi:1871`) — the SE lives in the CX
  rail/GDSC power domain managed via RPMh (see §5). TLMM pad supply (`vdd_io`) for GPIO4/5 is not
  encoded in the DTS — needs schematic/boardview (gap G-4, §8).
- **Pins / pinctrl**: `pinctrl-0 = <&qup_uart9_default>` (`sdm845.dtsi:1869`); function
  `qup9`, TX = GPIO4, RX = GPIO5 (`sdm845.dtsi:3240-3252`). The OnePlus overlay sets
  `drive-strength = <2>` on both pins plus `bias-pull-up` on RX and `bias-disable` on TX
  (`sdm845-oneplus-common.dtsi:729-737`).
- **Console wiring**: `aliases { serial0 = &uart9; }` and `stdout-path = "serial0:115200n8"`
  (`sdm845-oneplus-common.dtsi:63-66`, :73); `&uart9 { label = "LS-UART1"; status = "okay"; }`
  (`sdm845-oneplus-common.dtsi:903-906`). The Bluetooth UART of the WCN3990 combo chip is a
  *different* port (`&uart6` + `bluetooth { compatible = "qcom,wcn3990-bt" }`,
  `sdm845-oneplus-common.dtsi:885-901`), so uart9 is not shared with the modem/BT function.
- **DMA**: none. The node has **no** `dmas`/`dma-names` (contrast the SPI siblings under the same
  wrapper which do, `sdm845.dtsi:1855-1861`), and the debug-UART Linux match data selects FIFO
  transfer mode (`qcom_geni_serial.c:2140-2146`). This is a FIFO-mode-only port by design.
- **Interconnects**: `qup-core`/`qup-config` bw paths via `aggre2_noc`/`gladiator_noc`
  (`sdm845.dtsi:1873-1874`) plus `operating-points-v2 = <&qup_opp_table>` (:1872) — used by the
  Linux driver for ICC bandwidth votes (§5), not needed for a polled bring-up.

## 2. What firmware leaves configured, and what the driver must (re)do

- The downstream boot chain (XBL/ABL) is the source of ambiguity: whether ABL leaves the SE
  clocked, in FIFO UART mode, or idle-gated is **not verifiable from the local sources** (gap
  G-4, §8). Do not assume any prior state.
- Both working drivers therefore *re-program* the SE from scratch on a cold path:
  - U-Boot `msm_serial_probe` checks the SE firmware protocol first — reads the FW-revision
    register and refuses to attach unless the loaded SE FW protocol == UART
    (`u-boot drivers/serial/serial_msm_geni.c:540-552`); after RAM relocation it *skips*
    re-init unless `DEBUG_UART`/`SKIP_EARLY_DM` are set (same function) — i.e. a U-Boot/EDK2
    loaded on the phone may leave the SE in whatever mode it used (typically FIFO UART as the
    debug console on Qualcomm reference U-Boot builds).
  - The Linux earlycon setup likewise aborts any half-finished transfer first and rewrites all
    UART config registers (see §4 step 2, `qcom_geni_serial.c:1720-1737`).
- Required re-init behaviour for the Redox driver (mirroring §4):
  1. Detect SE state: `SE_GENI_STATUS` (0x40) bit 0 (M_CMD active) / bit 12 (S_CMD active)
     must be polled and any running command aborted before reconfig
     (`qcuart.c:31-33`; `serial_msm_geni.c:26-28`, `:313-321`).
  2. Optionally verify the SE FW protocol via the FW-revision RO register (offset conflict —
     see §8 G-1).
  3. Re-write word length, stop-bit length, packing, parity and watermark registers; recompute
     the baud clock divider (steps 5–9 in §4).
- Redox `src/dtb/serial.rs` has a `skip-init` DTB escape hatch: if the console node carries a
  `skip-init` property the kind is created *without* re-init
  (`redox/kernel src/dtb/serial.rs:70-74`; property parsing `src/dtb/mod.rs:618-635`). For
  uart9 keep re-init ON (no `skip-init` in the mainline DTS, `sdm845.dtsi:1863-1877`);
  `skip-init` is only for "firmware already configured the SE and left it powered" trust-cases
  that we cannot prove here.

## 3. Register map (what a minimal polled driver touches)

Two address spaces matter: the **wrapper** (GENI core `qupv3_id_1`, PA `0xac0000`, size 0x6000,
`sdm845.dtsi:1752-1753`) and the **SE** (uart9, PA `0xa84000`, `reg` length 0x4000,
`sdm845.dtsi:1865`). Offsets are SE-relative unless stated.

| Offset | Register / field | Meaning (minimal driver) | Cite |
|---|---|---|---|
| wrapper +0x4 | `QUPV3_HW_VER` | QUP/SE IP version; `>= 0x20050000` ("2.5") ⇒ sampling rate halves 32→16 | `linux drivers/soc/qcom/qcom-geni-se.c:188`, `:299-310`; `u-boot serial_msm_geni.c:22`, `:79-80`, `:442-475` |
| 0x20 | `GENI_FORCE_DEFAULT_REG` — write `FORCE_DEFAULT` | de-configures the SE interface (reset-to-default escape sequence) before re-init | `u-boot serial_msm_geni.c:362-373`; `linux qcom-geni-se.c:341-355` (io init + irq clear) |
| 0x40 | `SE_GENI_STATUS` bit 0 = M_CMD active, bit 12 = S_CMD active | busy check before abort/start | `u-boot serial_msm_geni.c:28`; `openbsd-src qcuart.c:31-33`; `qcom_geni_serial.c:1100-1160` (`active` branch) |
| 0x48 / 0x4c | `GENI_SER_M_CLK_CFG` / `S_CLK_CFG`: bit 0 `SER_CLK_EN`, `[15:4]` `CLK_DIV` | per-sequencer BAUD clock: enable + divider (see §4 step 8) | `u-boot include/soc/qcom/geni-se.h:36-37`, `:112-114`; `qcom_geni_serial.c:1480-1494` |
| 0x68 | `GENI_FW_REVISION_RO` `[15:8]` protocol | *U-Boot* reads protocol code here (conflict with Linux 0x1000 — gap G-1) | `u-boot serial_msm_geni.c:540-552`; `include/soc/qcom/geni-se.h:39` |
| 0x68 (or 0x1000/0x1004) | SE FW revision (`SE_GENI_FW_REVISION`, `_S_` variant) | Linux reads protocol+fw version from `0x1000`/`0x1004` | `linux qcom-geni-se.c:236-237`, :1421-1422 |
| 0x7c | `SE_GENI_CLK_SEL` `[2:0]` = clock-select index | newer path: choose DFS clock index instead of a divider | `u-boot include/soc/qcom/geni-se.h:41`, `:124`; `qcom_geni_serial.c:1495` |
| 0x254 | `SE_GENI_BYTE_GRAN` | byte granularity helper | `u-boot include/soc/qcom/geni-se.h:43` |
| 0x258 | `GENI_DMA_MODE_EN` bit 0 | 0 = FIFO mode (what we want), 1 = DMA mode | `u-boot include/soc/qcom/geni-se.h:44`; `qcom-geni-se.c` `geni_se_select_fifo_mode` `:386-406` |
| 0x25c | `SE_UART_TX_TRANS_CFG` — `UART_CTS_MASK` = bit 1 (CTS ignored ⇒ no HW flow ctl), `UART_TX_PAR_EN` = bit 0 | frame: flow/parity select | `qcom_geni_serial.c:39-40`, `:1713` |
| 0x260/0x264 | `SE_GENI_TX_PACKING_CFG0/1` | Polled TX packing: one 8-bit character/FIFO element, LSB-first; `CFG0=0xf, CFG1=0` | `u-boot serial_msm_geni.c:83-90`, `:540-565`; `qcom-geni-se.c:539-592` (`geni_se_config_packing`) |
| 0x268 | `SE_UART_TX_WORD_LEN` (10 = 8N1 on-wire bits) | set to `DEFAULT_BITS_PER_CHAR`=8 data bits (§4 step 6) | `qcom_geni_serial.c:39`, `:86-87`, `:1732-1737` |
| 0x26c | `SE_UART_TX_STOP_BIT_LEN` (value 0 = 1 stop bit) | stop-bit length config | `qcom_geni_serial.c:40-42`, `:1737` |
| 0x270 | `SE_UART_TX_TRANS_LEN` | number of bytes in the next TX-sequencer command | `qcom_geni_serial.c:41`, `:358-365` (`setup_tx`) |
| 0x280 | `SE_UART_RX_TRANS_CFG` (parity bit 4) | RX frame: parity enable | `qcom_geni_serial.c:43`, `:1732-1734` |
| 0x284/0x288 | `SE_GENI_RX_PACKING_CFG0/1` | RX unpacking: four 8-bit characters/FIFO element, offsets 0/8/16/24; `CFG0=0x4380e, CFG1=0xc3e0e` | `u-boot serial_msm_geni.c:83-90`, `:375-390` |
| 0x28c | `SE_UART_RX_WORD_LEN` `[9:0]` | RX word length (8) | `qcom_geni_serial.c:44`, `:1732-1737` |
| 0x294 | `SE_UART_RX_STALE_CNT` `[23:0]` | RX stale timer, ticks = `bits_per_char × STALE_TIMEOUT(16)` = 160 | `qcom_geni_serial.c:45-46` (mask), `:86`, `:1352` (`rxstale`), `:1374` (write) |
| 0x2a4/0x2a8 | `SE_UART_TX/RX_PARITY_CFG` (`PAR_CALC_EN` bit 0, even/odd/space) | parity type | `qcom_geni_serial.c:47-52` |
| 0x2ac | `SE_UART_MANUAL_RFR` bit 31 EN / bit 1 NOT_READY / bit 0 READY | RS485 manual RTS — not needed for a console | `qcom_geni_serial.c:50-52`, `:205-217` |
| 0x600 | `SE_GENI_M_CMD0`: opcode `[31:27]`, `UART_START_TX` = 1 ⇒ `0x08000000` | start a TX transfer (with TRANS_LEN set first) | `qcom_geni_serial.c:361-364`, `:74`; `u-boot serial_msm_geni.c:65` (`M_OPCODE_SHIFT` 27); `qcuart.c:35-36` |
| 0x604 | `SE_GENI_M_CMD_CTRL_REG` — write `M_GENI_CMD_ABORT` (bit 16) | abort current TX command | `qcom_geni_serial.c:371-379` (poll_tx_done), `:1350-1400` |
| 0x610/0x614/0x618 | `SE_GENI_M_IRQ_STATUS/EN/CLEAR`: bit 0 `CMD_DONE`, bit 30 `TX_FIFO_WM`, bit 31 `SEC_IRQ` (write-1-to-clear in STATUS/CLEAR) | handshake bits for polled & IRQ paths | `qcuart.c:37-42`; `qcom_geni_serial.c:425-455` (poll put/get), `:1100-1160` |
| 0x630 | `SE_GENI_S_CMD0`: opcode `[31:27]`, `UART_START_READ` = 1, params `UART_PARAM`=1 / RFR bit 7 | start/stop the RX sequencer | `qcom_geni_serial.c:76-78`; `qcuart.c:43-44`; `u-boot serial_msm_geni.c:347-360` |
| 0x634 | `SE_GENI_S_CMD_CTRL_REG` — `S_GENI_CMD_ABORT` | abort current RX command | `qcom_geni_serial.c:386-389`; `u-boot serial_msm_geni.c:362-366` |
| 0x640/0x644/0x648 | `SE_GENI_S_IRQ_STATUS/EN/CLEAR`: bit 26 `RX_FIFO_WM`, bit 27 `RX_FIFO_LAST` | RX watermark/last-char event bits | `qcuart.c:45-49` |
| 0x700 | `SE_GENI_TX_FIFOn` (FIFO write port) | write 32-bit FIFO words (4 packed bytes); `BYTES_PER_FIFO_WORD`=4 | `qcom_geni_serial.c:106-107`, `:446`, `:1084-1092`; `qcuart.c:50` |
| 0x780 | `SE_GENI_RX_FIFOn` (FIFO read port) | read 32-bit FIFO words; word 1 may be partial | `qcuart.c:51`, `:179-191`; `qcom_geni_serial.c:395-443` |
| 0x800 | `SE_GENI_TX_FIFO_STATUS`: `TX_FIFO_WC` `[27:0]` = words queued | `0` ⇒ TX idle/empty (basis of the busy check + timeout math) | `qcom_geni_serial.c:793` (empty test), `:1110`/`:1125` (avail math); `u-boot serial_msm_geni.c:69`, `:305-311` |
| 0x804 | `SE_GENI_RX_FIFO_STATUS`: `RX_FIFO_WC` `[24:0]` words; `RX_LAST` + `RX_LAST_BYTE_VALID` field | RX drain loop counts; last-word byte count extraction | `qcuart.c:54-55` (mask `0x1ffffff`); `qcom_geni_serial.c:408-433` (poll path), `:520-532` |
| 0x80c | `SE_GENI_TX_WATERMARK_REG` | TX FIFO watermark; default `DEF_TX_WM` = 2 words | `u-boot serial_msm_geni.c:58-59` (`DEF_TX_WM 2`), `:383-402`; `qcuart.c:56-58` (`QCUART_TX_WATERMARK 2`) |
| 0x810 / 0x814 | `SE_GENI_RX_WATERMARK_REG` / `SE_GENI_RX_RFR_WATERMARK_REG` | RX watermark & RFR watermarks; init sets `rx_wm`, `rx_rfr` (§4 step 5) | `u-boot include/soc/qcom/geni-se.h:64-65`; `linux qcom-geni-se.c:364-383` (`geni_se_init`); `qcom_geni_serial.c:1396` |
| 0xe24 | `SE_HW_PARAM_0`: `[27:16]` TX FIFO depth (words), `[31:24]` TX FIFO width (bits) | read FIFO geometry at boot; default fallback 16 words × 32 bits | `u-boot serial_msm_geni.c:74-75`, `:160-183`; `qcom_geni_serial.c:89-91` (`DEF_FIFO_DEPTH_WORDS` 16, `DEF_FIFO_WIDTH_BITS` 32), `:1293-1318` (`setup_fifos`) |

Register-access rules carried by all three references: all registers are 32-bit LE MMIO
(`UPIO_MEM`, 32-bit FIFO ports — `qcom_geni_serial.c:186-190` `uport` defaults, `:1957`
`tx_fifo_width` = 32); IRQ-pending bits are *level/latched* — the TX watermark bit re-asserts and
must be cleared again **after** FIFO writes (`qcom_geni_serial.c:1141-1147` comment + write).

## 4. Sequences

### 4.1 Init (cold / takeover)

Modelled on `qcom_geni_serial_earlycon_setup` (`qcom_geni_serial.c:1702-1755`), the full-port
`qcom_geni_serial_port_setup` (`:1350-1400`) and U-Boot `geni_serial_init`
(`serial_msm_geni.c:540-565`):

1. **Safety abort of anything in flight** (do this *before* touching config — "unconditional
   cancel on the main sequencer… else we could end up in data loss scenarios",
   `qcom_geni_serial.c:1723-1727`): poll `SE_GENI_STATUS` until M_CMD bit 0 and S_CMD bit 12 are
   clear, or issue `M_GENI_CMD_ABORT` (0x604)/`S_GENI_CMD_ABORT` (0x634) and poll until the
   abort-self bit clears, then write-1-to-clear the IRQ status bits and `FORCE_DEFAULT` to
   `GENI_FORCE_DEFAULT_REG` (0x20) to deconfigure the interface
   (`qcom-geni-se.c:341-355`; `u-boot serial_msm_geni.c:362-373`).
2. **Check SE firmware protocol** == UART via the FW-revision register (offset ambiguity:
   `qcom-geni-se.c:236-237` reads 0x1000/0x1004 in current mainline — where a mismatch means the
   SE FW is loaded for another protocol, `qcom_geni_serial.c:1356-1367`; the U-Boot path checks
   0x68 `FW_REV_PROTOCOL`, `serial_msm_geni.c:540-552`). If UART FW is not loaded: this is a
   *gap* on Redox — the Linux driver can load `qcom,se_fw` firmware from disk; we cannot, so
   verify on device (§7). On mainline the wrapper loads UART SE FW automatically for
   `qcom,geni-se-qup` wrappers, so expect "already UART" — but re-check anyway.
3. **Enable clocks** (§5): wrapper `m-ahb`/`s-ahb` + `se` branch clock; then compute the
   divider for `baud × 32` (§4.3) and write `GENI_SER_M_CLK_CFG`/`S_CLK_CFG` with
   `SER_CLK_EN | (clk_div << 4)` (`qcom_geni_serial.c:1444-1496`; `u-boot
   serial_msm_geni.c:134-149`, `:187-196`). Skip the `SE_GENI_CLK_SEL` write (0x7c) unless the
   on-device wrapper version proves a DFS-style clock (≥ 2.5 path, `qcom_geni_serial.c:1493-1495`).
4. **Clear interrupts / events**: write 0 to `SE_IRQ_EN`/`SE_GSI_EVENT_EN` and all-ones to the
   four IRQ-clear registers (0x618, 0x648, DMA 0xc44/0xd44) (`qcom-geni-se.c:344-351`
   `geni_se_irq_clear`; the default `geni_se_init` call also writes these, `:364-383`).
5. **Watermarks**: `RX_WATERMARK = FIFO_DEPTH/2` (earlycon: `8` from `DEF_FIFO_DEPTH_WORDS/2`) and
   `RFR_WATERMARK = FIFO_DEPTH − 2` (`=14`) via `geni_se_init` (`qcom_geni_serial.c:1726-1728`;
   full-port uses `UART_RX_WM`=2 and `rx_fifo_depth−2`, `:1396`). TX watermark = 2 (default, §3).
6. **Frame format**: TX/RX `WORD_LEN = 8` data bits (10 on-wire with start/stop),
   `STOP_BIT_LEN = 0` (1 stop bit), `TX_TRANS_CFG = UART_CTS_MASK` (ignore CTS ⇒ no HW flow
   control), parity disabled (`TX/RX_PARITY_CFG = 0`, `RX_TRANS_CFG = 0`)
   (`qcom_geni_serial.c:1713-1737`).
7. **Packing**: polled TX uses `CFG0=0xf, CFG1=0`, a single 8-bit vector
   with its stop bit set. RX must use `CFG0=0x4380e, CFG1=0xc3e0e` for the driver's
   four-byte FIFO drain: vectors start at 0/8/16/24, each length field is7 and
   only the fourth stop bit is set. The earlier claim that0xf represented four
   characters was wrong and reproduced NUL-padding in the emulator shell.
   Register interpretation: [upstream GENI packing fields](https://github.com/torvalds/linux/blob/master/drivers/soc/qcom/qcom-geni-se.c),
   `PACKING_START_SHIFT=5`, `PACKING_LEN_SHIFT=1`, stop bit0, vector shift10.
   This is register-format corroboration; no upstream implementation was copied.
8. **Mode**: ensure FIFO mode — keep `SE_GENI_DMA_MODE_EN` (0x258) bit 0 **clear** and select
   FIFO (`geni_se_select_mode(GENI_SE_FIFO)` → `geni_se_select_fifo_mode` clears the bit,
   `qcom-geni-se.c:386-406`, `:456-475`; console dev_data mode=FIFO, `qcom_geni_serial.c:2140-2146`).
9. **Start RX sequencer**: `geni_se_setup_s_cmd(se, UART_START_READ, 0)` — i.e. write
   `S_CMD0 = (1 << 27)` (params 0; `UART_PARAM_RFR_OPEN` bit 7 only for auto-RFR boards) and
   keep the RX watermark/last IRQ bits enabled (`qcom_geni_serial.c:1690-1694` early read-enable,
   `:1396-1399`; `u-boot serial_msm_geni.c:347-360` `start_rx`; `qcuart.c:583-585` cngetc
   re-arms RX the same way).

### 4.2 Poll-mode TX (what Redox `SerialKind::write` must do first)

Per character, polled (reference-identical in U-Boot and OpenBSD):

1. `SE_GENI_TX_WATERMARK_REG = 2` (`DEF_TX_WM`) — request granularity (one command per char
   while polling; batched chunks are §4.4) (`u-boot serial_msm_geni.c:383-402`; `qcuart.c:611-637`).
2. `SE_UART_TX_TRANS_LEN = 1` (one byte in this command).
3. `SE_GENI_M_CMD0 = UART_START_TX<<27` (`0x08000000`) — issue the command; **then** write the
   FIFO word (`qcom_geni_serial.c:445-446` `setup_tx` + FIFO write; `serial_msm_geni.c:383-402`
   `putc`; `qcuart.c:611-637` `cnputc`). The sequencer stalls the command until TX FIFO satisfies
   the watermark request (TX watermark IRQ = "FIFO empty/need data"; the WM bit is latched and
   must be re-cleared after the data write, `qcom_geni_serial.c:1141-1147`).
4. Wait `M_IRQ_STATUS` bit 30 (TX FIFO watermark — data accepted), write-clear it, then wait
   `M_IRQ_STATUS` bit 0 (CMD_DONE) and write-clear that too (loop:
   `serial_msm_geni.c:383-402`; `qcuart.c:611-637`).
5. Timeout/abort path (§4.4) applies if CMD_DONE never arrives.

Poll-mode TX batching (optional second step): up to a whole FIFO of bytes can go in one
command — `CONSOLE_TX_CHUNK_SIZE = 32` bytes max per command for console writes, and `avail =
tx_fifo_depth − TX_FIFO_WC(words) × 4` for the interrupt path (`qcom_geni_serial.c:93`, `:660-700`,
`:1100-1160`).

### 4.3 Baud rate (clock math, not a divider-register "SCON"-style write)

- Desired SER clock = `baud × 32` (`UART_OVERSAMPLING` = 32; halves to 16 only if the wrapper
  IP version at `0xac0000+0x4` ≥ `QUP_SE_VERSION_2_5` = `0x20050000` — verify on device, §7)
  (`qcom_geni_serial.c:85-87`, `:1455-1458`; `u-boot serial_msm_geni.c:442-475`).
- `clk_div = ceil(se_clk_rate / (baud × 32))`; pick a supported `se` clock that yields an exact
  or near divisor — U-Boot searches the QUP DFS table `{7372800, 14745600, 19200000, 29491200,
  32000000, 48000000, 64000000, …}` (`serial_msm_geni.c:103-149`); Linux matches rates against
  the OPP/clock tree (`geni_se_clk_freq_match`, `qcom_geni_serial.c:1465-1470`). For 115200 the
  natural target is 14.7456 MHz (4× 3.6864 MHz) or 3.6864 MHz — both exact ⇒ no fractional baud
  error (§7 check).
- Write the *same* `SER_CLK_EN | clk_div<<4` value to **both** `GENI_SER_M_CLK_CFG` (0x48) and
  `GENI_SER_S_CLK_CFG` (0x4c); on the divider path do **not** write `SE_GENI_CLK_SEL` (§3).

### 4.4 Operate RX (poll), error recovery, shutdown

- **Poll RX** (first char of a burst may be partial): check `RX_FIFO_STATUS` word count; if one
  word with `RX_LAST` set, take the valid-byte count from `RX_LAST_BYTE_VALID` (0 ⇒ 4); read FIFO
  words 4 bytes at a time (`qcom_geni_serial.c:395-443` `get_char`, `:520-532` IRQ count math);
  OpenBSD re-issues `S_CMD0 = START_RX` before each polled read (`qcuart.c:583-585`).
- **Error/timeout recovery**: if CMD_DONE times out → write `M_GENI_CMD_ABORT` to 0x604, poll the
  abort-self bit, clear it in `M_IRQ_CLEAR` (`qcom_geni_serial.c:371-379`;
  `u-boot serial_msm_geni.c:289-311`). RX stale-timeout (160 bits ≈ one char + fluff) bounds the
  per-char RX poll loop (`qcom_geni_serial.c:86`, `:1352`; U-Boot derives its per-FIFO poll
  timeout `fifo_bits×10⁶/baud + 500 µs`, `serial_msm_geni.c:241-262`).
- **Shutdown/suspend**: stop TX (abort active command, disable M watermark/CMD_DONE bits —
  `qcom_geni_serial.c:914`, `:954`, sys-shutdown `:2180-2189`) and stop RX (abort S_CMD + mask
  bits); Redox equivalent = suspend/resume pair like OpenBSD `qcuart_activate` (disable IRQ
  enable bits on suspend; on resume re-enable and re-issue `S_CMD0 = START_RX`,
  `openbsd-src qcuart.c:123-156`).

## 5. Interrupts / DMA / power

**Polled first (recommended default).** A polled TX/TX-drain driver (OpenBSD's
`qcuartcnputc`/`cngetc`, §4.2/§4.4) is safe from IRQ-off contexts and matches what Redox's
`SerialKind::write` for PL011/Meson already assumes for early logging. Start with polling; wire
IRQs only after the register model matches Linux byte-for-byte (§7).

**IRQ path (phase 2).**
- Line: GICv3 **SPI 354, level-high** (`sdm845.dtsi:1870`; `arm,gic-v3` intc
  `sdm845.dtsi:5463-5464`). IRQ-pending bits live in the SE (`M/S_IRQ_STATUS`, §3) and must be
  write-1-cleared *and re-cleared after FIFO writes* (level-triggered & latched,
  `qcom_geni_serial.c:1141-1147`).
- Redox side: the second-stage `arch/aarch64/device/serial.rs::init` already looks up the
  console node's first `interrupts` entry, translates it through the dtb irqchip framework and
  registers an `InterruptHandler` that drains `COM1.receive` + raises an event (`redox/kernel
  src/arch/aarch64/device/serial.rs:29-71`; framework `src/dtb/irqchip.rs`). ⚠ Gap G-5: SPI 354
  must survive the u8-sized irq numbering (the `add_irq` FIXME comment,
  `src/arch/aarch64/device/serial.rs:21-24`) — check `irq_xlate` numbering space before trusting
  the IRQ path for a device whose parent is GICv3 rather than the Meson-style chips.
- IRQ path also needs RX re-arm (re-issue `S_CMD0 = START_RX`) like
  `qcuart_activate` DVACT_RESUME (`qcuart.c:137-152`).

**DMA.** None. The mainline node carries no `dmas` (contrast SPI siblings,
`sdm845.dtsi:1855-1861`) and the debug-UART class is FIFO-mode only
(`qcom_geni_serial.c:2140-2146`). Do not plan GPI/DMA (0xc40/0xd40 register space,
`u-boot include/soc/qcom/geni-se.h:67-72`) for this port.

**Power.**
- The SE sits in the **CX** rail/GDSC (`power-domains = <&rpmhpd SDM845_CX>`,
  `sdm845.dtsi:1871`). Linux keeps the domain+clocks pinned while active (runtime PM:
  `geni_se_resources_init/activate` attach the CX power domain, `qcom-geni-se.c`
  domain code `:1117-1151`; `qcom_geni_serial.c` pm ops `:2171-2174`) and drops them at idle;
  the `se` branch clock is likewise enabled/disabled per use (`qcom-geni-se.c:595-650`).
- Consequences for Redox: until an RPMh/CX driver exists, CX must be assumed **always-on**
  (do not attempt CX collapse); verify on device whether RSC can gate SE clocks while the SoC
  is awake — if a read of `SE_GENI_STATUS`/FIFO status returns all-F/G after idle, SE power
  collapsed and the port must be re-initialised (§4.1) on resume (gap G-4, §8).
- Interconnect bandwidth (`qup-core`/`qup-config` votes, `sdm845.dtsi:1873-1874`;
  `Bps_to_icc` votes in `qcom_geni_serial.c:1486-1492`): out of scope for boot; the
  `interconnects` property only matters if the `geniqup` wrapper + icc drivers are ported.

**SMMU.** uart9 itself performs no SMMU-translated DMA traffic (FIFO mode, no `iommus` on the
node); the wrapper's `iommus = <&apps_smmu 0x6c3 0x0>` (`sdm845.dtsi:1757`, :1763) serves the
wrapper DMA/GPI engines, not the SE FIFO MMIO. ⇒ MMIO-only mapping required (§6), no SID
plumbing needed for this block.

## 6. Redox interface

**Kernel piece (primary).** This block is the *diagnostic UART* in Redox's model — it must be
driven from the kernel console path, not as an ordinary user daemon, because (a) kernel logging
(`/dev/kmsg`/`debug` scheme) writes must work before and independently of userspace, and (b) the
chosen-node/console discovery path already exists:

- `kernel/src/dtb/serial.rs::init_early` (file `src/dtb/serial.rs:63-171`) matches the
  `chosen/stdout-path` node's `compatible` string (discovery via `diag_uart_range`/
  `diag_uart_node`/`diag_uart_params`, `src/dtb/mod.rs:610-635`) and instantiates the matching
  `SerialKind` variant. **Add**: a `uart_qcom_geni::SerialPort` module next to
  `uart_meson`/`uart_pl011` (`src/devices/`), a `SerialKind::QcomGeni(...)` variant +
  `receive/write/enable_irq/init_full` dispatch (`src/devices/serial.rs:12-70`, enum
  `SerialKind` at :12), and a `compatible_with("qcom,geni-debug-uart")` branch in `init_early`
  beside the Meson branch (`src/dtb/serial.rs:101-153`). `stdout_baud()` (`:16-24`) already
  parses `115200n8`-style params for the kind constructor.
- MMIO: the exact `reg` range is kept mapped as **Device** memory (`register_memory_region
  (…, BootloaderMemoryKind::Device)`, `src/dtb/mod.rs:441-447`) and reached through the direct
  map via `RmmA::phys_to_virt` (`src/dtb/serial.rs:71-73`) — same mechanism as Meson; the
  wrapper-node `ranges` identity (§1) means `dtb::translate_mmio_address` resolves 0xa84000 →
  0xa84000 unchanged (unit-test that helper with a fixture DTB, §7).
- IRQ/event: the second-stage wiring hooks the new kind's `receive()` drain into
  `Com1Irq::irq_handler` → `debug_input/debug_notify` → `DebugScheme` INPUT `WaitQueue` and
  `event::trigger` (`src/arch/aarch64/device/serial.rs:14-71`; `src/scheme/debug.rs:16-45`), so
  a user daemon that opens the `debug` scheme (`GlobalSchemes::Debug`) receives console input
  without owning the hardware — the Redox analogue of Linux `earlycon` + `console=ttyMSM0`
  (`.name = "ttyMSM"`, `qcom_geni_serial.c:1769`).
- **User-space daemon?** Not needed for the debug role. If uart9 later serves as a
  general-purpose serial port for userspace, the Redox pattern is a `exampled`-style scheme
  daemon under `base/drivers` mapping MMIO via **`/scheme/memory/physical@uc`** and taking
  interrupt events via **`/scheme/irq` + `/scheme/event`** (`redox/base/drivers/README.md:112-116`,
  `:128-136`; exampled recipe exists at `redox/recipes/demos/exampled/recipe.toml`, dev-redox
  group `redox/recipes/groups/dev-redox/recipe.toml`). That must be mutually exclusive with the
  kernel kind — one owner per port; and the §8 "start from exampled" guidance applies to any
  such daemon skeleton.
- Start from `exampled`: yes — new scheme daemon code starts from `exampled` +
  `redox-scheme`/`redox_event` (`base/drivers/README.md:128-136`); the kernel kind starts from
  `uart_meson.rs` (closest existing: FIFO-based non-8250 SE-style port with its own clock math).

## 7. Verification

1. **Host tests (before the phone).**
   - DTB fixture: build a test devicetree with an `aliases { serial0 = &uart9; }` +
     `chosen { stdout-path = "serial0:115200n8"; }` + the verbatim `serial@a84000` node from
     `sdm845.dtsi:1863-1877`; assert (a) `diag_uart_range` returns (0xa84000, 0x4000,
     skip_init=false, cts=false, "qcom,geni-debug-uart") through the `ranges`-identity
     translation (pattern: existing translation unit test `src/dtb/mod.rs:695` test module),
     (b) `init_early` selects the new kind, (c) `stdout_baud` → 115200 (§6).
   - Register model: replay a **captured Linux trace** (step 2) against a mock SE implemented
     from §3/§4 in `kernel/src/dtb/testdata` + host unit tests, byte-exact compare.
2. **Linux oracle (pmOS mainline on the device).** pmaports `device-oneplus-enchilada` boots
   mainline with `deviceinfo_dtb="qcom/sdm845-oneplus-enchilada"` and
   `deviceinfo_append_dtb="true"`, fastboot boot-image offsets (kernel@0x8000000 etc.)
   (`pmaports device/community/device-oneplus-enchilada/deviceinfo:18-32`) and
   `kernel-cmdline.conf:1 = console=ttyMSM0,115200` — ttyMSM0 is exactly this block's mainline
   console driver (`qcom_geni_serial.c:1769`, `:1755-1756` OF_EARLYCON_DECLARE `qcom_geni`).
   Capture on the running pmOS phone (serial console or SSH over USB):
   - `busybox devmem 0xa84000 32` (SE-relative absolute = `0xa84000 + offset`): dump
     `0xa84040` (status), `0xa84048/0xa8404c` (clk cfg → live baud divider), `0xa84600`
     area (cmd/irq regs), `0xa84800/804` (FIFO status) before/after `echo > /dev/console`
     spam — needs a kernel with `CONFIG_DEVMEM`+`STRICT_DEVMEM` off (test build; note in gap
     G-6).
   - `cat /sys/kernel/debug/tracing/...` for the `qcom_geni_serial` ftrace events (the driver
     compiles in tracepoints via `#include <trace/events/qcom_geni_serial.h>`,
     `qcom_geni_serial.c:11`; e.g. `geni_serial_clk_cfg` fired at `:1476`) — record one Linux
     boot + one big `dd` write to /dev/console and replay against the host model.
   - Clock oracle: `cat /sys/kernel/debug/clk/clk_summary | grep -i qup` → confirm what rate
     `gcc_qupv3_wrap1_s1_clk` actually locks to for the console (resolves gap G-2/G-3).
   - Wrapper IP version: `devmem 0xac0004` (wrapper+`QUPV3_HW_VER_REG`) → decide sampling 32/16.
   - Loopback option: if uart9's pads are not externally reachable (gap G-4), drive the SE in
     `SE_UART_LOOPBACK_CFG` (0x22c, RX_TX_SORTED `BIT(0)`, `qcom_geni_serial.c:37`, `:93-97`)
     under Linux to self-test RX↔TX, and under Redox as an acceptance mode.
3. **On-phone acceptance (Redox).** Flash/boot via `fastboot boot` (same offsets as the pmOS
   image; `fastboot` is the device's flash method, `deviceinfo:18`; `fastboot boot` a
   Redox kernel+appended-DTB built for `aarch64`):
   - cold boot shows the Redox banner + early log at 115200 8N1 on the uart9 pair;
   - `write()` path: no byte loss on 4 KiB+ blocks (checks the ≤32-byte command chunking rule,
     §4.2/`qcom_geni_serial.c:660-700`);
   - `receive()`/echo path through the `debug` scheme round-trips typed bytes (IRQ-off polled
     fallback must also work if IRQ is deferred);
   - idle 10 min at console → one char typed; if the reply stream is corrupted, RSC/CX
     collapse fired (gap G-4) → re-init-on-resume required (§4.1);
   - diff Redox's captured console bytes against the Linux-oracle stream captured with the same
     input: byte-exact, framing-only differences allowed.

## 8. License path

**License path: MIT clean-room** (per template §8 and `redox/base/drivers/README.md` driver
policy: reimplement from BSD references rather than copying GPL logic).

- **Primary functional reference (portable / ISC)**: OpenBSD `sys/dev/ic/qcuart.c`,
  `qcuartvar.h`, `sys/dev/fdt/qcuart_fdt.c` — ISC permissive licence (header in each file;
  `qcuart.c:1-15`, `qcuartvar.h:1-15`, `qcuart_fdt.c:1-15` at openbsd-src `20cb344`). It
  already implements the polled FIFO protocol (§4.2/§4.4), matches `qcom,geni-debug-uart`
  (`qcuart_fdt.c:52-53`) and handles `geni-uart`/`geni-debug-uart` for the console
  (`qcuart_fdt.c:35-46`). **Quote behaviour, not code** (its bus_space/tty plumbing is
  OpenBSD-internal); write the Rust from scratch, citing it for behaviour.
- **Reference-only (GPL — do NOT transcribe)**: Linux `qcom_geni_serial.c` (GPL-2.0, SPDX line
  1) and `qcom-geni-se.c` (GPL-2.0, `EXPORT_SYMBOL_GPL` exports, e.g. `:383`); U-Boot
  `drivers/serial/serial_msm_geni.c` is **GPL-2.0+ and "Based on Linux driver"** (file header
  `:1-7`) — i.e. U-Boot's *BSD-licensed driver pool does not help here; this file is
  GPL-derived, so treat exactly like the Linux driver: behaviour cross-check only. If any
  implementation decision turns out to need code *ported* from these GPL sources, switch the
  crate to **GPL-2.0, separate crate** (template rule) and record that here.
- Redox driver-licensing guidance followed: `redox/base/drivers/README.md` ("Write a Driver"
  section, `:128-136`) — reverse-engineering from BSD drivers is the sanctioned route for MIT.

**Gaps / conflicts (resolve before implementation):**

- **G-1 (conflict)**: SE FW-protocol register offset differs — `0x68`
  `GENI_FW_REVISION_RO [15:8]` (u-boot `include/soc/qcom/geni-se.h:39`; used by
  `serial_msm_geni.c:540-552`) vs `0x1000`/`0x1004` `SE_GENI_FW_REVISION`/`_S_` (current mainline
  `qcom-geni-se.c:236-237`). Resolution must come from a device read (§7) — do not hard-code one.
- **G-2**: whether SDM845's SE is "≥ 2.5" (sampling 16) or older (sampling 32) is not pinned by
  local sources; the code paths differ (`qcom_geni_serial.c:1455-1458`; `serial_msm_geni.c:469-472`).
  Read `0xac0004` first (§7).
- **G-3**: whether ABL/EDK2 (edk2-sdm845 checkout `d7fa8bd` contains no GENI serial driver —
  grepped) leaves the SE powered/FIFO-configured or powered-off is unverified from local
  sources; assume "unknown" and re-init unconditionally (§2/§4.1). RSC/CX gating of the SE while
  the SoC runs is unverified (→ §5, §7 test 3).
- **G-4**: physical accessibility of the uart9 pads (GPIO4/5, function `qup9`,
  `sdm845.dtsi:3240-3252`) on the OnePlus 6 board — test points vs never-routed vs
  shared-with-what — is not answered by any local source (the `"LS-UART1"` label is inherited
  from reference-board wording, `sdm845-oneplus-common.dtsi:903-906`). Needs boardview/
  schematics or a scope on the pads; also blocks the physical oracle in §7 until decided
  (loopback fallback provided).
- **G-5**: Redox IRQ plumbing — confirm SPI 354 (level-high, GICv3) survives the u8 irq
  numbering / `add_irq` limitation and Meson-oriented irqchip assumptions
  (`redox/kernel src/arch/aarch64/device/serial.rs:21-24`, `src/dtb/irqchip.rs` generic
  framework). Polled path works without this (§4.2), so treat as phase-2 blocker only.
- **G-6**: capturing the Linux oracle needs a devmem-capable kernel (STRICT_DEVMEM off); the
  shipped pmOS kernel may restrict `/dev/mem` — budget a test-kernel build, or capture via
  loopback + ftrace only.
- **G-7**: `include/linux/soc/qcom/geni-se.h` is absent from the local sparse `linux`
  checkout, so any statement about mainline-only register fields was cross-checked only
  against the U-Boot header and OpenBSD defines; where the two disagree, both are cited
  inline rather than guessed.
