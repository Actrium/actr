//! 最小 asyncify 测试 guest
//!
//! host import: host_get_value(x: i32) -> i32
//! 模拟一个需要"异步 IO"才能得到值的 host 调用。
//!
//! compute(x) 调用 host_get_value(x)，然后将返回值与 x 相加。
//! 验证：compute(5) 当 host 返回 10（即 x*2）时应得 15。

#[link(wasm_import_module = "env")]
extern "C" {
    fn host_get_value(x: i32) -> i32;
}

#[no_mangle]
pub extern "C" fn compute(x: i32) -> i32 {
    let from_host = unsafe { host_get_value(x) };
    x + from_host
}
