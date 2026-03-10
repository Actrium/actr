//! WASM actor 端到端测试 fixture
//!
//! 验证目标：一个真实的 wasm32 actor，通过 `actr-runtime-wasm` 实现 `Context` trait，
//! 在 handler 内部调用 `ctx.call_raw()` 触发 asyncify 挂起/恢复，
//! 并将宿主返回的响应作为自身的响应返回。
//!
//! # 测试协议
//!
//! - request：raw bytes，包含 4 字节 little-endian i32 值 `x`
//! - actor 调用 `ctx.call_raw(self_id, "test/double", payload_bytes)`
//! - 宿主 mock 返回 `x * 2`（4 字节 little-endian i32）
//! - actor response：宿主返回的 bytes（即 `x * 2`）
//!
//! # 期望
//!
//! host 调用 `actr_handle(request=5_i32_le)` 最终得到 response `10_i32_le`

use actr_framework::{Context, MessageDispatcher, Workload};
use actr_protocol::{ActorResult, ActrError, RpcEnvelope};
use actr_runtime_wasm::entry;
use async_trait::async_trait;
use bytes::Bytes;
use prost::Message as ProstMessage;

// ── Workload ──────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct DoubleActor;

pub struct DoubleDispatcher;

#[async_trait]
impl MessageDispatcher for DoubleDispatcher {
    type Workload = DoubleActor;

    async fn dispatch<C: Context>(
        _workload: &Self::Workload,
        envelope: RpcEnvelope,
        ctx: &C,
    ) -> ActorResult<Bytes> {
        match envelope.route_key.as_str() {
            "test/double" => {
                // payload：4 字节 little-endian i32（RpcEnvelope.payload 是 optional）
                let payload = envelope.payload.unwrap_or_default();
                if payload.len() < 4 {
                    return Err(ActrError::InvalidArgument("payload 太短".into()));
                }
                let x = i32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);

                // 调用 ctx.call_raw() → 触发 asyncify！
                let target = ctx.self_id().clone();
                let resp = ctx
                    .call_raw(&target, "test/double_impl", Bytes::from(x.to_le_bytes().to_vec()))
                    .await?;

                Ok(resp)
            }
            _ => Err(ActrError::UnknownRoute(envelope.route_key)),
        }
    }
}

impl Workload for DoubleActor {
    type Dispatcher = DoubleDispatcher;
}

// ── ABI 导出（由 entry! 宏生成）──────────────────────────────────────────────

entry!(DoubleActor);
