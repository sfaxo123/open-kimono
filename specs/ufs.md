# SDM845 UFS host controller — read-only core (K10)

Scope: enough of UFSHCI 2.x + the `qcom,sdm845-ufshc` vendor bits to bring the link up, read
descriptors, enumerate logical units, issue READ(10)/READ CAPACITY(10) and parse the GPT — **no
media writes, no persistent device writes**. Crate: `crates/ufs` (`ufs-core`, MIT, `no_std` + alloc).

Revisions cited (path:line):
- **FreeBSD** `sys/dev/ufshci/` @ `81f86c9a9417d7ad7445d46739e19a26ab916e6b`, BSD-2-Clause — primary reference.
- **Fuchsia** `src/devices/block/drivers/ufs/` @ `729cefa69788350583738b1e5890f10710f64dff`, BSD-3-style — cross-check.
- **Linux** @ `5b0764a53235d3c77e1ed674e836e1b3fe1a4b1f` (DT, `drivers/ufs/`, `include/ufs/unipro.h` +
  `include/ufs/ufshci.h` fetched from kernel.org at the same HEAD), GPL-2.0 — **oracle/facts only**.
- **U-Boot** @ `8649a54d301a4d0ac526324c7a7ad2574939ff27` — `drivers/ufs/ufs-qcom.c` (GPL-2.0-only),
  `drivers/iommu/qcom-hyp-smmu.c` (GPL-2.0+), `phy-qcom-qmp-ufs.c` (GPL-2.0) — **oracle/facts only**.

## 1. Block

| Item | Value | Source |
|---|---|---|
| Node | `ufs_mem_hc: ufshc@1d84000` | `arch/arm64/boot/dts/qcom/sdm845.dtsi:2619` |
| compatible | `"qcom,sdm845-ufshc", "qcom,ufshc", "jedec,ufs-2.0"` | `sdm845.dtsi:2620-2621` |
| reg `std` | `0x01d84000` size `0x2500` (UFSHCI + Qualcomm vendor regs) | `sdm845.dtsi:2622` |
| reg `ice` | `0x01d90000` size `0x8000` (inline crypto; **unused**) | `sdm845.dtsi:2623-2624` |
| interrupt | GIC SPI 265, level-high (virq ≥ 256 → needs K6 wide IRQs; polling first) | `sdm845.dtsi:2625` |
| PHY | `ufs_mem_phy: phy@1d87000`, `qcom,sdm845-qmp-ufs-phy`, reg `0x01d87000/0x1000` | `sdm845.dtsi:2696-2698` |
| lanes | `lanes-per-direction = <2>` | `sdm845.dtsi:2628` |
| power domain / reset | `gcc UFS_PHY_GDSC`; reset `GCC_UFS_PHY_BCR` ("rst"); PHY reset is the HC's reset 0 | `sdm845.dtsi:2629-2632,2709-2710` |
| IOMMU | `apps_smmu 0x100 0xf` | `sdm845.dtsi:2634` |
| clocks | core (AXI), bus_aggr, iface (AHB), core_clk_unipro, ref (CXO), tx/rx symbol, ice | `sdm845.dtsi:2636-2655` |
| OPP | top OPP: core_clk 200 MHz, **core_clk_unipro 150 MHz**, ice 300 MHz | `sdm845.dtsi:2681-2692` |
| OnePlus 6 | `status = "okay"`, `reset-gpios = <&tlmm 150 GPIO_ACTIVE_LOW>`, `vcc-supply = <&vreg_l20a_2p95>` | `sdm845-oneplus-common.dtsi:908-915` |
| PHY supplies | `vdda-phy` = L1A 0.88 V, `vdda-pll` = L26A 1.2 V | `sdm845-oneplus-common.dtsi:917-922,328-334,401-405` |

The OnePlus 6 DT declares **no `vccq`/`vccq2` supply**; firmware keeps them on. Root filesystem of
the pmOS install (LUKS) lives on this device — a misbehaving writer destroys the oracle.

## 2. What firmware leaves configured — and what the driver (re)does

- **U-Boot** (our chainloader) probes UFS before booting: `board/qualcomm/qcom-phone.env:12`
  (`preboot=scsi scan`); `configs/qcom_defconfig:115,145-146` enable `PHY_QCOM_QMP_UFS`, `UFS`, `UFS_QCOM`.
  It therefore powers the GDSC, enables clocks, programs the **QMP PHY** (`phy-qcom-qmp-ufs.c:184-260`
  sdm845 tables) and completes link startup.
- U-Boot's SMMU driver (`configs/qcom_defconfig:98`) gives each probed `iommus` master an SMR +
  S2CR of type TRANS into a context bank configured as S1-bypass (`drivers/iommu/qcom-hyp-smmu.c:235-268`,
  comment at :250 — S2CR BYPASS is turned into FAULT by the hypervisor). Its remove hook does
  nothing (`:374-383`), so **UFS DMA with physical addresses keeps working after ExitBootServices**.
  ABL→Linux without U-Boot is not covered here.
- **Not done by this driver (dependencies):** GDSC/clock enable, regulator control, the GCC BCR
  `core_reset` and the QMP PHY init/calibration (Linux does both in `ufs-qcom.c:436-476` and
  `:488-551` *before* HCE). We must **not** assert `GCC_UFS_PHY_BCR` — it also resets the PHY, which
  we cannot re-initialise. The device reset GPIO (TLMM 150) is left as firmware set it.
- **Done by this driver:** HCE disable→enable (resets UniPro + UTP, keeps PHY registers), QUniPro
  select, post-enable PHY-alive check, Qualcomm pre-link-startup DME setup, `DME_LINKSTARTUP`,
  UTRL programming, NOP, `fDeviceInit`, descriptor reads, SCSI reads. The driver stays in the
  post-link-startup PWM-G1 power mode (no gear switch) — slow but sufficient for GPT + boot reads.

## 3. Register map used (offsets from `std` base)

UFSHCI standard (FreeBSD `ufshci_reg.h`, struct at :14-73, fields at the cited lines):

| Off | Reg | Fields used | Cite |
|---|---|---|---|
| 0x00 | CAP | NUTRS [7:0] (slots−1), 64AS [24], LSDBS [29] (1 = legacy single doorbell **not** supported) | `ufshci_reg.h:99-125`, `ufshci_ctrlr.c:380-391` |
| 0x08 | VER | MNR [7:4], MJR [15:8] (diagnostic only) | `ufshci_reg.h:146-156` |
| 0x20 | IS | UTRCS 0, UE 2, ULSS 8, UCCS 10, DFES 11, UTPES 12, HCFES 16, SBFES 17 (W1C) | `ufshci_reg.h:188-231` |
| 0x24 | IE | written 0 (polling) | `ufshci_ctrlr.c:268`, Fuchsia `ufs.cc:812` |
| 0x30 | HCS | DP 0, UTRLRDY 1, UCRDY 3 | `ufshci_reg.h:283-301` |
| 0x34 | HCE | HCE 0 | `ufshci_reg.h:304-311` |
| 0x50/0x54 | UTRLBA/UTRLBAU | UTRL base (1 KiB aligned) | `ufshci_reg.h:337-347`, `ufshci_req_sdb.c:302-305` |
| 0x58 | UTRLDBR | doorbell, bit per slot; HW clears on completion | `ufshci_req_sdb.c:542-571` |
| 0x5C | UTRLCLR | write **0** in a slot bit to abort it (Linux writes `~mask`) | Linux `core/ufshcd.c:875-890` |
| 0x60 | UTRLRSR | run/stop = 1 | `ufshci_req_sdb.c:473-474` |
| 0x90 | UICCMD | opcode: DME_GET 0x01, DME_SET 0x02, DME_PEER_GET 0x03, LINK_STARTUP 0x16 | `ufshci.h:79-94` |
| 0x94/98/9C | UCMDARG1/2/3 | ARG1 = `attr<<16 \| selector`; ARG2 [7:0] = config result code; ARG3 = value | `ufshci_reg.h:440-465`, Linux `include/ufs/ufshci.h:316-318` |

UTRLCNR (0x64) is a UFSHCI 3.0+ register; SDM845 reports itself as 2.0 (`ufs-qcom.c:1302-1305`) — never written.

Qualcomm vendor (Linux `drivers/ufs/host/ufs-qcom.h`, facts only):

| Off / attr | Name | Use | Cite |
|---|---|---|---|
| 0xE4 | `REG_UFS_HW_VERSION` | major [31:28], minor [27:16], step [15:0] — read, recorded | `ufs-qcom.h:20-22,110` |
| 0xDC | `REG_UFS_CFG1` | bit0 `QUNIPRO_SEL` set (RMW) before HCE | `ufs-qcom.h:108,167`, `ufs-qcom.c:425-431` |
| 0xD8 | `REG_UFS_CFG0` | bit5 `QUNIPRO_G4_SEL` cleared only when HW major ≥ 5 (not SDM845) | `ufs-qcom.h:107,164`, `ufs-qcom.c:429-430` |
| 0xC0 | `REG_UFS_SYS1CLK_1US` | only mandatory for HW major ≥ 4 → skipped on SDM845 | `ufs-qcom.c:652-668` |
| DME 0x0041 sel lane | `MPHY_TX_FSM_STATE` | must read 1 (`TX_FSM_HIBERN8`) after HCE → PHY alive | `ufs-qcom.h:13-14`, `ufs-qcom.c:385-423` |
| DME 0xD002 | `DME_VS_CORE_CLK_CTRL` | [7:0] = core_clk_unipro MHz (150), clear bit 8 `CORE_CLK_DIV_EN` | `ufs-qcom.h:205,209-211`, `ufs-qcom.c:1798-1868` |
| DME 0x1561 | `PA_ConnectedTxDataLanes` | lane count for the LCC loop | `unipro.h:156`, FreeBSD `ufshci.h:30` |
| DME 0x002C sel lane | `TX_LCC_ENABLE` | set 0 per TX lane before link startup | `unipro.h:26`, `core/ufshcd.c:5233-5260`, `ufs-qcom.c:733-739` |

## 4. Data structures and sequences

**UTRD** (32 B, little-endian; FreeBSD `ufshci.h:185-223`, fill at `ufshci_req_queue.c:425-456`):
DW0 `[31:28]` command type 1, `[26:25]` direction (0 none, 2 device→host; **1 = host→device is
refused by this driver**), `[24]` interrupt; DW2 `[7:0]` OCS (preset 0x0F, 0 = success); DW4/5 UCD
base (128 B aligned); DW6 response length/offset in dwords; DW7 PRDT entries/offset in dwords.
**UCD** layout chosen: request UPIU @0, response UPIU @512 (128 dwords), PRDT @1024; one PRDT entry
`{addr lo, addr hi, 0, byte_count−1}` (`ufshci.h:248-263`, `ufshci_req_queue.c:341-374`).

**UPIUs** (big-endian multi-byte fields; header `ufshci.h:323-361`):
- NOP OUT `0x00` → NOP IN `0x20` (`ufshci.h:626-643`, `ufshci_ctrlr_cmd.c:43-66`).
- QUERY REQUEST `0x16` / RESPONSE `0x36`: function byte5 (0x01 read, 0x81 write); byte12 opcode
  (1 read desc, 3 read attr, 5 read flag, 6 set flag), 13 IDN, 14 index, 15 selector, 18-19 length,
  20-23 value; response data segment @32 (`ufshci.h:494-623`, `ufshci_ctrlr_cmd.c:68-98`).
- COMMAND `0x01` / RESPONSE `0x21`: flags byte1 = 0x40 for reads (R in `[7:4]` = 0x4), byte2 LUN,
  byte3 task tag, bytes 12-15 expected length, 16-31 CDB; response byte6 = response, byte7 = SCSI
  status, sense length @32, sense @34 (`ufshci.h:384-431`, `ufshci_sim.c:186-198`).
- Response checks: OCS == 0, transaction code matches, task tag matches, response == 0
  (`ufshci_req_queue.c:144-165,283-285`).

**Init** (polled; every wait is a bounded poll loop with a delay hook):
1. Read CAP, VER, (qcom) HW_VERSION. Require single-doorbell support; require 64AS if any DMA
   address ≥ 4 GiB (`ufshci_ctrlr.c:373-391`).
2. IE ← 0. If HCE.0: HCE ← 0, poll HCE.0 == 0 (`ufshci_ctrlr.c:188-220`, Fuchsia `ufs.cc:810-821`).
3. qcom: CFG1 |= QUNIPRO_SEL (`ufs-qcom.c:425-431`, called from the HCE pre-change hook `:589-605`).
4. HCE ← 1, delay ≥ 100 µs, poll HCE.0 == 1 (`ufshci_ctrlr.c:222-260`; Linux `core/ufshcd.c:5153-5199`).
5. qcom: poll DME_GET `MPHY_TX_FSM_STATE`(lane 0) == HIBERN8 → else `PhyNotReady`
   (`ufs-qcom.c:385-423,606-611`). This is the check that the firmware-initialised PHY survived.
6. qcom pre-link-startup: DME_VS_CORE_CLK_CTRL RMW (step 3 table), then DME_GET
   `PA_ConnectedTxDataLanes` and DME_SET `TX_LCC_ENABLE`(lane i) = 0 per lane (`ufs-qcom.c:715-739`).
7. UIC `DME_LINKSTARTUP`: poll HCS.UCRDY, write ARG1..3 = 0, UICCMD = 0x16, poll IS.UCCS, W1C it,
   ARG2[7:0] must be 0 (`ufshci_uic_cmd.c:105-235`). Then HCS.DP must be 1 (`ufshci_ctrlr.c:290-299`).
8. W1C any residual IS bits (Fuchsia `ufs.cc:963-964`). UTRLBA/UTRLBAU ← list base; HCS.UTRLRDY
   must be 1; UTRLDBR must read 0; UTRLRSR ← 1 (`ufshci_req_sdb.c:392-489`).
9. NOP OUT/IN, retried up to 10× (Linux `core/ufshcd.c:57-59`; FreeBSD `ufshci_ctrlr.c:544-562`).
10. QUERY SET_FLAG `fDeviceInit` (IDN 0x01), poll READ_FLAG until 0 (`ufshci_dev.c:195-229`).
11. READ DESCRIPTOR device (IDN 0), geometry (IDN 7) → `bNumberLU` @0x06, `bMaxNumberLU` @0x0C
    (0 → 8 LUs, 1 → 32) (`ufshci.h:688-737,839-887`).
12. For LUN 0..max: READ DESCRIPTOR unit (IDN 2, index = LUN): `bLUEnable` @0x03,
    `bLUWriteProtect` @0x05, `bLogicalBlockSize` @0x0A (log2), `qLogicalBlockCount` @0x0B
    (`ufshci.h:894-918`; enumeration as Fuchsia `ufs.cc:1276-1347`).

**Transfer** (one slot, slot 0): build UCD + UTRD, UTRLDBR ← 1<<slot, poll until the bit clears while
checking IS for UE/DFES/UTPES/HCFES/SBFES; W1C IS.UTRCS; then check OCS + response
(`ufshci_req_queue.c:688-762`). **Timeout recovery:** UTRLCLR ← !(1<<slot), poll UTRLDBR bit clear
(bounded), return `Timeout` (Linux `core/ufshcd.c:875-890`). Fatal IS bits → `Fatal(is)`, no retry.

**SCSI (read-only allowlist):** TEST UNIT READY (0x00), REQUEST SENSE (0x03), INQUIRY (0x12),
READ CAPACITY(10) (0x25), READ(10) (0x28). UFS has no READ(12) (`ufshci_sim.c:133-136`; Fuchsia
`ufs.cc:1337-1340`). CHECK CONDITION returns sense key/ASC/ASCQ; UNIT ATTENTION on the first
command after init is retried once.

**GPT:** READ(10) LBA 1 → header (`"EFI PART"`, revision, header size, header CRC32 with the CRC field
zeroed, entries LBA/count/size, entries CRC32), then entry array; entries with a zero type GUID
are skipped. Block size comes from READ CAPACITY (OnePlus 6 LUs use 4096 B).

**Shutdown:** leave the controller enabled (Linux/U-Boot can re-init). No SSU, no hibernate.

## 5. Interrupts / DMA / power

- Polling only. IRQ path later: SPI 265 → GICv3 (virq ≥ 256 needs `op6/irq-width`, K6); enable
  IE.UTRCE/UEE/fatal bits as FreeBSD `ufshci_ctrlr.c:301-311`.
- DMA: UTRL (1 KiB aligned), UCD (128 B aligned), data buffer (dword aligned, ≤ 256 KiB per PRDT
  entry) in one physically contiguous uncached region; phys == bus address only because U-Boot left
  the SMMU stream in S1-bypass (§2). With ABL-only boot, verify `apps_smmu` SMR/S2CR first.
- Power: none touched. Regulators, GDSC, clocks, interconnect votes stay as firmware left them.

## 6. Redox interface

User-space daemon `ufsd` (later; this task delivers the core): map `std` via
`/scheme/memory/physical@uc` (0x1d84000, 0x3000 pages), allocate DMA with the base `common::dma`
helper, read `/scheme/kernel.dtb` and skip startup when `qcom,sdm845-ufshc` is absent (K8 `spmid`
gotcha). Expose enumerated LUs read-only as `disk.ufs-lun<N>` via the existing `driver-block`
scheme. Kernel piece: none.

## 7. Verification

- **Host (done):** `crates/ufs` tests — a register + UTRD/UPIU device model, a recorded golden MMIO
  trace replayed strictly, and negative tests (link startup failure, PHY not ready, OCS error, SCSI
  CHECK CONDITION, doorbell timeout + UTRLCLR, fatal IS, write opcode refused, bad GPT CRC).
- **Oracle capture (pmOS phone, passive only — root is on this device; never reset the HC there):**
  1. Registers: `busybox devmem 0x1d84000 32` for offsets 0x00-0x9C and 0xC0-0xFC, plus 0xE4
     (HW version) → convert to `mmio-replay` read events.
  2. `cat /sys/kernel/debug/clk/clk_summary | grep -i ufs` (core_clk_unipro rate — must be 150 MHz
     for the DME_VS_CORE_CLK_CTRL value used here).
  3. `/sys/bus/platform/devices/1d84000.ufshc/{device_descriptor,geometry_descriptor,power_info}/*`,
     `/sys/class/scsi_device/*/device/unit_descriptor/*` → expected descriptor fields + LU table.
  4. `cat /sys/block/sd*/queue/logical_block_size`, `cat /proc/partitions`, and
     `dd if=/dev/sda bs=4096 skip=1 count=1 | xxd` (GPT header, read-only) per LU.
  5. ftrace `events/ufs/{ufshcd_upiu,ufshcd_uic_command,ufshcd_command}` during ordinary reads →
     real UPIU headers, task tags, OCS.
  6. U-Boot console after `scsi scan`: `md.l 0x1d84000 0x40` — the **exact** state Redox inherits
     (HCE, HCS.DP, UTRLRSR, CFG1). This is the capture that decides whether step 2's HCE cycle is
     safe on hardware.
- **On-phone acceptance:** `fastboot boot` U-Boot → Redox EFI; `ufsd` logs HW version, LU table and
  GPT partition names matching the oracle's `/proc/partitions`; no write UPIU appears.

## 8. License path and gaps

MIT clean-room in `crates/ufs`: structure layouts and sequences follow FreeBSD (BSD-2) and Fuchsia
(BSD) — no code copied. Qualcomm vendor facts from Linux/U-Boot (GPL) are register offsets, bit
positions and ordering only. Gaps / conflicts:
- **HCE cycle vs PHY:** Linux always re-runs PHY init around HCE; whether the QMP PHY survives an HCE
  toggle with U-Boot's settings is unproven (step 5 detects it; oracle capture 6 decides).
- **No gear switch:** stays in PWM-G1 after link startup. HS-G3 needs PA_* power-mode change
  (FreeBSD `ufshci_dev.c:320-430`) plus Qualcomm PHY rate settings.
- **core_clk_unipro = 150 MHz** is the DT top OPP; firmware may run it lower (oracle capture 2).
- `fDeviceInit` SET_FLAG is the only Query write; it is volatile protocol state (no media change).
  No WRITE DESCRIPTOR/ATTRIBUTE, no bRefClkFreq write (FreeBSD writes it if wrong, `ufshci_dev.c:240-271`).
- UFSHCI 3.0+ registers (UTRLCNR, MCQ) are not used; Task Management (UTMRL) is not set up, so no
  ABORT TASK — a stuck slot goes straight to UTRLCLR.
- ICE (inline crypto) disabled/untouched; if firmware left crypto enabled, OCS 0x09/0x0A would show.
