# Bounded RAM-shell emulator check

Run only after Root admits a new device-free acceptance phase:

```sh
python3 scripts/oracle/ramboot-shell/run.py /absolute/fresh-package-directory EXPECTED_ANDROID_IMAGE_SHA256
```

The package must contain `u-boot-nodtb.bin` and `op6-embedded-efi-redox.img`. The runner verifies the image hash, decompressed raw U-Boot and appended checked-in phone DTB, then creates a new DTB/nonce/profile. OneCPU,8GiB sparse RAM, noUSB/disks/network; host cgroup2GiB/swap0, runtime130seconds. Serial commands prove help/unique echo/pwd/initfs directory listing. It stops only its own unique unit. Do not replay consumed protected VM phases or treat this tool as authorization.

Artifacts go under `$HOME/artifacts/handoff/redox-perplexity/`. It uses the local pinned QEMU build; this is not clean-machine provisioning. No phone commands.

The original corrected-RX run returned1 because its oracle expected `etc`; captured output instead shows the actual initfs root `lib`/`bin`. Root verified command responses in the immutable log; the expected directory is corrected to `bin`. That parser correction is not a second runtime result. See `geni-rx-evidence/ROOT-ACCEPTANCE.json`.
