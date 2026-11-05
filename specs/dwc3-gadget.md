# DWC3 device mode (USB gadget) — OnePlus 6 `usb_1` (K11)

Goal: the phone enumerates as a USB 2.0 high-speed device exposing CDC-NCM (network) and CDC-ACM
(serial) to a host, through a polled DWC3 device-mode core implementing the `embassy-usb-driver`
traits. Host-tested against a scripted controller model; the phone run is a later hardware gate.

Source revisions (all line numbers below refer to these):

| Tag | Repository @ commit | License |
|---|---|---|
| `FX` | Fuchsia `src/devices/usb/drivers/dwc3/` @ `729cefa69788350583738b1e5890f10710f64dff` | BSD-3-Clause |
| `DT` | Linux `arch/arm64/boot/dts/qcom/` @ `5b0764a53235d3c77e1ed674e836e1b3fe1a4b1f` | GPL-2.0 (facts only) |
| `LX` | Linux `drivers/usb/dwc3/` same commit | GPL-2.0 (oracle only, no code) |
| `OB` | OpenBSD `sys/dev/fdt/qcdwusb.c` @ `20cb34434d36871c39729f7369062cba28fd06cb` | ISC |
| `EU` | crates.io `embassy-usb-driver` 0.2.2, `embassy-usb` 0.6.0 | MIT OR Apache-2.0 |

## 1. Block

- Glue node `usb_1: usb@a6f8800`, `compatible = "qcom,sdm845-dwc3", "qcom,dwc3"`, `reg = <0x0a6f8800 0x400>`
  (QSCRATCH) — `DT sdm845.dtsi:4189-4191`. Clocks `cfg_noc, core, iface, sleep, mock_utmi`
  (`:4198-4207`), assigned rates mock_utmi 19.2 MHz / master 150 MHz (`:4209-4211`), interrupts
  `pwr_event` SPI 130, `hs_phy_irq` SPI 131, PDC 9/8/6 (`:4213-4222`), power domain `USB30_PRIM_GDSC`
  (`:4224`), reset `GCC_USB30_PRIM_BCR` (`:4226`).
- Core node `usb_1_dwc3: usb@a600000`, `compatible = "snps,dwc3"`, `reg = <0x0a600000 0xcd00>`, core IRQ
  **SPI 133** level-high, `iommus = <&apps_smmu 0x740 0>` — `DT sdm845.dtsi:4232-4236`. Quirks
  `snps,dis_u2_susphy_quirk`, `snps,dis_enblslpm_quirk` (`:4237-4238`).
- OnePlus 6: `&usb_1 { qcom,select-utmi-as-pipe-clk; }` (`DT sdm845-oneplus-common.dtsi:924-932`);
  `&usb_1_dwc3 { dr_mode = "peripheral"; maximum-speed = "high-speed"; phys = <&usb_1_hsphy>; }`
  (`:934-946`). USB3/QMP PHY unused.
- HS PHY `usb_1_hsphy: phy@88e2000`, `qcom,sdm845-qusb2-phy`, `reg = <0x088e2000 0x400>`
  (`DT sdm845.dtsi:4080-4092`), tuned by `sdm845-oneplus-common.dtsi:948-958`. **Assumed
  firmware-initialised** (ABL/U-Boot fastboot uses it); this driver never touches it.
- QSCRATCH is the glue window at core base + `0xf8800` (`LX dwc3-qcom.c:41`, `FX dwc3.cc:99` uses the
  flat offset `0xf8810` for `HS_PHY_CTRL`).

## 2. What firmware leaves configured

ABL (and U-Boot `fastboot`) run this controller in device mode, so clocks, GDSC, QUSB2 PHY tuning and
pinless VBUS signalling are expected live at hand-off. The driver must still: soft-reset the core,
re-select device mode, program the event buffer, configure EP0, and assert the QSCRATCH VBUS override
before setting RUN/STOP. **Unverified (oracle gate):** clock/GDSC state after U-Boot hands off to the
Redox EFI payload, `GUSB2PHYCFG` value, whether `apps_smmu` SID 0x740 is bypass or translated.

## 3. Register map (core offsets from `0x0a600000`)

| Reg | Off | Fields used | Cite |
|---|---|---|---|
| GSBUSCFG0 | c100 | not touched (firmware value kept) | FX regs.h:396-413 |
| GCTL | c110 | PRTCAPDIR[13:12]=2 device | FX regs.h:13-35 |
| GSNPSID | c120 | core_id[31:16] 0x5533 (usb3) / 0x3331 (usb31) | FX regs.h:450-455; dwc3.cc:971-1013 |
| GUCTL1 | c11c | DEV_FORCE_20_CLK_FOR_30_CLK bit 26 (HS-only) | FX regs.h:54-82; dwc3.cc:1020-1024 |
| GHWPARAMS3 | c14c | NUM_IN_EPS[22:18], NUM_EPS[17:12] | FX regs.h:157-167; dwc3.cc:806-818 |
| GUSB2PHYCFG(0) | c200 | SUSPENDUSB20 bit 6 = 0, ENBLSLPM bit 8 = 0, USBTRDTIM[13:10] = 9 | FX regs.h:98-119; dwc3.cc:1126-1132 |
| GEVNTADR(0) | c400/c404 | event ring bus address lo/hi | FX regs.h:170-174 |
| GEVNTSIZ(0) | c408 | EVNTINTRPTMASK bit 31, EVENTSIZ[15:0] bytes | FX regs.h:177-182 |
| GEVNTCOUNT(0) | c40c | EVNTCOUNT[15:0] bytes pending; write = bytes consumed | FX regs.h:185-190; events.cc:231-244 |
| DCFG | c700 | DEVSPD[2:0]=0 HS, DEVADDR[9:3], NUMP[21:17] | FX regs.h:193-209 |
| DCTL | c704 | RUN_STOP bit 31, CSFTRST bit 30 | FX regs.h:212-229 |
| DEVTEN | c708 | DISCONN 0, USBRST 1, CONNECTDONE 2, WKUP 4, U3L2L1Susp 6 | FX regs.h:232-249 |
| DSTS | c70c | DEVCTRLHLT bit 22, CONNECTSPD[2:0] (0 HS, 1 FS) | FX regs.h:252-295 |
| DALEPENA | c720 | bit n = physical endpoint n active | FX regs.h:298-312 |
| DEPCMDPAR2/1/0(n) | c800/c804/c808 + n·0x10 | command parameters | FX regs.h:315-333 |
| DEPCMD(n) | c80c + n·0x10 | CMDTYP[3:0], CMDIOC bit 8, CMDACT bit 10, CMDSTATUS[15:12], COMMANDPARAM[31:16] | FX regs.h:373-393 |

DEPCFG parameters: PAR0 `EP_TYPE[2:1] MAX_PACKET_SIZE[13:3] FIFO_NUM[21:17] ACTION[31:30]` (0 init, 2
modify); PAR1 `XFER_COMPLETE_EN 8, XFER_NOT_READY_EN 10, INTERVAL[23:16], EP_NUMBER[29:25]` —
`FX regs.h:336-370`, `commands.cc:61-91`. FIFO_NUM is 0 for OUT and EP0-IN, else `phys >> 1`
(`commands.cc:66-67`). DEPXFERCFG PAR0 = 1 (`commands.cc:94-104`). DEPSTRTXFER PAR0 = TRB address
high, PAR1 = low (`commands.cc:107-131`); the transfer resource index is read back from
`DEPCMD[22:16]` once CMDACT clears (oracle fact `LX gadget.h:131-137`, `gadget.c:438-444`) — Fuchsia
instead sets CMDIOC and takes it from the command-complete event; the polled driver uses the
readback and does not set CMDIOC. DEPSTARTCFG COMMANDPARAM 0 for EP0, 2 at
SET_CONFIGURATION (`commands.cc:34-58`, `ep0.cc:96-109`). DEPENDXFER COMMANDPARAM = resource index,
HIPRI_FORCERM bit 11 (`commands.cc:157-207`).

Physical endpoint `n = (ep_number << 1) | is_in` (EP0-OUT = 0, EP0-IN = 1) — `FX ep0.cc:52-53`,
`commands.cc:66`.

QSCRATCH (offsets from `0x0a6f8800`):

| Reg | Off | Field | Cite |
|---|---|---|---|
| GENERAL_CFG | 0x08 | PIPE_UTMI_CLK_SEL bit 0, PIPE3_PHYSTATUS_SW bit 3, PIPE_UTMI_CLK_DIS bit 8 | LX dwc3-qcom.c:33-36 (oracle fact) |
| HS_PHY_CTRL | 0x10 | UTMI_OTG_VBUS_VALID bit 20, SW_SESSVLD_SEL bit 28 | FX dwc3.cc:95-100 (bit 20); LX dwc3-qcom.c:26-28 (bit 28) |
| SS_PHY_CTRL | 0x30 | LANE0_PWR_PRESENT bit 24 | LX dwc3-qcom.c:30-31 (oracle fact) |

TRB (16 bytes, little-endian): `ptr_lo, ptr_hi, status{BUFSIZ[23:0], SPR 26, TRBSTS[31:28]},
control{HWO 0, LST 1, CHN 2, CSP 3, TRBCTL[9:4], ISP 10, IOC 11}`; TRBCTL 1 normal, 2 setup,
3 status-2, 4 status-3, 5 control-data — `FX types.h:9-48`.

Events (32-bit): bit 0 = 0 endpoint event `{PHYS_EP[5:1], TYPE[9:6], STATUS[15:12], PARAMS[31:16]}`,
types 1 XferComplete, 2 XferInProgress, 3 XferNotReady (stage bits 13:12: 1 data, 2 status; bit 15
"transfer active"), 7 EndpointCommandComplete (cmd type `[27:24]`, resource index `[22:16]`);
bit 0 = 1 device event `{TYPE[14:8], INFO[31:16]}`, types 0 disconnect, 1 USB reset, 2 connect done,
3 link state, 4 wakeup, 6 suspend entry, 11 event-buffer overflow — `FX types.h:50-122`.

## 4. Sequences

**Init** (`FX dwc3.cc:785-818, 1016-1063`): (1) read GSNPSID, accept core id 0x5533/0x3331 else fail;
(2) read GHWPARAMS3, validate `2 ≤ NUM_EPS ≤ 32`, `1 ≤ NUM_IN_EPS ≤ 16`, OUT count ≥ 1; (3) HS-only:
set GUCTL1 bit 26 before reset (`:1020-1024`); (4) DCTL ← RUN_STOP 0, CSFTRST 1; poll until CSFTRST
clears, bounded (`:1026-1037`); (5) GCTL.PRTCAPDIR ← device (`:1041`); (6) GEVNTSIZ ← mask
(`:1045`); (7) GUCTL1 bit 26 again (`:1084-1086`).

**Glue** (driver `connect`): if `qcom,select-utmi-as-pipe-clk`: GENERAL_CFG set bit 8, ~100 µs, set
bits 0|3, ~100 µs, clear bit 8 (oracle sequence `LX dwc3-qcom.c:422-437`); then VBUS override:
SS_PHY_CTRL |= bit 24, HS_PHY_CTRL |= bits 20|28 (`LX :116-129`; Fuchsia sets only bit 20 at
start/resume, `FX dwc3.cc:141`). Clear the same bits on disconnect (`FX dwc3.cc:128,133`).

**Start peripheral** (`FX dwc3.cc:1114-1167`): GUSB2PHYCFG clear SUSPENDUSB20/ENBLSLPM, USBTRDTIM 9
(`:1126-1132`, DT quirks agree); DCFG DEVSPD 0 (HS), DEVADDR 0 (`:1150-1158`); event buffer
GEVNTADR lo/hi, GEVNTSIZ = size (unmasked), GEVNTCOUNT = 0, DEVTEN (`events.cc:261-300`); EP0:
DEPSTARTCFG(0), DEPCFG + DEPXFERCFG on phys 0 and 1, DALEPENA bits 0|1, SETUP TRB (8 bytes) +
DEPSTRTXFER on phys 0 (`ep0.cc:58-93`, `endpoints.cc:13-81`); DCTL.RUN_STOP ← 1 and poll
DSTS.DEVCTRLHLT = 0 (bounded).

**Events** (`FX events.cc:207-245`): read GEVNTCOUNT; for each 4 bytes read the ring word at the
software read offset (wraps at EVENTSIZ), dispatch, write GEVNTCOUNT = bytes consumed. Linux writes
the plain count (`LX gadget.c:4743`); Fuchsia also sets bit 31 (`EVNT_HANDLER_BUSY`, usb31 only) —
we write the plain count.

**USB reset** (`FX dwc3.cc:1190-1224`): end active non-EP0 transfers, clear stalls, DCFG.DEVADDR ← 0,
restart EP0 SETUP if not armed. **Connect done** (`:1226-1285`): DSTS.CONNECTSPD → EP0 MPS 64 for
HS/FS, DEPCFG(modify) on phys 0 and 1. SuperSpeed is refused (this phone is USB2 only).

**Control transfers** (`FX ep0.cc:111-393`): XferComplete on phys 0 in Setup state → setup packet in
EP0 buffer. Three-stage OUT: CONTROL_DATA TRB on phys 0 sized `round_up(wLength, MPS)`
(`:171-185`, OUT TRB lengths must be MPS multiples). Three-stage IN: CONTROL_DATA TRB on phys 1;
if `transferred % MPS == 0 && transferred < wLength` queue a ZLP (`:273-279`). Status: the
controller raises XferNotReady(stage=status) on the status direction; respond with STATUS_2 TRB
(two-stage) or STATUS_3 TRB (three-stage) (`:336-380`); XferComplete of the status → queue SETUP.
Stall: end transfers, DEPSSTALL on phys 0, queue SETUP (`:422-425`). SET_ADDRESS writes
DCFG.DEVADDR before the status stage (`:440-443`). SET_CONFIGURATION issues DEPSTARTCFG(2) before
endpoints > 1 are configured (`:96-109`).

**Bulk/interrupt** (`FX endpoints.cc:33-81, 497-581`): enable = DEPCFG(init) + DEPXFERCFG +
DALEPENA; one NORMAL TRB (LST|IOC|HWO) per packet + DEPSTRTXFER; XferComplete → actual length =
TRB size − BUFSIZ remaining; OUT TRBs sized MPS. Disable: DEPENDXFER if a transfer is active, then
clear DALEPENA. Stall/clear: DEPSSTALL/DEPCSTALL (`commands.cc:209-245`).

**Commands** (`FX commands.cc:15-31`): write PAR2/1/0 then DEPCMD with CMDACT; poll CMDACT clear,
bounded; non-zero CMDSTATUS = failure.

**Shutdown**: VBUS override off, RUN_STOP ← 0, poll DEVCTRLHLT = 1, mask events.

**Mapping to `embassy-usb-driver`** (`EU lib.rs` trait docs): `Bus::poll` first reports
`PowerDetected` (VBUS is overridden, not sensed — without it the stack never calls `enable`);
then USB reset → `Reset` (user endpoints disabled, the stack re-enables them at
SET_CONFIGURATION/SET_INTERFACE), disconnect and suspend-entry → `Suspend`, wakeup → `Resume`.
`enable`/`disable` = connect/shutdown above. `ControlPipe::data_in` chunks are gathered into the EP0
buffer and sent as one CONTROL_DATA TRB on `last` (an empty non-first chunk = the ZLP); `data_out`
arms the whole MPS-rounded OUT stage on `first`; `accept`/`accept_set_address` choose STATUS_2 vs
STATUS_3. Failed commands are recorded (`Dwc3::fault`) and surface as `EndpointError::Disabled`.

## 5. Interrupts / DMA / power

Polling first: every async call drains the event ring. IRQ path later: core SPI 133 (GIC direct, no
PDC/TLMM; virq ≥ 256 → needs K6 IRQ width). **DMA:** event ring, TRBs and packet buffers are
device-written memory; the controller sits behind `apps_smmu` SID 0x740. No Redox SMMU driver
exists (GOTCHAS) — bus address = physical only if firmware left the stream in bypass. DMA memory
must be uncached/coherent and accessed in program order (the `Mmio` contract). Power: no runtime
PM; clocks/GDSC are firmware-owned.

## 6. Redox interface

User-space daemon `usbgadgetd` (future): maps the core window (`0x0a600000`, 0xcd00) and QSCRATCH
(`0x0a6f8800`, 0x400) via `/scheme/memory/physical@uc`, allocates one physically contiguous DMA
region (`DMA_SIZE` = 22 KiB, `dwc3-gadget::layout`), runs `embassy-usb` with CDC-NCM → a Redox
network scheme (smolnet adapter) and CDC-ACM → a pty/console scheme. This task delivers the core crate
`crates/dwc3-gadget` only.

## 7. Verification

- Host: `crates/dwc3-gadget` tests — strict mmio-replay golden trace for init/connect, scripted
  controller model running real `embassy-usb` (enumeration reset → GET_DESCRIPTOR → SET_ADDRESS →
  GET_DESCRIPTOR(config) → SET_CONFIGURATION, CDC-ACM bulk echo, CDC-NCM NTB in/out) and negative tests.
- Oracle (pmOS phone, Linux g_ncm/configfs gadget active): `devmem` dumps of `0x0a60c110`
  (GCTL), `c120` (GSNPSID), `c14c` (GHWPARAMS3), `c200` (GUSB2PHYCFG), `c700` (DCFG), `c704` (DCTL),
  `c70c` (DSTS), `c720` (DALEPENA), `0x0a6f8808/8810/8830` (QSCRATCH); `cat
  /sys/kernel/debug/usb/a600000.usb/regdump` and `.../ep*/trb_ring`; `trace-cmd` with `dwc3:*`
  events during a replug (`dwc3_event`, `dwc3_gadget_ep_cmd`, `dwc3_prepare_trb`) — the event
  sequence is the reference for the model; `/sys/kernel/debug/iommu` or SMMU SMR/S2CR dump for
  SID 0x740.
- Phone: `fastboot boot` the Redox payload with the daemon, host sees `cdc_ncm` + `cdc_acm`
  interfaces (`lsusb -v`), ping over NCM, echo over ACM.

## 8. License path

Clean-room MIT. Register/field facts from Fuchsia (BSD-3, spec-only, no code copied); DT facts from
Linux (GPL-2.0, facts only); QSCRATCH `GENERAL_CFG`, `SW_SESSVLD_SEL`, `SS_PHY_CTRL` exist only in
GPL sources (Linux `dwc3-qcom.c`, U-Boot `dwc3-generic.c` is `GPL-2.0`) — used as oracle facts, no
code copied, flagged for oracle capture. OpenBSD `qcdwusb.c` at this commit only enables
power/clocks/resets (`OB qcdwusb.c:55-66`): no QSCRATCH logic to borrow. `embassy-usb-driver` is a
dependency (MIT/Apache-2.0), not copied.

**Gaps/conflicts:** SMMU translation for SID 0x740; GSNPSID revision on SDM845 (model uses
`0x5533_310a`, unverified); `USBTRDTIM` 9 assumes an 8-bit UTMI (Fuchsia value); GEVNTCOUNT busy
bit (Linux vs Fuchsia differ, plain count chosen); TX FIFO sizing left at reset values;
isochronous endpoints unsupported; no IRQ path yet.
