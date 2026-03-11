//! WasmContext - WASM guest-side Context implementation
//!
//! Implements the `Context` trait via host imports, allowing actor business code
//! to run with the exact same interface on the wasm32 target as on native.
//!
//! # Design notes
//!
//! - Constructed at `actr_handle` entry point, obtains and caches context data via host imports
//! - `call/tell` communication methods implemented via host imports; asyncify ensures transparency to upper layers
//! - WebRTC media-related methods are not supported in WASM environment, returning `NotImplemented`

use crate::{Context, Dest, MediaSample};
use actr_protocol::{ActorResult, ActrError, ActrId, ActrType, DataStream, PayloadType};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::future::BoxFuture;
use prost::Message as ProstMessage;

use super::imports;

/// WASM guest-side actor execution context
///
/// Constructed on each `actr_handle` call, holds context data for the current invocation.
/// Implements `Context` trait so actor business code need not be aware of the underlying WASM environment.
#[derive(Clone)]
pub struct WasmContext {
    self_id: ActrId,
    caller_id: Option<ActrId>,
    request_id: String,
}

impl WasmContext {
    /// Called at `actr_handle` entry to obtain current call context data via host imports
    pub fn from_host() -> Result<Self, ActrError> {
        let self_id = fetch_self_id()?;
        let caller_id = fetch_caller_id()?;
        let request_id = fetch_request_id()?;
        Ok(Self {
            self_id,
            caller_id,
            request_id,
        })
    }
}

// -- Context data fetch helper functions ------------------------------------------

fn fetch_self_id() -> Result<ActrId, ActrError> {
    let mut buf = vec![0u8; 256];
    let n = unsafe { imports::actr_host_self_id(buf.as_mut_ptr() as i32, buf.len() as i32) };
    if n <= 0 {
        return Err(ActrError::Internal(
            "actr_host_self_id returned empty".into(),
        ));
    }
    ActrId::decode(&buf[..n as usize])
        .map_err(|e| ActrError::Internal(format!("self_id decode failed: {e}")))
}

fn fetch_caller_id() -> Result<Option<ActrId>, ActrError> {
    let mut buf = vec![0u8; 256];
    let n = unsafe { imports::actr_host_caller_id(buf.as_mut_ptr() as i32, buf.len() as i32) };
    if n < 0 {
        // -1 means no caller (internal system call, e.g., lifecycle hooks)
        return Ok(None);
    }
    if n == 0 {
        return Err(ActrError::Internal("actr_host_caller_id returned 0".into()));
    }
    let id = ActrId::decode(&buf[..n as usize])
        .map_err(|e| ActrError::Internal(format!("caller_id decode failed: {e}")))?;
    Ok(Some(id))
}

fn fetch_request_id() -> Result<String, ActrError> {
    let mut buf = vec![0u8; 128];
    let n = unsafe { imports::actr_host_request_id(buf.as_mut_ptr() as i32, buf.len() as i32) };
    if n <= 0 {
        return Ok(String::new());
    }
    String::from_utf8(buf[..n as usize].to_vec())
        .map_err(|e| ActrError::Internal(format!("request_id UTF-8 decode failed: {e}")))
}

// -- Dest serialization -----------------------------------------------------------

/// Encode Dest as a byte sequence for host import use
///
/// Format: `[tag: u8] [ActrId protobuf bytes (Actor variant only)]`
/// - `0x00` = Shell
/// - `0x01` = Local
/// - `0x02` + protobuf ActrId bytes = Actor(id)
pub fn encode_dest(dest: &Dest) -> Vec<u8> {
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
        let route_key = R::route_key();
        let payload = request.encode_to_vec();
        let dest_bytes = encode_dest(target);

        // Pre-allocate response buffer (max 64 KB, sufficient for most RPC responses)
        let mut resp_buf = vec![0u8; 64 * 1024];
        let mut resp_len: i32 = 0;

        let err = unsafe {
            imports::actr_host_call(
                route_key.as_ptr() as i32,
                route_key.len() as i32,
                dest_bytes.as_ptr() as i32,
                dest_bytes.len() as i32,
                payload.as_ptr() as i32,
                payload.len() as i32,
                resp_buf.as_mut_ptr() as i32,
                resp_buf.len() as i32,
                &mut resp_len as *mut i32 as i32,
            )
        };

        if err != 0 {
            return Err(abi_error_to_actr(err));
        }

        resp_buf.truncate(resp_len as usize);
        R::Response::decode(resp_buf.as_slice())
            .map_err(|e| ActrError::DecodeFailure(format!("response decode failed: {e}")))
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
            imports::actr_host_tell(
                route_key.as_ptr() as i32,
                route_key.len() as i32,
                dest_bytes.as_ptr() as i32,
                dest_bytes.len() as i32,
                payload.as_ptr() as i32,
                payload.len() as i32,
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
        // DataStream callback registration is not yet supported in WASM environment
        // Requires additional host-side protocol support, to be implemented in future versions
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
        let type_bytes = target_type.encode_to_vec();
        let mut out_buf = vec![0u8; 256];

        let n = unsafe {
            imports::actr_host_discover(
                type_bytes.as_ptr() as i32,
                type_bytes.len() as i32,
                out_buf.as_mut_ptr() as i32,
                out_buf.len() as i32,
            )
        };

        if n <= 0 {
            return Err(abi_error_to_actr(n));
        }

        ActrId::decode(&out_buf[..n as usize])
            .map_err(|e| ActrError::DecodeFailure(format!("discover result decode failed: {e}")))
    }

    async fn call_raw(
        &self,
        target: &ActrId,
        route_key: &str,
        payload: Bytes,
    ) -> ActorResult<Bytes> {
        let target_bytes = target.encode_to_vec();
        let mut resp_buf = vec![0u8; 64 * 1024];
        let mut resp_len: i32 = 0;

        let err = unsafe {
            imports::actr_host_call_raw(
                route_key.as_ptr() as i32,
                route_key.len() as i32,
                target_bytes.as_ptr() as i32,
                target_bytes.len() as i32,
                payload.as_ptr() as i32,
                payload.len() as i32,
                resp_buf.as_mut_ptr() as i32,
                resp_buf.len() as i32,
                &mut resp_len as *mut i32 as i32,
            )
        };

        if err != 0 {
            return Err(abi_error_to_actr(err));
        }

        resp_buf.truncate(resp_len as usize);
        Ok(Bytes::from(resp_buf))
    }

    // -- WebRTC media methods (not supported in WASM environment) -----------------

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

// -- Error code conversion --------------------------------------------------------

/// Convert ABI error code (from host import return value) to `ActrError`
fn abi_error_to_actr(code: i32) -> ActrError {
    use crate::guest::abi::code;
    match code {
        code::GENERIC_ERROR => ActrError::Internal("WASM host returned generic error".into()),
        code::INIT_FAILED => ActrError::Internal("WASM host initialization failed".into()),
        code::HANDLE_FAILED => ActrError::Internal("WASM host message handling failed".into()),
        code::ALLOC_FAILED => ActrError::Internal("WASM host memory allocation failed".into()),
        code::PROTOCOL_ERROR => ActrError::DecodeFailure("WASM host protocol error".into()),
        // discover: 0 or negative means not found
        n if n <= 0 => ActrError::NotFound(format!("discover found no candidates (code={n})")),
        _ => ActrError::Internal(format!("WASM host unknown error code {code}")),
    }
}
