//! A user-built service: each `run` increments `count` in the service's KV
//! namespace, logs, and emits `service.hello` with `{"count":N}`; it asks for
//! a timer and reports `timer.fired` when called back.

#![no_std]

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

#[link(wasm_import_module = "op6")]
extern "C" {
    fn log(level: i32, ptr: *const u8, len: usize);
    fn trace_emit(name: *const u8, name_len: usize, payload: *const u8, payload_len: usize) -> i32;
    fn timer_after(ms: u32, token: i32) -> i32;
    fn kv_get(key: *const u8, key_len: usize, out: *mut u8, out_cap: usize) -> i32;
    fn kv_set(key: *const u8, key_len: usize, value: *const u8, value_len: usize) -> i32;
}

const INFO: i32 = 2;
const KEY: &[u8] = b"count";

fn trace(name: &[u8], payload: &[u8]) {
    unsafe { trace_emit(name.as_ptr(), name.len(), payload.as_ptr(), payload.len()) };
}

fn load_count() -> u32 {
    let mut buf = [0u8; 4];
    let n = unsafe { kv_get(KEY.as_ptr(), KEY.len(), buf.as_mut_ptr(), buf.len()) };
    if n == 4 {
        u32::from_le_bytes(buf)
    } else {
        0
    }
}

/// Writes `{"count":N}` into `out`, returning the used prefix.
fn count_json(count: u32, out: &mut [u8; 32]) -> &[u8] {
    let mut digits = [0u8; 10];
    let mut i = digits.len();
    let mut v = count;
    loop {
        i -= 1;
        digits[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    let parts: [&[u8]; 3] = [b"{\"count\":", &digits[i..], b"}"];
    let mut len = 0;
    for part in parts {
        out[len..len + part.len()].copy_from_slice(part);
        len += part.len();
    }
    &out[..len]
}

#[no_mangle]
pub extern "C" fn run() {
    let msg = b"hello from rust";
    unsafe { log(INFO, msg.as_ptr(), msg.len()) };
    let count = load_count() + 1;
    let bytes = count.to_le_bytes();
    unsafe { kv_set(KEY.as_ptr(), KEY.len(), bytes.as_ptr(), bytes.len()) };
    let mut json = [0u8; 32];
    trace(b"service.hello", count_json(count, &mut json));
}

#[no_mangle]
pub extern "C" fn schedule() {
    unsafe { timer_after(250, 42) };
}

#[no_mangle]
pub extern "C" fn on_timer(token: i32) {
    if token == 42 {
        trace(b"timer.fired", b"{}");
    }
}
