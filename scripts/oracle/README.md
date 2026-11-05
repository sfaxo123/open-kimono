# Phone oracle snapshot

Run from the 3090 when the OnePlus 6 is back on postmarketOS:

```sh
scripts/oracle/snapshot.sh --dry-run <ssh-target> <outdir>
scripts/oracle/snapshot.sh <ssh-target> <outdir>
```

Every SSH invocation has an 8-second connection timeout and a 25-second outer timeout. The script uses `sudo -n` on the phone; the only phone mutation is mounting debugfs if needed. All captures are written on the 3090. `fdt.dtb` is the raw device tree, and `fdt.txt` is its host-side `dtc` decompile when `dtc` is installed. Check `summary.md` for failed or unavailable captures, then run `sha256sum -c SHA256SUMS` in the output directory. Treat these snapshots as private phone facts; do not commit them.

## Canonical context gate for new source packets

Use `verify_source_packet.py --handoff $HOME/Developer/experiments/redox-oneplus6/docs/HANDOFF.md KERNEL_ROOT PACKET SOURCE...` for new native packets. Packet metadata must include exactly one line of each form:

    Canonical handoff: $HOME/Developer/experiments/redox-oneplus6/docs/HANDOFF.md
    Canonical handoff SHA256: COMPUTED_CURRENT_FILE_HASH

Compute the hash from the actual canonical file; never search source worktrees for another handoff when it is missing locally. The new optional flag rejects missing, duplicate, wrong-path or stale declarations alongside existing byte-match checks. The JSON records the actual context hash. This proves declared context consistency, not that the author read or understood it. Save the check at collection time: later handoff updates intentionally make an older declaration stale. Without the flag, historical packets retain their original code-only check.

Regression: `python3 scripts/oracle/test_verify_source_packet.py`.
