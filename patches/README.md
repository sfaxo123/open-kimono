# Downstream Redox patches

The single active kernel stack is [KERNEL-SERIES.md](KERNEL-SERIES.md) and
`patches/kernel-series/`. It starts at kernel base
`a0a8ff96b065bd8f33ca3e0742fb09f246c1e32a` and is applied in filename
order by `scripts/integrate.sh`. The old kernel folders, including the
conflicting `0003` files, are retained only in Git history.

Base and bootloader patches target their own upstream repositories. For K8b,
`patches/integration-2/base/` is the ordered rebased series after base head
`ef40656a`; it includes simplefb and base-touch plus RAM initfs wiring. The
bootloader series is based on `92ff2e7`.

The emulator touch build applies `patches/touch-e2e/base/` after the base
integration patches. `EMU_TOUCH=1 scripts/integrate.sh` adds emulator-only
rmi4d/inputd services and produces `redox-ramboot-emu-touch.efi`.
