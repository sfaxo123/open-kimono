fn patterns() -> Vec<[u32;3]> {
    let mut v = vec![[0,0,0], [u32::MAX,u32::MAX,u32::MAX]];
    for bit in 0..32 { v.push([1<<bit,1,1]); }
    for summary in [8,9] {
        for bank in [0,1,2,1<<15,1<<31,u32::MAX] {
            let mut regs = [1<<summary,0,0]; regs[summary-7] = bank; v.push(regs);
        }
    }
    for basic in [(1<<8)|(1<<10),(1<<9)|(1<<20),(1<<8)|(1<<9),1|(1<<31)] {
        v.push([basic,1<<31,1<<15]);
    }
    v
}
#[test]
fn actual_source_parity() {
    let mut count=0;
    for regs in patterns() { for base in [0,960,1023,1024,usize::MAX] {
        for handler in [0,1,2] { for ack_only in [false,true] {
            assert_eq!(before::run(regs,base,handler,ack_only),after::run(regs,base,handler,ack_only),
                "regs={regs:?} base={base} handler={handler} ack_only={ack_only}");
            count+=1;
        }}
    }}
    println!("PARITY_CASES={count}");
}
#[test]
fn completion_and_quiet_missing_handler() {
    let quiet=after::run([1,0,0],0,0,false);
    assert_eq!(quiet.trace,vec!["read:0","eoi:0"]);
    let call=after::run([1,0,0],0,1,false);
    assert_eq!(call.trace,vec!["read:0","callback:0","eoi:0"]);
    let abort=after::run([1,0,0],0,2,false);
    assert!(abort.panicked);
    assert_eq!(abort.trace,vec!["read:0","callback:0"]);
    let invalid=after::run([1,0,0],1024,1,false);
    assert_eq!(invalid.trace,vec!["read:0","log:unexpected irq num 0","eoi:0"]);
}
#[test]
fn high_bit_and_zero_profile_semantics() {
    for regs in [[0,0,0],[1<<21,0,0],[1<<8,0,0],[1<<9,0,0]] {
        let a=after::run(regs,0,0,true);
        if cfg!(debug_assertions) { assert!(a.panicked); }
        else {
            assert!(!a.panicked);
            let expected=if regs[0]==1<<8 {31} else if regs[0]==1<<9 {63} else {u32::MAX};
            assert_eq!(a.result,Some(expected));
            let logs=a.trace.iter().filter(|e|e.starts_with("log:")).count();
            assert_eq!(logs,if expected==u32::MAX {1} else {0});
        }
    }
}
#[test]
fn owned_step_snapshot() { after::snapshot(); }
