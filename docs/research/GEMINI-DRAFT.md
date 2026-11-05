(Operator-supplied draft from Gemini, 2026-10-24 — verbatim content kept in the session transcript; summary:)
Port OnePlus 6 (SDM845) drivers from postmarketOS mainline Linux to Redox OS user-space Rust drivers.
Phases: 1 boot (ABL/fastboot/U-Boot) + GENI UART + MMIO; 2 DT parsing, GICv3 -> irq: scheme, RPMh/clocks;
3 display (simplefb/GOP first, then DSI + Samsung AMOLED DCS); 4 GENI I2C + touchscreen + GPIO keys -> input:;
5 SPMI/PMIC battery, PSCI reboot; 6 UFS storage, WCN3990 Wi-Fi, Hexagon/QRTR modem.
Next steps proposed: ingest sdm845.dtsi + enchilada dts -> Rust register structs; skeleton GENI UART driver; boot handoff test.
