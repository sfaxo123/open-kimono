# SDM845 SPMI PMIC buttons (K7)

## Static phone facts and scope

The SDM845 DT `spmi@c440000` is `qcom,spmi-pmic-arb` with `core` 0x0c440000/0x1100, `chnls` 0x0c600000/0x2000000, `obsrvr` 0x0e600000/0x100000, and `cnfg` 0x0c40a000/0x26000; `qcom,ee=<0>`. These are a fallback profile only; compare the live DT before enabling the daemon. PM8998 is SID 0. Its PON peripheral is 0x08, and its GPIO peripheral is 0xc0. The OnePlus 6 common DT assigns active-low volume down/up to PM8998 GPIO numbers 5/6 respectively, and power to PM8998 PON. Pins are 1-based; per-pin offsets are `(pin-1)*0x100`.

## Read-only v5 arbiter subset

From U-Boot's BSD-3-Clause `drivers/spmi/spmi-msm.c`: `core+0` is version (v5 range 0x50000000..0x6fffffff), `core+4` feature channel count (low 11 bits), and `core+0x900+4*apid` maps `(sid<<16)|(pid<<8)`. `cnfg+4*apid` low 3 bits name the owning EE. An APID of another owner is still usable for observer reads. The v5 observer window for an APID is `obsrvr+0x80*apid`; registers CONFIG=4, STATUS=8, RDATA=0x18, CMD0=0. For a one-byte extended long read, write CONFIG=0 then CMD0=`(1<<27)|(offset<<4)|1`, poll STATUS bit 0 with a finite limit, then take RDATA low byte. A missing mapping or timeout must fail closed. The first byte of PMIC revision is at peripheral 0x01, offset 0x04 (PMIC type), followed by subtype 0x05; this is a diagnostic read, not a hardcoded identity check.

## Button sampling

OpenBSD ISC `qcpon.c` reads PON real-time status at peripheral offset 0x10; bit 7 is its documented power-key state for both its `qcom,pm8998-pon` and newer PMIC compatibles. Use that bit for power-key events, with phone oracle confirmation still required before automatic startup. OpenBSD ISC `qcpmicgpio.c` reads GPIO pin status at offset 0x10, bit 0, using the per-pin stride. The OnePlus DT declares volume GPIOs active-low, so level 0 means pressed. Polling reports edges only and seeds initial state without a synthetic press. Never write PMIC registers or configure pins here; firmware/DT pin setup is assumed. No QEMU model of this peripheral exists: host replay is the verification gate, and phone confirmation is a separate gate.

## Sources and licensing

- U-Boot `drivers/spmi/spmi-msm.c`, commit `8649a54d301a4d0ac526324c7a7ad2574939ff27`, BSD-3-Clause, https://source.denx.de/u-boot/u-boot.git.
- OpenBSD `sys/dev/fdt/qcspmi.c`, `qcpon.c`, `qcpmicgpio.c`, commit `20cb34434d36871c39729f7369062cba28fd06cb`, ISC, https://github.com/openbsd/src.git.
- Linux DT `arch/arm64/boot/dts/qcom/{sdm845.dtsi,pm8998.dtsi,sdm845-oneplus-common.dtsi}`, commit `5b0764a53235d3c77e1ed674e836e1b3fe1a4b1f`, GPL-2.0; facts/oracle only, no code copied.
