# OnePlus 6 (enchilada) phone-return runbook

Script: `scripts/phone/return-runbook.sh` · tests: `scripts/phone/tests.sh`
Written device-free (2026-10-25) per `~/artifacts/handoff/op6-return-runbook/PLAN.md`.
A later agent runs this **when the phone is reachable again**. No phone, Mac,
or fastboot command was executed while authoring or testing it.

## What it is for

The OnePlus 6 went unreachable after the 8:55 AM PT (2026-10-25) RAM-boot of
`boot-autounlock.img` never reached Wi-Fi/mesh and macOS created no USB NIC —
cause UNKNOWN. This runbook takes the phone from "unknown state" back to a
classified state, into U-Boot via RAM boot only, and (optionally) makes the
one allowed Redox try, recording everything.

## Usage

```bash
scripts/phone/return-runbook.sh                 # DRY-RUN (default): prints every command, touches nothing
scripts/phone/return-runbook.sh --live          # actually execute against Mac/phone/fastboot
scripts/phone/return-runbook.sh --live --redox  # additionally unlock phase P4 (one Redox try per run)
scripts/phone/return-runbook.sh --selftest-guard # prove the flash/erase guard refuses known-bad strings
```

Every run writes a receipt to `out/phone-runbook/<run-id>/receipt.tsv`
(TSV: `ts phase action status detail`) plus per-step `*.out`/`*.err` files.
Run-id is timestamped; set `RUN_ID=...` to resume/extend a run dir (the
one-Redox-try marker lives there).

## Hard rules baked into the script

- **Dry-run is the default.** Every command is printed (`DRYRUN …`) and
  written to the receipt; nothing is executed. `--live` is the only opt-in.
- **`fastboot boot` (RAM) ONLY.** A guard scans every command string and
  refuses anything matching `fastboot flash|erase|format|oem unlock`,
  `flash.elf`, or any mention of `xbl`/`abl`/`devinfo`/`persist`/`modem`
  (word-boundary match, so `/dev/persist` and `echo modem` are caught too).
  Refusals are receipted with status `refused` and never executed.
  Verify with `--selftest-guard`.
- **Hard timeouts everywhere:** per-command `CMD_TIMEOUT` (20 s default) and
  phase budgets `P0/P2/P3/P4_TIMEOUT` (60/300/120/180 s). A timeout is
  receipted as `timeout` (rc 124), not `fail`. Local (3090) commands use GNU
  `timeout`; **Mac-side** commands use the portable
  `perl -e 'alarm shift; exec @ARGV' N cmd` form — macOS ships no GNU timeout.
  `fastboot` is always invoked by its explicit Mac path
  (`$HOME/tools/platform-tools/fastboot`, expanded on the Mac), and serial
  setup uses `stty -f` (BSD flag), never GNU `-F`.
- **One Redox try per run:** `--redox` unlocks P4; a `redox-tried` marker in
  the run dir makes any second attempt `refused`.
- **Never reboot casually** (LUKS): only P2 reboots to the bootloader, and
  only from a classified `LINUX_UP` state.

## Phases

| Phase | What it does |
|---|---|
| P0 reachability | mesh `ssh oneplus`, Mac `ssh macbook`, `fastboot devices`, USB serial (`ioreg … postmarketOS`), USB NIC (`networksetup`) → classify **LINUX_UP / FASTBOOT / STUCK_EARLY / ABSENT** |
| P1 (if LINUX_UP) | record `/proc/cmdline`, cryptkey presence, keyslot status, `journalctl -b -1` (the 8:55 AM PT run), copy logs back. **Does not reboot.** |
| P2 to fastboot | `systemctl reboot --reboot-argument=bootloader` (LINUX_UP only), then poll `fastboot devices` up to 300 s. STUCK_EARLY → note "operator power-cycle needed" and stop. |
| PS staging | fastboot runs **on the Mac**, so the 3090 payload paths don't exist there: `scp` every payload to `macbook:~/op6-runbook/<run-id>/`, then verify `sha256sum` (3090) against `shasum -a 256` (Mac) before any phase uses the Mac-side path. |
| P3 U-Boot | `fastboot boot u-boot-enchilada-v0.img` (RAM only), then **discover the console**: U-Boot's serial on the Mac is a CDC-ACM `/dev/cu.usbmodem*` that only exists after U-Boot enables its USB gadget, so it is found by polling on the Mac (`ACM_TIMEOUT` budget), never hardcoded. Then `stty -f` raw + a portable `perl -e 'alarm shift; exec @ARGV'` timeout around `cat` (macOS has no GNU `timeout`, its `stty` uses `-f`, no `screen`), assert the U-Boot banner, `bdinfo`, then `reset` → back to pmOS. Skipped when ABSENT/STUCK_EARLY; console steps skipped (receipted) when no ACM device appears. |
| P4 Redox (`--redox`) | payloads are **not on the phone** (and its storage is UFS, not mmc), so they load over USB via U-Boot's own fastboot gadget. The phone has **one DWC3 gadget**: the moment `fastboot usb 0` starts it, the CDC-ACM console is gone — so the console is typed **exactly once** (`fastboot usb 0 -l 0xa0000000 -s 0x6000000`, or the boot-menu "Enable fastboot mode") and U-Boot is then driven only over fastboot: Mac `fastboot stage efi-diag.efi`, Mac `fastboot oem run:'bootefi 0xa0000000 ${fdtcontroladdr}'` — then the same stage + `oem run` pair for `redox-ramboot.efi`. No console after Redox is **expected** → receipt `awaiting-operator` and stop; the operator presses the power button. |

## Facts the runbook assumes (verified 2026-10-25)

- Phone USB is on the MacBook (`ssh macbook`); fastboot/adb at
  `~/tools/platform-tools`; USB net needs `phone-via-mac.sh setup`
  (phone at <phone-usb-ip> = `ssh oneplus-usb` **from the Mac**); mesh alias
  from the 3090 is `ssh oneplus`.
- Backups: Mac `~/op6-backup-20261025/{boot,dtbo,vbmeta}_{a,b}.img` +
  SHA256SUMS; active slot `_a`.
- Payloads: `~/artifacts/handoff/oneplus-redox-20261024/phone-boot/`
  (`u-boot-enchilada-v0.img`, `efi-diag.efi`, `redox-ramboot.efi`);
  paths are env-overridable (`PAYLOAD_DIR`, `UBOOT_IMG`, `EFI_DIAG`, `REDOX_IMG`).
  These live on the **3090**; PS stages them to `macbook:~/op6-runbook/<run-id>/`
  (scp + sha256 verified on both ends) because fastboot executes on the Mac.
- Reaching fastboot from pmOS previously took > 2 min — P2's 300 s poll
  budget accounts for that.
- The U-Boot serial console on the Mac is a **CDC-ACM** `/dev/cu.usbmodem*`
  that appears only after U-Boot enables its USB gadget — the runbook polls
  for it (`ACM_TIMEOUT`, default 60 s) and uses the discovered path for every
  console read/write.
- U-Boot fastboot gadget facts (fork `~/Developer/forks/u-boot`, verified in
  source): `fastboot usb <idx>` starts the gadget from the console
  (`cmd/fastboot.c`, `do_fastboot_usb`); the download buffer may be pinned on
  the command line with `-l <addr>` / `-s <size>` (`doc/android/fastboot.rst`);
  a completed `stage` sets `${filesize}` (`drivers/fastboot/fb_command.c`,
  `env_set_hex("filesize", image_size)`). This fork does **not** set
  `${fastboot_buffer}`/`${fastboot_blocks}` (only
  `doc/board/bsh/imx6ulz_bsh_smm_m2.rst` sets `fastboot_buffer` by hand), so
  the runbook pins the buffer with `-l 0xa0000000` and boots from that address.
- **`-l` is mandatory, not cosmetic:** the shipped build's
  `CONFIG_FASTBOOT_BUF_ADDR=0x1A000000`
  (`~/artifacts/handoff/oneplus-redox-20261024/phone-boot/u-boot-build/.config`)
  is outside SDM845 RAM, which starts at `0x80000000` — the default buffer
  address is unusable, hence the explicit `-l 0xa0000000 -s 0x6000000`.
- **One DWC3 gadget:** while `fastboot usb 0` is active the CDC-ACM console
  does not exist, so nothing may be written to `/dev/cu.usbmodem*` after the
  switch. U-Boot is driven via `fastboot oem run:'<ucmd>'` instead:
  `CONFIG_FASTBOOT_OEM_RUN=y` (same `.config`) maps `oem run` to `run_ucmd`
  → `run_command(cmd_parameter)` (`drivers/fastboot/fb_command.c`), and this
  fork's `fastboot_handle_command()` splits the wire command at the first
  `:` (`strsep(&cmd_parameter, ":")`), which is why the client syntax is
  `fastboot oem run:'bootefi 0xa0000000 ${fdtcontroladdr}'` — the fork's own
  example is `fastboot oem run:'echo Hello at $baudrate baud'`
  (`doc/android/fastboot.rst`); U-Boot's hush expands `${fdtcontroladdr}`.

## Tests

`bash scripts/phone/tests.sh` — offline only. Fake `ssh`/`scp`/`fastboot`/
`ioreg`/`shasum` shims on PATH prove: dry-run executes nothing, `--live`
drives only the shims, all four P0 classifications work, payloads are staged
to the Mac and sha256-verified both ends, Mac-side commands are macOS-
portable (explicit fastboot path, no GNU `timeout`, `stty -f`), the ACM
console is discovered by polling (never hardcoded), P4 starts the fastboot
gadget exactly once and drives U-Boot only via `fastboot oem run` (no ACM
write after the gadget switch, no mmc anywhere), hard timeouts record as
`timeout`, the guard refuses forbidden strings even when injected via
config, and the one-Redox-try marker holds across invocations.
55 checks, all green.

## Hardware findings (Claude, 2026-10-25 8:40 PM PT — real OnePlus 6)
- pmOS on boot_a now auto-unlocks (keyfile in initramfs, cmdline `cryptkey=/crypt.key`) and autologs in; verified unattended
  RAM boot AND flashed boot_a reboot (image sha256 ed2ba40a…, backup boot_a.img on the Mac). P1/P2 can reboot freely.
- `systemctl reboot --reboot-argument=bootloader` reaches ABL fastboot in ~125-130 s (budget ≥ 300 s, as the script does).
- The Mac's USB NCM interface is not being created by macOS (driver binds, no BSD interface); use the shared Wi-Fi LAN
  instead: phone `user@<phone-lan-ip>` from the Mac (~20 MB/s). Mesh via satellite uplink is ~0.1–0.5 Mbit/s — stage files on the Mac.
- **Blocker for P3/P4 unattended:** `fastboot boot u-boot-enchilada-v0.img` starts U-Boot but it exposes NO USB device
  (no CDC-ACM, no fastboot gadget) — it waits at its on-screen boot menu. Unattended use needs a U-Boot build whose
  preboot/bootmenu default enables the ACM console and/or `fastboot usb 0` (tracked as a follow-up).

## Unattended Linux reboot acceptance — 2026-10-26 05:59 AM PT

Operator recovered the phone from EDL with the physical buttons, then authorized verification of password-free remote boot/access as priority1. Root ran two normal Linux `sudo -n systemctl reboot` cycles through MacBook SSH, without bootloader arguments or user interaction. Both passed: 161.9seconds and 162.9seconds to verified readiness.

Boot IDs:
- baseline: e4b94659-217a-4812-b83b-4d6f674f53a6
- cycle1: 8aaffb9c-f8a9-4ea5-b87d-6a573ba9322f
- cycle2: 48b59e5b-a78e-4454-854d-9f92f0bd3904

Each acceptance required a changed boot ID, `/dev/mapper/root` mounted, sshd/greetd active, a user seat0 session, and successful `sudo -n true`. Both MacBook→Wi-Fi SSH (`user@<phone-lan-ip>`) and fleet `ssh oneplus` work after the final cycle. Current greetd config is `/etc/phrog/greetd-config.toml`, with an initial session launching `systemd-cat phosh-session` as user. Boot cmdline has `cryptkey=/crypt.key`; persistent cmdline config is `/etc/kernel-cmdline.d/90-luks-autounlock.conf`; mkinitfs maps `/etc/luks/root.key` to `/crypt.key`. No key contents were read or logged. Startup sshd/greetd/NetworkManager are enabled. Existing auto-unlock/autologin settings needed no change to pass.

Raw receipt and sanitized config proof: `$HOME/artifacts/handoff/redox-perplexity/phone-unattended/`. Owned unit `redox-phone-unattended-20261026.service` ended; durable journal says TWO_REBOOTS_PASS, and receipt status is passed. A removed unit's default exit value was not used as proof.

This accepts normal Linux reboot/unlock/login/admin access. It does not prove automatic recovery from EDL, a kernel hang or an experimental U-Boot boot. Before further RAM-boot experiments, verify an unattended safe return to this known-good Linux environment. Never use U-Boot's Reset device menu or its `reset` command on this phone; preserve the known-good boot slot and backups. No flash, boot-slot change, EDL tool, credential removal, or kernel experiment occurred in this acceptance.
