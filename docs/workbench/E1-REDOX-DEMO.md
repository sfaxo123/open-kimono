# E1 — first Redox OS-layer timeline demo in QEMU

Verified 2026-10-24 5:32 PM PT. No phone access or push. All source edits are
in the E1 project/base/bootloader worktrees; the kernel E1 worktree has no
source edit. Large build outputs are in the Redox build checkout.

## Result

The DT-only aarch64 live image boots with `-device ramfb` and the K8b simplefb
overlay, starts `traced`, and switches to its RAM-backed RedoxFS root. The
`timeline-demo-publish` process sends the 28 C4 fixture records to
`trace:publish`. Orbital starts `timeline --stream trace:subscribe/{}`; the
QEMU monitor screenshot shows the final mic, transcript, cleaned text,
reasoning, tool, and services/power state. The screenshot is
[`E1-REDOX-TIMELINE.png`](E1-REDOX-TIMELINE.png) (800×600). The serial
capture, with terminal control bytes normalized for reading, is
[`E1-REDOX-SERIAL.txt`](E1-REDOX-SERIAL.txt); the raw bytes remain in ignored
`out/integration/e1-live-qemu.log`.

The serial capture includes `Auto-selecting boot resolution`, `Switching to
live disk`, `vesad: simplefb 800x600 ... from device tree`, `init: switchroot
to /usr`, and `timeline-demo-publish: sent 28 events to trace:publish`. The
on-screen final state proves the app consumed the journal backlog. The
drive-free `scripts/integrate.sh --qemu` run separately reaches `ram:~$`,
starts `traced`, sends the 28 records, and confirms the graceful SPMI skip;
Orbital needs the live root filesystem's fonts and UI assets.

## Source commits

| Worktree | Commit | Change |
| --- | --- | --- |
| `base-e1` | `5113ced2` | Apply the B1 `traced` scheme patch |
| `base-e1` | `82a70629` | Start the E1 fixture publisher in RAM boot |
| `bootloader-e1` | `55ded92` | Auto-select GOP mode for live media |
| `redox-oneplus6-e1` | `524a080` | Redox `trace:` feed and fixture publisher |
| `redox-oneplus6-e1` | `a005ef3`, `1c0a705` | Reproducible build/capture scripts and exported fork patches |

`kernel-e1` remains at `d73469ef` (`op6/integration-3`) with no E1 source
change. The base and bootloader patch exports are under `patches/e1/`.

## Exact reproduction

From `$HOME/Developer/experiments/redox-oneplus6-e1`:

```sh
# Once per clean Redox build checkout: build the official desktop-minimal
# live image with the existing aarch64 cookbook toolchain and binary repo.
podman run --rm -t --workdir /mnt/redox --pids-limit=1024 \
  --volume $HOME/Developer/forks/redox/redox:/mnt/redox \
  --volume $HOME/Developer/forks/redox/redox/build/podman:/root \
  --env PATH=/root/.cargo/bin:/mnt/redox/prefix/aarch64-unknown-redox/sysroot/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
  redox-base make build/aarch64/desktop-minimal/redox-live.iso \
  ARCH=aarch64 CONFIG_NAME=desktop-minimal PODMAN_BUILD=0 FSTOOLS_NO_MOUNT=1

cargo test --manifest-path apps/timeline/Cargo.toml --quiet
bash scripts/integrate.sh --qemu
bash scripts/e1-live.sh
bash scripts/e1-qemu.sh
```

`e1-live.sh` uses the real aarch64 Redox cookbook build to stage E1 base's
full initfs, copies the E1 kernel and cross-built app binaries into a copy of
the live image, and verifies them after a RedoxFS unmount/remount. It builds
the E1 live bootloader and serves it over PXE. The bootloader auto-selects a
GOP mode because QEMU has no guest keyboard attached. `e1-qemu.sh` sets
`bootindex=1` on the PXE NIC and `bootindex=2` on the live disk, then waits
for orange timeline pixels before accepting a monitor screendump. It stops
QEMU on exit.

## Verification and artifacts

- `cargo test --manifest-path apps/timeline/Cargo.toml --quiet`: 31 passed,
  including scheme-feed polling, journal-drop visibility, and the C4 model.
- `bash scripts/integrate.sh --qemu`: passed; kernel/base/EFI builds and the
  DT-only drive-free serial checks passed. The final RAM payload SHA-256 is
  recorded in `out/phone-boot/SHA256SUMS`.
- `bash scripts/e1-live.sh`: passed its unmount/remount byte comparisons for
  the kernel, initfs, and two timeline executables.
- `bash scripts/e1-qemu.sh`: passed; screenshot visually inspected. Image
  SHA-256 `c02012eaa182800cc7ed937e0e3ebf7b436f2af3e9dacb5c57bb00c89df361bc`;
  PXE live bootloader SHA-256
  `b624be2eeb930c8db1f2cf94d977e580b0ff1edb6d4ca79b62b124e0319ce3d5`;
  screenshot SHA-256
  `27966a473bee4877e8478f683d3e2d326c1524814813483966c210be68509070`.

## Gaps and stop conditions

- The graphical run uses a QEMU virtio disk to load the 253 MiB live image,
  then copies it to RAM. The separate drive-free run proves `traced` and the
  producer but has no Orbital rootfs. This does not prove a drive-free
  graphical phone boot.
- QEMU has no SDM845 display, SPMI, touch, or phone IRQ 386 hardware. No
  physical device behavior is inferred.
- The live serial log still records nonfatal virtio block/network driver
  panics (`MSI-X is not implemented on this architecture`) and missing
  network/audio devices. The live root mounts and the UI renders despite
  them. Stop if the root switch, 28-event marker, or rendered timeline
  disappears on reproduction.

Rollback: discard the ignored `e1-live.iso` and `out/` artifacts, then revert
the E1 commits in their respective worktrees. The original
`desktop-minimal/redox-live.iso` and the K8c integration worktrees are not
modified by these commands.
