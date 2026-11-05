#!/usr/bin/env bash
# shellcheck disable=SC2016 # Remote shell commands and literal Markdown deliberately use single quotes.
# Read-only postmarketOS oracle capture; only debugfs may be mounted remotely.
set -u
usage() { echo "Usage: $0 [--dry-run] <ssh-target> <outdir>" >&2; exit 2; }
dry_run=0
if [[ ${1:-} == --dry-run ]]; then dry_run=1; shift; fi
[[ $# == 2 && -n $1 && -n $2 ]] || usage
target=$1 outdir=$2
quote() { local s=${1//\'/\'\\\'\'}; printf "'%s'" "$s"; }
run_remote() {
    local name=$1 cmd=$2 remote
    remote="sudo -n sh -c $(quote "$cmd")"
    if (( dry_run )); then
        printf '%q ' timeout 25s ssh -o BatchMode=yes -o ConnectTimeout=8 "$target" "$remote"
        printf '> %q\n' "$outdir/$name.txt"
        return
    fi
    if timeout 25s ssh -o BatchMode=yes -o ConnectTimeout=8 "$target" "$remote" >"$outdir/$name.txt" 2>&1; then
        printf '| %s | OK |\n' "$name" >> "$outdir/.status"
    else
        printf '| %s | FAILED |\n' "$name" >> "$outdir/.status"
        failed=1
    fi
}
if (( !dry_run )); then mkdir -p "$outdir"; : > "$outdir/.status"; fi
failed=0
run_remote fdt 'cat /sys/firmware/fdt'
if (( dry_run )); then
    printf 'mv %q %q\n' "$outdir/fdt.txt" "$outdir/fdt.dtb"
    if command -v dtc >/dev/null 2>&1; then
        printf 'dtc -I dtb -O dts %q > %q\n' "$outdir/fdt.dtb" "$outdir/fdt.txt"
    else
        printf 'dtc unavailable locally: write note to %q\n' "$outdir/fdt.txt"
    fi
fi
if (( !dry_run )); then
    mv "$outdir/fdt.txt" "$outdir/fdt.dtb"
    if command -v dtc >/dev/null 2>&1; then
        if dtc -I dtb -O dts "$outdir/fdt.dtb" >"$outdir/fdt.txt" 2>&1; then
            printf '| fdt-decompile | OK |\n' >> "$outdir/.status"
        else
            printf '| fdt-decompile | FAILED |\n' >> "$outdir/.status"; failed=1
        fi
    else
        printf 'dtc unavailable on capture host; fdt.dtb retained.\n' > "$outdir/fdt.txt"
        printf '| fdt-decompile | UNAVAILABLE |\n' >> "$outdir/.status"
    fi
fi
run_remote dmesg 'dmesg | grep -iE "EL[12]|psci|smmu|gic|ufs|dwc3|geni" || true'
run_remote iomem 'cat /proc/iomem'
run_remote interrupts 'cat /proc/interrupts'
run_remote cmdline 'cat /proc/cmdline'
run_remote debugfs-mount 'grep -q " /sys/kernel/debug debugfs " /proc/mounts || mount -t debugfs debugfs /sys/kernel/debug; grep " /sys/kernel/debug debugfs " /proc/mounts'
run_remote clk 'cat /sys/kernel/debug/clk/clk_summary'
run_remote regulator 'cat /sys/kernel/debug/regulator/regulator_summary'
run_remote pinctrl 'for d in /sys/kernel/debug/pinctrl/*; do [ -d "$d" ] || continue; echo "### $d/pins"; cat "$d/pins" 2>&1; echo "### $d/pinmux-pins"; cat "$d/pinmux-pins" 2>&1; done'
run_remote gpio 'cat /sys/kernel/debug/gpio'
run_remote interconnect 'if [ -d /sys/kernel/debug/interconnect ]; then find /sys/kernel/debug/interconnect -type f -print -exec cat {} \; ; else echo "unavailable"; fi'
run_remote iommu 'if [ -d /sys/kernel/debug/iommu ]; then find /sys/kernel/debug/iommu -type f -print -exec cat {} \; ; else echo "unavailable"; fi'
run_remote config 'zcat /proc/config.gz'
run_remote partitions 'ls -l /dev/disk/by-partlabel'
run_remote lsmod 'lsmod'
if (( dry_run )); then exit 0; fi
(
    cd "$outdir" || exit 1
    sha256sum -- *.txt fdt.dtb > SHA256SUMS
)
{
    echo '# OnePlus 6 postmarketOS oracle snapshot'
    echo
    echo 'Capture ran with `sudo -n` over timeout-bounded SSH. Only a debugfs mount was permitted on the phone.'
    echo
    echo '| Capture | Result |'
    echo '| --- | --- |'
    cat "$outdir/.status"
    echo
    echo 'The raw device tree is `fdt.dtb`; its host-side decompile is `fdt.txt`. `SHA256SUMS` covers both and all text captures.'
} > "$outdir/summary.md"
rm "$outdir/.status"
exit "$failed"
