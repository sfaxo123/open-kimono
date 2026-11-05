# Embedded EFI packager input provenance

`baseline.config` is a byte-for-byte copy of the validated headless U-Boot
configuration at `$HOME/artifacts/handoff/oneplus-redox-20261024/phone-boot/u-boot-headless/.config`
(SHA-256 `d584c7e3cfd8bcd4ade5800db2bb024e1ceb8c799513c31aeb1f046475864a98`).
The packager starts from this checked-in snapshot so a deleted build directory
or old artifact tree is not required.

`sdm845-oneplus-enchilada.dtb` is a byte-for-byte copy of the validated phone
DTB at `$HOME/artifacts/handoff/oneplus-redox-20261024/phone-boot/sdm845-oneplus-enchilada.dtb`
(SHA-256 `48518e1de2b2024aa078f4ba2ce2dfc1e3f9e8923c4a212b320413dbb1259383`,
155,913 bytes). It is the exact phone DTB packaged into both Android images.

`sdm845-oneplus-enchilada.dts` is a source representation decompiled from
that pinned binary with `dtc`. Its SHA-256 is
`170d27162c31cc0972982af1fb3edcef70e3f5840cb78f728124f4b2b77addb7`.
Recompiling it with `dtc` and decompiling the result yields byte-identical
canonical DTS (191,177 bytes), verified on 2026-10-27. The packager repeats
that canonical round-trip check and packages the original pinned DTB bytes,
preserving the exact validated binary. The original upstream source revision
and build recipe were not present, so the DTS is reconstructed source, not a
claim about the original source provenance.
