use actr_framework::{Bytes, Context, DataStream, Dest};
use actr_protocol::{ActrId, PayloadType};
use actr_runtime::context::RuntimeContext;
use async_trait::async_trait;
use std::any::TypeId;
use std::sync::Arc;

use crate::{ActrError, ActrResult};

/// Callback interface for DataStream events.
#[uniffi::export(callback_interface)]
#[async_trait]
pub trait DataStreamCallback: Send + Sync + 'static {
    /// Handle an incoming DataStream chunk.
    async fn on_stream(
        &self,
        chunk: crate::types::DataStream,
        sender: crate::types::ActrId,
    ) -> ActrResult<()>;
}

/// Context provided to the workload
#[derive(uniffi::Object, Clone)]
pub struct ContextBridge {
    pub(crate) inner: RuntimeContext,
}

impl ContextBridge {
    pub fn new(inner: RuntimeContext) -> Arc<Self> {
        Arc::new(Self { inner })
    }

    /// Try to create a ContextBridge from a generic Context implementation.
    ///
    /// This performs a runtime type check and fails if the context is not a
    /// `RuntimeContext`.
    pub fn try_from_context<C: Context + 'static>(ctx: &C) -> ActrResult<Arc<Self>> {
        if TypeId::of::<C>() != TypeId::of::<RuntimeContext>() {
            return Err(ActrError::InternalError {
                msg: format!(
                    "Context type mismatch: expected RuntimeContext, got {}",
                    std::any::type_name::<C>()
                ),
            });
        }

        let runtime_ctx = unsafe { &*(ctx as *const C as *const RuntimeContext) };
        Ok(Arc::new(Self {
            inner: runtime_ctx.clone(),
        }))
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl ContextBridge {
    /// Call a remote actor via RPC (simplified for FFI)
    ///
    /// # Arguments
    /// - `target`: Target actor ID
    /// - `route_key`: RPC route key (e.g., "echo.EchoService.Echo")
    /// - `payload_type`: Payload transmission type (RpcReliable, RpcSignal, etc.)
    /// - `payload`: Request payload bytes (protobuf encoded)
    /// - `timeout_ms`: Timeout in milliseconds
    ///
    /// # Returns
    /// Response payload bytes (protobuf encoded)
    pub async fn call_raw(
        &self,
        target: crate::types::ActrId,
        route_key: String,
        payload_type: crate::types::PayloadType,
        payload: Vec<u8>,
        timeout_ms: i64,
    ) -> crate::error::ActrResult<Vec<u8>> {
        let target_id: ActrId = target.into();
        let proto_payload_type: PayloadType = payload_type.into();
        let resp = self
            .inner
            .call_raw(
                &Dest::Actor(target_id),
                route_key,
                proto_payload_type,
                Bytes::from(payload),
                timeout_ms,
            )
            .await?;
        Ok(resp.to_vec())
    }

    /// Send a one-way message to an actor (fire-and-forget)
    ///
    /// # Arguments
    /// - `target`: Target actor ID
    /// - `route_key`: RPC route key (e.g., "echo.EchoService.Echo")
    /// - `payload_type`: Payload transmission type (RpcReliable, RpcSignal, etc.)
    /// - `payload`: Message payload bytes (protobuf encoded)
    pub async fn tell_raw(
        &self,
        target: crate::types::ActrId,
        route_key: String,
        payload_type: crate::types::PayloadType,
        payload: Vec<u8>,
    ) -> crate::error::ActrResult<()> {
        let target_id: ActrId = target.into();
        let proto_payload_type: PayloadType = payload_type.into();
        self.inner
            .tell_raw(
                &Dest::Actor(target_id),
                route_key,
                proto_payload_type,
                Bytes::from(payload),
            )
            .await?;
        Ok(())
    }

    /// Send a DataStream to a remote actor (Fast Path)
    ///
    /// # Arguments
    /// - `target`: Target actor ID
    /// - `chunk`: DataStream containing stream_id, sequence, payload, etc.
    pub async fn send_data_stream(
        &self,
        target: crate::types::ActrId,
        chunk: crate::types::DataStream,
    ) -> crate::error::ActrResult<()> {
        let target_id: ActrId = target.into();
        let chunk: DataStream = chunk.into();
        self.inner
            .send_data_stream(&Dest::Actor(target_id), chunk)
            .await?;
        Ok(())
    }

    /// Register a DataStream callback for a stream ID.
    pub async fn register_stream(
        &self,
        stream_id: String,
        callback: Box<dyn DataStreamCallback>,
    ) -> crate::error::ActrResult<()> {
        let callback: Arc<dyn DataStreamCallback> = Arc::from(callback);
        self.inner
            .register_stream(stream_id, move |chunk, sender| {
                let callback = callback.clone();
                Box::pin(async move {
                    let chunk: crate::types::DataStream = chunk.into();
                    let sender: crate::types::ActrId = sender.into();
                    callback
                        .on_stream(chunk, sender)
                        .await
                        .map_err(actr_protocol::ProtocolError::from)
                })
            })
            .await?;
        Ok(())
    }

    /// Unregister a DataStream callback for a stream ID.
    pub async fn unregister_stream(&self, stream_id: String) -> crate::error::ActrResult<()> {
        self.inner.unregister_stream(&stream_id).await?;
        Ok(())
    }

    /// Discover an actor of the specified type
    ///
    /// # Arguments
    /// - `target_type`: Actor type to discover (manufacturer + name)
    ///
    /// # Returns
    /// The ActrId of a discovered actor
    pub async fn discover(
        &self,
        target_type: crate::types::ActrType,
    ) -> crate::error::ActrResult<crate::types::ActrId> {
        let proto_type: actr_protocol::ActrType = target_type.into();
        let id = self.inner.discover_route_candidate(&proto_type).await?;
        Ok(id.into())
    }
}
