#!/usr/bin/env python3
"""Extract actual PLIC + syscall Io items into include!-able harness sources.

usage:
  extract.py io  SYSCALL_IO_DIR OUT
  extract.py plic PLIC_RS OUT [--control NAME]

Items are copied verbatim (brace-matched, with leading attributes/docs).
The only rewrites: `read_volatile(`/`write_volatile(` -> trace hooks in the
generic (non-x86) Mmio/MmioPtr Io impls, their `#[cfg(...)]` attribute removed
so they apply on the x86_64 host, and `crate::PHYS_OFFSET` -> harness const.
Controls apply a single textual mutation to the extracted copy only.
"""
import hashlib
import re
import sys


def block(src, header, attrs=True):
    start = src.find(header)
    if start < 0:
        return None
    if src.find(header, start + 1) >= 0:
        raise SystemExit(f"ambiguous header {header!r}")
    line_start = src.rfind("\n", 0, start) + 1
    if attrs:
        while True:
            prev_end = line_start - 1
            prev_start = src.rfind("\n", 0, max(prev_end, 0)) + 1
            prev = src[prev_start:prev_end].strip()
            if prev_end > 0 and (prev.startswith("#[") or prev.startswith("///")):
                line_start = prev_start
            else:
                break
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


def need(src, header, attrs=True):
    it = block(src, header, attrs)
    if it is None:
        raise SystemExit(f"missing required item {header!r}")
    return it


def sub1(text, old, new, what):
    if text.count(old) != 1:
        raise SystemExit(f"{what}: pattern {old!r} count {text.count(old)}")
    return text.replace(old, new)


def strip_cfg(item):
    return "\n".join(l for l in item.split("\n") if not l.strip().startswith("#[cfg("))


def extract_io(io_dir):
    mod = open(f"{io_dir}/mod.rs").read()
    mmio = open(f"{io_dir}/mmio.rs").read()
    mptr = open(f"{io_dir}/mmio_ptr.rs").read()
    parts = [need(mod, "pub trait Io {")]
    parts.append(need(mmio, "pub struct Mmio<T> {"))
    parts.append(need(mmio, "impl<T> Mmio<T> {"))
    g = strip_cfg(need(mmio, "impl<T> Io for Mmio<T>"))
    g = sub1(g, "ptr::read_volatile(", "crate::hook::read_volatile(", "mmio read")
    g = sub1(g, "ptr::write_volatile(", "crate::hook::write_volatile(", "mmio write")
    parts.append(g)
    parts.append(need(mptr, "pub struct MmioPtr<T> {"))
    parts.append(need(mptr, "impl<T> MmioPtr<T> {"))
    g = strip_cfg(need(mptr, "impl<T> Io for MmioPtr<T>"))
    g = sub1(g, "core::ptr::read_volatile(", "crate::hook::read_volatile(", "mmioptr read")
    g = sub1(g, "core::ptr::write_volatile(", "crate::hook::write_volatile(", "mmioptr write")
    parts.append(g)
    return parts


CONTROLS = {
    "wrong-context-stride": [(
        "THRESHOLDS_OFFSET + THRESHOLD_CONTEXT_STRIDE * context",
        "THRESHOLDS_OFFSET + ENABLE_CONTEXT_STRIDE * context")],
    "extra-claim-read": [(
        "        let claim = self\n",
        "        let _ = self.reg(Self::threshold_block(context) + CLAIM_COMPLETE_OFFSET).read();\n"
        "        let claim = self\n")],
    "wrong-enable-bit": [(".writef(1u32 << irq, enable)", ".writef(1u32 << irq_lane, enable)")],
    "rmw-before-priority": [(
        "        regs.set_priority(irq_num as usize, 1);\n"
        "        regs.enable(self.context, NonZero::new(irq_num as usize).unwrap(), true);\n",
        "        regs.enable(self.context, NonZero::new(irq_num as usize).unwrap(), true);\n"
        "        regs.set_priority(irq_num as usize, 1);\n")],
    # Positive control for the auto-trait fixtures: drops the !Send/!Sync marker.
    "drop-marker": [
        ("    _not_send_sync: PhantomData<*mut ()>,\n", ""),
        ("            _not_send_sync: PhantomData,\n", ""),
    ],
}

ACCESSOR_OLD = """
impl crate::Accessor for PlicAcc {
    unsafe fn at(base: usize) -> Self { PlicAcc(base as *mut PlicRegs) }
    fn set_priority(&mut self, irq: usize, p: usize) { unsafe { &mut *self.0 }.set_priority(irq, p) }
    fn pending(&mut self, lane: usize) -> u32 { unsafe { &*self.0 }.pending(lane) }
    fn enable(&mut self, c: usize, irq: NonZero<usize>, e: bool) { unsafe { &mut *self.0 }.enable(c, irq, e) }
    fn set_priority_threshold(&mut self, c: usize, p: usize) { unsafe { &mut *self.0 }.set_priority_threshold(c, p) }
    fn claim(&mut self, c: usize) -> Option<NonZero<usize>> { unsafe { &mut *self.0 }.claim(c) }
    fn complete(&mut self, c: usize, v: NonZero<usize>) { unsafe { &mut *self.0 }.complete(c, v) }
}
pub struct PlicAcc(*mut PlicRegs);
"""

ACCESSOR_NEW = """
impl crate::Accessor for PlicAcc {
    unsafe fn at(base: usize) -> Self { PlicAcc(unsafe { PlicMmio::new(base) }.unwrap()) }
    fn set_priority(&mut self, irq: usize, p: usize) { self.0.set_priority(irq, p) }
    fn pending(&mut self, lane: usize) -> u32 { self.0.pending(lane) }
    fn enable(&mut self, c: usize, irq: NonZero<usize>, e: bool) { self.0.enable(c, irq, e) }
    fn set_priority_threshold(&mut self, c: usize, p: usize) { self.0.set_priority_threshold(c, p) }
    fn claim(&mut self, c: usize) -> Option<NonZero<usize>> { self.0.claim(c) }
    fn complete(&mut self, c: usize, v: NonZero<usize>) { self.0.complete(c, v) }
}
pub struct PlicAcc(PlicMmio);
"""


def extract_plic(src):
    parts = []
    for line in src.split("\n"):
        if re.match(r"^(const|static) ", line):
            if not line.endswith(";"):
                raise SystemExit(f"multi-line top-level const not supported: {line!r}")
            parts.append(line)
    for h in ["struct InterruptThresholdRegs {", "struct PlicRegs {", "pub struct Plic {", "impl Plic {"]:
        parts.append(need(src, h))
    old = "impl PlicRegs {" in src
    if old:
        parts.append(need(src, "impl PlicRegs {"))
    else:
        parts.append(need(src, "struct PlicMmio {"))
        parts.append(need(src, "impl PlicMmio {"))
    ic = need(src, "impl InterruptController for Plic {")
    methods = [need(ic, f"    fn {m}(", attrs=False) for m in
               ["irq_ack", "irq_eoi", "irq_enable", "irq_disable"]]
    parts.append("impl crate::PlicOps for Plic {\n" + "\n\n".join(methods) + "\n}")
    init = need(ic, "    fn irq_init(", attrs=False)
    assign = [l for l in init.split("\n") if l.strip().startswith("self.regs = ")]
    if len(assign) != 1:
        raise SystemExit(f"irq_init regs assignment count {len(assign)}")
    tail = init[init.index('info!("PLIC: using context {}", self.context);'):init.index("Ok(())")]
    tail = tail.split("\n", 1)[1]
    assign = assign[0].replace("crate::PHYS_OFFSET", "PHYS_OFFSET")
    parts.append(
        "impl crate::Adapter for Plic {\n"
        "    fn fresh() -> Self { Plic::new() }\n"
        "    fn set_fields(&mut self, ndev: usize, context: usize) { self.ndev = ndev; self.context = context; }\n"
        f"    fn install(&mut self, addr: usize) {{\n{assign}\n    }}\n"
        f"    fn init_tail(&mut self) {{\n{tail}    }}\n"
        "}")
    parts.append(ACCESSOR_OLD if old else ACCESSOR_NEW)
    return parts


def main():
    kind, src_path, out_path = sys.argv[1], sys.argv[2], sys.argv[3]
    control = sys.argv[5] if len(sys.argv) > 5 and sys.argv[4] == "--control" else None
    if kind == "io":
        parts = extract_io(src_path)
    else:
        parts = extract_plic(open(src_path).read())
    text = "\n\n".join(parts) + "\n"
    for old, new in CONTROLS.get(control, []) if control else []:
        text = sub1(text, old, new, f"control {control}")
    open(out_path, "w").write(text)
    h = hashlib.sha256(text.encode()).hexdigest()
    print(f"extracted {len(parts)} items from {src_path} -> {out_path} control={control} sha256={h}")


main()
