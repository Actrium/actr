//! Dynclib actor fixture for e2e tests
//!
//! A simple cdylib actor implementing:
//! - "test/double": reads i32 from payload, calls ctx.call_raw() to get x*2
//! - "test/echo": returns payload as-is (no outbound calls)
//! - unknown route: returns error

use actr_framework::{Context, MessageDispatcher, Workload, entry};
use actr_protocol::{ActorResult, ActrError, RpcEnvelope};
use async_trait::async_trait;
use bytes::Bytes;


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
                let payload = envelope.payload.unwrap_or_default();
                if payload.len() < 4 {
                    return Err(ActrError::InvalidArgument("payload too short".into()));
                }
                let x = i32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);

                // Call ctx.call_raw() -> triggers vtable.call trampoline
                let target = ctx.self_id().clone();
                let resp = ctx
                    .call_raw(&target, "test/double_impl", Bytes::from(x.to_le_bytes().to_vec()))
                    .await?;

                Ok(resp)
            }
            "test/echo" => {
                let payload = envelope.payload.unwrap_or_default();
                Ok(Bytes::from(payload))
            }
            _ => Err(ActrError::UnknownRoute(envelope.route_key)),
        }
    }
}

impl Workload for DoubleActor {
    type Dispatcher = DoubleDispatcher;
}

entry!(DoubleActor);
