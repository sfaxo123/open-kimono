# BCM2836 inclusive IRQ domain regression

    PATH=$HOME/.cargo/bin:$PATH python3 scripts/oracle/bcm2836-range/run.py KERNEL_ROOT NEW_EVIDENCE_DIRECTORY

The pinned accepted29 baseline fixture must compile and fail the named domain test. The corrected production source must pass three tests. Three compiled controls (old count, descriptor-cap regression, lost root-selection store) must fail named assertions.

Actual `irq_init`/`irq_to_virq` function bodies and constants are extracted. The single MPIDR assembly instruction becomes cpuid0; fake FDT parsing and a no-op hardware initializer provide host boundaries. No registry arithmetic, loop bounds, root stores or mapping algorithm is rewritten. Tests cover inclusive IDs0..9, rejected10, next domain starts atbase+10, root stores, parse-error state preservation and the existing1024descriptor cap. Near the cap, range/index advance still uses declared count; that existing truncation behavior is preserved.

This proves the source allocation/mapping regression only. It does not test actual DT parsing, CPU identity, MMIO initialization, publication/alias safety, SMP, emulator or phone behavior. Baseline fixture is upstream-derived source, not another implementation. AArch64 compilation/linking is a separate acceptance gate.
