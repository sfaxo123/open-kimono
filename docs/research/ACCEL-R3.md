# ACCEL-R3: Using the working Linux OnePlus 6 as an oracle for Redox driver ports

Date 2026-10-24. Target: SDM845 (enchilada), postmarketOS mainline 7.1-rc, root + systemd.
Confidence tags: [V] verified from web this session, [K] from knowledge (not re-verified, check on device), [?] uncertain.
Run everything as root on the phone. Save traces off-device (scp) early; ftrace buffers are RAM.

## TL;DR
- Best register-level oracle: **rwmmio trace events** (`CONFIG_TRACE_MMIO_ACCESS`, arm64, merged ~v6.0 [V series / K merge version]). Gives every readl/writel with caller, width, value, address. Boot-time capture via cmdline `trace_event=rwmmio:*`.
- Complement with **regmap events** (for PMIC/SPMI/I2C/syscon, where bus traffic is not MMIO), **kprobe/bpftrace** on `readl/writel` fallbacks, and **devmem/debugfs snapshots** for state.
- Firmware/DSP protocol: **QRTR** (AF_QIPCRTR sockets, `qrtr-lookup`, and the kernel `qrtr` tracepoints/ns), **rpmsg/glink/smd** tracepoints, and the raw GLINK/SMEM data via debugfs. QMI messages are decodable with the TLV schemas in the kernel `qmi_helpers` users / `qmic`.
- Rust-on-hardware-under-Linux: **UIO (uio_pdrv_genirq) or /dev/mem with unbound driver**; vfio-platform needs an IOMMU group + reset handling and is awkward on SDM845 (SMMU is shared/Qualcomm-specific). Put MMIO behind a trait with 3 impls: `DevMem`, `Replay`, `Redox scheme`.
- No QEMU or Renode model of SDM845/GENI/UFS exists as far as found [V: search found none; K: Renode has no SDM845; QEMU has `sa8775p`? no. Treat as nonexistent]. You would write your own (see section 4); a *record/replay* mock is far cheaper than a device model.
- m1n1-style hypervisor tracing needs EL2: on Qualcomm Android bootloader the kernel typically starts at EL1 (Gunyah/QHEE is not on SDM845; SDM845 uses QHEE hypervisor => Linux at EL1). [K/?] So no EL2 tracing without replacing the hypervisor. Use in-kernel tracing instead.

---

## 1. Recording register accesses during Linux probe/operation (arm64)

### 1.1 rwmmio trace events (primary tool)
- Kconfig: `CONFIG_TRACE_MMIO_ACCESS` (depends on `ARCH_HAVE_TRACE_MMIO_ACCESS`, selected by arm64; needs `CONFIG_FTRACE`/`TRACING`). Events in `include/trace/events/rwmmio.h`: `rwmmio_write`, `rwmmio_post_write`, `rwmmio_read`, `rwmmio_post_read` (fields: caller via `%pS`, width, val, addr). [V for event names; Kconfig symbol name appears in series, final name [K]]
- Caveat from the series: only accessors in `asm/io.h` (readl/writel and *_relaxed) are traced; `__raw_readl/__raw_writel`, `memcpy_toio/fromio`, and `readsl/writesl`-style string ops may not be. Drivers using `__raw_*` or `iowrite32_rep` will be invisible. Cross-check with the kprobe fallback (1.3). [K]
- Address is the *virtual* address (e.g. 0xfffffbfffdbff700), not physical. Map to physical via `/proc/iomem` (physical) + `/sys/kernel/debug/...` or by recording `ioremap` (1.3). Practical trick: trace `ioremap` return values with the kprobe/bpftrace script below to build a VA->PA table.
- Enable at runtime:
```
mount -t tracefs none /sys/kernel/tracing 2>/dev/null
cd /sys/kernel/tracing
ls events/rwmmio || echo "NOT BUILT IN"
echo 0 > tracing_on; echo > trace
echo 65536 > buffer_size_kb          # per CPU; raise for probe storms
echo 1 > events/rwmmio/enable
echo 1 > options/print-tgid 2>/dev/null
echo 1 > tracing_on
# trigger: unbind/bind the device (see 1.6) or do the operation
echo 0 > tracing_on; cat trace > /root/trace-<drv>.txt
```
- Filter by caller/addr to cut noise (events support filters): `echo 'addr >= 0xffff800010a00000 && addr < 0xffff800010a10000' > events/rwmmio/rwmmio_write/filter` (addr is a u64 field; compare after you know the VA). Or filter by pid: `echo <pid> > set_event_pid`.
- Trace a probe from boot: kernel cmdline `trace_event=rwmmio:rwmmio_write,rwmmio:rwmmio_read tp_printk=0 trace_buf_size=64M` (pmOS: edit `deviceinfo_kernel_cmdline` in deviceinfo or the boot.img cmdline; on enchilada via fastboot `--cmdline`/boot.img rebuild with `pmbootstrap`). Boot traces need a big buffer; read after boot from `/sys/kernel/tracing/trace`.
- Perfetto/trace-cmd alternative: `trace-cmd record -e rwmmio -o rw.dat -- sleep 10; trace-cmd report > rw.txt` (apk add trace-cmd). [K]
- Post-process into a **register log** (`op width physaddr value caller_symbol`). Write a ~40 line Python/Rust tool: parse lines `rwmmio_(read|write): <sym>+off width=N val=0x.. addr=0x..` into JSON lines. Group by driver function (`caller`) to produce the init sequence for the port.

### 1.2 regmap tracepoints
- Kconfig: `CONFIG_REGMAP` + tracing; events `regmap:regmap_reg_write`, `regmap_reg_read`, `regmap_reg_read_cache`, `regmap_bulk_write/read`, `regmap_hw_read_start/done`, `regmap_hw_write_*`, `regmap_async_*`, `regmap_cache_*`, `regmap_cache_bypass`, `regmap_lock_*` (in `include/trace/events/regmap.h`). Fields: device name, register, value. [K]
```
cd /sys/kernel/tracing; echo 1 > events/regmap/enable
```
- Use for: PMIC (PM8998/PMI8998 via SPMI regmap), I2C touch/charger/codec chips (the OnePlus 6 panel/touch: Synaptics S3320 via RMI4 over I2C; audio WCD934x via SLIMbus regmap), regmap-mmio blocks. Beware `regmap_reg_read_cache` shows cache hits that never hit the bus; use `regmap_hw_*` for real bus ops.
- For SPMI also: `events/spmi/` (`spmi_read_begin/end`, `spmi_write_*`), for I2C: `events/i2c/` (`i2c_write`, `i2c_read`, `i2c_result`) with `CONFIG_I2C_DEBUG...`; for SLIMbus: dynamic debug. [K]

### 1.3 Fallbacks if CONFIG_TRACE_MMIO_ACCESS is missing
- **ftrace function filter / kprobe on non-inlined accessors**: on arm64 `readl`/`writel` are inline, so probing them doesn't work. Instead: kprobe `ioremap`-family to learn mappings, then use **hardware watchpoints** (perf `mem:` breakpoints) on key registers — only 4 slots. [K]
- **bpftrace**: `bpftrace -e 'kretprobe:ioremap_prot,kretprobe:__ioremap_caller { @[comm,kstack(3)]=count(); }'` and `kprobe:devm_ioremap_resource`, `devm_platform_ioremap_resource` to record `(phys,size)` per driver: `bpftrace -e 'kprobe:__ioremap_caller { printf("%s phys=%lx size=%lx\n", comm, arg0, arg1); }'` [K, symbol names vary by kernel; check `/proc/kallsyms`]. Needs `CONFIG_BPF_SYSCALL, CONFIG_KPROBES, CONFIG_BPF_EVENTS, CONFIG_DEBUG_INFO_BTF`.
- **Kernel patch fallback** (always possible since you own the pmOS kernel build): add `trace_printk` into a wrapper, or just enable the rwmmio config in a pmOS kernel build via `pmbootstrap kconfig edit linux-postmarketos-qcom-sdm845`. Cheapest robust approach: rebuild the kernel with the options in section 5.
- **Dynamic debug**: `echo 'file drivers/tty/serial/qcom_geni_serial.c +p' > /sys/kernel/debug/dynamic_debug/control` for driver-authored messages (needs `CONFIG_DYNAMIC_DEBUG`).

### 1.4 ftrace function_graph (control flow, not registers)
- Shows the call tree of probe, which tells you the *order of high-level steps* (clk_prepare_enable, regulator_enable, reset_control_deassert, phy_init) interleaved with register ops.
```
cd /sys/kernel/tracing
echo function_graph > current_tracer
echo 'ufshcd_*' 'ufs_qcom_*' 'ufs_qcom_phy*' > set_graph_function    # restrict
echo funcgraph-proc > trace_options; echo funcgraph-abstime > trace_options
echo 1 > events/rwmmio/enable        # interleave register ops
```
- Needs `CONFIG_FUNCTION_GRAPH_TRACER` (and `CONFIG_DYNAMIC_FTRACE`). On arm64 requires `CONFIG_FTRACE_SYSCALLS` not required. [K]
- Also use events `clk:clk_enable`, `clk_set_rate`, `regulator:regulator_enable*`, `regulator_set_voltage`, `power:*`, `gpio:*` (`events/gpio`), `irq:*`: these cheaply record the *resource setup* preceding register ops. `echo 1 > events/clk/enable events/regulator/enable`.

### 1.5 Snapshots: before/after state
- **devmem / /dev/mem** (needs `CONFIG_DEVMEM=y` and `CONFIG_STRICT_DEVMEM=n` or `iomem=relaxed` cmdline; note STRICT_DEVMEM blocks ranges claimed by drivers). `apk add devmem2` or busybox `devmem`:
```
devmem 0x1d84000 32          # one UFS HCI reg
dd if=/dev/mem bs=4 skip=$((0x1d84000/4)) count=$((0x3000/4)) | xxd > ufs-hci.before
```
  Reading some registers has side effects (FIFO pops, clear-on-read IRQ status). Mark such registers as "volatile" in your oracle DB; never blind-dump GENI FIFO data regs or IRQ status regs. Prefer `/sys/kernel/debug/regmap/<dev>/registers` for regmap-backed devices (read-only, safe, needs `CONFIG_DEBUG_FS`, `CONFIG_REGMAP`).
- **Clock/regulator/pinctrl/gpio/interconnect/iommu** (needs `CONFIG_DEBUG_FS`, `CONFIG_COMMON_CLK`, etc.):
```
cat /sys/kernel/debug/clk/clk_summary          # rates, enable/prepare counts, parents
cat /sys/kernel/debug/clk/clk_dump             # JSON-ish; includes per-clk registers on some
cat /sys/kernel/debug/regulator/regulator_summary
cat /sys/kernel/debug/pinctrl/*/pinmux-pins ; .../pinconf-pins ; .../pinctrl-handles
cat /sys/kernel/debug/gpio                      # per-line direction/value/IRQ
cat /sys/kernel/debug/interconnect/interconnect_summary   # needs CONFIG_INTERCONNECT (+ debugfs)
ls /sys/kernel/debug/iommu/ ; cat /sys/kernel/iommu_groups/*/devices   # arm-smmu mappings; /sys/kernel/debug/iommu/... is driver-dependent
cat /sys/kernel/debug/pm_genpd/pm_genpd_summary # power domains (GDSCs!)
cat /sys/kernel/debug/gcc... (or `clk_summary`)  # SDM845 GCC/DISPCC/etc. clocks
cat /sys/kernel/debug/devices_deferred ; /sys/kernel/debug/qcom_*
cat /proc/iomem /proc/interrupts /proc/device-tree/...
```
  Take the snapshot **in three states** (driver bound/idle, active, unbound) and diff: `diff -u clk.idle clk.active`. The clock/GDSC/regulator/pinmux diff is the exact "resources to enable" list for the Redox driver.
- **Live device tree**: `/sys/firmware/devicetree/base` (properties as files) and `dtc -I fs -O dts /sys/firmware/devicetree/base > live.dts` (use `-@` symbols if present). `/sys/firmware/fdt` is the raw blob only on some archs/kernels (`CONFIG_OF_...`); on arm64 DT booted via the bootloader it exists if the kernel exposes it (`CONFIG_OF_EARLY_FLATTREE`/arm64 exposes `/sys/firmware/fdt`) [K]. Also take the static DT from the pmOS build: `arch/arm64/boot/dts/qcom/sdm845-oneplus-enchilada.dts` + `sdm845-oneplus-common.dtsi` + `sdm845.dtsi`: the DT *is* the hardware description (addresses, IRQs, clocks, resets, power-domains, interconnects, pinctrl states) so Redox needs a DT parser or generated constants. Resolve phandles with `dtc -I dtb -O dts -s`.
- Mapping to firmware blobs: `/lib/firmware/...`, `dmesg | grep -i firmware`, `/sys/kernel/debug/remoteproc/` and `/sys/class/remoteproc/*/{state,firmware}` for ADSP/CDSP/MPSS/SLPI (needs `CONFIG_REMOTEPROC`, `CONFIG_QCOM_Q6V5_*`).

### 1.6 Triggering a clean probe sequence
```
dev=1d84000.ufshc          # name from /sys/bus/platform/devices
drv=$(basename $(readlink /sys/bus/platform/devices/$dev/driver))
echo $dev > /sys/bus/platform/drivers/$drv/unbind
# start trace here
echo $dev > /sys/bus/platform/drivers/$drv/bind
```
Careful: unbinding UFS kills the root filesystem; display (MDSS/DSI/panel) and touch are safe to cycle (SSH over USB gadget/Wi-Fi stays alive); GENI serial engines used as I2C/SPI for touch are safe; do NOT unbind rootfs UFS, USB gadget if SSH goes over it (use Wi-Fi SSH or run from initramfs/netboot), or the SMMU/clock controllers. Use a script run via `systemd-run --scope` or `nohup` with a watchdog `sleep 60; reboot -f` hard timer. For UFS, boot a *separate* test: use pmOS on **sdcard-less initramfs** or `boot from RAM` (`fastboot boot`) with root on USB mass storage so UFS can be unbound/driven. [K]
- Boot-time probe capture is the most faithful for UFS: `trace_event=rwmmio` and `tp_printk` unnecessary; read `/sys/kernel/tracing/trace` after boot (buffer must be big enough: `trace_buf_size=128M`).

---

## 2. Tracing DSP/firmware protocols (QRTR/QMI/GLINK/APR)

SDM845 wiring: AP <-> ADSP/CDSP/MPSS/SLPI over **GLINK** (SMEM-based transport `qcom_glink_smem`, RPMSG), with **QRTR** (AF_QIPCRTR 42) as the IPC router over the `IPCRTR` glink channel; QMI services (PD mapper, TFTP, wlan `WLFW`, modem `DMS/NAS/WDS`, LOC, SSCTL, etc.) over QRTR; **APR/GPR** (audio packet router) over rpmsg for audio (q6core/q6adm/q6asm/q6afe/q6routing) on ADSP; **SMP2P/SMEM** for state bits; RPMh for regulators/clock votes (`rpmh_send_msg` tracepoints); SCM/SMC calls to TZ.

### 2.1 QRTR
- Kconfig: `CONFIG_QRTR`, `CONFIG_QRTR_SMD`, `CONFIG_QRTR_TUN`, `CONFIG_QRTR_MHI`, `CONFIG_RPMSG_QCOM_GLINK_SMEM`, `CONFIG_RPMSG_QCOM_SMD`, `CONFIG_RPMSG_CHAR`, `CONFIG_RPMSG_CTRL`, `CONFIG_QRTR_NS`. [K]
- Userspace tools: `qrtr-ns` daemon (from `qrtr` package; pmOS has `qrtr`, `qmic`, `rmtfs`, `tqftpserv`, `pd-mapper`). `qrtr-lookup` lists all services (service id, version, instance, node:port) — a ready-made inventory of what firmware exposes. `qrtr-cfg`. [K]
- **Tracing messages**: tracepoints `qrtr:*`? There is `include/trace/events/qrtr.h` added ~v6.x (`qrtr_ns_message`, `qrtr_ns_service_announce_new/del`, `qrtr_ns_server_add`) [?]. Reliable method: **Wireshark** has a QRTR dissector; capture with `tcpdump`-like **AF_QIPCRTR** is not supported by libpcap. Instead use the QRTR **TUN** or write a 30-line passive proxy: a QRTR socket with `QRTR_...` control "new lookup" cannot sniff other nodes' point-to-point traffic. Practical option: **ftrace function_graph / kprobe on `qrtr_node_enqueue` and `qrtr_endpoint_post`** with bpftrace to dump the skb header (`struct qrtr_hdr_v1/v2`: type, src node/port, dst node/port, size) + payload bytes:
  `bpftrace -e 'kprobe:qrtr_endpoint_post { printf("rx len=%d\n", arg2); }'` (extend to print the buffer with `hex(buf(arg1,len))` style via `printf("%r", buf(...))`) and `kprobe:qrtr_node_enqueue` for tx (arg0=node, arg1=skb; read skb->data via BTF). [K; needs `CONFIG_DEBUG_INFO_BTF`]. Alternatively patch the kernel with `print_hex_dump` / a tracepoint (small patch, robust).
- Decoding QMI: QMI = TLV with `struct qmi_elem_info` tables inside kernel users (`drivers/net/wireless/ath/ath10k/qmi.c` for WLFW, `drivers/soc/qcom/pdr_interface.c`, `qcom_pd_mapper`, `ssr`). Use `qmic` (qmic from andersson/qmic: compiles `.qmi` IDL to C) or write decoders in Rust from Linux's elem_info tables. Record both directions as `(timestamp, direction, src, dst, qmi_type, msg_id, txn_id, tlvs)` JSON-lines.
- For **service-level replay**: run the Redox side's QRTR client against the *real* firmware (best), or a mock that replays recorded responses keyed by `(service, msg_id)` plus txn_id substitution.

### 2.2 GLINK / SMD / rpmsg
- Channel inventory: `ls /sys/bus/rpmsg/devices/`, `cat /sys/bus/rpmsg/devices/*/name`, `/sys/class/rpmsg/` and `rpmsg_char` devices; `/sys/kernel/debug/qcom_glink`? [?]. Glink core tracepoints: `include/trace/events/rpmsg.h`: `rpmsg_trace` (`qcom_glink` has `trace_rpmsg_send`/`rpmsg_ept_*`? present since ~5.x via `CONFIG_RPMSG_TRACE`) [?]. Check `ls /sys/kernel/tracing/events | grep -E 'rpm|glink|qcom|smem|rpmh'`.
- Reliable trace points that exist: `rpmh:*`? (`include/trace/events/rpmh.h`: `rpmh_tx_done`, `rpmh_send_msg`) [K]; `qcom_smem`? none. For SMP2P: `smp2p` debug via `CONFIG_QCOM_SMP2P` + dynamic debug.
- Low-level fallback: kprobes on `qcom_glink_send`/`qcom_glink_rx_*` (`glink_native.c`), or function_graph with args using `trace-cmd`/`bpftrace`.
- **APR/GPR audio**: Kconfig `CONFIG_QCOM_APR`, `SND_SOC_QDSP6*`. Trace with dynamic debug on `apr.c` and `q6dsp-common`, or kprobe `apr_send_pkt` (print `struct apr_hdr`: opcode, src/dst port, payload size). Also `events/asoc/*` and `events/snd_pcm`. The ADSP firmware loads from `/lib/firmware/qcom/sdm845/oneplus6/adsp.mbn` (+ `.mdt/.bNN`) — Redox needs the PIL/MDT loader + SCM `pas_init_image/auth_and_reset` (trace `qcom_scm_*` via function trace).
- **SCM/SMC**: ftrace function `qcom_scm_call` `__scm_smc_call` (calls are the TZ ABI; record `(svc, cmd, args)` — Redox must replicate with the same SMCCC conduit; needs EL1 `smc` to be permitted, which it is).

### 2.3 Wi-Fi (ath10k_snoc / WCN3990)
- WLAN firmware: `ath10k_snoc` + QMI WLFW (`qrtr`). Enable `dyndbg` for `ath10k` with `echo 0xffffffff > /sys/kernel/debug/ieee80211/phy0/ath10k/... debug_mask` (module param `debug_mask`). Record HTT/WMI via ath10k `trace` events (`ath10k:ath10k_wmi_cmd`, `ath10k_htt_*`, `ath10k_log_*`) [K].

### 2.4 USB and others
- USB gadget (dwc3 + configfs): `usbmon` (`CONFIG_USB_MON`) on host side only sees the host's view; on device side use `events/dwc3/*` tracepoints (`CONFIG_USB_DWC3` has `dwc3_readl/dwc3_writel/dwc3_ep_*`, `dwc3_gadget_*` events) [K] — very good oracle for the gadget port. Also `events/gadget/*`. UFS: `events/ufs/*` (`ufshcd_command`, `ufshcd_upiu`, `ufshcd_uic_command`, `ufshcd_clk_gating`) show UPIU/UIC at command level — the *protocol-level* oracle for UFS. SDHCI/MMC: `events/mmc/*`. DRM: `events/drm/*` and `drm.debug=0x1ff` module param plus `/sys/kernel/debug/dri/*/{state,clients,gem_names}` and `dpu` debug (`/sys/kernel/debug/dri/0/...`). Touch (RMI4): `events/i2c/*` + `rmi4` dynamic debug.

---

## 3. Developing a Rust driver against real hardware while Linux runs

### 3.1 Architecture: MMIO trait
```rust
pub trait Mmio { fn r32(&self, off: usize) -> u32; fn w32(&self, off: usize, v: u32); /* r8,r16,r64, barrier */ }
pub trait Platform { fn clk_enable(&self, id:&str); fn regulator(&self, id:&str, uv:u32); fn gpio_*; fn irq_wait(&self)->...; fn dma_alloc(...); fn delay_us(u64); }
```
Impls: `DevMem` (mmap `/dev/mem` or `/dev/uioN`), `Replay` (reads trace; asserts writes match order), `Record` (wraps another impl, logs every access in the same JSONL format as the Linux oracle), `RedoxScheme` (physmap via `memory:physical@dev`/`scheme:memory`). **Also make `Record`'s format identical to the rwmmio-parsed oracle log** so `diff` of Linux-vs-Rust sequences is trivial (normalize polling reads).
Driver core is `no_std + alloc`-friendly; only sequencing + register offsets. Use `tock-registers`/`bitflags` for typed regs, `volatile`-correct reads/writes (`core::ptr::read_volatile`), and an explicit `dsb sy` barrier on arm64.

### 3.2 Handing hardware to userspace under Linux
1. **Unbind the Linux driver** (1.6) — leaves clocks/regulators/pinctrl in the state Linux left them *unless* the driver's remove path disables them (runtime PM may gate clocks => your user-space access hits a gated block => **bus hang/SError/instant reboot**). Mitigations: keep clocks on via `clk_summary`-guided approach: before unbinding, mark `echo on > /sys/devices/.../power/control` (disable runtime PM autosuspend) and consider `clk_ignore_unused pd_ignore_unused` cmdline so unused clocks/GDSCs are not shut down after boot [K]; or write a tiny **kernel shim module/DT overlay** that binds `uio_pdrv_genirq` and *requests the clocks, resets, regulators, power-domains, pinctrl states* from DT (a "resource holder"). That shim is the cleanest: DT node with `compatible = "generic-uio"` plus the same `clocks/power-domains/pinctrl` properties; add `of_id` via `uio_pdrv_genirq.of_id=generic-uio` cmdline. UIO's pdrv does `pm_runtime_enable` + `get_sync` ... but does *not* enable arbitrary DT clocks by itself [K/?]; a ~100-line out-of-tree module (kmod, built with the pmOS kernel headers) that does `devm_clk_bulk_get_all` + `clk_bulk_prepare_enable`, `devm_pm_runtime_enable`, `pm_runtime_get_sync`, deassert resets, then registers a `uio_info` with the MMIO region + IRQ is the dependable path. Expose reg regions as /dev/uioX maps and IRQ via read() on the fd.
2. **/dev/mem**: quick path for polling-only experiments. Needs `CONFIG_DEVMEM`, `STRICT_DEVMEM` off or `iomem=relaxed`, and 'nopat' doesn't apply on arm64. Map with `memmap2`/`libc::mmap` on `/dev/mem` with offset physaddr (page aligned), `O_SYNC` for device memory attributes (arm64 /dev/mem uses `pgprot_noncached` for non-RAM => Device-nGnRnE, fine). Access must be aligned and volatile; no `memcpy` over device memory (use 32-bit volatile loops, otherwise unaligned/Device-memory faults).
3. **vfio-platform** (`CONFIG_VFIO_PLATFORM`, plus `CONFIG_VFIO_IOMMU_TYPE1`, `CONFIG_VFIO`): lets user space own MMIO + IRQ + DMA through an IOMMU, ideal for DMA drivers (UFS PRDT/UTRD, DPU buffers) *if* the device has an arm-smmu stream ID in its own IOMMU group. On SDM845 many masters (UFS, MDSS, GENI-DMA) sit behind the apps SMMU (`arm-smmu` `qcom,smmu-500`, with Qualcomm quirks: bootloader-set bypass/context banks). Needs vfio-platform *reset* handling (`vfio-platform-base` requires either a reset function or `reset_required=0` module param) [K]. Bind: `echo vfio-platform > /sys/bus/platform/devices/$dev/driver_override; echo $dev > /sys/bus/platform/drivers/vfio-platform/bind`. Medium effort; use only when DMA through the SMMU matters; otherwise UIO + a CMA/`udmabuf`/`/dev/dma_heap/system` buffer works only if the device is not behind a translating SMMU, or you need the SMMU mapping — `udmabuf` buffers have kernel IOVAs and need `dma_map` => not usable by raw MMIO programming. Alternative: allocate DMA memory in the **shim kernel module** with `dma_alloc_coherent` and expose both CPU mmap and the *device-visible IOVA* to user space through a chardev ioctl (simplest robust approach for UFS/MDSS tests).
4. Keep interrupts out initially: **poll status registers** (the Rust driver core should support poll mode first). Add IRQ via UIO (`read(fd)` + re-enable write) later.
5. Safety net: serial-less phone => reboot on hang. Use `echo 1 > /proc/sys/kernel/panic; panic_on_oops`, `watchdog` (`/dev/watchdog`, `qcom-wdt`), pstore/ramoops (`CONFIG_PSTORE_RAM`; pmOS enchilada has ramoops region in DT) so a crash leaves a trace in `/sys/fs/pstore` after reboot. Persist `trace` output to tmpfs before each risky step with `sync`.

### 3.3 Order of trust
Linux driver (oracle) -> Rust core under Linux via `DevMem` (compare sequences with `Record`) -> same core with `RedoxScheme` MMIO on Redox. Anything that diverges under Linux-userspace cannot be a Redox-OS bug; anything that only diverges on Redox is plumbing (memory map, clock/regulator bring-up, interrupts, IOMMU).

---

## 4. Record/replay, differential testing, emulators

### 4.1 Other projects
- **Asahi Linux m1n1 hypervisor**: runs the *macOS* kernel/drivers as a guest at EL1 under m1n1 at EL2, traces MMIO via stage-2 faults (`hv.trace_range`, `hv.trace_device`), and replays/pokes registers from Python (`m1n1.hv`, `proxyclient`). Needs EL2 control. On SDM845 the Linux kernel is entered at EL1 because the Qualcomm secure hypervisor (QHEE) owns EL2 [K/?]; replacing it needs an unlocked-bootloader chain that boots in EL2 (some projects, e.g. `sdm845 EL2` via `edk2-sdm845` (Project Aloha) / "Qualcomm EL2 hyp" patches, exist: edk2-sdm845 targets 845 phones [V: appears in search], and Linux KVM on SDM845 requires a EL2-entry hack with `slbounce`/`Gunyah` for newer SoCs; for SDM845 the `slbounce` trick works for Windows-on-Arm style devices [?]). Treat EL2 tracing as a **stretch goal**; in-kernel rwmmio + regmap tracing delivers 90% of the value with no hypervisor.
- **Linux in-kernel equivalent of m1n1 tracing**: `rwmmio` events (above) were literally added for Qualcomm debugging ("hangs and SErrors/NOC errors during bring-up") [V from cover letter].
- **QEMU**: no SDM845 machine upstream (QEMU's arm boards: virt, sbsa-ref, raspi, aspeed, ...; no Qualcomm phone SoC models) [K+V negative search]. Out-of-tree: none known for GENI/UFS. Writing a QEMU `sysbus` device for GENI SE (UART/I2C/SPI modes) or the UFS HCI is feasible (UFSHCI is a *standard* JEDEC host controller; QEMU already has `hw/ufs/` (since 8.2, PCI `ufs` device, `-device ufs`) — it models UFSHCI v3 over PCI. That is a legitimate **spec-level test target for the Rust UFSHCI core** (host controller registers, UTRL/UTMRL, UPIU), but not for Qualcomm-specific parts (`ufs-qcom` vendor regs, QMP PHY, ICE, clocks). [K; verify QEMU version]
- **Renode**: peripheral models in C#/`.repl`; no SDM845, GENI or Qualcomm UFS models [K, search negative]. Renode is practical for *generic* things (UART/I2C/SPI behaviors) but you'd be authoring the Qualcomm models yourself — only worthwhile as a **replay device**: a Renode/QEMU "register mock" generated automatically from recorded traces (reads return recorded values by (addr, sequence index), writes are checked). That is identical in spirit to the `Replay` Mmio impl and costs far less; run it in plain `cargo test` on the dev PC (no emulator).
- **Fuchsia**: uses real-hardware testing with DFv2 and per-driver unit tests via fake MMIO (`mmio-ptr`/`fdf-testing` fake MMIO buffers, `fake-mmio-reg`: `ddk::MmioBuffer` backed by `fake_mmio::FakeMmioRegRegion` with per-register read/write callbacks) — the pattern to copy: **a `FakeMmio` with per-register hooks** that you drive from recorded traces. [K]
- **Genode**: drivers ported from Linux via `dde_linux` (Device Driver Environment): runs unmodified Linux driver source against Genode's `Platform` session; alternative strategy for Redox: *run the Linux driver source in a shim* — heavy, skip. Genode's driver platform session (`Platform::Device`, `Mmio`, `Irq`) is the model for the Redox `Platform` trait. [K]
- **Differential testing of the init sequence**: normalise Linux oracle log: collapse poll loops (N identical reads until a bit changes -> single "wait_until" assertion), drop volatile reads, group by function; compare Rust `Record` log against it, *ordered writes must match* (value mask for bits that legitimately differ e.g. timestamps/ids). Tooling: a small `oraclediff` (Rust or Python) with per-block ignore lists.

### 4.2 Hardware-in-the-loop CI
- Phone as a test runner: SSH over USB gadget (usb0 ncm <PHONE_USB_IP>) running `cargo test --features hw`, results via scp. Wi-Fi as a fallback. Keep a `pmbootstrap`-built rescue image for when a hang needs reflash (`fastboot boot` of a known-good boot.img doesn't touch UFS).

---

## 5. Kernel config needed, and how to check it

| Purpose | Options |
|---|---|
| MMIO trace | `CONFIG_FTRACE=y, CONFIG_TRACING=y, CONFIG_TRACE_MMIO_ACCESS=y` (needs arm64 select `ARCH_HAVE_TRACE_MMIO_ACCESS`), `CONFIG_EVENT_TRACING` |
| function tracing | `CONFIG_FUNCTION_TRACER`, `CONFIG_FUNCTION_GRAPH_TRACER`, `CONFIG_DYNAMIC_FTRACE`, `CONFIG_TRACEPOINTS`, `CONFIG_KPROBE_EVENTS`, `CONFIG_KPROBES`, `CONFIG_UPROBES` |
| trace access | `CONFIG_DEBUG_FS`, `CONFIG_TRACEFS`? (tracefs via `CONFIG_FTRACE`) |
| bpftrace | `CONFIG_BPF=y, CONFIG_BPF_SYSCALL=y, CONFIG_BPF_EVENTS=y, CONFIG_DEBUG_INFO_BTF=y, CONFIG_FTRACE_SYSCALLS, CONFIG_KPROBE_EVENTS, CONFIG_UPROBE_EVENTS` |
| debugfs views | `CONFIG_DEBUG_FS`, `CONFIG_COMMON_CLK_DEBUG`?, `CONFIG_REGMAP_DEBUG`(older name; modern: debugfs on), `CONFIG_PM_GENERIC_DOMAINS_DEBUG`?/`CONFIG_PM_DEBUG`, `CONFIG_INTERCONNECT`, `CONFIG_DEBUG_GPIO`, `CONFIG_PINCTRL` (debugfs auto), `CONFIG_DYNAMIC_DEBUG` |
| devmem | `CONFIG_DEVMEM=y`, `CONFIG_STRICT_DEVMEM=n` (or `iomem=relaxed`), `CONFIG_IO_STRICT_DEVMEM=n` |
| UIO/vfio | `CONFIG_UIO`, `CONFIG_UIO_PDRV_GENIRQ`, `CONFIG_UIO_DMEM_GENIRQ`, `CONFIG_VFIO`, `CONFIG_VFIO_PLATFORM`, `CONFIG_VFIO_IOMMU_TYPE1`, `CONFIG_IOMMUFD`, `CONFIG_UDMABUF`, `CONFIG_DMABUF_HEAPS(_SYSTEM)` |
| DT/FDT | `CONFIG_PROC_DEVICETREE`? (legacy), `CONFIG_OF` (sysfs `/sys/firmware/devicetree`), `CONFIG_OF_DYNAMIC`+`CONFIG_OF_OVERLAY` (overlays), `CONFIG_OF_CONFIGFS` |
| IPC | `CONFIG_QRTR`, `CONFIG_QRTR_SMD`, `CONFIG_RPMSG_CHAR`, `CONFIG_RPMSG_QCOM_GLINK_SMEM`, `CONFIG_QCOM_APR`, `CONFIG_REMOTEPROC`, `CONFIG_QCOM_SCM`, `CONFIG_QCOM_RPMH` |
| crash retention | `CONFIG_PSTORE`, `CONFIG_PSTORE_RAM`, `CONFIG_PSTORE_CONSOLE`, `CONFIG_PSTORE_FTRACE` (ftrace into pstore survives hangs!) |
| module build | kernel headers `linux-postmarketos-qcom-sdm845-dev` for out-of-tree shim module (`apk add linux-postmarketos-qcom-sdm845-dev`) |

Likelihood on pmOS enchilada [?]: The device uses the shared mainline `linux-postmarketos-qcom-sdm845` package (defconfig-style `config-postmarketos-qcom-sdm845.aarch64`) [V from search]. postmarketOS configs are Alpine-style and generally have `DEBUG_FS`, `KPROBES`, `FTRACE/FUNCTION_TRACER`, `BPF_SYSCALL` on (pmOS ships `bpftrace`/`perf` in repos) [K, medium confidence]; `TRACE_MMIO_ACCESS` is rarely enabled by default (off by default, "If unsure, say N"); `UIO_PDRV_GENIRQ`, `VFIO_PLATFORM` likely modules or absent; `STRICT_DEVMEM` is probably y (distro default) [?]. Expect to rebuild with `pmbootstrap kconfig edit linux-postmarketos-qcom-sdm845` (adds options, then `pmbootstrap build --force linux-postmarketos-qcom-sdm845`, `pmbootstrap install/flasher`). Be wary: pmOS `kconfigcheck` only warns about required options; extra tracing options won't be rejected.

### Check commands (on device)
```
zcat /proc/config.gz | grep -E 'TRACE_MMIO_ACCESS|FUNCTION_GRAPH_TRACER|KPROBE_EVENTS|BPF_SYSCALL|DEBUG_INFO_BTF|DEVMEM|STRICT_DEVMEM|UIO|VFIO|DEBUG_FS|DYNAMIC_DEBUG|PSTORE|QRTR|OF_OVERLAY|DWC3|UFS|RPMSG|QCOM_APR'
# if /proc/config.gz missing (CONFIG_IKCONFIG_PROC not set):
ls /boot/config-* ; apk info -L linux-postmarketos-qcom-sdm845 | grep config   # or look under /usr/share/kernel/*/config
ls /sys/kernel/tracing/events | head -100 ; ls /sys/kernel/tracing/events/rwmmio
ls /sys/kernel/debug ; which bpftrace perf trace-cmd devmem devmem2 dtc
cat /proc/cmdline ; cat /sys/kernel/tracing/available_tracers
```
Source of truth for the config is the pmaports tree: `main/linux-postmarketos-qcom-sdm845/config-postmarketos-qcom-sdm845.aarch64` (gitlab.postmarketos.org/postmarketOS/pmaports) [V that package exists].

---

## 6. Oracle workflow (run per driver)

Prereqs (once): pmOS kernel rebuilt with section-5 options; `apk add trace-cmd bpftrace perf dtc devmem2 qrtr qmic python3`; tools repo with `rwlog2jsonl`, `oraclediff`, and the `Mmio` trait crate. Create `~/artifacts/oracle/<driver>/` on the dev host; scp results there.

1. **Inventory (read-only, 10 min).** Find the device: `ls /sys/bus/platform/devices | grep <block>`; read DT node in `sdm845.dtsi` + `sdm845-oneplus-*.dtsi` (reg, interrupts, clocks, resets, power-domains, pinctrl, iommus, interconnects, vdd supplies, phys). Save `live.dts` (`dtc -I fs`), `/proc/iomem`, `/proc/interrupts`, module list, driver name/source path (`readlink .../driver`).
2. **Static spec.** Read the Linux driver source + datasheet-like headers (register offsets in `.h`/DT bindings: `dt-bindings/clock/qcom,gcc-sdm845.h`). Write the register map / `tock-registers` definitions first.
3. **Snapshot baseline.** Save `clk_summary`, `regulator_summary`, `pinmux-pins`, `gpio`, `pm_genpd_summary`, `interconnect_summary`, `/sys/kernel/debug/regmap/*` for the device, and `devmem` register dump of non-volatile registers, in *idle-bound* state (`snap.sh before`).
4. **Capture probe trace.** Reboot with `trace_event=rwmmio,regmap,clk,regulator,power,gpio,irq` + big `trace_buf_size`, or unbind->enable events->bind (when safe, section 1.6). Add `function_graph` restricted to the driver's functions in a second capture for call-structure. Copy `trace` + `dmesg` off-device. Run `rwlog2jsonl` to produce `probe.jsonl` (VA->PA mapped through ioremap log or `/proc/iomem`).
5. **Capture operation traces.** Per use case (e.g. display: panel on, modeset, flip, off; UFS: read 4K, write, flush, suspend; touch: touch events; USB: enumerate; Wi-Fi: scan/associate; audio: play tone). One capture per scenario, each with the same event set, labelled by scenario in the filename. Add protocol traces from section 2 in parallel (`ufs:*`, `dwc3:*`, QRTR/APR kprobes, `drm:*`).
6. **Snapshot after.** `snap.sh active`; `diff` against `before` -> definitive list of clocks/GDSCs/regulators/pinmux/GPIO the driver needs; record rates and voltages.
7. **Normalise + annotate.** Collapse polling loops, mark volatile/clear-on-read regs, tag with caller functions, produce `sequence.md` (human spec: step, register, value, why) and `sequence.jsonl` (machine). Peer-review it against the Linux source: every write in the trace must be explained by a source line (catches `__raw_*` blind spots; any unexplained write means missed accessor).
8. **Write the Rust core behind traits** (`Mmio`, `Platform`, `Clock`, `Regulator`, `Gpio`, `Delay`, `DmaAlloc`). First test: `cargo test` with `Replay` Mmio driven by `probe.jsonl` (fast, no hardware): assert same ordered writes, feed recorded read values, check state machine reaches "ready".
9. **Hardware test under Linux userspace.** Build resource-holder shim/UIO (section 3.2) or unbind driver; run the Rust core with `DevMem` + `Record`; `oraclediff record.jsonl probe.jsonl`; iterate until identical writes and the hardware visibly works (e.g., panel lights, UFS read matches `dd if=/dev/sda` checksum, touch coordinates stream). Keep pstore + watchdog + hard-reset plan; run each experiment from a script with a timeout.
10. **Protocol layer check** (DSP/firmware-facing drivers): replay recorded QMI/APR requests from Rust against live firmware, compare responses with recorded ones (ignoring txn ids/timestamps).
11. **Port to Redox.** Implement `Platform` for Redox (physmap via `scheme:memory`/`/scheme/memory/physical`, IRQ scheme, DT parsing, clock/regulator/GDSC/RPMh/SCM shim in userspace — this is usually the biggest gap vs Linux: Redox needs the clock controller (GCC), RPMh and PMIC drivers before peripherals have power; record those in the same way). Run the *same* `cargo test` replay suite on the Redox target, then hardware.
12. **Gate + regression.** Store `probe.jsonl`, `sequence.md`, snapshots, kernel config hash, pmOS kernel version, and `oraclediff` allowlist in the repo; the replay test is the regression suite. When the pmOS kernel updates, recapture and diff the sequence (differences = Linux behaviour changes to review).

### Pitfalls checklist
- rwmmio logs VA; correlate with ioremap mapping (the VA base changes every boot — capture mapping in the *same* boot).
- Poll loops produce thousands of identical reads; the buffer wraps. Use per-CPU big buffers or `tracing_thresh`/filters; `echo 'val != 0' ...` filters can hide needed reads.
- Writes may be posted: the `rwmmio_post_write` event and ordering barriers matter for timing-sensitive sequences (QMP PHY, DSI link training); capture delays with timestamps (`udelay/usleep_range` appear only in function_graph; use `timer` events or record `abstime`).
- Qualcomm blocks need TZ/bootloader-initialised state (SMMU context banks, XPU/VMID ownership, IMEM/SMEM). A block that "works under Linux" because the bootloader pre-configured it will fail on Redox; diff `before-boot-state` (bootloader hand-off) by capturing *early* (cmdline-enabled events) and comparing first reads of each register to the reset value.
- Accesses via firmware (SCM calls, RPMh votes, remoteproc-owned blocks) don't appear as MMIO at all: trace them with section 2.
- Some registers fault on read when clocks are off (-> reset). Always enable clocks/GDSC first; never `devmem` a block whose clock `clk_summary` shows disabled.

## Sources consulted
- rwmmio series cover letters (event names, arm64 enablement): https://lore.kernel.org/all/YmqCYZwHoRuKtFSN%40kroah.com/T/ ; https://lkml.iu.edu/hypermail/linux/kernel/2112.0/04720.html ; https://lists.infradead.org/pipermail/linux-arm-kernel/2022-May/742261.html
- SDM845 upstream material (no emulator models found): https://lkml.iu.edu/hypermail/linux/kernel/1805.2/06498.html ; https://github.com/wode2016501/edk2-sdm845
- pmOS shared SDM845 kernel package: https://gitlab.postmarketos.org/postmarketOS/pmaports/-/merge_requests/898
- Everything else tagged [K] is from prior knowledge and should be verified on the device (`zcat /proc/config.gz`, `ls /sys/kernel/tracing/events`) before relying on it.
