#!/usr/bin/env python3
"""Compile actual select_mode against a host Os and check keyboard behavior.

RAMBOOT forbids key reads; legacy accepts Enter. No UEFI/framebuffer proof.
Usage: run.py LOADER_ROOT NEW_EVIDENCE_DIRECTORY [legacy]
"""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

root, out = map(Path, sys.argv[1:3])
legacy = len(sys.argv) == 4 and sys.argv[3] == "legacy"
out.mkdir(parents=True, exist_ok=False)
source = (root / 'src/main.rs').read_text()
start = source.index('fn select_mode(')
end = source.index('\nfn redoxfs', start)
function = source[start:end]
stubs = r'''
use std::{cmp, cell::Cell};
#[derive(Clone, Copy, Debug)]
struct OsVideoMode { id: u32, width: u32, height: u32 }
#[allow(dead_code)]
enum OsKey { Left, Right, Up, Down, Enter, Char(char), Other }
trait Os {
    fn video_modes(&self, _: usize) -> Vec<OsVideoMode>;
    fn best_resolution(&self, _: usize) -> Option<(u32,u32)>;
    fn get_key(&self) -> OsKey { panic!("keyboard read forbidden in RAMBOOT") }
    fn get_text_position(&self) -> (usize, usize) { (0,0) }
    fn set_text_position(&self, _: usize, _: usize) {}
    fn set_text_highlight(&self, _: bool) {}
}
struct Fake { modes: Vec<OsVideoMode>, preferred: Option<(u32,u32)>, reads: Cell<usize> }
impl Os for Fake {
 fn video_modes(&self, _: usize) -> Vec<OsVideoMode> { self.modes.clone() }
 fn best_resolution(&self, _: usize) -> Option<(u32,u32)> { self.preferred }
 fn get_key(&self) -> OsKey { self.reads.set(self.reads.get()+1); if cfg!(feature="ramboot") { panic!("keyboard read forbidden in RAMBOOT") } else { OsKey::Enter } }
}
fn mode(id:u32,w:u32,h:u32)->OsVideoMode { OsVideoMode{id,width:w,height:h} }
fn main() {
 for (modes,preferred,want) in [
   (vec![],None,None),
   (vec![mode(7,1080,2280)],None,Some(7)),
   (vec![mode(4,1920,1080),mode(9,800,600)],Some((800,600)),Some(9)),
   (vec![mode(9,800,600),mode(4,1920,1080)],Some((1,1)),Some(4)),
 ] {
  let os=Fake{modes,preferred,reads:Cell::new(0)};
  let(mut live,mut edit)=(false,false);
  assert_eq!(select_mode(&os,0,&mut live,&mut edit).map(|m|m.id),want);
  assert_eq!(os.reads.get(), if cfg!(feature="ramboot") || want.is_none() {0} else {1}); assert!(!live); assert!(!edit);
 }
 println!("PASS four selection cases; feature-appropriate keyboard reads; no edit/live side effects");
}
'''
rs=out/'selection.rs';rs.write_text(stubs+'\n'+function)
binary=out/'selection'
flags = [] if legacy else ['--cfg', 'feature="ramboot"']
build=subprocess.run(['rustc','--edition=2024',*flags,str(rs),'-o',str(binary)],capture_output=True,text=True)
(out/'compile.log').write_text(build.stdout+build.stderr)
run=None
if build.returncode==0:
 run=subprocess.run([str(binary)],capture_output=True,text=True,timeout=5)
 (out/'run.log').write_text(run.stdout+run.stderr)
receipt={'source':str(root/'src/main.rs'),'sha256':hashlib.sha256(source.encode()).hexdigest(),'compile_exit':build.returncode,'run_exit':None if run is None else run.returncode,'mode':'legacy' if legacy else 'ramboot','scope':'actual select_mode; fake Os; no firmware/runtime proof'}
(out/'RESULT.json').write_text(json.dumps(receipt,indent=2)+'\n')
print(json.dumps(receipt));sys.exit(build.returncode or (run.returncode if run else 1))
