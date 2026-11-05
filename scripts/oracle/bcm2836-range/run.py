#!/usr/bin/env python3
"""BCM2836 actual initializer/mapper range regression with explicit host boundaries."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

HERE=Path(__file__).resolve().parent
REL='src/arch/aarch64/device/irqchip/irq_bcm2836.rs'
BASE_SHA='986fd93246dd4695bbc003b3b1dfa20476578f8407a93f89a2b7daf642cb721b'

def function(s,name):
    a=s.index('fn '+name+'(');o=s.index('{',a);i=o+1;n=1
    while n:
        n+=(s[i]=='{')-(s[i]=='}');i+=1
    return s[a:i]

def main():
    root,out=map(Path,sys.argv[1:]);out.mkdir(parents=True,exist_ok=False)
    old=(HERE/'baseline.rs').read_text();new=(root/REL).read_text()
    assert hashlib.sha256(old.encode()).hexdigest()==BASE_SHA
    # Preserve actual constants, init allocation/return/root-store logic and mapper.
    constants=new[new.index('const LOCAL_IRQ_CNTPNSIRQ'):new.index('#[inline(always)]')]
    template=(HERE/'harness.rs').read_text()
    variants={'baseline':old,'production':new,
              'count_regression':new.replace('LOCAL_IRQ_LAST as usize + 1','LOCAL_IRQ_LAST as usize'),
              'cap_regression':new.replace('idx + i < 1024','idx + i < 1025'),
              'root_store_regression':new.replace('ROOT_IC_IDX_IS_SET.store(1,','ROOT_IC_IDX_IS_SET.store(0,')}
    receipts=[]
    for label,s in variants.items():
        assert label in ['baseline','production'] or s!=new
        init=function(s,'irq_init')
        asm='asm!("mrs {}, mpidr_el1", out(reg) cpuid);'
        assert init.count(asm)==1
        init=init.replace(asm,'cpuid = 0; // host MPIDR boundary')
        fragment=constants+'\nimpl Bcm2836ArmInterruptController {\n'+init+'\n'+function(s,'irq_to_virq')+'\n}'
        path=out/(label+'.rs');path.write_text(template.replace('// EXTRACTED_SOURCE',fragment))
        binary=out/label
        c=subprocess.run(['rustc','--edition=2024','--test',str(path),'-o',str(binary)],capture_output=True,text=True,timeout=60)
        (out/(label+'.compile.log')).write_text(c.stdout+c.stderr);assert c.returncode==0,(label,c.stderr)
        r=subprocess.run([str(binary),'--test-threads=1'],capture_output=True,text=True,timeout=60)
        (out/(label+'.run.log')).write_text(r.stdout+r.stderr)
        assert r.returncode==(0 if label=='production' else 101),(label,r.stdout,r.stderr)
        expected={'baseline':'inclusive_last_and_next_domain','count_regression':'inclusive_last_and_next_domain','cap_regression':'descriptor_cap_preserved','root_store_regression':'inclusive_last_and_next_domain'}
        if label!='production':assert '    '+expected[label]+'\n' in r.stdout
        receipts.append({'variant':label,'compile':c.returncode,'run':r.returncode})
    report={'baseline_sha256':BASE_SHA,'candidate_sha256':hashlib.sha256(new.encode()).hexdigest(),'results':receipts}
    (out/'ACCEPTANCE.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))

if __name__=='__main__':main()
