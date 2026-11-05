# Downstream U-Boot patches (GPL-2.0, upstream source.denx.de/u-boot)
Base: upstream commit `8649a54d` (the U-Boot 2026.10 build used for `u-boot-enchilada-v0.img`).
- 0001: OnePlus 6 fastboot-first environment (`qcom-phone-headless.config/.env`): 3 s menu, default `fastboot -l 0xa0000000 -s 0x6000000 usb 0`.
- 0002: doc/board/qualcomm/oneplus6-headless.rst — build commands, Mac-side agent commands, emulator proof.
Built image: ~/artifacts/handoff/oneplus-redox-20261024/phone-boot/u-boot-headless/u-boot-enchilada-headless.img (sha256 ca2e6d24…).
These files are GPL-2.0 patches to U-Boot, kept here as separate patch files (license separation as in docs/PLAN.md).
