//! Dynclib guest-side `Context` implementation backed by the compressed ABI.

use crate::guest::abi::{
    self, AbiPayload, AbiReply, HostCallRawV1, HostCallV1, HostDiscoverV1, HostTellV1,
    InvocationContextV1, abi_error_to_actr, dest_to_v1, reply_to_actr_error,
};
use crate::guest::vtable::HostVTable;
use crate::{Context, Dest, MediaSample};
use actr_protocol::{ActorResult, ActrError, ActrId, ActrType, DataStream, PayloadType};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::future::BoxFuture;
use prost::Message as ProstMessage;

/// cdylib guest-side actor execution context.
#[derive(Clone)]
pub struct DynclibContext {
    vtable: *const HostVTable,
    self_id: ActrId,
    caller_id: Option<ActrId>,
    request_id: String,
}

unsafe impl Send for DynclibContext {}
unsafe impl Sync for DynclibContext {}

impl DynclibContext {
    /// Construct a context from host-injected invocation data.
    ///
    /// # Safety
    ///
    /// `vtable` must remain valid for the lifetime of the returned context.
    pub unsafe fn from_invocation(
        vtable: *const HostVTable,
        ctx: InvocationContextV1,
    ) -> Result<Self, ActrError> {
        if vtable.is_null() {
            return Err(ActrError::Internal("HostVTable pointer is null".into()));
        }

        Ok(Self {
            vtable,
            self_id: ctx.self_id,
            caller_id: ctx.caller_id,
            request_id: ctx.request_id,
        })
    }

    fn vt(&self) -> &HostVTable {
        unsafe { &*self.vtable }
    }

    fn invoke_frame(&self, frame: abi::AbiFrame) -> Result<AbiReply, ActrError> {
        let frame_bytes = abi::encode_message(&frame).map_err(abi_error_to_actr)?;
        let mut reply_ptr: *mut u8 = std::ptr::null_mut();
        let mut reply_len: usize = 0;

        let code = unsafe {
            (self.vt().invoke)(
                frame_bytes.as_ptr(),
                frame_bytes.len(),
                &mut reply_ptr,
                &mut reply_len,
            )
        };

        if code != abi::code::SUCCESS {
            return Err(abi_error_to_actr(code));
        }

        let bytes = if reply_ptr.is_null() || reply_len == 0 {
            Vec::new()
        } else {
            let data = unsafe { std::slice::from_raw_parts(reply_ptr, reply_len).to_vec() };
            unsafe { (self.vt().free_host_buf)(reply_ptr, reply_len) };
            data
        };

        abi::decode_message::<AbiReply>(&bytes).map_err(abi_error_to_actr)
    }
}

#[async_trait]
impl Context for DynclibContext {
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
        let frame = payload.to_frame().map_err(abi_error_to_actr)?;
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
        let frame = payload.to_frame().map_err(abi_error_to_actr)?;
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
            "register_stream is not supported in dynclib environment".into(),
        ))
    }

    async fn unregister_stream(&self, _stream_id: &str) -> ActorResult<()> {
        Err(ActrError::NotImplemented(
            "unregister_stream is not supported in dynclib environment".into(),
        ))
    }

    async fn send_data_stream(
        &self,
        _target: &Dest,
        _chunk: DataStream,
        _payload_type: PayloadType,
    ) -> ActorResult<()> {
        Err(ActrError::NotImplemented(
            "send_data_stream is not supported in dynclib environment".into(),
        ))
    }

    async fn discover_route_candidate(&self, target_type: &ActrType) -> ActorResult<ActrId> {
        let payload = HostDiscoverV1 {
            target_type: target_type.clone(),
        };
        let frame = payload.to_frame().map_err(abi_error_to_actr)?;
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
        let frame = request.to_frame().map_err(abi_error_to_actr)?;
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
            "WebRTC media tracks are not supported in dynclib environment".into(),
        ))
    }

    async fn unregister_media_track(&self, _track_id: &str) -> ActorResult<()> {
        Err(ActrError::NotImplemented(
            "WebRTC media tracks are not supported in dynclib environment".into(),
        ))
    }

    async fn send_media_sample(
        &self,
        _target: &Dest,
        _track_id: &str,
        _sample: MediaSample,
    ) -> ActorResult<()> {
        Err(ActrError::NotImplemented(
            "WebRTC media tracks are not supported in dynclib environment".into(),
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
            "WebRTC media tracks are not supported in dynclib environment".into(),
        ))
    }

    async fn remove_media_track(&self, _target: &Dest, _track_id: &str) -> ActorResult<()> {
        Err(ActrError::NotImplemented(
            "WebRTC media tracks are not supported in dynclib environment".into(),
        ))
    }
}
