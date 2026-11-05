#!/usr/bin/env python3
"""Host regression for production IRQ callback EOI ordering.

Extracts the actual InterruptHandler trait and IrqChipCore methods from a kernel
worktree, compiles them with minimal host stubs, then exercises callback/EOI
ordering and descriptor-to-hardware mapping. This does not prove global static
alias safety, Miri soundness, or AP behavior.
"""

from __future__ import annotations

import argparse
import pathlib
import subprocess
import sys
import tempfile


HOOK = "irq_eoi_after_handler"


class NotReady(Exception):
    pass


def item_with_body(source: str, marker: str, *, required: bool = True) -> str | None:
    """Return a Rust item from marker through its balanced body."""
    start = source.find(marker)
    if start < 0:
        if required:
            raise NotReady(f"production source lacks `{marker}`")
        return None

    # Find the item's opening brace, ignoring braces in comments and strings.
    brace = find_open_brace(source, start)
    if brace < 0:
        raise NotReady(f"cannot find body for `{marker}`")
    end = matching_brace(source, brace)
    if end < 0:
        raise NotReady(f"cannot balance body for `{marker}`")
    return source[start : end + 1]


def find_open_brace(source: str, start: int) -> int:
    i = start
    state = "code"
    while i < len(source):
        c = source[i]
        n = source[i + 1] if i + 1 < len(source) else ""
        if state == "line":
            if c == "\n":
                state = "code"
        elif state == "block":
            if c == "*" and n == "/":
                state = "code"
                i += 1
        elif state == "string":
            if c == "\\":
                i += 1
            elif c == '"':
                state = "code"
        elif c == "/" and n == "/":
            state = "line"
            i += 1
        elif c == "/" and n == "*":
            state = "block"
            i += 1
        elif c == '"':
            state = "string"
        elif c == "{":
            return i
        i += 1
    return -1


def matching_brace(source: str, opening: int) -> int:
    depth = 0
    i = opening
    state = "code"
    while i < len(source):
        c = source[i]
        n = source[i + 1] if i + 1 < len(source) else ""
        if state == "line":
            if c == "\n":
                state = "code"
        elif state == "block":
            if c == "*" and n == "/":
                state = "code"
                i += 1
        elif state == "string":
            if c == "\\":
                i += 1
            elif c == '"':
                state = "code"
        elif c == "/" and n == "/":
            state = "line"
            i += 1
        elif c == "/" and n == "*":
            state = "block"
            i += 1
        elif c == '"':
            state = "string"
        elif c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return -1


def extract_trait(source: str) -> str:
    trait = item_with_body(source, "pub trait InterruptHandler")
    assert trait is not None
    if HOOK not in trait:
        raise NotReady(f"production InterruptHandler has no `{HOOK}` hook yet")
    return trait


HARNESS = r'''#![allow(dead_code)]
use std::sync::{Arc, Mutex};

pub struct CleanLockToken;
static IRQ_TRIGGERED: Mutex<Vec<String>> = Mutex::new(Vec::new());
fn irq_trigger(irq: u16, _token: &mut CleanLockToken) {
    IRQ_TRIGGERED.lock().unwrap().push(format!("scheme:{irq}"));
}

__PRODUCTION_TRAIT__

trait TestInterruptController: Send {
    fn irq_eoi(&mut self, hardware_irq: u32);
    fn irq_handler(&mut self, irq: u32, token: &mut CleanLockToken);
}
struct ChipSlot { ic: Box<dyn TestInterruptController> }
struct IrqChipList { chips: Vec<ChipSlot> }
struct Basic { ic_idx: usize, ic_irq: u32, child_ic_idx: Option<usize> }
struct IrqDesc { basic: Basic, handler: Option<Box<dyn InterruptHandler>> }
const IRQ_DESC_COUNT: usize = 2048;
struct IrqChipCore { irq_chip_list: IrqChipList, irq_desc: Vec<IrqDesc> }

impl IrqChipCore {
    __PRODUCTION_IRQ_EOI__
    __PRODUCTION_TRIGGER_VIRQ__
}

#[derive(Default)]
struct Events(Mutex<Vec<String>>);
impl Events {
    fn push(&self, value: String) { self.0.lock().unwrap().push(value); }
    fn take(&self) -> Vec<String> { std::mem::take(&mut *self.0.lock().unwrap()) }
}

struct Controller { events: Arc<Events>, label: &'static str }
impl TestInterruptController for Controller {
    fn irq_eoi(&mut self, hardware_irq: u32) {
        self.events.push(format!("eoi:{}:{hardware_irq}", self.label));
    }
    fn irq_handler(&mut self, irq: u32, _token: &mut CleanLockToken) {
        self.events.push(format!("child:{}:{irq}", self.label));
    }
}
fn core(events: Arc<Events>) -> IrqChipCore {
    let chips = vec![
        ChipSlot { ic: Box::new(Controller { events: events.clone(), label: "wrong" }) },
        ChipSlot { ic: Box::new(Controller { events, label: "selected" }) },
    ];
    let descs = (0..IRQ_DESC_COUNT).map(|_| IrqDesc {
        basic: Basic { ic_idx: 0, ic_irq: 0, child_ic_idx: None }, handler: None,
    }).collect();
    IrqChipCore { irq_chip_list: IrqChipList { chips }, irq_desc: descs }
}
struct Handler { events: Arc<Events>, request_eoi: bool }
impl InterruptHandler for Handler {
    fn irq_handler(&mut self, irq: u32, _token: &mut CleanLockToken) {
        self.events.push(format!("handler:{irq}"));
    }
    fn irq_eoi_after_handler(&self) -> bool { self.request_eoi }
}
struct DefaultHandler(Arc<Events>);
impl InterruptHandler for DefaultHandler {
    fn irq_handler(&mut self, irq: u32, _token: &mut CleanLockToken) {
        self.0.push(format!("handler:{irq}"));
    }
}

#[test]
fn opt_in_eoi_follows_callback_once_and_uses_descriptor_mapping() {
    let events = Arc::new(Events::default());
    let mut core = core(events.clone());
    core.irq_desc[386].basic.ic_idx = 1;
    core.irq_desc[386].basic.ic_irq = 91;
    core.irq_desc[386].handler = Some(Box::new(Handler { events: events.clone(), request_eoi: true }));
    core.trigger_virq(386, &mut CleanLockToken);
    assert_eq!(events.take(), ["handler:386", "eoi:selected:91"]);
}

#[test]
fn production_default_false_hook_does_not_eoi() {
    let events = Arc::new(Events::default());
    let mut core = core(events.clone());
    core.irq_desc[7].basic.ic_irq = 42;
    core.irq_desc[7].handler = Some(Box::new(DefaultHandler(events.clone())));
    core.trigger_virq(7, &mut CleanLockToken);
    assert_eq!(events.take(), ["handler:7"]);
}

#[test]
fn no_handler_fallback_and_child_controller_do_not_gain_handler_eoi() {
    let events = Arc::new(Events::default());
    let mut core = core(events.clone());
    core.irq_desc[8].basic.child_ic_idx = Some(1);
    core.trigger_virq(8, &mut CleanLockToken);
    assert_eq!(events.take(), ["child:selected:8"]);

    core.irq_desc[9].basic.child_ic_idx = None;
    IRQ_TRIGGERED.lock().unwrap().clear();
    core.trigger_virq(9, &mut CleanLockToken);
    assert!(events.take().is_empty());
    assert_eq!(*IRQ_TRIGGERED.lock().unwrap(), ["scheme:9"]);
}

#[test]
fn out_of_range_virq_is_ignored() {
    let events = Arc::new(Events::default());
    let mut core = core(events.clone());
    core.trigger_virq(IRQ_DESC_COUNT as u32, &mut CleanLockToken);
    assert!(events.take().is_empty());
}
'''


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kernel_source", type=pathlib.Path,
                        help="kernel worktree root containing src/dtb/irqchip.rs")
    args = parser.parse_args()
    source = args.kernel_source / "src/dtb/irqchip.rs"
    if not source.is_file():
        parser.error(f"not a kernel source tree: missing {source}")
    text = source.read_text()
    try:
        trait = extract_trait(text)
        eoi = item_with_body(text, "pub fn irq_eoi(&mut self, virq: u32)")
        trigger = item_with_body(text, "pub fn trigger_virq(&mut self, virq: u32")
        assert eoi is not None and trigger is not None
    except NotReady as exc:
        print(f"NOT READY: {exc}", file=sys.stderr)
        return 2

    # The extracted trigger body calls the production `irq_eoi`; test that the
    # opt-in is located after the handler call, rather than accepting a stub.
    if trigger.find("irq_eoi_after_handler") < trigger.find("handler.irq_handler"):
        print("NOT READY: EOI hook appears before handler callback", file=sys.stderr)
        return 2

    generated = (HARNESS.replace("__PRODUCTION_TRAIT__", trait)
                 .replace("__PRODUCTION_IRQ_EOI__", eoi)
                 .replace("__PRODUCTION_TRIGGER_VIRQ__", trigger))
    temp_root = pathlib.Path("$HOME/artifacts/tmp")
    temp_root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="irq-callback-eoi-", dir=temp_root) as temp:
        temp_path = pathlib.Path(temp)
        rust = temp_path / "production_irq_callback.rs"
        binary = temp_path / "production_irq_callback_tests"
        rust.write_text(generated)
        built = subprocess.run(["rustc", "--edition=2024", "--test", str(rust), "-o", str(binary)],
                               text=True, capture_output=True)
        if built.returncode:
            sys.stderr.write(built.stderr)
            return built.returncode
        return subprocess.run([str(binary)], check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
