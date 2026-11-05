#![allow(dead_code,unused_unsafe)]
use std::sync::atomic::{AtomicUsize,Ordering};
static ROOT_IC_IDX: AtomicUsize=AtomicUsize::new(0);
static ROOT_IC_IDX_IS_SET: AtomicUsize=AtomicUsize::new(0);
const PHYS_OFFSET:usize=0;
const EINVAL:usize=22;
#[derive(Debug,PartialEq)] struct Error(usize);
impl Error {fn new(code:usize)->Self {Self(code)}}
type Result<T>=std::result::Result<T,Error>;
struct Fdt {valid:bool}
#[derive(Clone,Copy)] struct Basic {ic_idx:usize,ic_irq:u32,used:bool}
#[derive(Clone,Copy)] struct IrqDesc {basic:Basic}
struct Bcm2836ArmInterruptController {address:usize,active_cpu:u32,irq_range:(usize,usize)}
macro_rules! info {($($arg:tt)*)=>{ let _=format!($($arg)*); };}
fn descriptors()->[IrqDesc;2048] {
[IrqDesc{basic:Basic{ic_idx:usize::MAX,ic_irq:u32::MAX,used:false}};2048]
}
impl Bcm2836ArmInterruptController {
fn parse(f:&Fdt)->Result<(usize,usize)> {if f.valid {Ok((4096,256))} else {Err(Error(EINVAL))}}
unsafe fn init(&mut self) {}
}
// EXTRACTED_SOURCE
#[test]
fn inclusive_last_and_next_domain() {
 let mut c=Bcm2836ArmInterruptController{address:0,active_cpu:0,irq_range:(0,0)};
 let mut d=descriptors();let mut idx=100;
 c.irq_init(Some(&Fdt{valid:true}),&mut d,7,&mut idx).unwrap();
 assert_eq!(c.irq_range,(100,110)); assert_eq!(idx,110);
 for i in 0..=9 {assert_eq!((d[100+i].basic.ic_idx,d[100+i].basic.ic_irq,d[100+i].basic.used),(7,i as u32,true));}
 assert!(!d[110].basic.used);
 assert_eq!(c.irq_to_virq(9),Some(109));assert_eq!(c.irq_to_virq(10),None);
 // Next chip begins after the inclusive last slot, with no overlap.
 let mut next=Bcm2836ArmInterruptController{address:0,active_cpu:0,irq_range:(0,0)};
 next.irq_init(Some(&Fdt{valid:true}),&mut d,8,&mut idx).unwrap();
 assert_eq!(next.irq_range,(110,120)); assert_eq!(d[109].basic.ic_idx,7);assert_eq!(d[110].basic.ic_idx,8);
 assert_eq!(ROOT_IC_IDX.load(Ordering::Relaxed),8);assert_eq!(ROOT_IC_IDX_IS_SET.load(Ordering::Relaxed),1);
}
#[test]
fn descriptor_cap_preserved() {
 let mut c=Bcm2836ArmInterruptController{address:0,active_cpu:0,irq_range:(0,0)};
 let mut d=descriptors();let mut idx=1019;
 c.irq_init(Some(&Fdt{valid:true}),&mut d,4,&mut idx).unwrap();
 for i in 1019..1024 {assert!(d[i].basic.used);assert_eq!(d[i].basic.ic_irq,(i-1019) as u32);}
 assert!(!d[1024].basic.used); assert_eq!(idx,1029);assert_eq!(c.irq_range,(1019,1029));
}
#[test]
fn parse_error_preserves_state() {
 ROOT_IC_IDX.store(55,Ordering::Relaxed);ROOT_IC_IDX_IS_SET.store(66,Ordering::Relaxed);
 let mut c=Bcm2836ArmInterruptController{address:3,active_cpu:2,irq_range:(5,6)};
 let mut d=descriptors();let mut idx=100;
 assert_eq!(c.irq_init(Some(&Fdt{valid:false}),&mut d,7,&mut idx),Err(Error(EINVAL)));
 assert_eq!((c.address,c.active_cpu,c.irq_range,idx),(3,2,(5,6),100));
 assert!(!d.iter().any(|x|x.basic.used));
 assert_eq!(ROOT_IC_IDX.load(Ordering::Relaxed),55);assert_eq!(ROOT_IC_IDX_IS_SET.load(Ordering::Relaxed),66);
}
