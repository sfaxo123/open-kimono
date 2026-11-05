#!/usr/bin/env python3
"""Compile real loader paging source and walk tables with a host allocator.

Usage: run.py LOADER_ROOT NEW_EVIDENCE_DIRECTORY
No UEFI, MMU execution, cache, CPU, VM or phone proof.
"""
from pathlib import Path
import hashlib
import json
import subprocess
import sys

root, out = map(Path, sys.argv[1:])
out.mkdir(parents=True, exist_ok=False)
source = root / 'src/arch/aarch64.rs'
# Preserve the whole source module; only its Os/allocator boundary is hosted.
stubs = r'''
mod os {
 #[derive(Clone,Copy,Debug,PartialEq,Eq)]
 pub enum OsMemoryAttribute { Device, NormalNonCacheable, NormalWriteThrough, NormalWriteBack }
 #[derive(Clone,Copy)]
 pub struct OsMemoryAttributeRange { pub base:u64,pub size:u64,pub attribute:OsMemoryAttribute }
 pub enum OsMemoryKind { Reclaim }
 pub struct OsMemoryEntry { pub base:u64,pub size:u64,pub kind:OsMemoryKind }
 pub trait Os {
  fn alloc_zeroed_page_aligned(&self,size:usize)->*mut u8;
  fn memory_attribute_ranges(&self)->Vec<OsMemoryAttributeRange>;
 }
}
fn area_add(_:os::OsMemoryEntry) {}
struct Host;
impl os::Os for Host {
 fn alloc_zeroed_page_aligned(&self,size:usize)->*mut u8 {
  unsafe { std::alloc::alloc_zeroed(std::alloc::Layout::from_size_align(size,4096).unwrap()) }
 }
 fn memory_attribute_ranges(&self)->Vec<os::OsMemoryAttributeRange> {
  vec![os::OsMemoryAttributeRange{base:0x8000_0000,size:0x2_0000_0000,attribute:os::OsMemoryAttribute::NormalWriteBack}]
 }
}
fn translate(root:usize,addr:u64)->Option<u64> {
 let mut table=root as *const u64;
 for (level,shift) in [39,30,21,12].into_iter().enumerate() {
  let entry=unsafe{*table.add(((addr>>shift)&511) as usize)};
  if entry&1==0 { return None; }
  if level==3 || entry&2==0 {
   let mask=(1u64<<shift)-1;
   return Some((entry&paging::ENTRY_ADDRESS_MASK&!mask)|(addr&mask));
  }
  table=(entry&paging::ENTRY_ADDRESS_MASK) as *const u64;
 }
 None
}
fn main() {
 let os=Host;
 let root=unsafe{paging::paging_create(&os,0x9000_0000,4096).unwrap()};
 // Existing low framebuffer and former QEMU4G range remain mapped.
 for address in [0x9d40_0000,0x17fe_61000] {
  assert_eq!(translate(root,address),Some(address),"low map regression");
 }
 // Representative observed physical high U-Boot, DTB and stack/EFI region.
 for address in [0x27c6_44000,0x27ac_d2000,0x27ff_f0000] {
  assert_eq!(translate(root,address),Some(address),"high bootstrap address unmapped");
  assert_eq!(translate(root,address+paging::PHYS_OFFSET),Some(address),"high physmap alias unmapped");
 }
 // Reusing an already-mapped high framebuffer must not overwrite L1 entries.
 let high=0x27c0_00000;
 assert_eq!(unsafe{paging::paging_framebuffer(&os,root,high,4096)},Some(high+paging::PHYS_OFFSET));
 assert_eq!(translate(root,high+paging::PHYS_OFFSET),Some(high));
 assert_eq!(translate(root,0x4_0000_0000),None,"unexpected map beyond16GiB");
 println!("PASS low/high/alias mappings, high framebuffer reuse and16GiB upper boundary");
}
'''
rs = out / 'harness.rs'
rs.write_text(stubs+'\nmod paging {\n'+source.read_text()+'\n}\n')
binary=out/'harness'
build=subprocess.run(['rustc','--edition=2024',str(rs),'-o',str(binary)],capture_output=True,text=True)
(out/'compile.log').write_text(build.stdout+build.stderr)
run=None
if build.returncode==0:
 run=subprocess.run([str(binary)],capture_output=True,text=True,timeout=10)
 (out/'run.log').write_text(run.stdout+run.stderr)
r={'source':str(source),'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'compile_exit':build.returncode,'run_exit':None if run is None else run.returncode,'scope':'whole actual paging module with host Os/allocation boundaries; table walk only'}
(out/'RESULT.json').write_text(json.dumps(r,indent=2)+'\n')
print(json.dumps(r));sys.exit(build.returncode or (run.returncode if run else 1))
