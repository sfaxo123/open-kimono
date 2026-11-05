#!/bin/sh
# Sent through stdin to BusyBox sh. No remote file is created.
set -u
profile=$1 seconds=$2
root=/sys/kernel/tracing
if [ ! -d "$root/events" ]; then root=/sys/kernel/debug/tracing; fi
if [ ! -d "$root/events" ]; then echo '# UNAVAILABLE tracefs is not mounted'; exit 0; fi
if [ -r /proc/config.gz ] && command -v zcat >/dev/null 2>&1; then
    echo '# CONFIG (selected):'
    zcat /proc/config.gz 2>/dev/null | grep -E '^CONFIG_(FTRACE|TRACE_MMIO_ACCESS|REGMAP|SPMI|I2C|SCSI_UFS|USB_DWC3|QRTR|QCOM_APR|KPROBES|BPF_SYSCALL)=' || true
else
    echo '# CONFIG unavailable: /proc/config.gz or zcat missing'
fi
case "$profile" in
    rmi4) echo '# PROBE LOG (historical, not register trace)'; dmesg 2>/dev/null | grep -iE 'rmi|s3706|synaptics' | tail -n 40 || true ;;
    ufs) echo '# PROBE LOG (historical, not register trace)'; dmesg 2>/dev/null | grep -iE 'ufshc|ufshcd|ufs' | tail -n 40 || true ;;
    audio) if command -v qrtr-lookup >/dev/null 2>&1; then echo '# QRTR SERVICE INVENTORY (not messages)'; qrtr-lookup 2>/dev/null || true; fi ;;
esac
# A tracefs instance isolates event enables and buffers from system tracing.
instance="$root/instances/op6_k12_$$"
if ! mkdir "$instance" 2>/dev/null; then echo '# UNAVAILABLE tracefs instance creation failed'; exit 0; fi
cleanup() {
    echo 0 > "$instance/tracing_on" 2>/dev/null || true
    # Event enables are local to the instance; disable before removal.
    echo 0 > "$instance/events/enable" 2>/dev/null || true
    rmdir "$instance" 2>/dev/null || true
}
trap cleanup EXIT HUP INT TERM
case "$profile" in
    geni-i2c) groups='rwmmio i2c regmap' ;;
    rmi4) groups='rwmmio i2c regmap input' ;;
    spmi) groups='rwmmio spmi regmap gpio input' ;;
    ufs) groups='rwmmio ufs scsi' ;;
    dwc3) groups='rwmmio dwc3 gadget' ;;
    audio) groups='qrtr rpmsg asoc snd_pcm' ;;
    *) echo '# ERROR unknown profile'; exit 2 ;;
esac
# 1 MiB per CPU keeps a short trace bounded. Never clear the global buffer.
echo 0 > "$instance/tracing_on"
echo 1024 > "$instance/buffer_size_kb" 2>/dev/null || true
: > "$instance/trace"
selected=0
for group in $groups; do
    if [ -f "$instance/events/$group/enable" ]; then
        if echo 1 > "$instance/events/$group/enable" 2>/dev/null; then
            echo "# ENABLED $group"
            selected=$((selected + 1))
        else
            echo "# UNAVAILABLE $group (enable failed)"
        fi
    else
        echo "# UNAVAILABLE $group (not built or no tracepoints)"
    fi
done
if [ "$selected" -eq 0 ]; then echo '# NO_EVENTS'; exit 0; fi
echo "# BEGIN profile=$profile seconds=$seconds"
echo 1 > "$instance/tracing_on"
sleep "$seconds"
echo 0 > "$instance/tracing_on"
cat "$instance/trace"
echo '# END'
