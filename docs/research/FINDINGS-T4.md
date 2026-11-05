# T4 Findings: alternatives to a Redox port for OnePlus 6 (SDM845) voice-assistant phone OS
Date: 2026-10-24. Method: web search (snippets plus two page fetches). Items marked UNKNOWN were not verified. Time estimates are my judgment, not sourced.

## BLUF
- No non-Linux OS in this survey has been shown on an SDM845 phone with display, touch, audio, modem and Wi-Fi. The only microkernel-class phone result is Genode/Sculpt on the PinePhone. That is Allwinner A64 hardware, which is open and documented, unlike Qualcomm.
- The Qualcomm boot chain blocks EL2 on most Snapdragon devices. That kills the "microkernel plus Linux driver VM" path on the OnePlus 6 unless a workaround is found. No SDM845 workaround was found (UNKNOWN).
- Rust-in-Linux is real, but the drivers that matter here (audio DSP, Wi-Fi, modem, Adreno) are not Rust. The phone is not blocked on that anyway.
- Best path: keep postmarketOS and mainline SDM845. Own everything above the kernel (Rust or Python shell plus voice service). Put Rust at the userspace layer.

## 1. Non-Linux OSes on phones and ARM boards

| OS | Status (source, date) | What it needed / blockers |
|---|---|---|
| Genode/Sculpt, PinePhone | Official image. RC2 with all features posted 2025-10-27; an earlier 2025-10-10 image had caveats. PinePhone v1.2 only. https://lists.genode.org/mailman3/hyperkitty/list/users%40lists.genode.org/message/ZUEZP6ETJZKKXO7D6I2PHWQWIPY52WPJ (via search summary). 2023 write-up: https://genodians.org/nfeske/2023-02-01-mobile-sculpt | Works as of 2023-02/04: touch UI, mic input, voice calls, mobile data, Wi-Fi (added 2023-04), GPU OpenGL, camera test. Limits: battery percentage unreliable, "targeted at enthusiasts and developers". The page does not say how drivers were obtained. UNKNOWN whether this was DDE Linux. Genode's PinePhone port took years, from the 2021 roadmap (https://www.phoronix.com/news/Genode-OS-2021-Roadmap) to the 2023 image. |
| seL4 on ARM phones | The seL4 Summit 2024 talk on a commercial phone effort says "major challenges with Qualcomm platform", no ability to alter the bootchain and run seL4 in EL2, and "not specific to seL4". It shifted to hardware separation. https://sel4.systems/Summit/2024/slides/building-commercial.pdf. The 2023 talk covers the Bittium partnership: https://sel4.systems/Summit/2023/slides/day3-03-sebranek.pdf | Qualcomm EL2 lock. No public seL4 port to a Qualcomm phone found (UNKNOWN beyond that). |
| Fuchsia, non-Google hardware | No non-Google phone found. Only a 2018 Huawei Zircon kernel boot on Honor Play (reported). A March 2026 "Armadillo" web demo is not hardware. https://androidworld.nl/nieuws/google-os-fuchsia-is-weer-tot-leven-gewekt-maar-niet-helemaal-door-google/ | Hardware support is limited to Google's Nest devices and emulators. Dead end for SDM845. |
| SerenityOS aarch64 | Docs (2021-2023 mirrors): does not boot on Raspberry Pi, UART only, no display. https://github.com/matjojo/serenity/blob/master/Documentation/RunningOnRaspberryPi.md . 2026 status UNKNOWN. | Not usable. |
| Haiku ARM64 | Boots under QEMU, SMP added 2026-05-12, Tracker launches. Real hardware is "in progress". https://phoronix.com/news/Haiku-OS-ARM64-SMP , https://linuxfr.org/news/nouvelles-de-haiku-printemps-2026 | No phone path. |
| Redox aarch64 | 0.8.0 (2022): boots to login on QEMU, real hardware not working (https://redox-os.org/news/release-0.8.0). Phoronix, status for August 2026 (article dated 2026-09-25): aarch64 has initial multi-core support (https://phoronix.com/news/Redox-OS-August-2026). Real aarch64 hardware: UNKNOWN. | No Qualcomm drivers (no UFS/eMMC, DSI panel, touch, WCN3990 Wi-Fi, audio DSP via APR/ADSP). Everything would be written from scratch. |
| Plan 9/9front on ARM, Theseus, Hubris/Tock | Searches found no phone or Qualcomm work. UNKNOWN. (Hubris and Tock target MCUs, not application SoCs, as far as I know. Not verified.) | n/a |

What a from-scratch OS needs on the OnePlus 6:
- Bootloader handoff: a fastboot-loadable boot image with a device tree.
- Display: Samsung AMOLED over DSI plus panel init and the Adreno display subsystem.
- Touch: Synaptics over I2C.
- Audio: WCD9340 codec plus ADSP firmware over APR/Q6 (needs Qualcomm firmware blobs).
- Wi-Fi: WCN3990 over ath10k/SNOC with firmware.
- Storage: UFS.
- Genode's own PinePhone work shows this is multi-year even on the open Allwinner A64 hardware.

## 2. Custom userspace on a Linux kernel

| Option | Kernel | Notes (source) |
|---|---|---|
| postmarketOS (Phosh, Plasma Mobile, SXMO, Sway) | Mainline-based fork for SDM845 (`linux-postmarketos-qcom-sdm845`), https://pkgs.postmarketos.org/package/main/postmarketos/aarch64/linux-postmarketos-qcom-sdm845 | OnePlus 6 device package rebuilt 2026-05-26, so it is maintained. Current camera/audio/modem status for 2026: UNKNOWN from search; check the wiki page for enchilada. Alpine base, so you can swap the entire shell. **Keeps mainline drivers and owns everything above.** |
| Ubuntu Touch | Halium (Android vendor kernel) | OnePlus 6 (fajita and enchilada) listed "Advanced support"; focal OTA-12 on 2026-02-06; idle drain higher than Android. https://devices.ubuntu-touch.io/device/fajita . Android kernel plus HAL, not mainline. |
| FuriOS / Furi Labs | Halium, not mainline | Shipping FLX1/FLX1s/FLX1s+ in 2026. https://liliputing.com/flx1s-is-a-new-linux-phone-thats-mostly-a-downgrade-from-the-flx1/ . Vendor hardware, not an SDM845 target. |
| Sailfish, LuneOS, Mobian | UNKNOWN | Not researched for the OnePlus 6. UNKNOWN. |
| AOSP-minimal | Android vendor kernel | Reuses the Android HAL (camera, modem, audio). Heavy, and the AI shell could be a replacement launcher. |
| "AI phone OS" prior art | | OpenPhone, an open-source AI-native overlay on Android (no URL captured; UNKNOWN). StepFun StepX Neo with Step AOS, commercial and closed, July 2026: https://www.businesstoday.in/latest/trends/photo/the-worlds-first-ai-phone-doesnt-run-android-heres-what-it-runs-instead-542822-2026-07-14 . No open Linux/postmarketOS AI-shell project found (UNKNOWN). That is a gap you could fill. |

## 3. Hybrid options and the EL2 question
- seL4 libvmm driver-VM work reuses Linux drivers inside a VM behind native sDDF interfaces. It is the closest prior art. Qualcomm SoC support: UNKNOWN. https://docs.sel4.systems/projects/virtualization
- Genode's DDE Linux: not confirmed by search. UNKNOWN for Qualcomm.
- EL2 on Snapdragon:
  - On modern Snapdragon, EL2 is almost never available to the user, because Qualcomm's QHEE hypervisor firmware owns it (Qcom-Secure-Launch project, https://github.com/TravMurav/Qcom-Secure-Launch/).
  - Chromebooks are the exception.
  - slbounce works on Windows-on-Arm devices that shipped with Windows. Per its README, Android devices cannot use it. Example target is SC8280XP. https://github.com/TravMurav/slbounce/blob/main/README.md
  - qhypstub restores EL2 on MSM8916 only (Snapdragon 410/615), and only on devices without firmware secure boot. https://github.com/msm8916-mainline/qhypstub
  - Whether the OnePlus 6 (SDM845) can run KVM at EL2: UNKNOWN. Nothing found says it can, and the evidence points to no. Treat Linux-driver-VM under a microkernel on this phone as blocked until someone proves otherwise (for example, by checking `dmesg` for "Hyp mode not available" on the device).
- Redox with a Linux driver VM: no prior art found, and it would hit the same EL2 wall.

## 4. Rust in the Linux kernel (2025-2026)
- Wikipedia states that in December 2025 Rust in the kernel was declared no longer experimental. I could not confirm this from a primary source, and an LWN comment thread from November 2025 disputed that it had happened yet. https://en.wikipedia.org/wiki/Rust_for_Linux
- Nova (NVIDIA GPU, Rust) merged in Linux 6.15 (2025) and is still being built up in 7.x. Tyr (Arm Mali, Rust) is still experimental. https://phoronix.com/news/NVIDIA-Nova-Linux-7.4-Changes (reported October 2026)
- Implication: you can write your own out-of-tree or in-tree Rust drivers (for example a custom sensor or LED driver) on mainline. The SDM845 core stack (Adreno, DSI, WCN3990, audio DSP) stays C. This does not block a voice assistant.

## 5. Time to first useful voice assistant (mic, speaker, network, screen), OnePlus 6
These are my estimates for one developer. The voice stack already works on postmarketOS.

| Option | Estimate | Reasoning |
|---|---|---|
| Stay on postmarketOS, swap in a custom kiosk/Wayland shell (cage or Sway plus web UI or Rust app) | Days (now: it already works) | Everything works today. Custom shell takes 1-3 weeks to polish. |
| Ubuntu Touch / Halium | 2-4 weeks | Voice stack needs repackaging (click/snap); audio goes through the Android HAL. |
| AOSP minimal plus app | 1-3 months | Large build and ROM maintenance; the voice stack must be ported to Android/NDK. |
| Rust kernel drivers on mainline | Months to hard-to-estimate | Not needed for the goal. |
| seL4/Genode plus Linux VM | Likely blocked (EL2); otherwise 6-18 months | |
| Genode with native drivers on SDM845 | 2+ years (guess) | The PinePhone port took roughly 2021 to 2023 on documented hardware. |
| Redox on SDM845 | 2+ years and probably never practical (guess) | Real aarch64 hardware support unclear; no Qualcomm drivers; local Gemma inference needs a GPU/NPU or a good CPU stack, which Redox lacks. |
| Fuchsia, Haiku, SerenityOS | Not viable | No hardware path. |

## Ranked comparison

| Rank | Option | Effort | Risk | Customizable / shareable | Reuses |
|---|---|---|---|---|---|
| 1 | postmarketOS plus own shell (Rust/web) | Low | Low | High; ship as a pmOS package or image recipe, others flash it. | Mainline drivers, Alpine, existing voice stack |
| 2 | Same, plus Rust userspace daemons (audio, agent, UI) and optional Rust kernel modules | Low-Med | Low | High | As above |
| 3 | Ubuntu Touch / Halium custom app | Medium | Medium | Medium; tied to UBports and a vendor kernel. | Android kernel and HAL |
| 4 | AOSP minimal AI launcher | High | Medium | Medium; easy to share (APK/ROM) but heavy and Google-dependent. | Android HAL |
| 5 | seL4/Genode plus Linux driver VM | Very high | Very high (EL2 lock) | Low-Med | Linux drivers in a VM, if EL2 can be had |
| 6 | Genode native SDM845 | Extreme | Very high | Low | Genode framework |
| 7 | Redox on SDM845 | Extreme | Very high | Low-Med (Rust is attractive) | Almost nothing |

## Recommendation (10 lines)
1. Do not port to Redox: aarch64 real-hardware support is UNKNOWN, and every SDM845 driver would be new.
2. Keep postmarketOS on the OnePlus 6: mainline SDM845 is maintained (device packages rebuilt 2026-05-26).
3. Build the custom OS as a thin userspace layer: a Wayland kiosk shell (cage or Sway) running your web UI or a Rust UI.
4. Write the voice pipeline (STT, LLM, TTS) as Rust or Python systemd/OpenRC services with a clean local API.
5. Make it shareable as a pmOS device-agnostic package set (APKBUILDs) plus a pmbootstrap image recipe.
6. Use Rust where you control the code: userspace daemons first; kernel Rust only for small custom drivers, if ever.
7. Verify EL2 on the device once (`dmesg | grep -i hyp`) to close the virtualization question; expect "not available".
8. Check the pmOS wiki enchilada page for current audio, modem and camera status before relying on call or camera features.
9. Treat Genode/Sculpt as the reference for "non-Linux phone is possible", but note it needed years on open Allwinner hardware.
10. If the microkernel itch persists, prototype on a documented board (Pi 4/5 or PinePhone) rather than a locked Qualcomm phone.

## Sources (accessed 2026-10-24 via search; page text not read unless noted)
- Genode mailing list RC2 (2025-10-27), Mobile Sculpt (fetched, 2023-02-01): URLs above.
- seL4 Summit 2023/2024 slides; libsel4vm/libvmm docs.
- Qcom-Secure-Launch, slbounce (fetched), qhypstub.
- Phoronix: Haiku ARM64 SMP (2026-05-12), Redox Aug 2026 (dated 2026-09-25), Nova Linux 7.4 (2026-10).
- UBports fajita device page; pkgs.postmarketos.org; Liliputing FLX1s.
