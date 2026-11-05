#!/usr/bin/env bash
# Acceptance tests for the E1 QEMU gate ITSELF. No boot, no network, no guest:
# these fixtures inject the exact faults the reviewer found and assert the gate
# rejects them, while a healthy capture passes.
#
#   scripts/qemu-demo/tests.sh
#
# Fixtures are generated into out/qemu-demo/tests/ (gitignored) so the repo stays
# clean and every case is reproducible from this file alone.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
here="$repo/scripts/qemu-demo"
t="$repo/out/qemu-demo/tests"
rm -rf "$t"; mkdir -p "$t"

pass=0; fail=0
ok()   { printf '  PASS  %s\n' "$1"; pass=$((pass + 1)); }
bad()  { printf '  FAIL  %s\n     %s\n' "$1" "${2:-}"; fail=$((fail + 1)); }

# Section 7 asserts the fixtures do not destroy real staged artifacts, so it must know which
# existed BEFORE the run: on a checkout that never staged, "absent" is not "destroyed".
staged_before=()
for f in ramboot-initfs.img esp.img bootloader-e1/build/bootloader-ramboot.efi; do
    [[ -e "$repo/out/qemu-demo/$f" ]] && staged_before+=("$f")
done
expect_rc() { # name expected_rc actual_rc command-output
    if [[ "$2" == "$3" ]]; then ok "$1"; else bad "$1" "expected exit $2, got $3 — $4"; fi
}

# ---------------------------------------------------------------- fixtures ----
mk() { printf '%s\n' "$2" > "$t/$1"; }
H='sandbox-demo: SANDBOX_HELLO_OK trace=service.hello payload={"count":1} fuel_used=727 memory_bytes=65536'
R='sandbox-demo: SANDBOX_REFUSED_TRACEEMIT capability=trace.emit reason=NotGranted stop=Denied { capability: TraceEmit, reason: NotGranted }'
N='sandbox-demo: stopped service is never re-entered'
D='sandbox-demo: done'

mk healthy.txt "$H
$R
$N
$D"
# The two markers, then the app failed on the last assertion.
mk two_markers_plus_FAIL.txt "$H
$R
sandbox-demo: FAIL a stopped service must not re-enter, got Ok(Report)"
# The two markers and the re-entry line, but the app never declared success.
mk absent_done.txt "$H
$R
$N"
# Both markers and done, but the final assertion never ran.
mk no_reentry_assertion.txt "$H
$R
$D"
# done printed before the refusal marker: run 2 never finished.
mk done_before_markers.txt "$D
$H
$R
$N"
mk only_hello.txt "$H"
: > "$t/empty.txt"
# Healthy capture wrapped in ANSI control sequences, as the raw serial really is.
printf '\x1b[1;32m%s\x1b[0m\r\n\x1b[33m%s\x1b[0m\r\n%s\r\n\x1b[0m%s\r\n' "$H" "$R" "$N" "$D" > "$t/healthy_ansi.txt"

# ------------------------------------------------- 1. strict validator -------
printf 'validate.sh — strict acceptance\n'
for f in two_markers_plus_FAIL absent_done no_reentry_assertion done_before_markers only_hello empty; do
    rc=0; out=$("$here/validate.sh" "$t/$f.txt" 2>&1) || rc=$?
    expect_rc "rejects $f" 1 "$rc" "$out"
done
rc=0; out=$("$here/validate.sh" "$t/does-not-exist.txt" 2>&1) || rc=$?
expect_rc "missing capture is rejected in strict mode" 1 "$rc" "$out"
for f in healthy healthy_ansi; do
    rc=0; out=$("$here/validate.sh" "$t/$f.txt" 2>&1) || rc=$?
    expect_rc "accepts $f" 0 "$rc" "$out"
done

# The gap this closes: the OLD boot.sh predicate (two marker greps) accepted every
# fault fixture above. If this ever stops being true, the fault set is not faithful.
printf '\nlegacy predicate (grep for the two markers) — must still be fooled\n'
fooled=0; total=0
for f in two_markers_plus_FAIL absent_done no_reentry_assertion done_before_markers; do
    total=$((total + 1))
    if grep -q 'SANDBOX_HELLO_OK' "$t/$f.txt" && grep -q 'SANDBOX_REFUSED_TRACEEMIT' "$t/$f.txt"; then
        fooled=$((fooled + 1))
    fi
done
expect_rc "old greps accept $fooled/$total fault captures (documents the bug)" 0 0 ""

# ------------------------------------- 2. boot.sh's poll/stop decision -------
printf '\nvalidate.sh --poll — the decision boot.sh acts on\n'
# Markers present but not terminal: boot.sh must KEEP WAITING (exit 1), not break.
mk partial_markers.txt "$H
$R"
rc=0; out=$("$here/validate.sh" --poll "$t/partial_markers.txt" 2>&1) || rc=$?
expect_rc "keeps waiting on markers without done" 1 "$rc" "$out"
rc=0; out=$("$here/validate.sh" --poll "$t/two_markers_plus_FAIL.txt" 2>&1) || rc=$?
expect_rc "stops early when the app printed FAIL" 2 "$rc" "$out"
rc=0; out=$("$here/validate.sh" --poll "$t/healthy.txt" 2>&1) || rc=$?
expect_rc "stops when the capture is accepted" 0 "$rc" "$out"
rc=0; out=$("$here/validate.sh" --poll "$t/does-not-exist.txt" 2>&1) || rc=$?
expect_rc "missing capture is not-yet in poll mode" 1 "$rc" "$out"

# --------------------------------------------------- 3. digest gate ----------
printf '\ncheck-digest.sh — fresh build vs embedded asset vs source pin\n'
# A miniature tree, because include_bytes! is resolved relative to the .rs file
# that contains it — exactly the way rustc resolves it. A flat fixture directory
# would hide path bugs.
dg="$t/digest"
mkdir -p "$dg/src" "$dg/assets" "$dg/other"
asset="$dg/assets/asset.wasm"; fresh="$dg/fresh.wasm"; mainsrc="$dg/src/main.rs"
printf 'guest-bytes-v1' > "$asset"
asset_sha=$(sha256sum "$asset" | cut -d ' ' -f1)
mkrust() { # $1 = pin to write, $2 = include_bytes! path, relative to src/
    printf 'const HELLO_RS_WASM: &[u8] = include_bytes!("%s");\nconst HELLO_RS_SHA256: &str = "%s";\n' "$2" "$1" > "$mainsrc"
}
cp "$asset" "$fresh"
mkrust "$asset_sha" "../assets/asset.wasm"
rc=0; out=$("$here/check-digest.sh" "$fresh" "$asset" "$mainsrc" 2>&1) || rc=$?
expect_rc "agreeing fresh/asset/pin passes" 0 "$rc" "$out"

printf 'guest-bytes-V2-different' > "$fresh"
rc=0; out=$("$here/check-digest.sh" "$fresh" "$asset" "$mainsrc" 2>&1) || rc=$?
expect_rc "fresh build differing from the embedded asset fails" 1 "$rc" "$out"
case "$out" in *"STALE guest"*) ok "names the stale-asset hazard";; *) bad "names the stale-asset hazard" "$out";; esac

cp "$asset" "$fresh"
mkrust "0000000000000000000000000000000000000000000000000000000000000000" "../assets/asset.wasm"
rc=0; out=$("$here/check-digest.sh" "$fresh" "$asset" "$mainsrc" 2>&1) || rc=$?
expect_rc "asset not matching the source pin fails" 1 "$rc" "$out"

mkrust "$asset_sha" "../assets/some_other.wasm"
rc=0; out=$("$here/check-digest.sh" "$fresh" "$asset" "$mainsrc" 2>&1) || rc=$?
expect_rc "source embedding a different asset than checked fails" 1 "$rc" "$out"

# The bypass the reviewer found: a DIFFERENT file with the SAME basename. Every
# digest below agrees — fresh == supplied asset == pin — yet the binary embeds a
# third file entirely. A basename comparison calls that a pass.
printf 'OTHER-BYTES-not-the-guest-anyone-hashed' > "$dg/other/asset.wasm"
mkrust "$asset_sha" "../other/asset.wasm"
rc=0; out=$("$here/check-digest.sh" "$fresh" "$asset" "$mainsrc" 2>&1) || rc=$?
expect_rc "same basename in another directory is rejected" 1 "$rc" "$out"
case "$out" in *"../other/asset.wasm"*) ok "names the path the source really embeds";; *) bad "names the path the source really embeds" "$out";; esac

# The real tree must be internally consistent, or build.sh would now refuse to run.
rc=0; out=$("$here/check-digest.sh" \
    "$repo/out/qemu-demo/wasm/wasm32-unknown-unknown/release/hello_rs.wasm" \
    "$repo/apps/sandbox-demo/assets/hello_rs.wasm" \
    "$repo/apps/sandbox-demo/src/main.rs" 2>&1) || rc=$?
expect_rc "real tree: fresh wasm == embedded asset == pin" 0 "$rc" "$out"

# ------------------------- 4. bounded child: timeout, kill-after, trap -------
printf '\nboot.sh — bounded child (stub QEMU that ignores TERM)\n'
stub="$t/stub-qemu.sh"
cat > "$stub" <<'STUB'
#!/usr/bin/env bash
# Stub standing in for qemu-system-aarch64: records the -pidfile it was given,
# writes its own pid there, and ignores TERM so the KILL path must fire.
pf=''
args=("$@")
for i in "${!args[@]}"; do
    [[ "${args[$i]}" == "-pidfile" ]] && pf="${args[$((i + 1))]}"
done
if [[ -n "$pf" ]]; then printf '%s\n' "$$" > "$pf"; fi
printf 'stub launched pid=%s pidfile=%s\n' "$$" "$pf" >> "${STUB_LOG:-/dev/null}"
trap '' TERM
while :; do sleep 1; done
STUB
chmod +x "$stub"

staged="$repo/out/qemu-demo/ramboot-initfs.img"
if [[ ! -f "$staged" ]]; then
    printf '  SKIP  boot.sh needs staged artifacts (run stage.sh first)\n'
else
    # A scratch artifact dir, so boot.sh's `rm -f serial.raw` can never destroy a
    # real capture. The staged images are symlinked, not copied.
    sout="$t/out"
    mkdir -p "$sout/bootloader-e1/build"
    ln -sf "$repo/out/qemu-demo/ramboot-initfs.img" "$sout/ramboot-initfs.img"
    ln -sf "$repo/out/qemu-demo/esp.img" "$sout/esp.img"
    ln -sf "$repo/out/qemu-demo/bootloader-e1/build/bootloader-ramboot.efi" \
        "$sout/bootloader-e1/build/bootloader-ramboot.efi"

    # (a) the timeout path: the deadline passes, TERM is ignored, KILL must land.
    : > "$t/stub-a.log"
    rc=0
    QEMU_DEMO_OUT="$sout" QEMU_BIN="$stub" STUB_LOG="$t/stub-a.log" BOOT_TIMEOUT=5 \
        "$here/boot.sh" >"$t/boot-a.log" 2>&1 || rc=$?
    expect_rc "boot.sh rejects a capture with no markers" 1 "$rc" "$(tail -3 "$t/boot-a.log")"
    stub_pid=$(sed -n 's/.*pid=\([0-9]*\).*/\1/p' "$t/stub-a.log" | head -1)
    if [[ -n "$stub_pid" ]] && kill -0 "$stub_pid" 2>/dev/null; then
        bad "stub QEMU is gone after boot.sh returned" "pid $stub_pid still alive — orphaned child"
    else
        ok "stub QEMU is gone after boot.sh returned (kill-after/trap worked)"
    fi
    grep -q 'pidfile=' "$t/stub-a.log" && ok "-pidfile is passed to QEMU" || bad "-pidfile is passed to QEMU" "$(cat "$t/stub-a.log")"

    # (b) the trap path: this script is interrupted while the child is running.
    : > "$t/stub-b.log"
    QEMU_DEMO_OUT="$sout" QEMU_BIN="$stub" STUB_LOG="$t/stub-b.log" BOOT_TIMEOUT=300 \
        "$here/boot.sh" >"$t/boot-b.log" 2>&1 &
    bpid=$!
    sleep 6
    kill -TERM "$bpid" 2>/dev/null || true
    wait "$bpid" 2>/dev/null || true
    stub_pid=$(sed -n 's/.*pid=\([0-9]*\).*/\1/p' "$t/stub-b.log" | head -1)
    if [[ -n "$stub_pid" ]] && kill -0 "$stub_pid" 2>/dev/null; then
        bad "no orphan after boot.sh is interrupted" "pid $stub_pid still alive"
    else
        ok "no orphan QEMU after boot.sh is interrupted (EXIT trap)"
    fi
fi

# ------------------- 5. evidence.sh refuses to document a failed run ---------
printf '\nevidence.sh — no document without a validated capture\n'
S="$t/scratch-evidence"
mkdir -p "$S/scripts/qemu-demo" "$S/docs/workbench" "$S/out/qemu-demo" \
         "$S/apps/sandbox-demo/assets" "$S/apps/sandbox-demo/src"
cp "$here/evidence.sh" "$here/validate.sh" "$S/scripts/qemu-demo/"
cp "$t/two_markers_plus_FAIL.txt" "$S/out/qemu-demo/serial.raw"
printf 'guest-bytes-v1' > "$S/apps/sandbox-demo/assets/hello_rs.wasm"
mkrust "$asset_sha" "../assets/hello_rs.wasm"
cp "$mainsrc" "$S/apps/sandbox-demo/src/main.rs"
rc=0; ( cd "$S" && ./scripts/qemu-demo/evidence.sh ) >"$t/evidence-fail.log" 2>&1 || rc=$?
expect_rc "exits non-zero on a FAIL capture" 1 "$rc" "$(tail -3 "$t/evidence-fail.log")"
if [[ -e "$S/docs/workbench/QEMU-SANDBOX-DEMO.txt" ]]; then
    bad "writes no evidence document" "the document exists anyway"
else
    ok "writes no evidence document"
fi
if compgen -G "$S/docs/workbench/evidence/*" > /dev/null; then
    bad "writes no receipts for a rejected run" "receipts were written"
else
    ok "writes no receipts for a rejected run"
fi

# A validated capture does produce receipts, and a rerun on the SAME capture must
# not overwrite them.
cp "$t/healthy.txt" "$S/out/qemu-demo/serial.raw"
touch -d '2026-01-01T00:00:00Z' "$S/out/qemu-demo/serial.raw"
rc=0; ( cd "$S" && ./scripts/qemu-demo/evidence.sh ) >"$t/evidence-ok.log" 2>&1 || rc=$?
expect_rc "a validated capture produces evidence" 0 "$rc" "$(tail -3 "$t/evidence-ok.log")"
rc=0; ( cd "$S" && ./scripts/qemu-demo/evidence.sh ) >"$t/evidence-rerun.log" 2>&1 || rc=$?
expect_rc "re-running on the same capture refuses to overwrite receipts" 1 "$rc" "$(tail -2 "$t/evidence-rerun.log")"
if [[ -f "$S/docs/workbench/QEMU-SANDBOX-DEMO-runs.tsv" ]]; then
    rows=$(grep -c '^2026' "$S/docs/workbench/QEMU-SANDBOX-DEMO-runs.tsv" || true)
    if [[ "$rows" == "1" ]]; then ok "the ledger holds exactly one row for that run"; else bad "the ledger holds exactly one row for that run" "rows=$rows"; fi
else
    bad "the ledger holds exactly one row for that run" "no ledger"
fi

# ---- 6. evidence.sh provenance: an archived capture is not today's build ----
printf '\nevidence.sh — an archived capture never inherits the current build\n'
first_receipt() { # <scratch repo> [nth]
    find "$1/docs/workbench/evidence" -name '*-receipt.tsv' 2>/dev/null | sort | sed -n "${2:-1}p" || true
}
A="$t/scratch-arch"
mkdir -p "$A/scripts/qemu-demo" "$A/docs/workbench" "$A/out/qemu-demo" \
         "$A/apps/sandbox-demo/assets" "$A/apps/sandbox-demo/src"
cp "$here/evidence.sh" "$here/validate.sh" "$A/scripts/qemu-demo/"
printf 'guest-bytes-v1' > "$A/apps/sandbox-demo/assets/hello_rs.wasm"
mkrust "$asset_sha" "../assets/hello_rs.wasm"
cp "$mainsrc" "$A/apps/sandbox-demo/src/main.rs"

# The worktree has NO current capture at all. --serial must still work: the
# argument has to be parsed before the default path is checked for existence.
arch="$t/archive"
mkdir -p "$arch"
cp "$t/healthy.txt" "$arch/serial.raw"
touch -d '2026-02-02T00:00:00Z' "$arch/serial.raw"
# And the worktree holds a build from some OTHER run. If any of its digests land
# in the archived run's receipt, the evidence is fabricated.
printf 'CURRENT-BUILD-esp-not-the-archived-run' > "$A/out/qemu-demo/esp.img"
printf 'CURRENT-BUILD-initfs-not-the-archived-run' > "$A/out/qemu-demo/ramboot-initfs.img"
cur_esp=$(sha256sum "$A/out/qemu-demo/esp.img" | cut -d ' ' -f1)

rc=0; ( cd "$A" && ./scripts/qemu-demo/evidence.sh --serial "$arch/serial.raw" ) >"$t/ev-arch.log" 2>&1 || rc=$?
expect_rc "archive works with no default capture present" 0 "$rc" "$(tail -3 "$t/ev-arch.log")"
rec=$(first_receipt "$A")
if [[ -z "$rec" ]]; then
    bad "receipt written for the archived capture" "no receipt at all"
    bad "the current build is not attributed to the archived run" "no receipt"
    bad "unverifiable artifacts are labelled UNKNOWN" "no receipt"
else
    ok "receipt written for the archived capture"
    if grep -q "$cur_esp" "$rec"; then
        bad "the current build is not attributed to the archived run" "receipt carries the current esp.img digest"
    else
        ok "the current build is not attributed to the archived run"
    fi
    if grep -q 'UNKNOWN' "$rec" && grep -qi 'no manifest' "$rec"; then
        ok "unverifiable artifacts are labelled UNKNOWN, with the reason"
    else
        bad "unverifiable artifacts are labelled UNKNOWN, with the reason" "$(cat "$rec")"
    fi
fi

# With a paired manifest that vouches for the capture, the archived artifacts ARE
# attributable — and they must be the manifest's digests, never the worktree's.
arch2="$t/archive-manifested"
mkdir -p "$arch2"
cp "$t/healthy.txt" "$arch2/serial.raw"
touch -d '2026-03-03T00:00:00Z' "$arch2/serial.raw"
printf 'ARCHIVED-run-esp' > "$arch2/esp.img"
printf 'ARCHIVED-run-initfs' > "$arch2/ramboot-initfs.img"
printf 'ARCHIVED-run-dtb' > "$arch2/simplefb-virt.dtb"
arch_esp=$(sha256sum "$arch2/esp.img" | cut -d ' ' -f1)
( cd "$arch2" && sha256sum serial.raw esp.img ramboot-initfs.img simplefb-virt.dtb > SHA256SUMS )
rc=0; ( cd "$A" && ./scripts/qemu-demo/evidence.sh --serial "$arch2/serial.raw" ) >"$t/ev-man.log" 2>&1 || rc=$?
expect_rc "archive with a verifying manifest is accepted" 0 "$rc" "$(tail -3 "$t/ev-man.log")"
rec2=$(first_receipt "$A" 2)
if [[ -z "$rec2" ]]; then
    bad "manifest digests recorded" "no second receipt"
else
    if grep -q "$arch_esp" "$rec2"; then ok "the manifest's own artifact digests are recorded"; else bad "the manifest's own artifact digests are recorded" "$(cat "$rec2")"; fi
    if grep -q "$cur_esp" "$rec2"; then bad "the current build still stays out of it" "current digest present"; else ok "the current build still stays out of it"; fi
fi

# A manifest that does not vouch for THIS capture proves nothing about it.
arch3="$t/archive-lying"
mkdir -p "$arch3"
cp "$t/healthy.txt" "$arch3/serial.raw"
touch -d '2026-04-04T00:00:00Z' "$arch3/serial.raw"
printf 'ARCHIVED-run-esp' > "$arch3/esp.img"
( cd "$arch3" && sha256sum esp.img > SHA256SUMS )
rc=0; ( cd "$A" && ./scripts/qemu-demo/evidence.sh --serial "$arch3/serial.raw" ) >"$t/ev-lying.log" 2>&1 || rc=$?
expect_rc "a manifest that does not vouch for the capture is refused" 1 "$rc" "$(tail -3 "$t/ev-lying.log")"
case "$(cat "$t/ev-lying.log")" in *[Mm]anifest*) ok "the refusal names the manifest as the problem";; *) bad "the refusal names the manifest as the problem" "$(tail -3 "$t/ev-lying.log")";; esac

# A manifest whose files no longer match its recorded digests is refused too.
arch4="$t/archive-tampered"
mkdir -p "$arch4"
cp "$t/healthy.txt" "$arch4/serial.raw"
touch -d '2026-05-05T00:00:00Z' "$arch4/serial.raw"
printf 'ARCHIVED-run-esp' > "$arch4/esp.img"
( cd "$arch4" && sha256sum serial.raw esp.img > SHA256SUMS )
printf 'TAMPERED-AFTER-THE-MANIFEST-WAS-WRITTEN' > "$arch4/esp.img"
rc=0; ( cd "$A" && ./scripts/qemu-demo/evidence.sh --serial "$arch4/serial.raw" ) >"$t/ev-tamper.log" 2>&1 || rc=$?
expect_rc "a manifest whose files do not match is refused" 1 "$rc" "$(tail -3 "$t/ev-tamper.log")"
case "$(cat "$t/ev-tamper.log")" in *[Mm]anifest*) ok "the refusal says which manifest row broke";; *) bad "the refusal says which manifest row broke" "$(tail -3 "$t/ev-tamper.log")";; esac

if [[ -f "$A/docs/workbench/QEMU-SANDBOX-DEMO-runs.tsv" ]]; then
    rows=$(grep -c '^2026' "$A/docs/workbench/QEMU-SANDBOX-DEMO-runs.tsv" || true)
    if [[ "$rows" == "2" ]]; then ok "only the two accepted runs reached the ledger"; else bad "only the two accepted runs reached the ledger" "rows=$rows"; fi
else
    bad "only the two accepted runs reached the ledger" "no ledger"
fi

# ---- 6b. manifest rows bind by PATH, never by basename --------------------
# A verified manifest can still lie by proximity: it may list `other/esp.img`, a
# file that has nothing to do with the run's ESP, and a basename lookup would
# happily print its digest as `out/qemu-demo/esp.img`. Same for the capture: a row
# named `serial.raw` that resolves to some other file is not this capture.
printf '\nevidence.sh — manifest rows are bound by path, not by filename\n'
B="$t/scratch-paths"
mkdir -p "$B/scripts/qemu-demo" "$B/docs/workbench" "$B/out/qemu-demo" \
         "$B/apps/sandbox-demo/assets" "$B/apps/sandbox-demo/src"
cp "$here/evidence.sh" "$here/validate.sh" "$B/scripts/qemu-demo/"
printf 'guest-bytes-v1' > "$B/apps/sandbox-demo/assets/hello_rs.wasm"
mkrust "$asset_sha" "../assets/hello_rs.wasm"
cp "$mainsrc" "$B/apps/sandbox-demo/src/main.rs"

# Same basename, different file, inside a manifest that otherwise verifies.
arch5="$t/archive-alias"
mkdir -p "$arch5/other"
cp "$t/healthy.txt" "$arch5/serial.raw"
touch -d '2026-06-06T00:00:00Z' "$arch5/serial.raw"
printf 'UNRELATED-esp-in-another-directory' > "$arch5/other/esp.img"
alias_esp=$(sha256sum "$arch5/other/esp.img" | cut -d ' ' -f1)
( cd "$arch5" && sha256sum serial.raw other/esp.img > SHA256SUMS )
rc=0; ( cd "$B" && ./scripts/qemu-demo/evidence.sh --serial "$arch5/serial.raw" ) >"$t/ev-alias.log" 2>&1 || rc=$?
expect_rc "an archive whose manifest lists other/esp.img is still accepted" 0 "$rc" "$(tail -3 "$t/ev-alias.log")"
rec=$(first_receipt "$B")
if [[ -z "$rec" ]]; then
    bad "the alias digest is not recorded as the run's ESP" "no receipt"
    bad "the ESP is labelled UNKNOWN with the path reason" "no receipt"
else
    if grep -q "$alias_esp" "$rec"; then
        bad "the alias digest is not recorded as the run's ESP" "receipt carries other/esp.img as esp_img"
    else
        ok "the alias digest is not recorded as the run's ESP"
    fi
    if grep -q 'UNKNOWN' "$rec" && grep -q 'esp.img' "$rec"; then
        ok "the ESP is labelled UNKNOWN with the path reason"
    else
        bad "the ESP is labelled UNKNOWN with the path reason" "$(grep esp_img "$rec" || true)"
    fi
fi

# A row named serial.raw that resolves to a DIFFERENT file is not this capture.
arch6="$t/archive-wrong-capture"
mkdir -p "$arch6/other"
cp "$t/healthy.txt" "$arch6/serial.raw"
cp "$t/healthy.txt" "$arch6/other/serial.raw"
touch -d '2026-07-07T00:00:00Z' "$arch6/other/serial.raw"
printf 'ARCHIVED-run-esp' > "$arch6/esp.img"
( cd "$arch6" && sha256sum other/serial.raw esp.img > SHA256SUMS )
rc=0; ( cd "$B" && ./scripts/qemu-demo/evidence.sh --serial "$arch6/serial.raw" ) >"$t/ev-wrongcap.log" 2>&1 || rc=$?
expect_rc "a manifest pairing a same-named copy is refused" 1 "$rc" "$(tail -3 "$t/ev-wrongcap.log")"
case "$(cat "$t/ev-wrongcap.log")" in *"$arch6/serial.raw"*) ok "the refusal names the capture path it could not pair";; *) bad "the refusal names the capture path it could not pair" "$(tail -3 "$t/ev-wrongcap.log")";; esac

# An artifact stored under a nested path is UNKNOWN until it is mapped explicitly.
arch7="$t/archive-nested"
mkdir -p "$arch7/run"
cp "$t/healthy.txt" "$arch7/serial.raw"
touch -d '2026-08-08T00:00:00Z' "$arch7/serial.raw"
printf 'NESTED-run-esp' > "$arch7/run/esp.img"
nested_esp=$(sha256sum "$arch7/run/esp.img" | cut -d ' ' -f1)
( cd "$arch7" && sha256sum serial.raw run/esp.img > SHA256SUMS )
rc=0; ( cd "$B" && ./scripts/qemu-demo/evidence.sh --serial "$arch7/serial.raw" ) >"$t/ev-nested.log" 2>&1 || rc=$?
expect_rc "a nested artifact is UNKNOWN until it is mapped" 0 "$rc" "$(tail -3 "$t/ev-nested.log")"
rec=$(first_receipt "$B" 2)
if [[ -n "$rec" ]] && grep -q "$nested_esp" "$rec"; then
    bad "a nested artifact is UNKNOWN until it is mapped" "receipt took run/esp.img without a mapping"
else
    ok "a nested artifact is UNKNOWN until it is mapped"
fi

# The explicit role mapping is the only way a non-canonical layout is accepted.
rc=0; ( cd "$B" && ./scripts/qemu-demo/evidence.sh --serial "$arch7/serial.raw" --map esp=run/esp.img ) >"$t/ev-map.log" 2>&1 || rc=$?
expect_rc "a documented capture is not re-documented by adding a mapping" 1 "$rc" "$(tail -2 "$t/ev-map.log")"
if grep -qi 'overwrite' "$t/ev-map.log"; then ok "the same capture is never documented twice"; else bad "the same capture is never documented twice" "$(tail -3 "$t/ev-map.log")"; fi
arch8="$t/archive-mapped"
mkdir -p "$arch8/run"
cp "$t/healthy.txt" "$arch8/serial.raw"
touch -d '2026-09-09T00:00:00Z' "$arch8/serial.raw"
printf 'MAPPED-run-esp' > "$arch8/run/esp.img"
mapped_esp=$(sha256sum "$arch8/run/esp.img" | cut -d ' ' -f1)
( cd "$arch8" && sha256sum serial.raw run/esp.img > SHA256SUMS )
rc=0; ( cd "$B" && ./scripts/qemu-demo/evidence.sh --serial "$arch8/serial.raw" --map esp=run/esp.img ) >"$t/ev-map2.log" 2>&1 || rc=$?
expect_rc "a mapped archive records the mapped artifact's digest" 0 "$rc" "$(tail -3 "$t/ev-map2.log")"
rec=$(first_receipt "$B" 3)
if [[ -n "$rec" ]] && grep -q "$mapped_esp" "$rec"; then ok "the mapped digest is the one recorded"; else bad "the mapped digest is the one recorded" "$(cat "${rec:-/dev/null}" 2>/dev/null | grep esp_img || true)"; fi
rc=0; ( cd "$B" && ./scripts/qemu-demo/evidence.sh --serial "$arch8/serial.raw" --map nonsense=run/esp.img ) >"$t/ev-badmap.log" 2>&1 || rc=$?
expect_rc "an unknown --map role is rejected, not ignored" 1 "$rc" "$(tail -2 "$t/ev-badmap.log")"

if [[ -f "$B/docs/workbench/QEMU-SANDBOX-DEMO-runs.tsv" ]]; then
    rows=$(grep -c '^2026' "$B/docs/workbench/QEMU-SANDBOX-DEMO-runs.tsv" || true)
    if [[ "$rows" == "3" ]]; then ok "only the three accepted path runs reached the ledger"; else bad "only the three accepted path runs reached the ledger" "rows=$rows"; fi
else
    bad "only the three accepted path runs reached the ledger" "no ledger"
fi

# ------------------------------------------- 7. staged artifacts stay intact --
printf '\nthe fixtures never touch the real staged artifacts\n'
if ((${#staged_before[@]} == 0)); then
    printf '  SKIP  no staged artifacts before the run (run stage.sh to exercise this check)\n'
fi
for f in "${staged_before[@]}"; do
    if [[ ! -e "$repo/out/qemu-demo/$f" ]]; then
        bad "staged $f survived the fixture run" "it is gone — a fixture wrote into out/qemu-demo"
    else
        ok "staged $f survived the fixture run"
    fi
done

# ------------------------------------------------------- 8. syntax ----------
printf '\nbash -n on every gate script\n'
for s in "$here"/*.sh; do
    if bash -n "$s" 2>"$t/syntax.log"; then ok "bash -n $(basename "$s")"; else bad "bash -n $(basename "$s")" "$(cat "$t/syntax.log")"; fi
done

# ------------------------------------------------------- cleanup ------------
if [[ -n "${KEEP_TESTS:-}" ]]; then
    printf '\nfixtures kept under %s (KEEP_TESTS is set)\n' "$t"
else
    # The scratch artifact dir holds SYMLINKS to the staged images, so removing it
    # removes the links and never the artifacts themselves.
    rm -rf "$t"
    printf '\nscratch fixtures removed (%s)\n' "$t"
fi

printf '\n%d passed, %d failed\n' "$pass" "$fail"
exit "$((fail > 0))"
