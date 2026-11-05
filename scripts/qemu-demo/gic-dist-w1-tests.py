#!/usr/bin/env python3
"""Compile the actual GicDistIf enable/disable methods in a W1S/W1C host mock."""
import pathlib
import re
import subprocess
import sys
import tempfile


def extract_method(source: str, name: str) -> str:
    match = re.search(rf"pub unsafe fn {name}\s*\([^)]*\)\s*\{{", source)
    if not match:
        raise ValueError(f"could not find GicDistIf::{name}")
    start = match.start()
    brace = source.index("{", match.start())
    depth = 0
    for i in range(brace, len(source)):
        if source[i] == "{":
            depth += 1
        elif source[i] == "}":
            depth -= 1
            if depth == 0:
                return source[start : i + 1]
    raise ValueError(f"unterminated GicDistIf::{name}")


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} path/to/gic.rs", file=sys.stderr)
        return 2
    source_path = pathlib.Path(sys.argv[1])
    source = source_path.read_text()
    methods = "\n".join(extract_method(source, n) for n in ("irq_enable", "irq_disable"))
    rust = r'''use std::collections::BTreeMap;
const GICD_ISENABLER: u32 = 0x100;
const GICD_ICENABLER: u32 = 0x180;
struct GicDistIf { regs: BTreeMap<u32, u32>, reads: Vec<u32>, writes: Vec<(u32, u32)> }
impl GicDistIf {
    fn read(&mut self, offset: u32) -> u32 { self.reads.push(offset); *self.regs.get(&offset).unwrap_or(&0) }
    fn write(&mut self, offset: u32, value: u32) {
        self.writes.push((offset, value));
        let old = *self.regs.get(&offset).unwrap_or(&0);
        let new = if offset >= GICD_ISENABLER && offset < GICD_ISENABLER + 0x80 {
            old | value // ISENABLER is write-one-to-set
        } else if offset >= GICD_ICENABLER && offset < GICD_ICENABLER + 0x80 {
            old & !value // ICENABLER is write-one-to-clear
        } else { value };
        self.regs.insert(offset, new);
    }
METHODS
}
fn main() {
    // Disabling IRQ 5 must preserve another enabled IRQ in the same word.
    let mut g = GicDistIf { regs: BTreeMap::from([(GICD_ICENABLER, (1 << 5) | (1 << 7))]), reads: vec![], writes: vec![] };
    unsafe { g.irq_disable(5); }
    assert_eq!(g.regs[&GICD_ICENABLER], 1 << 7, "disable cleared an unrelated enabled IRQ");
    assert!(g.reads.is_empty(), "disable read a W1C register");
    assert_eq!(g.writes, vec![(GICD_ICENABLER, 1 << 5)], "disable must write only its target bit");

    // Boundary checks: IRQ 31 and 32 address adjacent words, with no reads.
    let mut g = GicDistIf { regs: BTreeMap::new(), reads: vec![], writes: vec![] };
    unsafe { g.irq_enable(31); g.irq_enable(32); }
    assert_eq!(g.regs[&GICD_ISENABLER], 1 << 31);
    assert_eq!(g.regs[&(GICD_ISENABLER + 4)], 1);
    assert_eq!(g.writes, vec![(GICD_ISENABLER, 1 << 31), (GICD_ISENABLER + 4, 1)]);
    assert!(g.reads.is_empty(), "enable read a W1S register");
    g.regs.insert(GICD_ICENABLER, (1 << 31) | (1 << 30));
    g.regs.insert(GICD_ICENABLER + 4, 1 | (1 << 1));
    unsafe { g.irq_disable(31); g.irq_disable(32); }
    assert_eq!(g.regs[&GICD_ICENABLER], 1 << 30);
    assert_eq!(g.regs[&(GICD_ICENABLER + 4)], 1 << 1);
    assert_eq!(g.writes[2..], vec![(GICD_ICENABLER, 1 << 31), (GICD_ICENABLER + 4, 1)]);
    assert!(g.reads.is_empty(), "disable read a W1C register");
    println!("PASS: extracted irq_enable/irq_disable preserve W1S/W1C semantics; IRQ 31/32 boundary");
}
'''.replace("METHODS", methods)
    with tempfile.TemporaryDirectory(prefix="gic-dist-w1-") as temp:
        src = pathlib.Path(temp) / "harness.rs"
        binary = pathlib.Path(temp) / "harness"
        src.write_text(rust)
        build = subprocess.run(["rustc", "--edition=2021", str(src), "-o", str(binary)], text=True, capture_output=True)
        if build.returncode:
            print(build.stdout, end="")
            print(build.stderr, end="", file=sys.stderr)
            return build.returncode
        run = subprocess.run([str(binary)], text=True, capture_output=True)
        print(run.stdout, end="")
        print(run.stderr, end="", file=sys.stderr)
        return run.returncode


if __name__ == "__main__":
    raise SystemExit(main())
