# T3 - OnePlus 6 (enchilada, SDM845) driver inventory for a Redox / Rust port
Date 2026-10-24. Reference = mainline Linux (arch/arm64/boot/dts/qcom/sdm845-oneplus-{common.dtsi,enchilada.dts}, sdm845-wcd9340.dtsi fetched live from torvalds/linux master).

## Evidence quality (read first)
- VERIFIED live: the DT facts below (compatibles, firmware names, which nodes are enabled), the June-2025 pmOS speaker-regression post.
- NOT VERIFIED: the pmOS wiki feature table. wiki.postmarketos.org 301s to wiki.nura.eco and that is blocked by an Anubis bot wall; gitlab.postmarketos.org also blocked. So the "what pmOS lists as working" section is reconstructed from DT enablement, blog/MR snippets and general knowledge. Re-check the wiki in a normal browser.
- LOC figures are my from-memory approximations of mainline files (order of magnitude, +/-30%). Table-heavy files (clock/pinctrl/interconnect tables) are mostly data, not logic.
- Correction to the brief: enchilada panel is NOT Samsung S6E3FC2. Enchilada (OP6) = `samsung,sofef00-ams628nw01` / `samsung,sofef00` (driver panel-samsung-sofef00.c, ~250 LOC). S6E3FC2x01 is fajita (OP6T). OP6T also uses TFA9894 amp; OP6 uses MAX98927.

## Board facts from the mainline DT (verified)
- Touch: Synaptics RMI4, `syna,rmi4-s3706b` on i2c12 @0x20 (VDD l28a 3.0V, VIO fixed 1.8V). Function F12 (rmi4_f12 node).
- Speaker amp: `maxim,max98927` on i2c4 @0x3a. Codec: WCD9340 on SLIMbus (`slim217,250`, IRQ TLMM gpio54, extclk RPMH_LN_BB_CLK2, 4 micbias @1.8V, has a Soundwire child `qcom,soundwire-v1.3.0`). Sound card `qcom,sdm845-sndcard` (Q6AFE DAIs SLIMBUS_0/1/2). No headset-amp node needed; jack via WCD MBHC.
- Fuel gauge: `ti,bq27411` on i2c10 @0x55. Charger: PMI8998 (`pmi8998_charger`, SPMI smb2-style). RGB LED via pmi8998_lpg, flash LEDs pmi8998_flash. No haptics node in the common dtsi (vibrator not wired up in mainline DT as of this fetch).
- Regulators: PM8998, PMI8998, PM8005 RPMh regulators + fixed (vph_pwr, panel vci 3.3V, vddi 1.8V). WCN3990 via `qcom,wcn3990-pmu`.
- Remoteprocs enabled: adsp_pas, cdsp_pas, slpi_pas, mss_pil (modem), plus ipa, venus, gpu. Firmware: adsp.mbn, cdsp.mbn, slpi.mbn, mba.mbn, modem.mbn, ipa_fws.mbn, venus.mbn, a630_zap.mbn; BT crnv21.bin; Wi-Fi board variant `oneplus_sdm845`.
- USB: usb_1 DWC3 `dr_mode = "peripheral"`, `maximum-speed = "high-speed"`, USB2 PHY only (no SS, no host/OTG, no Type-C stack in DT).
- Display: mdss + dsi0 + dsi0 phy; a simple `framebuffer` node (1080x2280, bootloader-provided) is in enchilada.dts. UFS (ufs_mem_hc + phy). No SD slot on OP6 (UFS only). NFC pn553 on i2c3. GPIO keys: alert slider (3 pos), hall sensor, vol up/down (power key is PMIC pwrkey).

## Per-subsystem inventory
Difficulty 1-5 = from-scratch Rust user-space driver, Linux as spec, no vendor docs (Qualcomm SDM845 TRM is not public; Linux source is the doc).

### Core platform
| Subsystem | Mainline files (~LOC) | Dependencies | Diff |
|---|---|---|---|
| GIC-v3 / arch timer / PSCI | irq-gic-v3.c (~2.3k), arm_arch_timer.c (~1.5k), psci.c (~0.7k) | Kernel-side in Redox (not user-space). Boots EL1 under Qualcomm hypervisor (no EL2). PSCI via SMC to TZ/ATF | 2 (kernel work, small; GIC/timer well understood) |
| TLMM pinctrl+GPIO | pinctrl-msm.c (~1.5k) + pinctrl-sdm845.c (~1.4k, tables) | Plain MMIO, GIC, no clocks. Tables copyable | 2 |
| GENI SE (UART/I2C/SPI) | qcom-geni-se.c (~1k), qcom_geni_serial.c (~1.8k), i2c-qcom-geni.c (~1k), spi-geni-qcom.c (~1.1k) | Clocks (GCC), pinctrl, interconnect votes, SE firmware loaded by bootloader/TZ (protection config), optional GPI DMA (gpi.c ~2k) - FIFO/PIO mode suffices | 3 |
| Clocks GCC/dispcc/rpmh-clk | gcc-sdm845.c (~4.3k tables), clk-rcg2.c, clk-branch.c, clk-alpha-pll.c (~2k), clk-rpmh.c (~0.8k), dispcc-sdm845 (~1k) | RPMh for XO/RF clocks, GDSC power domains (gdsc.c ~0.6k), reset ctrl | 4 (lots of table data, subtle PLL/RCG sequencing; many consumers depend on exact rates) |
| RPMh + cmd-db | rpmh.c (~0.5k), rpmh-rsc.c (~1.1k), cmd-db.c (~0.4k) | Reserved cmd-db memory region from bootloader, apps_rsc MMIO + IRQ, TCS slots | 3 |
| SPMI + PMIC | spmi-pmic-arb.c (~1.8k, v5 on SDM845) + qcom-spmi-pmic, pwrkey, rtc | MMIO only | 3 |
| Regulators | qcom-rpmh-regulator.c (~1.9k, mostly tables) | RPMh, cmd-db | 3 |
| Interconnect | icc-rpmh.c, bcm-voter.c (~0.6k), sdm845.c (~1.7k tables) | RPMh. Often safe to skip: bootloader leaves buses at usable rate; but without votes DDR/bus can be slow or glitch | 3 (or 1 if skipped) |
| SMMU / IOMMU | arm-smmu.c + arm-smmu-qcom.c (~5k) | Qualcomm hypervisor pre-programs context banks; SMMU is mandatory for modem/ADSP/WLAN/UFS-less DMA of many masters (display, venus, wifi snoc). Needs DMA mapping layer in Redox | 5 (biggest cross-cutting kernel-side risk) |
| SCM (TZ calls) | firmware/qcom_scm.c (~2.5k), SMC calling convention | Needed for PAS remoteproc auth, some secure-io | 3 |

### Storage
| UFS | ufshcd.c (~10k, mostly optional), ufs-qcom.c (~2.2k), phy-qcom-qmp-ufs (~1.5k+tables), SCSI/block layer | GCC clocks, regulators (UFS vccq etc.), interconnect, PHY tables, optionally ICE crypto. Only the UTP transfer path + link startup is needed for a basic block device (~4-5k LOC Rust) | 4 |
| SD | none (OP6 has no microSD) | - | n/a |
Fallback: root image loaded by bootloader (fastboot/ramdisk boot.img) avoids UFS at first.

### Display / input
| Display | msm DRM: dpu1/ (~25-30k), dsi/ (~5k), dsi_phy_10nm (~1k), mdss; panel-samsung-sofef00.c (~250) | dispcc clocks, GDSC, interconnect, SMMU, regulators, DSI DCS init sequence, backlight via DCS brightness (AMOLED). Real DPU port = very large | 5 for DPU; 1 for simplefb |
| simplefb fallback | DT `framebuffer` node, 1080x2280 (stride/format from bootloader) | Works if panel stays powered by ABL. If panel goes off/idle, can't re-enable without DSI/DPU driver; fine for a mostly-audio device | 1 |
| Touchscreen | Synaptics RMI4: rmi_driver.c, rmi_i2c.c, rmi_f01.c, rmi_f12.c (~4k total, ~1.5k needed) | i2c12 (GENI), IRQ gpio, regulators l28a/1.8V fixed | 2 |
| GPIO keys / pwrkey | gpio-keys.c (~0.7k), pm8941-pwrkey | TLMM, PMIC IRQ | 1 |
| Haptics | no node in mainline OP6 DT; PMI8998 haptics patches exist out of tree | SPMI | 2 (low value) |

### USB
| DWC3 + QUSB2 | dwc3 core+gadget (~8k, ~3k needed), dwc3-qcom.c (~1k), phy-qcom-qusb2.c (~1k) | GCC clocks, regulators, IRQ (pwr_event/hs_phy), tuning params from fuse/DT; high-speed only. Then need a USB device stack (RNDIS/ECM/NCM or serial) in Rust | 4 |
Value: this is the cheapest network + debug path (host = the 3090/5090).

### Power
| Charger PMI8998 (smb2) | power/supply/qcom_smbx.c (~1k) or qcom_smbb/smb2 | SPMI only | 3 |
| Fuel gauge bq27411 | bq27xxx_battery.c (~2k, ~600 for 27411) + _i2c.c | i2c10 | 2 |
Quirk: Dash charge (proprietary) not supported; plain 5V/9V negotiation only.

### Connectivity (modem is a hard prerequisite for Wi-Fi in pmOS practice)
| Modem MSS | qcom_q6v5_mss.c (~2.5k), qcom_q6v5.c, mdt_loader (~0.4k), sysmon, ssr, smp2p/smem/glink (~5k together), QMI helpers (qmi_interface.c ~0.9k), QRTR (~3k net/qrtr) | SCM auth, MBA+modem firmware (signed, from the phone's own /vendor), rmtfs userspace + rmtfs shared mem region, pd-mapper, tqftpserv, qrtr-ns | 5 |
| Wi-Fi WCN3990 | ath10k: snoc.c (~1.7k), qmi.c (~1.3k), core/htt/wmi/mac80211 glue (~40k total; a minimal STA-only data path ~6-8k Rust) | SMMU (WLAN SNOC uses iommu), wcn3990-pmu regulators, QMI wlfw handshake, board-2.bin (`oneplus_sdm845` variant), firmware wlanmdsp.mbn (pmOS ships via firmware-oneplus-sdm845), plus WPA2/3 supplicant, TCP/IP. I believe the modem must be up for wlfw/firmware; confidence medium | 5 |
| Bluetooth | btqca + hci_qca over uart6, crnv21.bin | skip | 3 |
Shortcut: USB gadget networking removes this whole block for a first voice-assistant prototype.

### Audio (critical for the product)
Signal path on SDM845/OP6 (Linux): Apps <-> ADSP (Hexagon) over APR on top of GLINK (fastrpc/rpmsg) ; ADSP runs q6 core/afe/asm/adm; WCD9340 hangs off the ADSP's SLIMbus master via DAI links SLIMBUS_0/1/2; Apps programs WCD9340 registers via SLIMbus *NGD* (Apps-side non-ported device, which itself talks to the ADSP via QMI/SLIMbus-QMI) ; speaker MAX98927 sits on an I2S/TDM DAI (primary/quaternary MI2S) driven by the ADSP, and is configured over I2C4. SDM845 uses classic **APR** (not AudioReach/GPR - that is newer SoCs like SM8450+), simplifying things.
| Piece | Mainline files (~LOC) | Dependencies | Diff |
|---|---|---|---|
| ADSP remoteproc | qcom_q6v5_pas.c (~1.4k), qcom_q6v5.c (~0.4k), qcom_common.c, mdt_loader | SCM PAS auth, adsp.mbn (+ `adsp.jsn`/dsp libs), reserved memory, SMMU/iommu for dsp, smp2p, smem | 4 |
| GLINK/SMEM/SMP2P | qcom_glink_smem.c, glink native (~3k), smem (~1.3k), smp2p (~0.9k), rpmsg core | MMIO + IRQ mailbox (apcs ipc) | 4 |
| QRTR / QMI | net/qrtr (~3k), soc/qcom/qmi_interface (~0.9k), pdr_interface (~0.7k) | glink transport | 3 |
| APR + q6 DSP | soc/qcom/apr.c (~0.7k), qdsp6: q6core (~0.6k), q6afe (~1.7k), q6afe-dai (~1.2k), q6asm (~1.9k), q6asm-dai (~1.2k), q6adm (~0.7k), q6routing (~1.2k), q6dsp-common | Pure protocol over glink; no DSP code needed. Routing graphs + calibrations from UCM/ACDB (userland-chosen) | 4 |
| SLIMbus NGD | slimbus/{core,messaging,sched,qcom-ngd-ctrl}.c (~3.5k) | ADSP up, QMI SLIMbus service, enumeration, DMA via ADSP-owned BAM | 5 |
| WCD9340 codec | codecs/wcd934x.c (~3.5k), wcd-clsh-v2 (~0.9k), wcd-mbhc-v2 (~1.5k), wcd9335 shared bits | SLIMbus regmap, 9.6MHz extclk from RPMH_LN_BB_CLK2, gpio54 IRQ, micbias config, reset gpio/regulators; MBHC for jack | 4 |
| Soundwire (WCD9340 internal) | soundwire/qcom.c (~2k) | Used for headphone/mic ports on WCD9340 (soundwire v1.3 child in DT) | 4 |
| Sound card machine | qcom/sdm845.c (~0.8k), common.c | Wires DAIs, MI2S for amp | 2 |
| MAX98927 amp | codecs/max98927.c (~1k) | i2c4, TDM/I2S, rst gpio | 2 |
Microphone note: a voice assistant needs only capture. Capture path = WCD9340 (analog mics via ADC/AMIC or DMIC) -> SLIMbus -> ADSP q6afe -> q6asm -> shared memory ring to Apps. So the **entire** ADSP + glink + APR + SLIMbus + WCD9340 chain is required even for mic-only; no simpler route (no I2S mic on the AP side). Playback (TTS) additionally needs the MAX98927 + MI2S route (or headphone/earpiece through WCD9340 which is already in the chain). Realistic subset: capture + earpiece/speaker, skipping jack detect (MBHC), clsh tuning, voice-call/VoLTE, ANC, and sidetone. Cheaper alternative: stream mic/speaker over the USB-gadget (needs same codec chain anyway) - no gain; the DSP chain is the cost.

### GPU / camera / sensors (not needed)
| GPU Adreno 630 | drm/msm/adreno a6xx (~10k) + gmu + SMMU, a630_sqe.fw, a630_gmu.bin, a630_zap.mbn | Mesa/freedreno-equivalent is another entire project | 5 (skip) |
| Camera | CAMSS + IMX519/IMX371 sensors | skip | 5 (skip) |
| Sensors | SLPI (slpi.mbn) "SSC"; no mainline IIO sensor driver; accel/gyro behind SLPI via FastRPC/QMI SNS. pmOS: unsupported | skip | 5 (skip) |
Also: Venus, IPA, CDSP, NFC (pn553 over i2c) - skip.

## What postmarketOS (mainline) reportedly has working / known quirks
(Compiled from DT enablement, pmaports MR snippets and a June-2025 blog; wiki table itself NOT read - verify.)
- Reported working: boot, display (DPU+sofef00), touch (RMI4), UFS, Wi-Fi (ath10k_snoc), Bluetooth, GPU (freedreno, 3D accel), USB gadget (peripheral), battery gauge/charger, modem (calls/SMS/data via ModemManager + rmtfs/pd-mapper/tqftpserv), audio: speaker + headphone jack declared fully functional in a ~kernel-5.12 MR; microphone and call audio have needed UCM tuning (a later MR says call audio still needed UCM tweaks).
- Reported not working / partial: cameras, fingerprint (Goodix over SPI, out of tree), sensors (SLPI/IIO), GPS, USB host/OTG (DT is fixed `peripheral`, no Type-C), USB SuperSpeed (maximum-speed = high-speed), NFC, Dash charge, haptics. xengineering OP6 article (third-party) also lists camera, fingerprint, inertial sensors, GPS, OTG, NFC as milestone-2 unsupported and "stabilize audio during calls" as milestone 1.
- Quirks:
  - June 2025 regression: main speaker silent on OP6/6T on edge/v25.06 images before 2025-06-17; workaround: disable `alsa-restore`, delete `/var/lib/alsa/asound.state`, reboot (upstream issue #3747). Lesson: speaker routing is ALSA-mixer-state dependent (UCM), i.e. userland must set the q6routing mixer correctly. Source: postmarketos.org blog (moved to nura.eco).
  - Firmware is the phone's own signed blobs (package `firmware-oneplus-sdm845`: adsp/cdsp/slpi/modem/mba/venus/ipa/a630 + wlan board file). These are proprietary; licensing/redistribution is a concern for a shipped Redox image. Extract from device's /vendor/firmware_mnt.
  - SMMU/hypervisor: no EL2 access on this SoC; device assignment relies on Qualcomm's pre-programmed SMMU state.
  - Modem requires userspace helpers (rmtfs, pd-mapper, tqftpserv, qrtr-ns) - these are ALSO what Redox must reimplement.
  - sdm845 mainline cannot do DT-less boot: bootloader (ABL) needs Android boot.img with DTB appended, `fastboot boot` flow. Unlocked bootloader needed.
  - Note: the DT lists `status = "disabled"` for the display panel in common.dtsi, enabled per-variant in enchilada.dts.

## Table, ordered by need for a voice-assistant phone
Effort = person-months, experienced systems engineer, hardware in hand, UART/USB debugging, Linux source as spec, assuming Redox already boots aarch64 kernel+DTB on the device (not included).
| # | Block | Needed for | Diff | PM | Notes |
|---|---|---|---|---|---|
| 1 | Boot: GIC/timer/PSCI/SCM, MMIO+IRQ scheme, SMMU | everything | 4-5 | 3 | SMMU is the sleeper risk |
| 2 | TLMM, GENI (UART first, then I2C) | debug console, all I2C devices | 3 | 1.5 | UART via 3.5mm? OP6 debug UART needs test pads; use USB |
| 3 | GCC clocks + GDSC, RPMh/cmd-db, SPMI, regulators (+ interconnect) | gate for UFS/USB/audio/wifi | 4 | 3.5 | |
| 4 | USB DWC3 gadget + QUSB2 + RNDIS/ECM | dev loop, networking, network-assist | 4 | 2.5 | cheapest connectivity |
| 5 | **Audio capture+playback** (ADSP rproc, glink/QRTR/APR, q6afe/asm/adm/routing, SLIMbus NGD, WCD9340, soundwire, MAX98927) | the product | 5 | 9 | dominant cost; zero shortcuts |
| 6 | Storage UFS | persistent rootfs, models | 4 | 3 | could defer with ramdisk |
| 7 | Wi-Fi: MSS modem + QMI/rmtfs/tqftpserv + ath10k snoc + WPA | standalone networking | 5 | 8-10 | defer; USB tether for v1 |
| 8 | Touchscreen RMI4 | any UI | 2 | 1 | |
| 9 | simplefb display; (later real DSI/DPU: 6+ PM) | status UI | 1 (5) | 0.3 (6) | |
| 10 | Battery/charger/fuel gauge, GPIO keys, pwrkey | untethered use, buttons | 3 | 2 | |
| 11 | Haptics, LEDs | polish | 2 | 0.5 | |
| 12 | Bluetooth | headset | 3 | 1.5 | optional |
| 13 | GPU, camera, SLPI sensors, NFC, cdsp/venus | not needed | 5 | skip | |

## Total effort estimate
- Minimal "talking brick" (items 1-6, 8, 9-simplefb, 10, USB-networked to a host that runs ASR/LLM/TTS): ~26-30 person-months (core 8 + USB 2.5 + audio 9 + UFS 3 + touch/display 1.3 + power 2 = ~26, +10-15% integration).
- Standalone phone-like unit with Wi-Fi: add 8-10 -> ~35-40 PM.
- Realistic with debugging overhead (no JTAG, blobs opaque, hypervisor-fixed SMMU, Redox driver framework gaps such as DMA/IOMMU/IRQ schemes and missing Rust crates): multiply by 1.3-1.5 -> **~35-45 PM for the minimal unit, ~50-60 PM with Wi-Fi.** A 3-person team: roughly 12-18 months to voice-in/voice-out over USB, 18-24 months with Wi-Fi.
- Reasoning: Linux code volume is not the cost; protocol depth and absent documentation are. Audio (APR/glink/SLIMbus/WCD9340) is the longest dependent chain (needs clocks+RPMh+SCM+SMMU+remoteproc all working first) and has no simplification because the mic hangs off the ADSP-owned codec. Wi-Fi needs the modem stack as well and a whole 802.11 + supplicant. A prototype is much cheaper if a different, better-documented board is chosen first, or if Linux runs the hardware and Redox sits in a VM/other core - out of scope here.

## Sources
- https://raw.githubusercontent.com/torvalds/linux/master/arch/arm64/boot/dts/qcom/sdm845-oneplus-common.dtsi
- https://raw.githubusercontent.com/torvalds/linux/master/arch/arm64/boot/dts/qcom/sdm845-oneplus-enchilada.dts
- https://raw.githubusercontent.com/torvalds/linux/master/arch/arm64/boot/dts/qcom/sdm845-wcd9340.dtsi
- https://postmarketos.org/edge/2025/06/16/OnePlus-6-and-6T:-main-speaker-not-working/ (redirects to nura.eco)
- https://gitlab.postmarketos.org/postmarketOS/pmaports/-/merge_requests/2206 (headphone jack/speaker working), .../3334, .../2982 (found via search, not opened)
- https://cgit.xengineering.eu/website/tree/content/articles/oneplus-6.md?h=uefi
- https://www.xda-developers.com/oneplus-6-6t-mainline-linux-kernel-support
