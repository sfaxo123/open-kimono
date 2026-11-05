use std::cell::RefCell;
thread_local! { static TRACE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) }; }
fn record(event: String) { TRACE.with(|t| t.borrow_mut().push(event)); }
macro_rules! error { ($($arg:tt)*) => { record(format!("log:{}", format!($($arg)*))) }; }
struct CleanLockToken;
struct Handler { panic: bool }
impl Handler {
    fn irq_handler(&mut self, irq: u32, _: &mut CleanLockToken) {
        record(format!("callback:{irq}"));
        if self.panic { panic!("callback panic"); }
    }
}
struct Desc { handler: Option<Handler> }
struct Registry { irq_desc: Vec<Desc> }
#[derive(Debug, PartialEq)]
struct Observation { result: Option<u32>, panicked: bool, trace: Vec<String> }
