//! Tier rules for the U-Boot source ledger — explicit path prefixes, no
//! judgement left implicit (every call is documented at its rule).
//!
//! - **T1** — enables/environments/drivers relevant to the OnePlus 6
//!   (msm8998/SDM845 family, downstream codename "enos"/"enos1001"):
//!   the whole downstream overlay neighbourhood (`board/qualcomm/`,
//!   `doc/board/qualcomm/`) plus its direct dependencies by path prefix:
//!   the Snapdragon SoC support under `arch/`, the Qualcomm vendor
//!   driver subtrees and the subsystem drivers the qcom-phone boot
//!   path touches (USB gadget/fastboot, UFS/SCSI storage, MMC, SPMI &
//!   PMIC power, clocks/pinctresets/reset/IOMMU for SDM845, display,
//!   input/buttons, serial console, network), and the matching device
//!   trees (`arch/arm/dts/` — where `sdm845-*.dtsi` live — plus the
//!   qcom DTS import subsets named below).
//! - **T2** — common drivers and common lib: the rest of `drivers/`
//!   (generic driver model), `common/` (shared boot/common runtime) and
//!   `lib/` (shared library code, including its vendored imports).
//! - **T3** — everything else.

/// The tier a U-Boot tree path belongs to. Rules are ordered: the first
/// matching prefix wins; anything not matched by T1/T2 rules is T3.
pub fn tier(path: &str) -> &'static str {
    for prefix in T1_PREFIXES {
        if path.starts_with(prefix) {
            return "T1";
        }
    }
    // Exact-file T1 rule: the defconfig the OnePlus 6 build starts from.
    if path == "configs/qcom_defconfig" {
        return "T1";
    }
    for prefix in T2_PREFIXES {
        if path.starts_with(prefix) {
            return "T2";
        }
    }
    "T3"
}

/// T1: the OnePlus 6 (SDM845/msm8998 family) neighbourhood, by explicit
/// path prefix. Judgement calls documented inline.
const T1_PREFIXES: [&str; 26] = [
    // The downstream overlay's own neighbourhood: every Qualcomm board
    // file (the enos/enos1001 env/config live here) and its docs.
    "board/qualcomm/",
    "doc/board/qualcomm/",
    // Snapdragon SoC support under arch/ — mach-snapdragon is the SDM845
    // board family; arch/arm/dts/ carries sdm845-*.dtsi (incl. the
    // samsung starqltechn = OnePlus 6 u-boot dtsi).
    "arch/arm/mach-snapdragon/",
    "arch/arm/dts/",
    // Qualcomm vendor driver subtrees (vendor-specific code paths for
    // the SoC the phone runs).
    "drivers/clk/qcom/",
    "drivers/pinctrl/qcom/",
    "drivers/soc/qcom/",
    "drivers/misc/qcom/",
    // The SDM845 device trees and binding headers in the kernel DTS
    // import (the phone's SoC: dts/upstream/src/arm64/qcom/ plus its
    // dt-bindings clock headers).
    "dts/upstream/src/arm64/qcom/",
    "dts/upstream/include/dt-bindings/clock/qcom,",
    // Storage boot path: UFS + SCSI (the phone's UFS storage, probed by
    // `preboot=scsi scan`), MMC as fallback media.
    "drivers/ufs/",
    "drivers/scsi/",
    "drivers/mmc/",
    // USB boot/console path: dwc3 controller, gadget stack (fastboot,
    // serial-gadget, mass-storage) and fastboot command support.
    "drivers/usb/dwc3/",
    "drivers/usb/gadget/",
    "drivers/fastboot/",
    // Power + PMIC bus: SPMI is the pm8998 PMIC bus; power/ holds the
    // PMIC drivers the phone environment uses.
    "drivers/spmi/",
    "drivers/power/",
    // Clock/pinctrl/reset/IOMMU outside the qcom-only subtrees are T2;
    // only the SDM845-specific files named by the provenance ledger are
    // pulled up here (path-prefix rule, so these are whole-subtree when
    // the subtree itself is qcom-scoped).
    "drivers/iommu/",
    "drivers/reset/",
    "drivers/interconnect/",
    // Console + human interface the headless env drives: serial, input
    // (button-kbd volume-down override), LED, video/framebuffer.
    "drivers/serial/",
    "drivers/input/",
    "drivers/button/",
    "drivers/video/",
    // Network stack drivers (the Mac-side agent's ethernet path).
    "drivers/net/",
];

/// T2: common drivers and common lib — generic driver-model subtrees
/// not named by the T1 rules, plus `common/` runtime and `lib/`.
const T2_PREFIXES: [&str; 3] = ["drivers/", "common/", "lib/"];
