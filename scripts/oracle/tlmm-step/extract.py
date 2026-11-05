#!/usr/bin/env python3
"""Extract actual TLMM items from a tlmm.rs source text into an include!-able file.

usage: extract.py SRC OUT [--control NAME]
Items are copied verbatim (brace-matched); missing required items abort.
Controls apply a single textual mutation to the extracted copy only.
"""
import re
import sys

REQUIRED = ["const GPIO_COUNT", "pub struct Tlmm {", "impl Tlmm {", "impl InterruptHandler for Tlmm {"]
OPTIONAL = ["pub struct TlmmStep {", "fn tlmm_pending_step("]


def item(src, header):
    start = src.find(header)
    if start < 0:
        return None
    line_start = src.rfind("\n", 0, start) + 1
    if header.startswith("const"):
        end = src.index(";", start) + 1
        return src[line_start:end]
    i = src.index("{", start)
    depth = 0
    for j in range(i, len(src)):
        if src[j] == "{":
            depth += 1
        elif src[j] == "}":
            depth -= 1
            if depth == 0:
                return src[line_start:j + 1]
    raise SystemExit(f"unbalanced item {header!r}")


CONTROLS = {
    # Clear omitted from the scan: event fires with no prior clear.
    "omit-clear": [("            write(status, 1);\n", "")],
    # Clear delayed until after the callback in the handler.
    "late-clear": [
        ("            write(status, 1);\n", ""),
        ("            irq_trigger(step.virq, token);\n",
         "            irq_trigger(step.virq, token);\n"
         "            self.write(qcom_irq::tlmm_pin_offset(step.pin as u32, 0xc).unwrap(), 1);\n"),
    ],
    # Resume skips the pin right after the one just handled.
    "skip-next": [("next: pin + 1", "next: pin + 2")],
    # Resume ends the scan after the first event (drops later pending pins).
    "drop-later": [("next: pin + 1", "next: GPIO_COUNT")],
}


def main():
    src_path, out_path = sys.argv[1], sys.argv[2]
    control = sys.argv[4] if len(sys.argv) > 4 and sys.argv[3] == "--control" else None
    src = open(src_path).read()
    parts = []
    for h in REQUIRED:
        it = item(src, h)
        if it is None:
            raise SystemExit(f"missing required item {h!r}")
        parts.append(it)
    for h in OPTIONAL:
        it = item(src, h)
        if it is not None:
            parts.append(it)
    text = "\n\n".join(parts) + "\n"
    if control:
        for old, new in CONTROLS[control]:
            if text.count(old) != 1:
                raise SystemExit(f"control {control}: pattern {old!r} count {text.count(old)}")
            text = text.replace(old, new)
    open(out_path, "w").write(text)
    print(f"extracted {len(parts)} items from {src_path} -> {out_path} control={control}")


main()
