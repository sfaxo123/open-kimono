# GENI Serial Engine — I2C on i2c12 (touchscreen bus)

**Block**: Qualcomm GENI SE (Serial Engine) v2 in **I2C master mode, FIFO transfer only** (no
GPI/DMA), driving the touchscreen I2C bus of the OnePlus 6 (codename *enchilada*, SoC SDM845 /
Snapdragon 845).
**DT `compatible`**: `qcom,geni-i2c` (also `qcom,geni-i2c-master-hub` / `qcom,sa8255p-geni-i2c`
in newer Linux — not this SoC).
**Role on this phone**: `i2c12` carries the Synaptics/PureSign RMI4-over-I2C touchscreen at
7-bit address **0x20** at **400 kHz** (Fast-mode). This is a *peripheral* bus port (unlike the
debug-UART spec): the Redox consumer is a user-space daemon, not the console path.

Reference sources and revisions (line citations below are against these checkouts):

| Repo | Commit | Files used |
|---|---|---|
| `~/Developer/forks/linux` | `5b0764a` | `drivers/i2c/busses/i2c-qcom-geni.c`, `drivers/soc/qcom/qcom-geni-se.c`, `arch/arm64/boot/dts/qcom/sdm845.dtsi`, `sdm845-oneplus-common.dtsi`, `sdm845-oneplus-enchilada.dts`, `drivers/clk/qcom/gcc-sdm845.c` |
| `~/Developer/forks/u-boot` | `8649a54d` | `drivers/i2c/geni_i2c.c`, `include/soc/qcom/geni-se.h` |
| `~/Developer/forks/openbsd-src` | `20cb344` | `sys/dev/fdt/qciic_fdt.c`, `sys/dev/fdt/files.fdt` (attach decl :752) |
| `~/Developer/forks/edk2-sdm845` | `d7fa8bd` | `sdm845Pkg/Include/Configuration/DeviceMemoryMap.h` — **no** GENI I2C driver in this checkout, only MMIO reservations |
| `~/Developer/forks/redox/kernel` | `8186c379` | `src/scheme/{memory,irq,event}.rs`, `src/devices/` (UART-only), `src/dtb/{mod,serial,irqchip}.rs` |
| `~/Developer/forks/redox/base` | `d2ffc4ca` | `drivers/README.md` (:112, :114, :122-134), `drivers/hwd/` (ACPI/DT enumeration daemon) |
| `~/Developer/forks/pmaports` | `e6ff10d` | `device/community/device-oneplus-enchilada/{deviceinfo,kernel-cmdline.conf,modules-initfs}` |

> The local `linux` checkout is sparse/blobless: `include/linux/soc/qcom/geni-se.h` is absent,
> so helper semantics (`geni_se_setup_m_cmd`, `geni_se_abort_m_cmd`, `geni_se_read_proto`,
> `geni_se_get_tx_fifo_depth`) are cited from their *call sites* plus the U-Boot header.

---

## 1. Block

- **DT node**: `i2c12: i2c@a90000` (`linux sdm845.dtsi:1993`), alias `i2c12 = &i2c12`
  (`sdm845.dtsi:52`), child of wrapper `qupv3_id_1: geniqup@ac0000`
  (`compatible = "qcom,geni-se-qup"`, `sdm845.dtsi:1751-1752`). Default `status = "disabled"`
  (`:2012`); enabled on OnePlus with the touch child (see below) — `&qupv3_id_1 { status =
  "okay"; }` and `&i2c12 { status = "okay"; }` (`sdm845-oneplus-common.dtsi:517-518, :706-708`).
- **compatible**: `qcom,geni-i2c` (`sdm845.dtsi:1994`). Linux matches it in
  `i2c-qcom-geni.c:1342-1348` (data `geni_i2c`: no `no_dma_support` flag ⇒ the *mainline*
  driver also runs GPI-DMA for ≥32-byte messages; the Redox port does not — §5).
  OpenBSD matches the same string (`obsd qciic_fdt.c:93`); U-Boot too (`u-boot
  geni_i2c.c:557-561`).
- **reg / address**: `reg = <0 0x00a90000 0 0x4000>` (`sdm845.dtsi:1995`). The wrapper node has
  `ranges;` (`sdm845.dtsi:1760`): identity translation, so the *physical* base of the SE window
  is `0xa90000` (16 KiB). Absolute MMIO = `0xa90000 + offset`. Wrapper MMIO `0xac0000` size
  `0x6000` (`:1753`). `spi12` shares the same SE address `spi@a90000` (`:2015-2016`) — pinmux
  multiplexes; we use the I2C function.
- **Interrupts**: `interrupts = <GIC_SPI 357 IRQ_TYPE_LEVEL_HIGH>` (`sdm845.dtsi:2000`).
  Parent irqchip is the SoC `intc` `arm,gic-v3` at `0x17a00000` (`sdm845.dtsi:34 default
  interrupt-parent`, `:5464`). No PDC/SPI-offset indirection for this node. Note: this IRQ is
  the *controller* completion/level signal — the touch chip's own INT line is separate (§1
  pins, §5).
- **Clocks**: `clock-names = "se"`, `clocks = <&gcc GCC_QUPV3_WRAP1_S4_CLK>`
  (`sdm845.dtsi:1996-1997`). That branch clock: `gcc_qupv3_wrap1_s4_clk`, `halt_reg
  0x184d4`, `enable_reg 0x5200c bit 26`, `BRANCH_HALT_VOTED`, parent RCG
  `gcc_qupv3_wrap1_s4_clk_src` (`clk_rcgr 0x184d8`, `mnd_width 16`, `hid_width 5`) with the
  wrapper-shared freq table `ftbl_gcc_qupv3_wrap0_s0_clk_src`
  (`linux gcc-sdm845.c:2368-2381`, table def `:429`, RCG def `:647-654`). The table carries
  the 19.2 MHz (BI_TCXO) and 32 MHz (GPLL0) rates that the driver's SCL counter math needs
  (§4.3) — confirm contents on device (§7). Wrapper AHB clocks `m-ahb`/`s-ahb`:
  `GCC_QUPV3_WRAP_1_M_AHB_CLK`/`_S_AHB_CLK` (`sdm845.dtsi:1754-1756`).
- **Resets**: none declared (`sdm845.dtsi:1993-2013` has no `resets`). SE-level "reset" is
  software `FORCE_DEFAULT` → `GENI_FORCE_DEFAULT_REG` 0x20 (§3/§4.1).
- **Supplies (touch child)**: `&i2c12` carries `synaptics-rmi4-i2c@20` — `compatible =
  "syna,rmi4-s3706b", "syna,rmi4-i2c"`, `reg = <0x20>`; `interrupts-extended = <&tlmm 125
  IRQ_TYPE_EDGE_FALLING>`; `vdd-supply = <&vreg_l28a_3p0>` (LDO28, 2.856-3.008 V,
  `sdm845-oneplus-common.dtsi:408-411`), `vio-supply = <&ts_1p8_supply>` (fixed 1.8 V rail
  gated by GPIO88; the DTSI itself comments "don't know how this is actually wired up..."
  `common.dtsi:191-205`). RMI4 function nodes `rmi4-f01@1` / `rmi4_f12@12`; panel extents
  `touchscreen-x-mm = <68>` (common) + `touchscreen-y-mm = <144>`, `syna,clip-x-high = <1079>`,
  `syna,clip-y-high = <2279>` (`common.dtsi:521-549`; `enchilada.dts:99-102` — matches the
  1080×2280 panel, cf. pmaports `deviceinfo` `deviceinfo_screen_height="2280":15`).
- **Pins / pinctrl**: `pinctrl-0 = <&qup_i2c12_default>` (`sdm845.dtsi:1999`) = GPIO49
  (SDA?)/GPIO50 pair, `function = "qup12"` (`sdm845.dtsi:2994-2997`), overridden on OnePlus
  with `drive-strength = <2>` + `bias-disable` (`common.dtsi:724-727` ⇒ no internal pulls; the
  board must supply external SCL/SDA pull-ups — gap G-7). Touch INT/RESET pins: `ts_default_pins
  = ts-int-state { pins = "gpio99", "gpio125"; function = "gpio"; drive-strength <16>;
  bias-pull-up; }` (`common.dtsi:1038-1042`). Pad supply rail (1.8 V vs 3.0 V) not encoded in
  DTS — gap G-6.
- **DMA**: `dmas = <&gpi_dma1 0 4 QCOM_GPI_I2C>, <&gpi_dma1 1 4 QCOM_GPI_I2C>` +
  `dma-names = "tx","rx"` (`sdm845.dtsi:2009-2011`). The mainline DT describes GPI DMA
  channels, but **the Redox port is FIFO-only**: keep `SE_GENI_DMA_MODE_EN` (0x258) bit 0
  clear (§4.1 step 8) and treat `dmas` as data the daemon ignores (gap G-3).
- **Power/ICC**: `power-domains = <&rpmhpd SDM845_CX>`, `operating-points-v2 =
  <&qup_opp_table>`, three `interconnects` paths (`qup-core`, `qup-config`, `qup-memory`)
  (`sdm845.dtsi:2003-2008`). For Redox: CX rail assumed always-on during bring-up (§5);
  interconnect/Opp votes are Linux-only machinery.
- **SMMU**: the *wrapper* carries `iommus = <&apps_smmu 0x6c3 0x0>` (`sdm845.dtsi:1757`,
  `apps_smmu` node `:5211`); the `i2c12` node itself has **no** `iommus`. FIFO-mode MMIO
  needs no SID (§5).

## 2. What firmware leaves configured, and what the driver must (re)do

- **Stock boot chain (XBL/ABL)**: whether ABL leaves the i2c12 SE clocked, idle, or in I2C
  FIFO mode is **not verifiable from the local sources** — the edk2-sdm845 checkout (the only
  boot-firmware source we have) contains no GENI I2C driver; its memory map merely reserves
  `QUPV3_1_GSI = 0xA00000..0xAD0000` as uncacheable Device MMIO, which covers the `0xa90000`
  SE window (`edk2-sdm845 sdm845Pkg/Include/Configuration/DeviceMemoryMap.h:214-216`). Gap
  G-1. **Do not assume any prior state.**
- **U-Boot** (its `geni_i2c.c` exists in the local checkout; it is not enabled for any
  sdm845 board — no sdm845 DTS in `u-boot/dts/upstream`): its probe still re-programs from
  scratch: check SE FW protocol via `GENI_FW_REVISION_RO` (0x68), load SE I2C firmware if
  invalid (`qcom_geni_load_firmware`), check FIFO_IF_DISABLE, read FIFO depth, init, packing,
  default 100 kHz (`u-boot geni_i2c.c:466-543`).
- **OpenBSD** (`obsd qciic_fdt.c`): attach only maps `reg` and sets pinctrl (§3/§4.2) — it
  performs **no** protocol check, clock or watermark re-programming; it presumes the SE was
  left powered and I2C-configured by firmware. That is a *trust assumption we cannot
  reproduce on this phone* (§8). Treat as reference for the polled transfer loop only.
- **Linux mainline** (pmOS kernel): runtime-PM autosuspend after **250 ms** idle disables the
  controller IRQ and gates the SE clocks (`i2c-qcom-geni.c:77`,
  `geni_i2c_runtime_suspend/resume :1244-1276`); on resume a full re-init runs (§4.1).
  Consequence: if the device slept before Redox boots, the SE may be gated/off —
  re-init is mandatory.
- **Required re-init (cold or takeover), mirroring §4.1**:
  1. Poll `SE_GENI_STATUS` (0x40) bits 0 (M_CMD active) and 12 (S_CMD active); abort anything
     in flight before reconfiguring (abort ladder §4.3). U-Boot/OpenBSD skip this only
     because a cold reset is assumed; on takeover (e.g. reboot from pmOS into Redox) do it.
  2. Verify the SE FW protocol register (offset conflict — gap G-2); a wrong/missing I2C SE
     firmware means the port is dead until `qcom,se_fw` blob handling exists (§8 gap G-8).
  3. Ensure the `se` branch clock + CX domain are on (§5); re-write CGC/IO-mux/watermarks;
     re-select FIFO mode (`SE_GENI_DMA_MODE_EN` bit 0 = **0** — it may be 1 if the previous
     OS ran GPI DMA on this SE, since mainline toggles the bit per transfer,
     `qcom-geni-se.c:456-475` + `i2c-qcom-geni.c:530-532`); re-write clock divider + SCL
     counters for the wanted speed (§4.3).
  - Redox `src/dtb/serial.rs`-style `skip-init` (trusting firmware) is NOT recommended here
    for the same reason as in the debug-UART spec: unverifiable prior state.

## 3. Register map (what the minimal driver touches)

Two windows: **wrapper** `qupv3_id_1` (PA `0xac0000`, 0x6000, `sdm845.dtsi:1753`) and **SE**
(PA `0xa90000`, 0x4000, `:1995`). Offsets are SE-relative unless stated. U-Boot and mainline
Linux disagree on two offsets — both are cited (G-2).

| Offset | Register / field | Meaning (minimal driver) | Cite |
|---|---|---|---|
| wrapper +0x4 | `QUPV3_HW_VER` / `QUP_HW_VER_REG` `[31:28].[27:16]` major.minor | wrapper IP version; **≥ 3.10 changes the TX/RX FIFO-depth masks (256-byte FIFOs)** below | `u-boot geni-se.h:27,251-257,270-278`; `u-boot geni_i2c.c:452-463`; `linux qcom-geni-se.c:188,304-310` |
| 0x20 | `GENI_FORCE_DEFAULT_REG` ← `FORCE_DEFAULT` (bit 0) | de-configure SE to default (reset escape) before re-init | `u-boot geni-se.h:32,89`; `u-boot geni_i2c.c:422`; `linux qcom-geni-se.c:342` |
| 0x24 | `GENI_OUTPUT_CTRL` ← `DEFAULT_IO_OUTPUT_CTRL_MSK` (bits 6:0) | enable 7 SE IO mux lines (SDA/SCL routing) | `u-boot geni-se.h:33,93`; `u-boot geni_i2c.c:421`; `linux qcom-geni-se.c:341,247` |
| 0x28 | `GENI_CGC_CTRL` \|= `DEFAULT_CGC_EN` (bits 6:0) | per-clock gating: keep CFG/DATA/SCLK/TX/RX/EXT clocks ungated | `u-boot geni-se.h:34,95-105`; `u-boot geni_i2c.c:417-419`; `linux qcom-geni-se.c:332-334` |
| 0x40 | `SE_GENI_STATUS` bit 0 M_CMD-active, bit 12 S_CMD-active | busy check before abort/re-init | `u-boot geni-se.h:35,107-109`; `u-boot geni_i2c.c:294-296` (poll); `linux i2c-qcom-geni.c:273,323+` (err dump) |
| 0x48 | `GENI_SER_M_CLK_CFG`: bit 0 `SER_CLK_EN`, `[15:4] CLK_DIV` | primary-sequencer SER clock = baud × sampling32; mainline writes **M only** for I2C | `u-boot geni-se.h:36,112-114`; `u-boot geni_i2c.c:135-136`; `linux i2c-qcom-geni.c:256-257` |
| 0x64 | `GENI_IF_DISABLE_RO` bit 0 `FIFO_IF_DISABLE` | if set ⇒ FIFO interface disabled by firmware ⇒ FIFO driver must bail | `u-boot geni-se.h:38,117`; `u-boot geni_i2c.c:518-523`; `linux i2c-qcom-geni.c:1095` |
| 0x68 | `GENI_FW_REVISION_RO` `[15:8]` protocol | protocol id check; `GENI_SE_I2C` == 3 | `u-boot geni-se.h:39,119-121`; `u-boot geni_i2c.c:501-516`; **conflict**: mainline reads `SE_GENI_FW_REVISION`/`_S_` at `0x1000/0x1004` (`linux qcom-geni-se.c:236-237`, used at `i2c-qcom-geni.c:1079`) — resolve on device (G-2) |
| 0x7c | `SE_GENI_CLK_SEL` `[2:0]` | DFS clock-index select (new path) | `u-boot geni-se.h:41,123-124`; `u-boot geni_i2c.c:133` (writes 0); `linux i2c-qcom-geni.c:254` |
| 0x254 | `SE_GENI_BYTE_GRAN` (`bits/16`) | FIFO byte granularity: `8/16 = 0` ⇒ 4×8-bit packing | `u-boot geni-se.h:43`; `u-boot geni_i2c.c:404-405`; `linux i2c-qcom-geni.c:1122-1123` (`PACKING_BYTES_PW 4`, `:78`) |
| 0x258 | `SE_GENI_DMA_MODE_EN` bit 0 | **0 = FIFO (ours)**, 1 = DMA/GPI mode | `u-boot geni-se.h:44,135-136`; `u-boot geni_i2c.c:428-430`; `linux qcom-geni-se.c:385-406,456-475` |
| 0x26c | `SE_I2C_TX_TRANS_LEN` | byte count for the next TX-sequencer command | `linux i2c-qcom-geni.c:24`; `u-boot geni_i2c.c:27`; `obsd qciic_fdt.c:36` |
| 0x270 | `SE_I2C_RX_TRANS_LEN` | byte count for the next RX-sequencer command | `linux i2c-qcom-geni.c:25`; `u-boot geni_i2c.c:28`; `obsd qciic_fdt.c:37` |
| 0x278 | `SE_I2C_SCL_COUNTERS`: HIGH `[29:20]`, LOW `[19:10]`, CYCLE `[9:0]` | SCL high/low/cycle counters driving real bus speed (§4.3) | `linux i2c-qcom-geni.c:26,51-55`; `u-boot geni_i2c.c:29,53-58,138-141` |
| 0x600 | `SE_GENI_M_CMD0`: opcode `[31:27]`, params `[26:0]` | op `I2C_WRITE=1`, `I2C_READ=2` (`WRITE_READ=3`, `ADDR_ONLY=4`, `BUS_CLEAR=6`, `STOP_ON_BUS=7`); params: `STOP_STRETCH` bit 2, slave addr bits `[15:9]` (shift 9) | `linux i2c-qcom-geni.c:33-49`; `u-boot geni_i2c.c:36-52`; `obsd qciic_fdt.c:38-42` |
| 0x604 | `SE_GENI_M_CMD_CTRL_REG`: bit 0 `DISABLE`, bit 1 `ABORT`, bit 2 `CANCEL` | command abort/cancel of the active M-sequencer command (error recovery, §4.3) | `u-boot geni-se.h:50,143-146`; `u-boot geni_i2c.c:291-296`; `linux i2c-qcom-geni.c:441,460` via `geni_se_abort/cancel_m_cmd` (helper header absent — G-5) |
| 0x610/0x614/0x618 | `SE_GENI_M_IRQ_STATUS/EN/CLEAR`: bit0 CMD_DONE, bit1 OVERRUN, bit2 ILLEGAL_CMD, bit3 CMD_FAILURE, bit4 CANCEL, bit5 ABORT, bit10 GP_IRQ_1 (NACK), bit12 GP3 (bus protocol), bit13 GP4 (arb-lost), bit26 RX_FIFO_WM, bit27 RX_FIFO_LAST, bit30 TX_FIFO_WM, bit31 SEC_IRQ — **write-1-to-clear** in STATUS/CLEAR | the whole polled + IRQ handshake lives here (`SE_I2C_ERR` mask = bits 1,2,3,10,12,13) | `u-boot geni-se.h:51-53,158-189`; `linux i2c-qcom-geni.c:28-29,337,349-362,371,386,406-407`; `obsd qciic_fdt.c:43-45,227-228` |
| 0x700 | `SE_GENI_TX_FIFOn` | FIFO write port; pack 4 bytes LSB-first per 32-bit word | `u-boot geni-se.h:59`; `u-boot geni_i2c.c:182`; `obsd qciic_fdt.c:46,187-210` |
| 0x780 | `SE_GENI_RX_FIFOn` | FIFO read port; last word may be partial (§4.2) | `u-boot geni-se.h:60`; `u-boot geni_i2c.c:228`; `linux i2c-qcom-geni.c:376`; `obsd qciic_fdt.c:47,162-184` |
| 0x800 | `SE_GENI_TX_FIFO_STATUS` `[27:0]` = words queued | OpenBSD polls `stat < 16` as "FIFO not full" (hard-codes depth 16) | `u-boot geni-se.h:61,217-218`; `obsd qciic_fdt.c:48,196-204` |
| 0x804 | `SE_GENI_RX_FIFO_STATUS`: `RX_FIFO_WC [24:0]` words, bit 31 `RX_LAST`, `[30:28]` `RX_LAST_BYTE_VALID` | count of valid words/bytes in the last word; drives RX drain math | `u-boot geni-se.h:62,220-224`; `linux i2c-qcom-geni.c:338,372`; `obsd qciic_fdt.c:49-50` (mask `0xffffff`, no LAST decode — G-4) |
| 0x80c | `SE_GENI_TX_WATERMARK_REG` | request granularity in FIFO words; polled path sets **1** per transfer and **0** when done | `u-boot geni-se.h:63`; `u-boot geni_i2c.c:252,163,185`; `linux i2c-qcom-geni.c:366,399-400,587` |
| 0x810 / 0x814 | `SE_GENI_RX_WATERMARK_REG` / `SE_GENI_RX_RFR_WATERMARK_REG` | RX watermark + RFR watermarks; I2C init sets `depth−1` / `depth` | `u-boot geni-se.h:64-65`; `u-boot geni_i2c.c:434-435`; `linux qcom-geni-se.c:372-373, i2c-qcom-geni.c:1120-1121` (`geni_se_init(se, tx_wm, depth)`, `tx_wm = depth−1`) |
| 0x908 | `SE_GENI_IOS` | input-status (SDA/SCL state visible) — diagnostic dumps only | `u-boot geni-se.h:66`; `linux i2c-qcom-geni.c:274` |
| 0xc40/0xc44, 0xc4c, 0xc58; 0xd40…0xd58 | DMA TX/RX IRQ + FSM_RST pairs | DMA-mode only — **unused in FIFO mode**; present so the model doesn't fake them | `u-boot geni-se.h:67-75`; `linux i2c-qcom-geni.c:339-340,410-412,475,487` |
| 0xe18 / 0xe1c | `SE_GSI_EVENT_EN` / `SE_IRQ_EN` (bits 0/1 = DMA RX/TX events, bits 2/3 = GENI M/S IRQ routing) | GSI/event routing (GIC-ITS path) vs raw-IRQ routing; FIFO-on-GIC path keeps GSI events 0, IRQ routing bits on | `u-boot geni-se.h:76-77,129-133`; `u-boot geni_i2c.c:412,424-432`; `linux qcom-geni-se.c:316-319,347,352` |
| 0xe24 | `SE_HW_PARAM_0`: `[21:16]` (or `[23:16]` on ≥3.10) TX FIFO depth; `[29:24]` TX FIFO width | read once for TX FIFO geometry; default fallback **16 words × 32 bit** | `u-boot geni-se.h:78,248-257`; `u-boot geni_i2c.c:448-464`; `linux i2c-qcom-geni.c:1108-1121` (via `geni_se_get_tx_fifo_depth`) |

Cross-cutting register rules (common to all three references):
- All registers are 32-bit little-endian MMIO; FIFO ports too (4 bytes/word, LSB-first pack —
  `obsd qciic_fdt.c:162-210`, `u-boot geni_i2c.c:146-246`, `linux i2c-qcom-geni.c:370-404`).
- The I2C block uses **only the primary (M) sequencer** — S_CMD machinery exists but I2C
  master never triggers S_CMD (`linux qcom-geni-se.c:394` "Non-UART use only primary
  sequencer").
- M_CMD params: `PRE_CMD_DELAY` bit0, `TIMESTAMP_BEFORE` bit1, `STOP_STRETCH` bit2,
  `TIMESTAMP_AFTER` bit3, `POST_COMMAND_DELAY` bit4, `IGNORE_ADD_NACK` bit6,
  `READ_FINISHED_WITH_ACK` bit7, `BYPASS_ADDR_PHASE` bit8 (`linux i2c-qcom-geni.c:40-48`;
  `u-boot geni_i2c.c:43-52`) — the polled path needs only bit2 + addr[15:9].
- Error/timeout budgets differ: U-Boot 100 ms per FIFO op (`u-boot geni_i2c.c:67
  I2C_TIMEOUT_MS`), OpenBSD ≈500 ms (50000×10 µs, `obsd qciic_fdt.c:149-154`), Linux
  1 s/transfer (`XFER_TIMEOUT HZ`, `i2c-qcom-geni.c:82,548,590`). Adopt **100 ms per FIFO
  op + 1 s per transfer** as the Redox budget (never rely on OpenBSD's assumption of a
  depth-16 non-full FIFO without reading `SE_HW_PARAM_0` first — depth is dynamic on ≥3.10,
  `u-boot geni_i2c.c:452-463`).

## 4. Sequences

### 4.1 Init (cold / takeover)

Modelled on U-Boot `geni_i2c_probe` + `geni_i2c_init` (`u-boot geni_i2c.c:466-543,
408-446`), Linux `geni_i2c_init` (`i2c-qcom-geni.c:1069-1131`), and OpenBSD's assumption
that this already happened (`obsd qciic_fdt.c:96-130` — skip it here, §2):

1. **Clocks & power**: enable wrapper `m-ahb`/`s-ahb` + `se` branch clock (`GCC_QUPV3_WRAP1_S4_CLK`:
   enable_reg `0x5200c` bit 26, halt-check `0x184d4` — `gcc-sdm845.c:2368-2381`); CX domain
   on (§5). U-Boot does exactly this via `clk_get_by_name("se")/clk_enable` before touching
   any SE register (`u-boot geni_i2c.c:491-499`).
2. **Read wrapper IP version** (wrapper+0x4) → pick FIFO-depth mask variant; **read
   `SE_HW_PARAM_0` (0xe24)** → TX FIFO depth (words) and width; fall back to 16×32 if the
   read is 0 (`u-boot geni_i2c.c:448-464`; `linux i2c-qcom-geni.c:1108-1112`).
3. **Abort any in-flight command** (takeover case): poll `SE_GENI_STATUS`, else write
   `M_GENI_CMD_ABORT` (0x604 bit1) and poll the self-clearing ABORT-done status bit (§4.3
   ladder) — then continue (mirrors the unconditional-cancel rule cited in the debug-UART
   spec; same hardware rule, `obsd qciic_fdt.c` omits it).
4. **Verify SE FW protocol** = I2C (3) via the FW-revision register — offset per G-2:
   `0x68 [15:8]` (U-Boot) or `0x1000` (mainline `geni_se_read_proto`,
   `i2c-qcom-geni.c:1079-1089`). If firmware must be (re)loaded: Redox has no loader — stop
   and report (§8 G-8). If protocol ok: continue.
5. **Clear IRQ/event state**: write 0 to `SE_GSI_EVENT_EN` (0xe18); all-ones to
   `M_IRQ_CLEAR`/`S_IRQ_CLEAR` (0x618/0x648) and both DMA IRQ-clears (0xc44/0xd44 — harmless
   in FIFO); then re-route IRQs via `SE_IRQ_EN` (0xe1c) setting bits 2/3 (§3)
   (`u-boot geni_i2c.c:412-432`; `linux qcom-geni-se.c:345-353,364-383 geni_se_init/io_init/io_set_mode`).
6. **Mode select**: keep/write `SE_GENI_DMA_MODE_EN` (0x258) bit 0 = **0** (FIFO) even if
   the DTB carries `dmas`; do this *after* the clears, like `geni_se_init/io_set_mode`
   (`linux qcom-geni-se.c:321-323,385-406`).
7. **Watermarks**: `RX_WATERMARK = depth−1`, `RX_RFR_WATERMARK = depth` (0x810/0x814);
   `TX_WATERMARK = 0` until a transfer starts (0x80c)
   (`u-boot geni_i2c.c:434-435`; `linux i2c-qcom-geni.c:1120-1121`).
8. **Packing**: TX and RX `PACKING_CFG0/1` (0x260/0x264, 0x284/0x288) = 8 bits/word,
   4 words/FIFO-entry, MSB→LSB ⇒ canned `CFG0 = 0xf`, `CFG1 = 0`, `BYTE_GRAN(0x254) = 0`
   (`u-boot geni_i2c.c:347-406 geni_i2c_config_packing + :538-539`;
   `linux i2c-qcom-geni.c:1122-1123 geni_se_config_packing(se, 8, 4, true, true, true)`).
9. **Clock/serial timing** (per wanted bus speed, here 400 kHz):
   - `SE_GENI_CLK_SEL (0x7c)` = clock index (U-Boot writes 0; mainline writes a matched DFS
     index `i2c-qcom-geni.c:254,201-246`);
   - `GENI_SER_M_CLK_CFG (0x48)` = `(clk_div << 4) | SER_CLK_EN`;
   - `SE_I2C_SCL_COUNTERS (0x278)` = `(t_high<<20) | (t_low<<10) | t_cycle`.
   400 kHz rows: Linux 19.2 MHz table `{div 2, high 5, low 11, cycle 22}`
   (`i2c-qcom-geni.c:186-191`), U-Boot `{2, 5, 12, 24}` (`u-boot geni_i2c.c:99-103`) —
   conflict, take **Linux's current table** and cross-check on device (G-3). Formula:
   `t_high = (t_high_cnt × clk_div) / f_src`, `t_cycle = (t_cycle_cnt × clk_div) / f_src`
   (`u-boot geni_i2c.c:87-98`; `linux i2c-qcom-geni.c:174-185`).
10. **Leave TX_WATERMARK at 0**; the transfer loop below sets it per-op (this differs from
    UART init where RX re-arm re-issues an S_CMD — I2C needs no persistent S_CMD, §3).

### 4.2 Operate — polled FIFO transfer (what the Redox driver must do first)

Reference behaviour is identical in all three; quoted order of operations differs in one way:
**all three issue the M_CMD *before* filling the FIFO** (command carries `TRANS_LEN`,
stalls until FIFO supplies), and **write TX_WATERMARK=1 before/after the FIFO refill**:
U-Boot writes TRANS_LEN → M_CMD → WATERMARK=1 → polled refill loop (`u-boot
geni_i2c.c:248-263`), Linux writes TRANS_LEN → `geni_se_setup_m_cmd(I2C_READ/WRITE, param)`
→ WATERMARK=1 → waits for completion (`i2c-qcom-geni.c:534-587`), OpenBSD TRANS_LEN →
M_CMD → FIFO writes → CMD_DONE wait (`qciic_fdt.c:226-276`).

Per combined transfer of `num` messages (`i2c_qcom_geni.c:966-989`, `u-boot geni_i2c.c:265-305`):

1. For each message `i`: `m_param = (addr << 9) & SLV_ADDR_MSK`; set `STOP_STRETCH` (bit2)
   on all but the last message (chains msgs with repeated start instead of STOP);
   (`linux i2c-qcom-geni.c:972-974`; `u-boot geni_i2c.c:274-276`; OpenBSD instead clears
   STOP_STRETCH for STOP-type ops — divergence noted, G-4).
2. Clear `M_IRQ_STATUS` (write-1-clear, then re-check; U-Boot re-clears every poll round,
   `u-boot geni_i2c.c:191,239`).
3. **Write message** (RMI4 command/register writes): `TX_TRANS_LEN(0x26c) = len`;
   `M_CMD0 = (1<<27) | m_param` (WRITE opcode 1); `TX_WATERMARK(0x80c) = 1`; then poll
   `M_IRQ_STATUS`: on TX_FIFO_WM bit (30) refill ≤`tx_wm` FIFO words (each word = ≤4 bytes
   LSB-first from the message), when the message is exhausted write `TX_WATERMARK = 0`;
   on CMD_DONE bit (0) → clear and done; on any error bit → §4.3
   (`u-boot geni_i2c.c:146-198,248-255`; `linux i2c-qcom-geni.c:386-404,557-597`).
4. **Read message**: `RX_TRANS_LEN(0x270) = len`; `M_CMD0 = (2<<27) | m_param` (READ
   opcode 2; with STOP_STRETCH cleared on the last op per §4.2.1); then poll
   `M_IRQ_STATUS` for RX_FIFO_WM (26) | RX_FIFO_LAST (27): drain `RX_FIFO_WC` words from
   `RX_FIFO (0x780)`, bytes LSB-first; a word with RX_LAST set contributes only the valid
   byte count from `[30:28]` (0 ⇒ 4); then CMD_DONE → clear
   (`u-boot geni_i2c.c:200-246`; `linux i2c-qcom-geni.c:370-384`; `obsd qciic_fdt.c:162-184`).
5. Timeouts per §3 cross-cutting rules → §4.3 ladder on expiry.
6. SMBus-style emulations (combined write-then-read, `I2C_FUNC_SMBUS_EMUL` minus QUICK) run
   as *chained* msg pairs (STOP_STRETCH between) — `linux i2c-qcom-geni.c:1022-1025`.

RMI4 client note (out of this block, needed by the daemon above it): F34 flash access and
F12 read bursts of 8-64 B fit the FIFO (≤ depth-1 words per command); RMI4 "get function
descriptor" reads are single messages — no multi-desc/GPI path required.

### 4.3 Error recovery (identical ladder in all three refs)

1. On error/timeout, first **cancel**: write `M_CMD_CANCEL (bit2)` to `M_CMD_CTRL (0x604)`
   and wait `M_CMD_CANCEL_EN` (bit4) in `M_IRQ_STATUS` (Linux: `geni_se_cancel_m_cmd` +
   `wait_for_completion(cancel_done)` 1 s, `i2c-qcom-geni.c:449-468`).
2. If cancel times out → **abort**: write `M_CMD_ABORT (bit1)`, wait ABORT-done bit (5) or
   the 1 s abort timeout (`i2c-qcom-geni.c:433-447`; `u-boot geni_i2c.c:287-299` waits on
   `M_CMD_ABORT_EN` then clears the status bits — but see divergence: U-Boot's post-abort
   `writel(status, M_IRQ_STATUS)` targets the *STATUS* register while Linux and OpenBSD
   write to `M_IRQ_CLEAR 0x618` (`obsd qciic_fdt.c:228`; `i2c-qcom-geni.c:407`); copy the
   write-to-CLEAR behaviour and treat the U-Boot write as suspect (G-4)).
3. NACK discrimination: on GP_IRQ_1 (bit10) error, a zero `SE_GENI_M_GP_LENGTH` ⇒ **address
   NACK** (target absent — re-init the bus), else **data NACK** (retry the message) —
   `linux i2c-qcom-geni.c:315-321` (register defined in the absent `geni-se.h` — G-5).
4. Full restart after persistent errors: re-run §4.1 from step 5 (FORCE_DEFAULT escape then
   re-init); optional bus-clear = `I2C_BUS_CLEAR` opcode (6) and `I2C_STOP_ON_BUS` (7)
   (`linux i2c-qcom-geni.c:37-38`).
5. FSM resets (0xc58/0xd58) belong to DMA mode — out of scope for FIFO (G-3).

### 4.4 Shutdown / idle

- Redox daemon on teardown: wait TX drain, write `TX_WATERMARK = 0`, clear all `M_IRQ_*`
  bits; optionally write FORCE_DEFAULT to park the SE (mirrors `linux
  i2c-qcom-geni.c:1236-1242 geni_i2c_shutdown`, which marks the adapter suspended so later
  transfers fail — Redox equivalent: mark the device dead, no HW action needed).
- Idle 10 min → idle-gating (power-collapse) may blank the clock divider + watermark
  registers (mainline's autosuspend is 250 ms, `:77`); **any long idle before the next
  transfer must re-run §4.1 from step 5**, not just poke TRANS_LEN/M_CMD (gap G-8).

## 5. Interrupts / DMA / Power

**Polled first (recommended default).** All three references drive FIFO mode without ever
taking the controller's own IRQ on the fast path (U-Boot's polled loop *polls* the status
bits a driver would otherwise handle in IRQ; OpenBSD's `qciic_fdt_exec` is fully polled and
establishes IRQs only for child ihidev use, `obsd qciic_fdt.c:212-279,281-304`). Touch
reports are short reads, so a 100 ms-timeout polled loop is sufficient for the MVP; the
RMI4 *device* INT (GPIO125) belongs to the client-side protocol layer, not this block.

**IRQ path (phase 2).**
- Line: GICv3 **SPI 357, level-high** (`sdm845.dtsi:2000`; `arm,gic-v3` intc `:5464`,
  default `interrupt-parent = <&intc>` `:34`). Pending bits latch in the SE (`M_IRQ_STATUS`)
  and must be W1C'd to 0x618 *after* FIFO servicing; TX-watermark re-asserts (level) exactly
  as on the UART path (cf. debug-UART spec §5 and `i2c-qcom-geni.c:406-407`).
- Redox plumbing: no kernel-side DTB device discovery exists for I2C in this checkout
  (`redox/kernel src/dtb/{mod,serial,irqchip}.rs` cover serial only) ⇒ if the IRQ path is
  ever needed it must be wired in the daemon via `/scheme/irq` (§6), which must be checked
  against SPI 357 (same u8-numbering caveat as the debug-UART spec §5, applied to a daemon
  this time).

**DMA.** None — deliberate design choice: FIFO mode only. Consequences:
- Keep `SE_GENI_DMA_MODE_EN` 0x258 bit0 = 0 (don't let a `dmas`-carrying DTB mislead the
  port; mainline treats ≥32-byte messages as DMA candidates via `i2c_get_dma_safe_msg_buf`,
  `i2c-qcom-geni.c:528,567`, and would flip the bit — the Redox daemon must never).
- No GPI channel request (`setup_gpi_dma/release_gpi_dma`, `i2c-qcom-geni.c:1032-1067`), no
  GSI event routing (0xe18 stays 0; phase-2 raw-IRQ path routes only M/S IRQs through
  0xe1c bits 2/3).
- **SMMU**: nothing translates this block's traffic in FIFO mode; the wrapper's `iommus =
  <&apps_smmu 0x6c3 0x0>` (`sdm845.dtsi:1757`) and `dma_chann