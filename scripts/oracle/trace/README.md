# K12: short phone oracle traces

The phone is offline during development. Run these commands **only when the operator has brought pmOS back online**. All six profiles are read-only to storage and device bindings. The remote program is streamed over stdin to BusyBox `sh` under `sudo -n`; it creates one private tracefs instance, limits its buffer to 1 MiB per CPU, records for at most 30 seconds, disables events and removes the instance. It neither unbinds drivers nor writes a persistent file on the phone. Do not run concurrently with another K12 capture. Raw traces can contain input, storage or audio metadata and stay in a private host directory; never commit them.

First, inspect the exact commands without connecting:

```sh
PHONE_SSH='<ssh-target>'
TRACE_OUT="$HOME/artifacts/oneplus-oracle/trace-$(date +%Y%m%d-%H%M%S)"
for p in geni-i2c rmi4 spmi ufs dwc3 audio; do scripts/oracle/trace/capture.sh --dry-run "$p" "$PHONE_SSH" "$TRACE_OUT" 10; done
```

When the phone and transport are deliberately available, run the snapshot first and inspect `config.txt` for `CONFIG_TRACE_MMIO_ACCESS`, `CONFIG_FTRACE`, `CONFIG_REGMAP`, `CONFIG_SPMI`, `CONFIG_USB_DWC3`, `CONFIG_QRTR` and `CONFIG_QCOM_APR`. The capture program also checks `/proc/config.gz` via BusyBox `zcat` and reports missing tracepoint groups as `# UNAVAILABLE`; absence of `config.gz` does not stop tracing. It never installs tools or enables kprobes globally. The generated `.raw.txt` includes enabled/missing groups, kernel trace lines and an end marker. A `# NO_EVENTS` result is a gap, not a successful device trace.

## Tonight's command sequence

Run from this repo on the capture host after setting `PHONE_SSH` to the actual SSH alias. Start each capture in one terminal and perform the corresponding physical/host action during its ten-second window. Keep the SSH route available throughout, especially for USB.

```sh
scripts/oracle/snapshot.sh --dry-run "$PHONE_SSH" "$TRACE_OUT/snapshot"
scripts/oracle/snapshot.sh "$PHONE_SSH" "$TRACE_OUT/snapshot"
scripts/oracle/trace/capture.sh geni-i2c "$PHONE_SSH" "$TRACE_OUT" 10
scripts/oracle/trace/capture.sh rmi4 "$PHONE_SSH" "$TRACE_OUT" 10
scripts/oracle/trace/capture.sh spmi "$PHONE_SSH" "$TRACE_OUT" 10
scripts/oracle/trace/capture.sh ufs "$PHONE_SSH" "$TRACE_OUT" 10
scripts/oracle/trace/capture.sh dwc3 "$PHONE_SSH" "$TRACE_OUT" 10
scripts/oracle/trace/capture.sh audio "$PHONE_SSH" "$TRACE_OUT" 10
sha256sum -c "$TRACE_OUT"/*.raw.txt.sha256
```

| Profile | Stimulus during window | What to look for | Typical bounded raw size |
| --- | --- | --- | --- |
| `geni-i2c` | Touch screen a few times to cause i2c12 traffic | `rwmmio`, `i2c`, `regmap` | 10 KiB–5 MiB |
| `rmi4` | A few single and multi-touch gestures; existing probe messages only if already buffered elsewhere | `i2c`, `regmap`, `input` | 5 KiB–1 MiB |
| `spmi` | Press power briefly and volume up/down; do not long-press power | `spmi`, `regmap`, `gpio`, `input` | 5 KiB–2 MiB |
| `ufs` | Normal read-only filesystem activity; no driver unbind or partition writes | `ufs`, `scsi`, `rwmmio` | 10 KiB–10 MiB |
| `dwc3` | With SSH on Wi-Fi, reconnect USB once to enumerate the existing gadget | `dwc3`, `gadget`, `rwmmio` | 10 KiB–5 MiB |
| `audio` | Play a short local sound through the existing pmOS stack | `qrtr`, `rpmsg`, `asoc`, `snd_pcm` | 5 KiB–5 MiB |

Sizes are estimates, not acceptance thresholds. Trace buffer overwrites can truncate a busy profile; inspect the trace header for overruns. The RMI4 and UFS profiles also include bounded historical `dmesg` probe lines, but RMI4 **probe register traffic** and UFS boot-time link initialization cannot be reconstructed by a runtime, no-unbind capture; those require a separately approved boot trace. The audio profile captures only tracepoint metadata the kernel exports. It does **not** promise QMI TLV or APR payload bytes: if the groups are missing or payloads are absent, record that gap and plan a kernel tracepoint or BTF-verified kprobe/BPF capture separately. Never infer messages from a service inventory. `rwmmio` may miss raw or bulk accessors; see `docs/research/ACCEL-R3.md`.

## Convert MMIO after mapping addresses

`rwmmio` prints kernel **virtual** addresses. Obtain a verified VA→PA window mapping for that boot from tracing `ioremap` or equivalent kernel mapping evidence; `/proc/iomem` alone supplies only physical windows and cannot establish the virtual base. Set the three mapping variables below from verified evidence; the shown values are deliberately not guessed. The converter accepts `rwmmio_post_read` (returned value) and `rwmmio_write` (once), drops the corresponding pre-read/post-write duplicates, filters to the mapped window, and emits physical addresses in `mmio-replay` JSONL. It fails if no mapped events remain. Regmap, SPMI, I2C, UFS, USB and firmware protocol events stay as separate protocol evidence because they are not interchangeable with MMIO transactions.

```sh
VA='0x...'; PA='0x...'; SIZE='0x...' # replace with verified values before running
python3 scripts/oracle/trace/convert.py "$TRACE_OUT/geni-i2c.raw.txt" "$TRACE_OUT/geni-i2c.mmio.jsonl" --virtual-base "$VA" --physical-base "$PA" --size "$SIZE" --caller 'geni|i2c' --dry-run
python3 scripts/oracle/trace/convert.py "$TRACE_OUT/geni-i2c.raw.txt" "$TRACE_OUT/geni-i2c.mmio.jsonl" --virtual-base "$VA" --physical-base "$PA" --size "$SIZE" --caller 'geni|i2c'
```

The same converter applies to `ufs.raw.txt` and `dwc3.raw.txt` with their **own** verified windows. For GENI, feed the JSONL into `mmio_replay::Trace::from_jsonl` and `Replayer::strict` in `crates/geni-i2c` host tests. For SPMI, `crates/spmid` tests need a validated arbiter MMIO trace; PMIC regmap events are separate bus transactions. For touch, `crates/rmi4-touch` tests consume an I2C transaction model, not a fabricated MMIO trace. K10 UFS and K11 DWC3 host tests can use their physical MMIO traces once those crates land and their DMA/UPIU or TRB payload fixtures are separately obtained. Strip unrelated kernel polling or compare with `Replayer::lenient` only with an explicit test rationale. Keep private raw and converted captures outside Git; commit only redacted synthetic fixtures if needed.

## Offline checks

```sh
bash -n scripts/oracle/trace/capture.sh
sh -n scripts/oracle/trace/remote.sh
python3 -m unittest discover -s scripts/oracle/trace/tests -v
scripts/oracle/trace/capture.sh --dry-run geni-i2c fake-phone /tmp/k12-dry-run 2
```
