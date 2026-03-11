//! 集成测试：WasmHost + WasmInstance ABI 验证
//!
//! 使用内联 WAT（WebAssembly Text Format）构造最小 guest 模块：
//! - bump allocator（满足 actr_alloc / actr_free 接口）
//! - actr_init：直接返回 SUCCESS
//! - actr_handle：将请求原样 echo 回来（用于验证内存读写正确性）

#![cfg(feature = "wasm-engine")]

use actr_protocol::ActrId;
use actr_hyper::wasm::{DispatchContext, IoResult, WasmActorConfig, WasmHost};

// ─── 最小 guest WAT 模块 ──────────────────────────────────────────────────

/// 返回实现了完整 actr ABI 的最小 WASM binary（echo guest）
///
/// guest 行为：
/// - `actr_alloc`：bump allocator（从 offset 4096 开始）
/// - `actr_free`：no-op
/// - `actr_init`：不做任何事，返回 0（SUCCESS）
/// - `actr_handle`：将请求数据 echo 到新分配的响应缓冲区
fn echo_guest_wasm() -> Vec<u8> {
    wat::parse_str(
        r#"
(module
  ;; 2 页内存（128 KB），对外导出
  (memory (export "memory") 2)

  ;; heap 起始位置（前 4096 字节留给输出区缓冲）
  (global $heap (mut i32) (i32.const 4096))

  ;; 内部 bump allocator
  (func $bump (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $n)))
    (local.get $p))

  ;; actr_alloc(size) -> ptr
  (func (export "actr_alloc") (param $n i32) (result i32)
    (call $bump (local.get $n)))

  ;; actr_free(ptr, size) — no-op（bump allocator 不回收）
  (func (export "actr_free") (param $p i32) (param $n i32))

  ;; asyncify 桩函数（echo guest 不触发 asyncify，但 instantiate() 要求导出）
  (func (export "asyncify_start_unwind") (param i32))
  (func (export "asyncify_stop_unwind"))
  (func (export "asyncify_start_rewind") (param i32))
  (func (export "asyncify_stop_rewind"))

  ;; actr_init(config_ptr, config_len) -> i32
  ;; 此 guest 忽略配置，直接返回 0（SUCCESS）
  (func (export "actr_init") (param $p i32) (param $n i32) (result i32)
    (i32.const 0))

  ;; actr_handle(req_ptr, req_len, resp_ptr_out, resp_len_out) -> i32
  ;; 分配与请求等长的响应缓冲区，复制请求内容（echo），写入输出指针
  (func (export "actr_handle")
    (param $req_ptr i32) (param $req_len i32)
    (param $resp_ptr_out i32) (param $resp_len_out i32)
    (result i32)
    (local $resp_ptr i32)

    ;; 分配响应内存
    (local.set $resp_ptr (call $bump (local.get $req_len)))

    ;; 复制请求到响应（echo）
    (memory.copy
      (local.get $resp_ptr)
      (local.get $req_ptr)
      (local.get $req_len))

    ;; 写入输出区：resp_ptr_out 和 resp_len_out（小端 i32）
    (i32.store (local.get $resp_ptr_out) (local.get $resp_ptr))
    (i32.store (local.get $resp_len_out) (local.get $req_len))

    ;; 返回 SUCCESS
    (i32.const 0))
)
"#,
    )
    .expect("WAT 解析失败")
}

/// 最小合法 `WasmActorConfig`
fn test_config() -> WasmActorConfig {
    WasmActorConfig {
        actr_type: "test-mfr:echo-actor:0.1.0".to_string(),
        credential_b64: "dGVzdA==".to_string(), // base64("test")
        actor_id_b64: "aWQ=".to_string(),       // base64("id")
        realm_id: 1,
    }
}

/// Echo guest 不调用 host import，call_executor 不会被触发
fn noop_ctx() -> DispatchContext {
    DispatchContext {
        self_id: ActrId::default(),
        caller_id: None,
        request_id: "test".to_string(),
    }
}

// ─── 测试用例 ─────────────────────────────────────────────────────────────────

/// 场景 1：正常流程 — 编译、实例化、init、dispatch（echo）
#[tokio::test]
async fn wasm_host_compile_and_echo() {
    let wasm_bytes = echo_guest_wasm();

    let host = WasmHost::compile(&wasm_bytes).expect("编译应成功");
    let mut instance = host.instantiate().expect("实例化应成功");

    instance.init(&test_config()).expect("init 应成功");

    let request = b"hello, wasm!".to_vec();
    let response = instance
        .dispatch(&request, noop_ctx(), |_| async { IoResult::Done })
        .await
        .expect("dispatch 应成功");

    assert_eq!(response, request, "echo guest 应原样返回请求数据");
}

/// 场景 2：多次 dispatch（验证 bump allocator 不会越界）
#[tokio::test]
async fn wasm_host_multiple_dispatches() {
    let wasm_bytes = echo_guest_wasm();
    let host = WasmHost::compile(&wasm_bytes).unwrap();
    let mut instance = host.instantiate().unwrap();
    instance.init(&test_config()).unwrap();

    for i in 0u8..10 {
        let req = vec![i; 64];
        let resp = instance
            .dispatch(&req, noop_ctx(), |_| async { IoResult::Done })
            .await
            .expect("每次 dispatch 应成功");
        assert_eq!(resp, req, "第 {i} 次 dispatch 应正确 echo");
    }
}

/// 场景 3：非法 WASM binary → WasmLoadFailed
#[test]
fn wasm_host_invalid_binary() {
    let bad_bytes = b"not a wasm file";
    let result = WasmHost::compile(bad_bytes);
    assert!(
        result.is_err(),
        "非法 WASM 字节应返回错误，实际: {result:?}"
    );
    let err = result.unwrap_err();
    assert!(
        matches!(err, actr_hyper::wasm::WasmError::LoadFailed(_)),
        "错误类型应为 WasmLoadFailed，实际: {err:?}"
    );
}

/// 场景 4：缺少必要导出函数的 WASM → WasmLoadFailed（实例化阶段）
#[test]
fn wasm_host_missing_exports() {
    // 只有 memory，没有任何 actr_* 函数
    let incomplete_wat = r#"
(module
  (memory (export "memory") 1)
)
"#;
    let wasm_bytes = wat::parse_str(incomplete_wat).unwrap();
    let host = WasmHost::compile(&wasm_bytes).unwrap();
    let result = host.instantiate();

    assert!(
        result.is_err(),
        "缺失导出函数应在实例化时报错，实际: {result:?}"
    );
    let err = result.unwrap_err();
    assert!(
        matches!(err, actr_hyper::wasm::WasmError::LoadFailed(_)),
        "错误类型应为 WasmLoadFailed，实际: {err:?}"
    );
}

/// 场景 5：空请求（0 字节）→ dispatch 应返回空响应
#[tokio::test]
async fn wasm_host_empty_dispatch() {
    let wasm_bytes = echo_guest_wasm();
    let host = WasmHost::compile(&wasm_bytes).unwrap();
    let mut instance = host.instantiate().unwrap();
    instance.init(&test_config()).unwrap();

    let response = instance
        .dispatch(&[], noop_ctx(), |_| async { IoResult::Done })
        .await
        .expect("空请求 dispatch 应成功");
    assert!(response.is_empty(), "空请求应返回空响应");
}

/// 场景 6：较大请求（16KB）→ 验证内存操作正确性
#[tokio::test]
async fn wasm_host_large_dispatch() {
    let wasm_bytes = echo_guest_wasm();
    let host = WasmHost::compile(&wasm_bytes).unwrap();
    let mut instance = host.instantiate().unwrap();
    instance.init(&test_config()).unwrap();

    let large_req: Vec<u8> = (0..16384u16).map(|i| (i % 251) as u8).collect();
    let resp = instance
        .dispatch(&large_req, noop_ctx(), |_| async { IoResult::Done })
        .await
        .expect("大请求 dispatch 应成功");
    assert_eq!(resp, large_req, "大请求 echo 内容应完全一致");
}
