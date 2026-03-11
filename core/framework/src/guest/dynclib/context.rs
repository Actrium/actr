//! DynclibContext -- cdylib guest-side Context implementation
//!
//! Implements the `Context` trait via HostVTable function pointers, allowing actor
//! business code in native cdylib to run with the exact same interface as WASM / mailbox.
//!
//! # Design notes
//!
//! - Constructed at each `actr_handle` entry, obtains and caches context data via vtable
//! - `call/tell` communication methods directly call vtable function pointers
//! - WebRTC media-related methods are not supported in dynclib environment, returning `NotImplemented`

use crate::guest::vtable::HostVTable;
use crate::{Context, Dest, MediaSample};
use actr_protocol::{ActorResult, ActrError, ActrId, ActrType, DataStream, PayloadType};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::future::BoxFuture;
use prost::Message as ProstMessage;

/// cdylib guest-side actor execution context
///
/// Constructed on each `actr_handle` call, holds context data for the current invocation.
/// Implements `Context` trait so actor business code need not be aware of the underlying dynclib environment.
#[derive(Clone)]
pub struct DynclibContext {
    vtable: *const HostVTable,
    self_id: ActrId,
    caller_id: Option<ActrId>,
    request_id: String,
}

// Safety: DynclibContext is only used within the same thread (host guarantees no concurrent calls to the same actor instance),
// vtable pointer is valid for the actor's lifetime. Send + Sync are required by the Context trait.
unsafe impl Send for DynclibContext {}
unsafe impl Sync for DynclibContext {}

impl DynclibContext {
    /// Obtain context data from HostVTable, construct DynclibContext
    ///
    /// # Safety
    ///
    /// `vtable` must point to a valid HostVTable and remain valid for the returned DynclibContext's lifetime.
    pub unsafe fn from_vtable(vtable: *const HostVTable) -> Result<Self, ActrError> {
        let vt = unsafe { &*vtable };
        let self_id = fetch_self_id(vt)?;
        let caller_id = fetch_caller_id(vt)?;
        let request_id = fetch_request_id(vt)?;
        Ok(Self {
            vtable,
            self_id,
            caller_id,
            request_id,
        })
    }

    /// Get vtable reference
    fn vt(&self) -> &HostVTable {
        unsafe { &*self.vtable }
    }
}

// -- Context data fetch helper functions ------------------------------------------

fn fetch_self_id(vt: &HostVTable) -> Result<ActrId, ActrError> {
    let mut ptr: *mut u8 = std::ptr::null_mut();
    let mut len: usize = 0;
    let err = unsafe { (vt.self_id)(&mut ptr, &mut len) };
    if err != 0 {
        return Err(ActrError::Internal(format!(
            "actr_host_self_id returned error code {err}"
        )));
    }
    if ptr.is_null() || len == 0 {
        return Err(ActrError::Internal(
            "actr_host_self_id returned empty".into(),
        ));
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let result = ActrId::decode(bytes)
        .map_err(|e| ActrError::Internal(format!("self_id decode failed: {e}")));
    unsafe { (vt.free_host_buf)(ptr, len) };
    result
}

fn fetch_caller_id(vt: &HostVTable) -> Result<Option<ActrId>, ActrError> {
    let mut ptr: *mut u8 = std::ptr::null_mut();
    let mut len: usize = 0;
    let code = unsafe { (vt.caller_id)(&mut ptr, &mut len) };
    if code == 1 {
        // No caller (internal system call, e.g., lifecycle hooks)
        return Ok(None);
    }
    if ptr.is_null() || len == 0 {
        return Err(ActrError::Internal(
            "actr_host_caller_id returned empty buffer".into(),
        ));
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let result = ActrId::decode(bytes)
        .map_err(|e| ActrError::Internal(format!("caller_id decode failed: {e}")));
    unsafe { (vt.free_host_buf)(ptr, len) };
    result.map(Some)
}

fn fetch_request_id(vt: &HostVTable) -> Result<String, ActrError> {
    let mut ptr: *mut u8 = std::ptr::null_mut();
    let mut len: usize = 0;
    let err = unsafe { (vt.request_id)(&mut ptr, &mut len) };
    if err != 0 {
        return Err(ActrError::Internal(format!(
            "actr_host_request_id returned error code {err}"
        )));
    }
    if ptr.is_null() || len == 0 {
        // request_id can be empty
        return Ok(String::new());
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let result = String::from_utf8(bytes.to_vec())
        .map_err(|e| ActrError::Internal(format!("request_id UTF-8 decode failed: {e}")));
    unsafe { (vt.free_host_buf)(ptr, len) };
    result
}

// -- Dest serialization -----------------------------------------------------------

/// Encode Dest as a byte sequence for vtable function pointer use
///
/// Format is identical to WASM guest:
/// - `0x00` = Shell
/// - `0x01` = Local
/// - `0x02` + protobuf ActrId bytes = Actor(id)
fn encode_dest(dest: &Dest) -> Vec<u8> {
    match dest {
        Dest::Shell => vec![0x00],
        Dest::Local => vec![0x01],
        Dest::Actor(id) => {
            let mut buf = vec![0x02];
            buf.extend_from_slice(&id.encode_to_vec());
            buf
        }
    }
}

// -- Context impl -----------------------------------------------------------------

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
        let route_key = R::route_key();
        let payload = request.encode_to_vec();
        let dest_bytes = encode_dest(target);

        let mut resp_ptr: *mut u8 = std::ptr::null_mut();
        let mut resp_len: usize = 0;

        let err = unsafe {
            (self.vt().call)(
                route_key.as_ptr(),
                route_key.len(),
                dest_bytes.as_ptr(),
                dest_bytes.len(),
                payload.as_ptr(),
                payload.len(),
                &mut resp_ptr,
                &mut resp_len,
            )
        };

        if err != 0 {
            return Err(abi_error_to_actr(err));
        }

        if resp_ptr.is_null() {
            return Err(ActrError::Internal("call returned empty response".into()));
        }

        let resp_bytes = unsafe { std::slice::from_raw_parts(resp_ptr, resp_len) };
        let result = R::Response::decode(resp_bytes)
            .map_err(|e| ActrError::DecodeFailure(format!("response decode failed: {e}")));
        unsafe { (self.vt().free_host_buf)(resp_ptr, resp_len) };
        result
    }

    async fn tell<R: actr_protocol::RpcRequest>(
        &self,
        target: &Dest,
        message: R,
    ) -> ActorResult<()> {
        let route_key = R::route_key();
        let payload = message.encode_to_vec();
        let dest_bytes = encode_dest(target);

        let err = unsafe {
            (self.vt().tell)(
                route_key.as_ptr(),
                route_key.len(),
                dest_bytes.as_ptr(),
                dest_bytes.len(),
                payload.as_ptr(),
                payload.len(),
            )
        };

        if err != 0 {
            return Err(abi_error_to_actr(err));
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
        let type_bytes = target_type.encode_to_vec();
        let mut out_ptr: *mut u8 = std::ptr::null_mut();
        let mut out_len: usize = 0;

        let err = unsafe {
            (self.vt().discover)(
                type_bytes.as_ptr(),
                type_bytes.len(),
                &mut out_ptr,
                &mut out_len,
            )
        };

        if err != 0 {
            return Err(abi_error_to_actr(err));
        }

        if out_ptr.is_null() || out_len == 0 {
            return Err(ActrError::NotFound(
                "discover returned no candidate node".into(),
            ));
        }

        let bytes = unsafe { std::slice::from_raw_parts(out_ptr, out_len) };
        let result = ActrId::decode(bytes)
            .map_err(|e| ActrError::DecodeFailure(format!("discover result decode failed: {e}")));
        unsafe { (self.vt().free_host_buf)(out_ptr, out_len) };
        result
    }

    async fn call_raw(
        &self,
        target: &ActrId,
        route_key: &str,
        payload: Bytes,
    ) -> ActorResult<Bytes> {
        // call_raw uses Dest::Actor encoding to pass ActrId as target
        let dest_bytes = encode_dest(&Dest::Actor(target.clone()));

        let mut resp_ptr: *mut u8 = std::ptr::null_mut();
        let mut resp_len: usize = 0;

        let err = unsafe {
            (self.vt().call)(
                route_key.as_ptr(),
                route_key.len(),
                dest_bytes.as_ptr(),
                dest_bytes.len(),
                payload.as_ptr(),
                payload.len(),
                &mut resp_ptr,
                &mut resp_len,
            )
        };

        if err != 0 {
            return Err(abi_error_to_actr(err));
        }

        if resp_ptr.is_null() {
            return Err(ActrError::Internal(
                "call_raw returned empty response".into(),
            ));
        }

        let resp_bytes = unsafe { std::slice::from_raw_parts(resp_ptr, resp_len) };
        let result = Bytes::copy_from_slice(resp_bytes);
        unsafe { (self.vt().free_host_buf)(resp_ptr, resp_len) };
        Ok(result)
    }

    // -- WebRTC media methods (not supported in dynclib environment) ---------------

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

// -- Error code conversion --------------------------------------------------------

/// Convert ABI error code to `ActrError`
fn abi_error_to_actr(code: i32) -> ActrError {
    use crate::guest::abi::code as c;
    match code {
        c::GENERIC_ERROR => ActrError::Internal("host returned generic error".into()),
        c::INIT_FAILED => ActrError::Internal("host initialization failed".into()),
        c::HANDLE_FAILED => ActrError::Internal("host message handling failed".into()),
        c::PROTOCOL_ERROR => ActrError::DecodeFailure("host protocol error".into()),
        n if n < 0 => ActrError::Internal(format!("host unknown error code {n}")),
        _ => ActrError::Internal(format!("host unknown error code {code}")),
    }
}
