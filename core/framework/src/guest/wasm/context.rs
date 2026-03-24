//! WASM guest-side `Context` implementation backed by the compressed ABI.

use crate::guest::abi::{
    self, AbiPayload, AbiReply, HostCallRawV1, HostCallV1, HostDiscoverV1, HostTellV1,
    InvocationContextV1, abi_error_to_actr, dest_to_v1, reply_to_actr_error,
};
use crate::{Context, Dest, MediaSample};
use actr_protocol::{ActorResult, ActrError, ActrId, ActrType, DataStream, PayloadType};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::future::BoxFuture;
use prost::Message as ProstMessage;

use super::imports;

/// WASM guest-side actor execution context.
#[derive(Clone)]
pub struct WasmContext {
    self_id: ActrId,
    caller_id: Option<ActrId>,
    request_id: String,
}

impl WasmContext {
    /// Build a context from invocation data injected by Hyper.
    pub fn from_invocation(ctx: InvocationContextV1) -> Self {
        Self {
            self_id: ctx.self_id,
            caller_id: ctx.caller_id,
            request_id: ctx.request_id,
        }
    }

    fn invoke_frame(&self, frame: abi::AbiFrame) -> Result<AbiReply, ActrError> {
        let frame_bytes = abi::encode_message(&frame).map_err(abi_error_to_actr)?;
        let mut reply_buf = vec![0u8; 64 * 1024];
        let mut reply_len: i32 = 0;

        let code = unsafe {
            imports::actr_host_invoke(
                frame_bytes.as_ptr() as i32,
                frame_bytes.len() as i32,
                reply_buf.as_mut_ptr() as i32,
                reply_buf.len() as i32,
                &mut reply_len as *mut i32 as i32,
            )
        };

        // TODO: Retry with a larger buffer when the host returns BUFFER_TOO_SMALL
        // and writes the required length back through reply_len_out.
        if code != abi::code::SUCCESS {
            return Err(abi_error_to_actr(code));
        }

        if reply_len < 0 {
            return Err(ActrError::Internal(
                "actr_host_invoke returned negative reply length".into(),
            ));
        }

        reply_buf.truncate(reply_len as usize);
        abi::decode_message::<AbiReply>(&reply_buf).map_err(abi_error_to_actr)
    }
}

#[async_trait]
impl Context for WasmContext {
    fn self_id(&self) -> &ActrId {
        &self.self_id
    }

    fn caller_id(&self) -> Option<&ActrId> {
        self.caller_id.as_ref()
    }

    fn request_id(&self) -> &str {
        &self.request_id
    }

    async fn call<R: actr_protocol::RpcRequest>(
        &self,
        target: &Dest,
        request: R,
    ) -> ActorResult<R::Response> {
        let payload = HostCallV1 {
            route_key: R::route_key().to_string(),
            dest: dest_to_v1(target),
            payload: request.encode_to_vec(),
        };
        let frame = payload.into_frame().map_err(abi_error_to_actr)?;
        let reply = self.invoke_frame(frame)?;

        if reply.status != abi::code::SUCCESS {
            return Err(reply_to_actr_error(reply));
        }

        R::Response::decode(reply.payload.as_slice())
            .map_err(|e| ActrError::DecodeFailure(format!("response decode failed: {e}")))
    }

    async fn tell<R: actr_protocol::RpcRequest>(
        &self,
        target: &Dest,
        message: R,
    ) -> ActorResult<()> {
        let payload = HostTellV1 {
            route_key: R::route_key().to_string(),
            dest: dest_to_v1(target),
            payload: message.encode_to_vec(),
        };
        let frame = payload.into_frame().map_err(abi_error_to_actr)?;
        let reply = self.invoke_frame(frame)?;

        if reply.status != abi::code::SUCCESS {
            return Err(reply_to_actr_error(reply));
        }

        Ok(())
    }

    async fn register_stream<F>(&self, _stream_id: String, _callback: F) -> ActorResult<()>
    where
        F: Fn(DataStream, ActrId) -> BoxFuture<'static, ActorResult<()>> + Send + Sync + 'static,
    {
        Err(ActrError::NotImplemented(
            "register_stream is not supported in WASM environment".into(),
        ))
    }

    async fn unregister_stream(&self, _stream_id: &str) -> ActorResult<()> {
        Err(ActrError::NotImplemented(
            "unregister_stream is not supported in WASM environment".into(),
        ))
    }

    async fn send_data_stream(
        &self,
        _target: &Dest,
        _chunk: DataStream,
        _payload_type: PayloadType,
    ) -> ActorResult<()> {
        Err(ActrError::NotImplemented(
            "send_data_stream is not supported in WASM environment".into(),
        ))
    }

    async fn discover_route_candidate(&self, target_type: &ActrType) -> ActorResult<ActrId> {
        let payload = HostDiscoverV1 {
            target_type: target_type.clone(),
        };
        let frame = payload.into_frame().map_err(abi_error_to_actr)?;
        let reply = self.invoke_frame(frame)?;

        if reply.status != abi::code::SUCCESS {
            return Err(reply_to_actr_error(reply));
        }

        ActrId::decode(reply.payload.as_slice())
            .map_err(|e| ActrError::DecodeFailure(format!("discover result decode failed: {e}")))
    }

    async fn call_raw(
        &self,
        target: &ActrId,
        route_key: &str,
        payload: Bytes,
    ) -> ActorResult<Bytes> {
        let request = HostCallRawV1 {
            route_key: route_key.to_string(),
            target: target.clone(),
            payload: payload.to_vec(),
        };
        let frame = request.into_frame().map_err(abi_error_to_actr)?;
        let reply = self.invoke_frame(frame)?;

        if reply.status != abi::code::SUCCESS {
            return Err(reply_to_actr_error(reply));
        }

        Ok(Bytes::from(reply.payload))
    }

    async fn register_media_track<F>(&self, _track_id: String, _callback: F) -> ActorResult<()>
    where
        F: Fn(MediaSample, ActrId) -> BoxFuture<'static, ActorResult<()>> + Send + Sync + 'static,
    {
        Err(ActrError::NotImplemented(
            "WebRTC media tracks are not supported in WASM environment".into(),
        ))
    }

    async fn unregister_media_track(&self, _track_id: &str) -> ActorResult<()> {
        Err(ActrError::NotImplemented(
            "WebRTC media tracks are not supported in WASM environment".into(),
        ))
    }

    async fn send_media_sample(
        &self,
        _target: &Dest,
        _track_id: &str,
        _sample: MediaSample,
    ) -> ActorResult<()> {
        Err(ActrError::NotImplemented(
            "WebRTC media tracks are not supported in WASM environment".into(),
        ))
    }

    async fn add_media_track(
        &self,
        _target: &Dest,
        _track_id: &str,
        _codec: &str,
        _media_type: &str,
    ) -> ActorResult<()> {
        Err(ActrError::NotImplemented(
            "WebRTC media tracks are not supported in WASM environment".into(),
        ))
    }

    async fn remove_media_track(&self, _target: &Dest, _track_id: &str) -> ActorResult<()> {
        Err(ActrError::NotImplemented(
            "WebRTC media tracks are not supported in WASM environment".into(),
        ))
    }
}
