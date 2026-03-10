//! WASM actor 端到端集成测试
//!
//! 验证完整调用链：
//! host (WasmHost/WasmInstance) → actr_handle → WasmContext::call_raw()
//!                            → asyncify unwind → drive loop → mock IO
//!                            → asyncify rewind → 返回响应
//!
//! # 测试场景
//!
//! 1. **无 IO 的简单调用**：handler 直接返回响应，不触发 asyncify
//! 2. **触发 asyncify 的 call_raw**：handler 调用 `ctx.call_raw()`，
//!    host mock 返回 payload 的 2 倍值，验证 asyncify 挂起/恢复正确

#![cfg(feature = "wasm-engine")]

include!("wasm_actor_fixture.rs");

use actr_protocol::{prost::Message as ProstMessage, ActrId, RpcEnvelope};
use actr_runtime::{
    wasm::{DispatchContext, IoResult, PendingCall, WasmActorConfig, WasmHost},
};

// ─── 辅助：构造测试用 RpcEnvelope bytes ──────────────────────────────────────

fn make_envelope(route_key: &str, payload: Vec<u8>) -> Vec<u8> {
    let envelope = RpcEnvelope {
        route_key: route_key.to_string(),
        payload: Some(payload.into()),
        ..Default::default()
    };
    envelope.encode_to_vec()
}

// ─── 辅助：构造 DispatchContext ──────────────────────────────────────────────

fn test_ctx() -> DispatchContext {
    DispatchContext {
        self_id: ActrId::default(),
        caller_id: None,
        request_id: "test-req-001".to_string(),
    }
}

// ─── 测试 1：简单路由不存在，返回错误 ────────────────────────────────────────

#[tokio::test]
async fn wasm_actor_unknown_route_returns_error() {
    let host = WasmHost::compile(WASM_ACTOR_FIXTURE).expect("compile failed");
    let mut instance = host.instantiate().expect("instantiate failed");

    let config = WasmActorConfig {
        actr_type: "test:double:0.1.0".to_string(),
        credential_b64: String::new(),
        actor_id_b64: String::new(),
        realm_id: 0,
    };
    instance.init(&config).expect("init failed");

    let req_bytes = make_envelope("unknown/route", vec![1, 0, 0, 0]);

    // 这个路由不存在，应该返回错误（dispatch 返回 Err，actr_handle 返回 HANDLE_FAILED）
    let result = instance
        .dispatch(&req_bytes, test_ctx(), |_pending| async { IoResult::Done })
        .await;

    // 应该失败（WASM 返回 HANDLE_FAILED 错误码）
    assert!(result.is_err(), "未知路由应返回错误");
    tracing::info!("✅ 未知路由返回错误验证通过");
}

// ─── 测试 2：触发 asyncify 的 call_raw 调用 ──────────────────────────────────

#[tokio::test]
async fn wasm_actor_call_raw_triggers_asyncify() {
    let host = WasmHost::compile(WASM_ACTOR_FIXTURE).expect("compile failed");
    let mut instance = host.instantiate().expect("instantiate failed");

    let config = WasmActorConfig {
        actr_type: "test:double:0.1.0".to_string(),
        credential_b64: String::new(),
        actor_id_b64: String::new(),
        realm_id: 0,
    };
    instance.init(&config).expect("init failed");

    // 发送 x = 7（4 字节 little-endian i32）
    let x: i32 = 7;
    let req_bytes = make_envelope("test/double", x.to_le_bytes().to_vec());

    // call_executor：处理 WASM 发出的 call_raw 请求
    // 期望收到 route_key = "test/double_impl"，payload = x 的 bytes
    // 返回 x * 2 的 bytes
    let result = instance
        .dispatch(&req_bytes, test_ctx(), |pending| async move {
            match pending {
                PendingCall::CallRaw { route_key, payload, .. } => {
                    assert_eq!(route_key, "test/double_impl", "route_key 不匹配");
                    assert_eq!(payload.len(), 4, "payload 应为 4 字节");

                    let val = i32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                    assert_eq!(val, 7, "WASM 应传入 x=7");

                    // mock IO：返回 x * 2
                    let doubled = (val * 2).to_le_bytes().to_vec();
                    tracing::info!(val, doubled = val * 2, "mock IO 完成");
                    IoResult::Bytes(doubled)
                }
                other => panic!("期望 PendingCall::CallRaw，得到 {:?}", std::mem::discriminant(&other)),
            }
        })
        .await
        .expect("dispatch 失败");

    // 响应应为 x * 2 = 14
    assert_eq!(result.len(), 4, "响应应为 4 字节");
    let resp_val = i32::from_le_bytes([result[0], result[1], result[2], result[3]]);
    assert_eq!(resp_val, 14, "响应值应为 7 * 2 = 14");

    tracing::info!(resp_val, "✅ WASM actor asyncify call_raw 端到端验证通过");
}

// ─── 测试 3：多次 dispatch，asyncify data buffer 在多次调用间正确重置 ─────────

#[tokio::test]
async fn wasm_actor_multiple_dispatches() {
    let host = WasmHost::compile(WASM_ACTOR_FIXTURE).expect("compile failed");
    let mut instance = host.instantiate().expect("instantiate failed");

    let config = WasmActorConfig {
        actr_type: "test:double:0.1.0".to_string(),
        credential_b64: String::new(),
        actor_id_b64: String::new(),
        realm_id: 0,
    };
    instance.init(&config).expect("init failed");

    for x in [1i32, 5, 42, 100] {
        let req_bytes = make_envelope("test/double", x.to_le_bytes().to_vec());

        let result = instance
            .dispatch(&req_bytes, test_ctx(), |pending| async move {
                match pending {
                    PendingCall::CallRaw { payload, .. } => {
                        let val =
                            i32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                        IoResult::Bytes((val * 2).to_le_bytes().to_vec())
                    }
                    _ => IoResult::Error(-1),
                }
            })
            .await
            .expect("dispatch 失败");

        let resp_val = i32::from_le_bytes([result[0], result[1], result[2], result[3]]);
        assert_eq!(resp_val, x * 2, "dispatch({x}) 应返回 {}", x * 2);
    }

    tracing::info!("✅ 多次 dispatch 验证通过");
}
