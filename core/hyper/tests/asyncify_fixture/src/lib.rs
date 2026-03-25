//! Minimal asyncify test guest
//!
//! host import: host_get_value(x: i32) -> i32
//! Simulates a host call that requires "async IO" to obtain a value.
//!
//! compute(x) calls host_get_value(x), then adds the return value to x.
//! Verification: compute(5) should yield 15 when host returns 10 (i.e. x*2).

#[link(wasm_import_module = "env")]
extern "C" {
    fn host_get_value(x: i32) -> i32;
}

#[no_mangle]
pub extern "C" fn compute(x: i32) -> i32 {
    let from_host = unsafe { host_get_value(x) };
    x + from_host
}
