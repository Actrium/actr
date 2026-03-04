use actr_framework::{Context, MessageDispatcher, Workload};
use async_trait::async_trait;
use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use std::sync::Arc;

use crate::context::ContextBridge;
use crate::types::RpcEnvelopeBridge;

/// ThreadsafeFunction for dispatch: (ContextBridge, RpcEnvelopeBridge) -> Promise<Buffer>.
type DispatchThreadsafeFunction = ThreadsafeFunction<
    (ContextBridge, RpcEnvelopeBridge),
    Promise<Buffer>,
    FnArgs<(ContextBridge, ObjectRef<false>)>,
>;

pub struct DynamicWorkload {
    on_start_fn: Arc<ThreadsafeFunction<ContextBridge>>,
    on_stop_fn: Arc<ThreadsafeFunction<ContextBridge>>,
    dispatch_fn: Arc<DispatchThreadsafeFunction>,
}

impl DynamicWorkload {
    pub fn new(callback: Object) -> Result<Self> {
        let on_start: Function<'_> = callback.get_named_property("onStart")?;
        let on_stop: Function<'_> = callback.get_named_property("onStop")?;
        let dispatch: Function<'_, Unknown<'_>, Promise<Buffer>> =
            callback.get_named_property("dispatch")?;

        let on_start_fn: ThreadsafeFunction<ContextBridge> = on_start
            .build_threadsafe_function::<ContextBridge>()
            .callee_handled::<true>()
            .build_callback(|ctx| Ok(ctx.value))?;
        let on_stop_fn: ThreadsafeFunction<ContextBridge> = on_stop
            .build_threadsafe_function::<ContextBridge>()
            .callee_handled::<true>()
            .build_callback(|ctx| Ok(ctx.value))?;
        let dispatch_fn: DispatchThreadsafeFunction = dispatch
            .build_threadsafe_function::<(ContextBridge, RpcEnvelopeBridge)>()
            .callee_handled::<true>()
            .build_callback(|ctx| {
                let (ctx_bridge, envelope) = ctx.value;
                let RpcEnvelopeBridge {
                    route_key,
                    payload,
                    request_id,
                } = envelope;

                let mut js_envelope = Object::new(&ctx.env)?;
                js_envelope.set("routeKey", route_key)?;
                js_envelope.set("payload", payload)?;
                js_envelope.set("requestId", request_id)?;

                let raw = unsafe { ToNapiValue::to_napi_value(ctx.env.raw(), js_envelope)? };
                let js_envelope =
                    unsafe { ObjectRef::<false>::from_napi_value(ctx.env.raw(), raw)? };

                Ok(FnArgs::from((ctx_bridge, js_envelope)))
            })?;

        Ok(Self {
            on_start_fn: Arc::new(on_start_fn),
            on_stop_fn: Arc::new(on_stop_fn),
            dispatch_fn: Arc::new(dispatch_fn),
        })
    }
}

#[async_trait]
impl Workload for DynamicWorkload {
    type Dispatcher = DynamicDispatcher;

    async fn on_start<C: Context>(&self, ctx: &C) -> actr_protocol::ActorResult<()> {
        let ctx_bridge = ContextBridge::try_from_context(ctx)?;
        self.on_start_fn
            .call(Ok(ctx_bridge), ThreadsafeFunctionCallMode::Blocking);
        Ok(())
    }

    async fn on_stop<C: Context>(&self, ctx: &C) -> actr_protocol::ActorResult<()> {
        let ctx_bridge = ContextBridge::try_from_context(ctx)?;
        self.on_stop_fn
            .call(Ok(ctx_bridge), ThreadsafeFunctionCallMode::Blocking);
        Ok(())
    }
}

pub struct DynamicDispatcher;

#[async_trait]
impl MessageDispatcher for DynamicDispatcher {
    type Workload = DynamicWorkload;

    async fn dispatch<C: Context>(
        workload: &Self::Workload,
        envelope: actr_protocol::RpcEnvelope,
        ctx: &C,
    ) -> actr_protocol::ActorResult<bytes::Bytes> {
        let ctx_bridge = ContextBridge::try_from_context(ctx)?;
        let envelope_bridge = RpcEnvelopeBridge::from(envelope);

        let promise = workload
            .dispatch_fn
            .call_async(Ok((ctx_bridge, envelope_bridge)))
            .await
            .map_err(|e| actr_protocol::ProtocolError::SerializationError(e.to_string()))?;

        let response = promise
            .await
            .map_err(|e| actr_protocol::ProtocolError::SerializationError(e.to_string()))?;

        Ok(bytes::Bytes::from(response.to_vec()))
    }
}
