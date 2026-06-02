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

async fn record_hook<C: Context>(ctx: &C, name: &'static str) {
    let _ = ctx
        .call_raw(ctx.self_id(), "test/record_hook", Bytes::from_static(name.as_bytes()))
        .await;
}

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
                    .call_raw(
                        &target,
                        "test/double_impl",
                        Bytes::from(x.to_le_bytes().to_vec()),
                    )
                    .await?;

                Ok(resp)
            }
            "test/echo" => {
                let payload = envelope.payload.unwrap_or_default();
                Ok(Bytes::from(payload))
            }
            "test/record_hook" => {
                let payload = envelope.payload.unwrap_or_default();
                Ok(Bytes::from(payload))
            }
            _ => Err(ActrError::UnknownRoute(envelope.route_key)),
        }
    }
}

#[async_trait]
impl Workload for DoubleActor {
    type Dispatcher = DoubleDispatcher;

    async fn on_start<C: Context>(&self, ctx: &C) -> ActorResult<()> {
        if ctx.request_id() == "lifecycle:on_start" {
            return Err(ActrError::Internal(
                "fixture lifecycle on_start failed".to_string(),
            ));
        }
        Ok(())
    }

    async fn on_signaling_connecting<C: Context>(&self, ctx: Option<&C>) {
        if let Some(ctx) = ctx {
            record_hook(ctx, "on_signaling_connecting").await;
        }
    }

    async fn on_signaling_connected<C: Context>(&self, ctx: Option<&C>) {
        if let Some(ctx) = ctx {
            record_hook(ctx, "on_signaling_connected").await;
        }
    }

    async fn on_signaling_disconnected<C: Context>(&self, ctx: &C) {
        record_hook(ctx, "on_signaling_disconnected").await;
    }

    async fn on_websocket_connecting<C: Context>(
        &self,
        ctx: &C,
        _event: &actr_framework::PeerEvent,
    ) {
        record_hook(ctx, "on_websocket_connecting").await;
    }

    async fn on_websocket_connected<C: Context>(
        &self,
        ctx: &C,
        _event: &actr_framework::PeerEvent,
    ) {
        record_hook(ctx, "on_websocket_connected").await;
    }

    async fn on_websocket_disconnected<C: Context>(
        &self,
        ctx: &C,
        _event: &actr_framework::PeerEvent,
    ) {
        record_hook(ctx, "on_websocket_disconnected").await;
    }

    async fn on_webrtc_connecting<C: Context>(
        &self,
        ctx: &C,
        _event: &actr_framework::PeerEvent,
    ) {
        record_hook(ctx, "on_webrtc_connecting").await;
    }

    async fn on_webrtc_connected<C: Context>(&self, ctx: &C, _event: &actr_framework::PeerEvent) {
        record_hook(ctx, "on_webrtc_connected").await;
    }

    async fn on_webrtc_disconnected<C: Context>(
        &self,
        ctx: &C,
        _event: &actr_framework::PeerEvent,
    ) {
        record_hook(ctx, "on_webrtc_disconnected").await;
    }

    async fn on_credential_renewed<C: Context>(
        &self,
        ctx: &C,
        _event: &actr_framework::CredentialEvent,
    ) {
        record_hook(ctx, "on_credential_renewed").await;
    }

    async fn on_credential_expiring<C: Context>(
        &self,
        ctx: &C,
        _event: &actr_framework::CredentialEvent,
    ) {
        record_hook(ctx, "on_credential_expiring").await;
    }

    async fn on_mailbox_backpressure<C: Context>(
        &self,
        ctx: &C,
        _event: &actr_framework::BackpressureEvent,
    ) {
        record_hook(ctx, "on_mailbox_backpressure").await;
    }
}

entry!(DoubleActor);
