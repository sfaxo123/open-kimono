#!/usr/bin/env python3
"""Build a RAM-bootable Android image with an EFI app embedded in U-Boot."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

PROJECT = Path(__file__).resolve().parents[2]
ARTIFACT = PROJECT / "out/r1-reset-v4"
SOURCE = ARTIFACT / "src"
SOURCE_REPO = ARTIFACT / "u-boot-repo"
UPSTREAM_UBOOT_REPO = Path("$HOME/Developer/forks/u-boot")
UBOOT_COMMIT = "3e15c2819caa7e14e1e4b2ff758a41cbc58a7311"
# Pins for the U-Boot overlay this script generates, taken from the
# PROVEN Attempt-#1 packager (build-embedded-efi.py, whose CMD_C/ENV are
# byte-identical) and confirmed against the Attempt-#2 build manifest
# (package-v3/manifest.json). Without these, the manifest hashes are
# self-referential: drift in the generated C glue or boot env would be
# recorded as its own new truth instead of failing (Wonder diff 9399dc3).
CMD_C_SHA256_PIN = "d882d4f62477fad6613ad2973438a77086566697b3244a62995f0675838fd5d7"
# The pinned phone DTB input (Attempt-#1/#2 carrier FDT, re-verified by
# root byte-inspection). The manifest records sha256(DTB); without this
# pin that receipt is self-attestation (Wonder diff 278daaf).
DTB_SHA256_PIN = "48518e1de2b2024aa078f4ba2ce2dfc1e3f9e8923c4a212b320413dbb1259383"
ENV_SHA256_PIN = "7b935c2a25af9c7bb6411cd3481530c6ce4602bb07f489f4327883765408d502"
INPUTS = PROJECT / "scripts/phone/embedded-efi"
BASE_CONFIG = INPUTS / "baseline.config"
DTB_SOURCE = INPUTS / "sdm845-oneplus-enchilada.dts"
PHONE_BOOT = Path("$HOME/artifacts/handoff/oneplus-redox-20261024/phone-boot")
HOST_TOOLS = PHONE_BOOT / "host-tools/usr/bin"
DTB = INPUTS / "sdm845-oneplus-enchilada.dtb"
DIAG = PROJECT / "tools/efi-diag/target/aarch64-unknown-uefi/release/op6-efi-diag.efi"


ENV = """bootdelay=-2
bootretry=-1
stdin=serial,button-kbd
stdout=serial,vidconsole
stderr=serial,vidconsole
preboot=echo OP6_EFI_DIAG_NOABORT_V2; printenv bootcmd bootdelay stdin stdout stderr
bootcmd=op6efi
"""

CMD_C = r'''// SPDX-License-Identifier: GPL-2.0+
#include <command.h>
#include <asm/global_data.h>
#include <asm-generic/sections.h>
#include <mapmem.h>
#include <linux/types.h>
#include <linux/printk.h>

DECLARE_GLOBAL_DATA_PTR;

extern char embedded_efi_start[];
extern char embedded_efi_end[];

static int do_op6efi(struct cmd_tbl *cmdtp, int flag, int argc,
                     char *const argv[])
{
        ulong start = map_to_sysmem(embedded_efi_start);
        ulong end = map_to_sysmem(embedded_efi_end);
        ulong image_start = map_to_sysmem(__image_copy_start);
        ulong image_end = map_to_sysmem(__image_copy_end);
        ulong monitor_start = gd->relocaddr;
        ulong monitor_end = monitor_start + gd->mon_len;
        ulong size = end - start;

        printf("op6efi: EFI [0x%lx,0x%lx), image [0x%lx,0x%lx), monitor [0x%lx,0x%lx)\n",
               start, end, image_start, image_end, monitor_start, monitor_end);
        if (!size || start < image_start || end > image_end ||
            start < monitor_start || end > monitor_end)
                return CMD_RET_FAILURE;

        /* bootefi requires the size after a colon for an in-memory EFI image. */
        return run_commandf("bootefi %lx:%lx %lx", start, size,
                            (ulong)map_to_sysmem(gd->fdt_blob)) ?
                            CMD_RET_FAILURE : CMD_RET_SUCCESS;
}

U_BOOT_CMD(op6efi, 1, 0, do_op6efi,
           "boot the embedded EFI application",
           "\nRuns the linked EFI payload with the control FDT.");
'''


def dtb_config_bootdelay_override(fdtget: str, dtb: Path, label: str) -> bool:
    """Measured /config/bootdelay probe for a DTB. Fail-closed: a
    missing /config node (fdtget FDT_ERR_NOTFOUND) is a real measured
    False (no node can carry no override); ANY other fdtget failure
    aborts instead of being recorded as False (Wonder diff bb6b111)."""
    r = subprocess.run([fdtget, "-p", str(dtb), "/config"], text=True,
                       capture_output=True)
    if r.returncode == 0:
        return "bootdelay" in r.stdout.splitlines()
    if "FDT_ERR_NOTFOUND" in (r.stderr or ""):
        return False
    raise SystemExit(
        f"{label}: fdtget /config probe failed (rc={r.returncode}): "
        f"{(r.stderr or '').strip()} - cannot verify, aborting")


def run(argv: list[str], *, env: dict[str, str] | None = None, cwd: Path | None = None) -> None:
    print("+", " ".join(map(str, argv)), flush=True)
    subprocess.run(argv, check=True, cwd=cwd, env=env)


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()

def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--payload-path", type=Path, required=True, help="verified RAM boot EFI payload")
    ap.add_argument("--payload-sha256", required=True, help="expected SHA-256 of the EFI payload")
    ap.add_argument("--out", type=Path, default=ARTIFACT / "package", help="fresh build/package directory")
    args = ap.parse_args()
    if hashlib.sha256(CMD_C.encode()).hexdigest() != CMD_C_SHA256_PIN:
        raise SystemExit("op6efi C glue drifted from the pinned Attempt-#1 overlay (CMD_C_SHA256_PIN)")
    if hashlib.sha256(ENV.encode()).hexdigest() != ENV_SHA256_PIN:
        raise SystemExit("boot environment drifted from the pinned Attempt-#1 overlay (ENV_SHA256_PIN)")
    payload = args.payload_path.resolve()
    if not payload.is_file():
        raise SystemExit(f"missing EFI payload: {payload}")
    actual_payload_sha256 = sha256(payload)
    if actual_payload_sha256 != args.payload_sha256:
        raise SystemExit(f"EFI payload SHA-256 mismatch: expected {args.payload_sha256}, got {actual_payload_sha256}")
    for p in (BASE_CONFIG, DTB, DTB_SOURCE, HOST_TOOLS / "mkbootimg", HOST_TOOLS / "unpack_bootimg"):
        if not p.exists():
            raise SystemExit(f"required input missing: {p}")
    if sha256(DTB) != DTB_SHA256_PIN:
        raise SystemExit(f"phone DTB drifted from DTB_SHA256_PIN: {sha256(DTB)}")
    dtc = shutil.which("dtc")
    if not dtc:
        raise SystemExit("dtc is required to verify the reconstructed phone-DTB source")
    fdtget = shutil.which("fdtget")
    if not fdtget or subprocess.check_output([fdtget, str(DTB), "/", "model"], text=True).strip() != "OnePlus 6":
        raise SystemExit(f"phone DTB is not confirmed as OnePlus 6: {DTB}")
    phone_dtb_config_bootdelay_override = dtb_config_bootdelay_override(
        fdtget, DTB, "phone DTB")
    if phone_dtb_config_bootdelay_override:
        raise SystemExit("phone DTB /config/bootdelay overrides the no-abort autoboot setting")
    ARTIFACT.mkdir(parents=True, exist_ok=True)
    if not SOURCE_REPO.exists():
        run(["git", "clone", "--shared", "--no-checkout", str(UPSTREAM_UBOOT_REPO), str(SOURCE_REPO)])
    if not SOURCE.exists():
        run(["git", "-C", str(SOURCE_REPO), "worktree", "add", "--detach",
             str(SOURCE), UBOOT_COMMIT])
    if not (SOURCE / ".git").exists():
        raise SystemExit(f"refusing non-worktree source path: {SOURCE}")
    actual = subprocess.check_output(["git", "-C", str(SOURCE), "rev-parse", "HEAD"], text=True).strip()
    if actual != UBOOT_COMMIT:
        raise SystemExit(f"U-Boot worktree must stay at {UBOOT_COMMIT}; found {actual}")
    allowed_dirty = {"cmd/Makefile", "board/qualcomm/qcom-phone-embedded-efi.env",
                     "cmd/op6efi-payload.S", "cmd/op6efi.c"}
    status = subprocess.check_output(["git", "-C", str(SOURCE), "status", "--porcelain",
                                      "--untracked-files=all"], text=True)
    dirty_paths = {line[3:] for line in status.splitlines()}
    if dirty_paths - allowed_dirty:
        raise SystemExit(f"refusing unrelated worktree changes: {sorted(dirty_paths - allowed_dirty)}")
    original_makefile = subprocess.check_output(["git", "-C", str(SOURCE), "show",
                                                  f"{UBOOT_COMMIT}:cmd/Makefile"], text=True)
    anchor = 'obj-$(CONFIG_CMD_BOOTEFI) += bootefi.o\n'
    expected_makefile = original_makefile.replace(
        anchor, anchor + 'obj-$(CONFIG_CMD_BOOTEFI) += op6efi.o op6efi-payload.o\n', 1)
    if (SOURCE / "cmd/Makefile").read_text() not in (original_makefile, expected_makefile):
        raise SystemExit("refusing an unexpected cmd/Makefile edit in U-Boot worktree")

    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    prior_manifest = out / "manifest.json"
    if prior_manifest.is_file():
        prior = json.loads(prior_manifest.read_text())
        if (prior.get("payload_kind") != "redox-v4-reset" or
                prior.get("payload_sha256") != sha256(payload)):
            raise SystemExit(f"output contains a different payload receipt; choose a fresh --out: {out}")
    roundtrip_dtb = out / "phone-dtb-source-roundtrip.dtb"
    roundtrip_dts = out / "phone-dtb-source-roundtrip.dts"
    run([dtc, "-I", "dts", "-O", "dtb", "-o", str(roundtrip_dtb), str(DTB_SOURCE)])
    run([dtc, "-I", "dtb", "-O", "dts", "-o", str(roundtrip_dts), str(roundtrip_dtb)])
    if roundtrip_dts.read_bytes() != DTB_SOURCE.read_bytes():
        raise SystemExit("reconstructed phone DTS does not round-trip canonically through dtc")
    (SOURCE / "cmd/op6efi.c").write_text(CMD_C)
    asm = (SOURCE / "cmd/op6efi-payload.S")
    payload_asm = str(payload.resolve()).replace('\\', '\\\\').replace('"', '\\"')
    asm.write_text(f'''/* SPDX-License-Identifier: GPL-2.0+ */\n.section .rodata\n.balign 16\n.global embedded_efi_start\n.global embedded_efi_end\nembedded_efi_start:\n.incbin "{payload_asm}"\nembedded_efi_end:\n''')
    mk = SOURCE / "cmd/Makefile"
    mktext = mk.read_text()
    if "obj-$(CONFIG_CMD_BOOTEFI) += op6efi.o op6efi-payload.o" not in mktext:
        if anchor not in mktext:
            raise SystemExit("cannot locate cmd/Makefile integration point")
        mk.write_text(mktext.replace(anchor, anchor + 'obj-$(CONFIG_CMD_BOOTEFI) += op6efi.o op6efi-payload.o\n', 1))
    envfile = SOURCE / "board/qualcomm/qcom-phone-embedded-efi.env"
    envfile.write_text(ENV)
    env_values = dict(line.split("=", 1) for line in ENV.splitlines() if "=" in line)
    if env_values.get("bootdelay") != "-2" or env_values.get("bootcmd") != "op6efi" or not env_values.get("preboot", "").startswith("echo OP6_EFI_DIAG_NOABORT_V2; printenv"):
        raise SystemExit("generated environment must autoboot op6efi with bootdelay=-2")

    config = out / ".config"
    shutil.copy2(BASE_CONFIG, config)
    run([str(SOURCE / "scripts/config"), "--file", str(config), "--set-str",
         "ENV_DEFAULT_ENV_TEXT_FILE", "board/qualcomm/qcom-phone-embedded-efi.env"])
    run([str(SOURCE / "scripts/config"), "--file", str(config), "--set-val", "BOOTDELAY", "-2"])
    hostenv = os.environ.copy()
    hostenv["PATH"] = f"{HOST_TOOLS}:{hostenv.get('PATH', '')}"
    hostenv["BISON_PKGDATADIR"] = str(PHONE_BOOT / "host-tools/usr/share/bison")
    hostenv["M4"] = str(HOST_TOOLS / "m4")
    hostenv["SOURCE_DATE_EPOCH"] = subprocess.check_output(
        ["git", "-C", str(SOURCE), "show", "-s", "--format=%ct", UBOOT_COMMIT], text=True
    ).strip()
    run(["make", f"O={out}", "ARCH=arm", "CROSS_COMPILE=aarch64-linux-gnu-", "olddefconfig"], cwd=SOURCE, env=hostenv)
    cfg = config.read_text()
    for required in ('CONFIG_BOOTDELAY=-2', 'CONFIG_CMD_BOOTEFI=y',
                     'CONFIG_ENV_DEFAULT_ENV_TEXT_FILE="board/qualcomm/qcom-phone-embedded-efi.env"'):
        if required not in cfg:
            raise SystemExit(f"configuration did not retain {required!r}")
    run(["make", f"O={out}", "ARCH=arm", "CROSS_COMPILE=aarch64-linux-gnu-", "-j8"], cwd=SOURCE, env=hostenv)
    built_dtb = out / "u-boot.dtb"
    if not built_dtb.is_file():
        raise SystemExit(f"built U-Boot DTB missing; /config/bootdelay check cannot run: {built_dtb}")
    built_dtb_config_bootdelay_override = dtb_config_bootdelay_override(
        fdtget, built_dtb, "built U-Boot DTB")
    if built_dtb_config_bootdelay_override:
        raise SystemExit("built U-Boot DTB /config/bootdelay overrides the no-abort setting")

    gzip_path = out / "u-boot-nodtb.bin.gz"
    import gzip
    with (out / "u-boot-nodtb.bin").open("rb") as src, gzip_path.open("wb") as dst:
        with gzip.GzipFile(fileobj=dst, mode="wb", filename="", compresslevel=9, mtime=0) as gz:
            shutil.copyfileobj(src, gz)
    payload_with_dtb = out / "u-boot-nodtb.bin.gz-dtb"
    payload_with_dtb.write_bytes(gzip_path.read_bytes() + DTB.read_bytes())
    mkbootimg = out / "mkbootimg.py"
    original = (HOST_TOOLS / "mkbootimg").read_text()
    original = re.sub(r"^from gki\.generate_gki_certificate import generate_gki_certificate\s*\n", "", original, flags=re.M)
    mkbootimg.write_text(original)
    image = out / "op6-embedded-efi-redox-v4-reset.img"
    run([sys.executable, str(mkbootimg), "--kernel", str(payload_with_dtb), "--output", str(image),
         "--pagesize", "4096", "--base", "0", "--kernel_offset", "0x8000",
         "--ramdisk_offset", "0x01000000", "--second_offset", "0x00f00000", "--tags_offset", "0x100",
         "--header_version", "0"], env=hostenv)
    unpack = out / "unpacked"
    unpack.mkdir(exist_ok=True)
    info = subprocess.check_output([str(HOST_TOOLS / "unpack_bootimg"), "--boot_img", str(image), "--out", str(unpack)], text=True)
    extracted = unpack / "kernel"
    if not extracted.is_file() or extracted.read_bytes() != payload_with_dtb.read_bytes():
        raise SystemExit("unpacked Android kernel payload differs from packaged gzip+DTB")
    nodtb = (out / "u-boot-nodtb.bin").read_bytes()
    blob = payload.read_bytes()
    offset = nodtb.find(blob)
    if offset < 0 or nodtb.find(blob, offset + 1) >= 0:
        raise SystemExit("embedded EFI blob not found exactly once in U-Boot image")
    syms = subprocess.check_output(["aarch64-linux-gnu-nm", "-n", str(out / "u-boot")], text=True)
    symmap = {}
    for line in syms.splitlines():
        fields = line.split()
        if len(fields) >= 3 and fields[-1] in (
                "embedded_efi_start", "embedded_efi_end",
                "__image_copy_start", "__image_copy_end"):
            symmap[fields[-1]] = int(fields[0], 16)
    start, end = symmap.get("embedded_efi_start"), symmap.get("embedded_efi_end")
    if start is None or end is None or end - start != len(blob):
        raise SystemExit(f"bad embedded payload symbols: {symmap}")
    image_start, image_end = symmap.get("__image_copy_start"), symmap.get("__image_copy_end")
    if image_start is None or image_end is None or start < image_start or end > image_end:
        raise SystemExit(f"EFI symbols lie outside the linked U-Boot image: {symmap}")
    if start - image_start != offset or end - image_start > len(nodtb):
        raise SystemExit("linked EFI symbol offsets do not match embedded bytes in u-boot-nodtb.bin")
    map_text = (out / "u-boot.map").read_text(errors="replace")
    rodata = re.search(r"^\.rodata\s+0x([0-9a-f]+)\s+0x([0-9a-f]+)", map_text, re.M)
    if not rodata:
        raise SystemExit("could not find .rodata bounds in U-Boot linker map")
    ro_start, ro_size = (int(rodata.group(1), 16), int(rodata.group(2), 16))
    if start < ro_start or end > ro_start + ro_size:
        raise SystemExit("embedded EFI symbols lie outside .rodata")
    if nodtb[offset:offset + len(blob)] != blob:
        raise SystemExit("embedded EFI differs from source")
    manifest = {
        "u_boot_commit": actual,
        "source_date_epoch": int(hostenv["SOURCE_DATE_EPOCH"]),
        "u_boot_source_repo": str(SOURCE_REPO),
        "packager_sha256": sha256(Path(__file__).resolve()),
        "project_inputs": {
            "baseline_config_sha256": sha256(BASE_CONFIG),
            "phone_dtb_sha256": sha256(DTB),
            "phone_dtb_dts_sha256": sha256(DTB_SOURCE),
            "phone_dtb_source_canonical_roundtrip": True,
            "command_source_sha256": hashlib.sha256(CMD_C.encode()).hexdigest(),
            "payload_assembly_sha256": sha256(asm),
            "boot_environment_sha256": sha256(envfile),
        },
        "efi_overlay_sha256": hashlib.sha256(
            CMD_C.encode() + asm.read_bytes() + envfile.read_bytes() +
            (SOURCE / "cmd/Makefile").read_bytes()
        ).hexdigest(),
        "base_config_sha256": sha256(BASE_CONFIG),
        "phone_dtb_sha256": sha256(DTB),
        "phone_dtb_model": "OnePlus 6",
        "phone_dtb_config_bootdelay_override": phone_dtb_config_bootdelay_override,
        "built_dtb_config_bootdelay_override": built_dtb_config_bootdelay_override,
        "bootdelay_environment": "-2",
        "bootdelay_config": "CONFIG_BOOTDELAY=-2",
        "phone_dtb_rebuild_provenance": "source/build recipe unavailable; pinned checked-in DTB input",
        "built_config_sha256": sha256(config),
        "payload_kind": "redox-v4-reset",
        "payload_path": str(payload),
        "payload_sha256": sha256(payload),
        "payload_size": len(blob),
        "embedded_symbol_start": hex(start),
        "embedded_symbol_end": hex(end),
        "linked_image_range": [hex(image_start), hex(image_end)],
        "rodata_range": [hex(ro_start), hex(ro_start + ro_size)],
        "embedded_file_offset_nodtb": offset,
        "dtb_sha256": sha256(DTB),
        "android_image_sha256": sha256(image),
        "android_header_inspection": info.strip(),
        "build_directory": str(out),
        "physical_boot_or_flash_performed": False,
    }
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps(manifest, indent=2))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except subprocess.CalledProcessError as exc:
        raise SystemExit(exc.returncode)
