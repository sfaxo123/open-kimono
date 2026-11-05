# W0: Redox aarch64 Podman build and QEMU boot

**Result (2026-10-24, PT):** The `minimal` and `server` aarch64 disk images built successfully. QEMU `virt` booted UEFI, the Redox bootloader, and the kernel, but **neither image reached a login prompt or shell**. Storage driver failures prevented mounting RedoxFS. This is a partial result, not a successful OS boot. No desktop image was built because minimal did not work. No `sudo` command or host package installation was used. The build's Podman image installed Debian build dependencies inside its container.

## Sources and scope

- Repository `README.md`, `config/minimal.toml`, `config/server.toml`, `mk/podman.mk`, `mk/qemu.mk`, and `mk/config.mk` at commit `e079d707b`.
- Redox Book: [Building Redox](https://github.com/redox-os/book/blob/master/src/podman-build.md), [ARM64](https://github.com/redox-os/book/blob/master/src/aarch64.md), and [Advanced Podman Build](https://github.com/redox-os/book/blob/master/src/advanced-podman-build.md). The ARM64 page says `ARCH=aarch64` selects the cross build and calls ARM64 support limited. The Podman page recommends `REPO_BINARY=1` for prebuilt packages.
- Work was confined to the Redox checkout/build tree and this artifact folder. Podman used its normal container storage, and the upstream installer used temporary `/tmp/redox_installer_*` paths during image creation.

## Images and build time

| Config | Image | Size | SHA-256 | Successful final make time |
| --- | --- | ---: | --- | ---: |
| minimal | `$HOME/Developer/forks/redox/redox/build/aarch64/minimal/harddrive.img` | 196 MiB | `357e42e88609b3e154a501720e312b5027cb2d5de105d8a19d18d62cd6e7b304` | 12.36 s, using already prepared toolchain |
| server | `$HOME/Developer/forks/redox/redox/build/aarch64/server/harddrive.img` | 512 MiB | `033c90f45843d71e0a78e205e13195fedc996efef602353c10849c4c25f10944` | 13.18 s, using cached packages |

The cold setup and build took about 4.5 minutes of measured commands across retries (initial 4.86 s, container build/Rust failure 1:47.95, toolchain plus failed TUI cook 2:16.26, final minimal make 12.36 s, plus about 11 s for the successful manual Rust installation). The first build had to download/build the Podman image, install Rust in `build/podman`, download the aarch64 toolchain, and build host filesystem tools. The exact per-command logs are `w0-build.log`, `w0-build-retry.log`, `w0-rustinstall.log`, `w0-build-main.log`, `w0-build-final.log`, and `w0-server-build.log` in this folder.

## Exact successful build commands

Run from `$HOME/Developer/forks/redox/redox`. The following `.config` was written in the repo:

```sh
cat > .config <<'CONFIG'
REPO_BINARY=1
COOKBOOK_MAKE_JOBS=2
CONFIG
```

The first attempts created the `redox-base` Podman image but failed before completing the container setup. This command successfully finished its Rust setup with an explicit process limit; `touch` recorded completion of that upstream make prerequisite:

```sh
podman run --rm --workdir /mnt/redox --interactive --cap-add SYS_ADMIN --device /dev/fuse --network=host --env TERM=dumb --pids-limit=1024 --volume "$PWD":/mnt/redox --volume "$PWD/build/podman":/root --env PATH=/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin --env PODMAN_BUILD=0 --env ARCH=aarch64 --env CONFIG_NAME=minimal --env FILESYSTEM_CONFIG=config/minimal.toml --env REPO_BINARY=1 --env COOKBOOK_MAKE_JOBS=2 redox-base bash -e podman/rustinstall.sh
touch build/container.tag
```

The image builds then completed with these commands (the shell redirected their output to the named logs):

```sh
/usr/bin/time -f 'BUILD_ELAPSED=%E BUILD_MAX_RSS_KB=%M BUILD_EXIT=%x' make ARCH=aarch64 CONFIG_NAME=minimal PODMAN_BUILD=1 USE_SELINUX=0 CI=1 PODMAN_OPTIONS='--rm --workdir /mnt/redox --interactive --cap-add SYS_ADMIN --device /dev/fuse --network=host --env TERM=dumb --pids-limit=1024' all > $HOME/artifacts/handoff/oneplus-redox-20261024/w0-build-final.log 2>&1
/usr/bin/time -f 'SERVER_BUILD_ELAPSED=%E SERVER_BUILD_MAX_RSS_KB=%M SERVER_BUILD_EXIT=%x' make ARCH=aarch64 CONFIG_NAME=server PODMAN_BUILD=1 USE_SELINUX=0 CI=1 PODMAN_OPTIONS='--rm --workdir /mnt/redox --interactive --cap-add SYS_ADMIN --device /dev/fuse --network=host --env TERM=dumb --pids-limit=1024' all > $HOME/artifacts/handoff/oneplus-redox-20261024/w0-server-build.log 2>&1
```

## Problems and fixes

1. The default Podman `:Z` volume suffix failed under this host's `runc`: `bind mounts cannot have any filesystem-specific options applied`. `USE_SELINUX=0` removed that suffix.
2. The upstream `--pids-limit=-1` resulted in `pids.max=1` in a diagnostic container, so Rust setup could not fork. An explicit `--pids-limit=1024` allowed it to complete. The make override above applies that limit to subsequent containers.
3. Cookbook panicked with `Inappropriate ioctl for device` when its terminal UI ran under redirected output. `CI=1` disabled the TUI and cooking completed.
4. The repo's `make qemu` firmware search does not include the installed `/usr/share/qemu-efi-aarch64/QEMU_EFI.fd`, so QEMU was launched directly with `-bios`.
5. Runtime storage remains blocked. A PCI virtio disk panicked at `MSI-X is not implemented on this architecture`; disabling vectors changed the failure to `device does not support MSI-X`. NVMe reached its Redox driver, then timed out during initialization and could not mount RedoxFS, on both minimal and server. AHCI alone was not bootable by this UEFI firmware. A diagnostic USB boot plus AHCI root copy loaded the kernel, but Redox attempted `/scheme/initfs/lib/drivers/ahcid` and reported `No such file or directory`. USB alone loaded the kernel but did not mount the root filesystem. `virt,acpi=off` with a virtio MMIO disk hit a kernel device tree panic: `MissingRequiredProperty("interrupt-cells")`.

## QEMU command line and boot evidence

The headless serial capture is [`w0-qemu-boot.log`](./w0-qemu-boot.log). It contains appended sessions for each disk experiment; `script` records terminal control codes and command headers. The direct command for the first minimal image attempt was:

```sh
script -q -f -c 'timeout --signal=TERM 180 qemu-system-aarch64 -machine virt -cpu max -smp 1 -m 2048 -bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd -drive file=build/aarch64/minimal/harddrive.img,format=raw,if=virtio -display none -serial stdio -monitor none -net none' $HOME/artifacts/handoff/oneplus-redox-20261024/w0-qemu-boot.log
```

The server image was tested with NVMe, the build system's default disk type:

```sh
script -q -a -f -c 'timeout --signal=TERM 50 qemu-system-aarch64 -machine virt -cpu max -smp 1 -m 2048 -bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd -drive file=build/aarch64/server/harddrive.img,format=raw,if=none,id=hd0 -device nvme,drive=hd0,serial=REDOXNVME -display none -serial stdio -monitor none -net none' $HOME/artifacts/handoff/oneplus-redox-20261024/w0-qemu-boot.log
```

The log proves UEFI loaded `Redox OS Bootloader 1.0.0 on aarch64/UEFI`, loaded the kernel and initfs, and entered kernel startup. It also records the storage failures above. There is no login prompt or shell in the log. QEMU was stopped after each failed attempt; no VM was left running. The repo's tracked working tree remained clean (`git status --porcelain` empty). The only lasting repo configuration edit is ignored `.config` with the two lines shown above.

## Next technical gate

A usable aarch64 storage path is needed before a login prompt can be verified. The direct candidates are an ARM MSI-X implementation for virtio, a working NVMe initialization path, or inclusion of a functional aarch64 AHCI driver in initfs. This would require source-level Redox driver/kernel work or an upstream fix; changing from minimal to a larger userland config did not resolve the storage failure.
