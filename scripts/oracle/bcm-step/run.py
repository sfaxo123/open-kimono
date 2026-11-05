#!/usr/bin/env python3
"""Actual-source BCM ack/handler parity; only hardware/log/registry/EOI boundaries adapted."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
REL = 'src/arch/aarch64/device/irqchip/irq_bcm2835.rs'
BASE_SHA = '0a932eb5677c5a04b5fbdbd12c6ab2dcaf90ad991d11f574f189237b23afe823'

def function(source, name):
    start = source.index('fn ' + name + '(')
    opening = source.index('{', start)
    depth = 1
    end = opening + 1
    while depth:
        depth += (source[end] == '{') - (source[end] == '}')
        end += 1
    return source[start:end]

def module(source, name, candidate):
    pieces = [f'mod {name} {{ use super::*;', function(source, 'ffs'),
              'const PENDING_0:u32=0; const PENDING_1:u32=4; const PENDING_2:u32=8;']
    if candidate:
        pieces.append(source[source.index('// Owned values only;'):source.index('impl Bcm2835ArmInterruptController {')])
    pieces.append('''struct Bcm2835ArmInterruptController { irq_range:(usize,usize), regs:[u32;3] }
impl Bcm2835ArmInterruptController {
unsafe fn read(&self, offset:u32)->u32 {
record(format!("read:{offset}")); self.regs[(offset/4) as usize]
}
''')
    names = ['irq_ack', 'irq_to_virq'] + (['pending_step', 'claim_step'] if candidate else [])
    pieces.extend(function(source, n) for n in names)
    # The production EOI is empty; record its invocation at this boundary.
    assert function(source, 'irq_eoi') == 'fn irq_eoi(&mut self, _irq_num: u32) {}'
    pieces.append('fn irq_eoi(&mut self, irq:u32) { record(format!("eoi:{irq}")); }')
    handler = function(source, 'irq_handler')
    handler = handler.replace('token: &mut CleanLockToken)', 'token: &mut CleanLockToken, registry: &mut Registry)')
    assert handler.count('IRQ_CHIP.irq_desc') == 1
    pieces.append(handler.replace('IRQ_CHIP.irq_desc', 'registry.irq_desc'))
    pieces.append('}')
    pieces.append('''pub fn run(regs:[u32;3],base:usize,handler:u8,ack_only:bool)->Observation {
TRACE.with(|t|t.borrow_mut().clear());
let mut controller=Bcm2835ArmInterruptController {irq_range:(base,0),regs};
let mut registry=Registry {irq_desc:(0..1024).map(|_|Desc {handler:if handler==0 {None} else {Some(Handler {panic:handler==2})}}).collect()};
let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
if ack_only {Some(controller.irq_ack())} else {controller.irq_handler(0,&mut CleanLockToken,&mut registry);None}
}));
Observation {panicked:result.is_err(),result:result.unwrap_or(None),trace:TRACE.with(|t|t.borrow().clone())}
}''')
    if candidate:
        pieces.append('''pub fn snapshot() {
let mut c=Bcm2835ArmInterruptController {irq_range:(10,0),regs:[1,0,0]};
let step=c.claim_step(); c.irq_range=(99,0); c.regs=[2,0,0];
assert_eq!(step.virq,Some(10)); assert_eq!(step.pending.hwirq,0);
assert!(step.pending.diagnostic.is_none());
}''')
    pieces.append('}')
    return '\n'.join(pieces)

def main():
    root, out = map(Path, sys.argv[1:])
    out.mkdir(parents=True, exist_ok=False)
    before = (HERE/'baseline.rs').read_text()
    after = (root/REL).read_text()
    assert hashlib.sha256(before.encode()).hexdigest() == BASE_SHA
    prefix = '#![allow(dead_code,unused_unsafe)]\n'+(HERE/'harness.rs').read_text()
    tests = (HERE/'tests.rs').read_text()
    variants = {'production': after,
        'wrong_fast_map': after.replace('7 + 32,', '8 + 32,', 1),
        'wrong_pending_read': after.replace('self.read(PENDING_1)', 'self.read(PENDING_2)', 1),
        'quiet_handler_log': after.replace('if let Some(virq) = step.virq {', 'if step.virq.is_some() { error!("extra log"); }\nif let Some(virq) = step.virq {', 1),
        'missing_eoi': after.replace('self.irq_eoi(irq);', 'if step.virq.is_some() { self.irq_eoi(irq); }', 1)}
    receipts = []
    for label, source in variants.items():
        assert label == 'production' or source != after
        generated = prefix+'\n'+module(before,'before',False)+'\n'+module(source,'after',True)+'\n'+tests
        path = out/(label+'.rs'); path.write_text(generated)
        for profile in ['debug','release']:
            binary=out/(label+'-'+profile)
            args=['rustc','--edition=2024','--test',str(path),'-o',str(binary)]
            if profile == 'release': args += ['-O','-C','debug-assertions=no','-C','overflow-checks=no']
            compiled=subprocess.run(args,capture_output=True,text=True,timeout=60)
            (out/(label+'-'+profile+'.compile.log')).write_text(compiled.stdout+compiled.stderr)
            assert compiled.returncode == 0, (label,profile,'compile failure')
            ran=subprocess.run([str(binary),'--nocapture','--test-threads=1'],capture_output=True,text=True,timeout=60)
            log=ran.stdout+ran.stderr
            (out/(label+'-'+profile+'.run.log')).write_text(log)
            expected=0 if label=='production' else 101
            assert ran.returncode == expected,(label,profile,ran.returncode)
            if label!='production':
                assert '    actual_source_parity\n' in ran.stdout, 'control must fail named parity assertion'
            receipts.append(dict(variant=label,profile=profile,compile=compiled.returncode,run=ran.returncode))
    (out/'ACCEPTANCE.json').write_text(json.dumps(dict(baseline_sha=BASE_SHA,candidate_sha=hashlib.sha256(after.encode()).hexdigest(),results=receipts),indent=2)+'\n')
    print(json.dumps(receipts))

if __name__ == '__main__':
    main()
