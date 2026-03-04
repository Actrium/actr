use actr_framework::Context;
use actr_runtime::context::RuntimeContext;
use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi_derive::napi;
use std::sync::Arc;

use crate::types::{ActrId, ActrType, DataStream, PayloadType, StreamSignal};

#[napi]
pub struct ContextBridge {
    inner: RuntimeContext,
}

impl ContextBridge {
    pub fn try_from_context<C: actr_framework::Context + 'static>(
        ctx: &C,
    ) -> actr_protocol::ActorResult<Self> {
        use std::any::TypeId;

        if TypeId::of::<C>() != TypeId::of::<actr_runtime::context::RuntimeContext>() {
            return Err(actr_protocol::ProtocolError::InvalidStateTransition(
                format!(
                    "Context type mismatch: expected RuntimeContext, got {}",
                    std::any::type_name::<C>()
                ),
            ));
        }

        let runtime_ctx =
            unsafe { &*(ctx as *const C as *const actr_runtime::context::RuntimeContext) };

        Ok(Self {
            inner: runtime_ctx.clone(),
        })
    }
}

#[napi]
impl ContextBridge {
    /// Call remote actor.
    #[napi]
    pub async fn call_raw(
        &self,
        target: ActrId,
        route_key: String,
        payload_type: PayloadType,
        payload: Buffer,
        timeout_ms: i64,
    ) -> Result<Buffer> {
        let target_id: actr_protocol::ActrId = target.into();
        let proto_payload_type: actr_protocol::PayloadType = payload_type.into();

        let response = self
            .inner
            .call_raw(
                &actr_framework::Dest::Actor(target_id),
                route_key,
                proto_payload_type,
                bytes::Bytes::from(payload.to_vec()),
                timeout_ms,
            )
            .await
            .map_err(crate::error::protocol_error_to_napi)?;

        Ok(response.to_vec().into())
    }

    /// Send one-way message.
    #[napi]
    pub async fn tell_raw(
        &self,
        target: ActrId,
        route_key: String,
        payload_type: PayloadType,
        payload: Buffer,
    ) -> Result<()> {
        let target_id: actr_protocol::ActrId = target.into();
        let proto_payload_type: actr_protocol::PayloadType = payload_type.into();

        self.inner
            .tell_raw(
                &actr_framework::Dest::Actor(target_id),
                route_key,
                proto_payload_type,
                bytes::Bytes::from(payload.to_vec()),
            )
            .await
            .map_err(crate::error::protocol_error_to_napi)?;

        Ok(())
    }

    /// Discover an actor of the given type.
    #[napi]
    pub async fn discover(&self, target_type: ActrType) -> Result<ActrId> {
        let proto_type: actr_protocol::ActrType = target_type.into();
        let id = Context::discover_route_candidate(&self.inner, &proto_type)
            .await
            .map_err(crate::error::protocol_error_to_napi)?;

        Ok(id.into())
    }

    /// Send DataStream chunk to target.
    #[napi]
    pub async fn send_data_stream(&self, target: ActrId, chunk: DataStream) -> Result<()> {
        let target_id: actr_protocol::ActrId = target.into();
        let chunk: actr_protocol::DataStream = chunk.into();

        Context::send_data_stream(&self.inner, &actr_framework::Dest::Actor(target_id), chunk)
            .await
            .map_err(crate::error::protocol_error_to_napi)?;

        Ok(())
    }

    /// Register a callback for an incoming DataStream.
    #[napi]
    pub async fn register_stream(
        &self,
        stream_id: String,
        #[napi(ts_arg_type = "(err: Error | null, signal: StreamSignal) => void")]
        callback: ThreadsafeFunction<StreamSignal>,
    ) -> Result<()> {
        use actr_framework::Context;
        let callback = Arc::new(callback);
        self.inner
            .register_stream(stream_id, move |chunk, sender| {
                let callback = callback.clone();
                Box::pin(async move {
                    let chunk_bridge: DataStream = chunk.into();
                    let sender_bridge: ActrId = sender.into();
                    callback.call(
                        Ok(StreamSignal {
                            chunk: chunk_bridge,
                            sender: sender_bridge,
                        }),
                        ThreadsafeFunctionCallMode::NonBlocking,
                    );
                    Ok(())
                })
            })
            .await
            .map_err(crate::error::protocol_error_to_napi)?;
        Ok(())
    }

    /// Unregister a DataStream callback.
    #[napi]
    pub async fn unregister_stream(&self, stream_id: String) -> Result<()> {
        use actr_framework::Context;
        self.inner
            .unregister_stream(&stream_id)
            .await
            .map_err(crate::error::protocol_error_to_napi)?;
        Ok(())
    }

    /// Get the current RPC call ID from the runtime context.
    #[napi]
    pub fn call_id(&self) -> Option<ActrId> {
        self.inner.caller_id().cloned().map(Into::into)
    }
}
