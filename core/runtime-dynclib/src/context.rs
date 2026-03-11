//! DynclibContext — cdylib guest 侧 Context 实现
//!
//! 通过 HostVTable 函数指针实现 `Context` trait，使 actor 业务代码在
//! native cdylib 中以与 WASM / mailbox 完全相同的接口运行。
//!
//! # 设计要点
//!
//! - 在每次 `actr_handle` 入口处构建，通过 vtable 获取并缓存上下文数据
//! - `call/tell` 等通信方法直接调用 vtable 函数指针
//! - WebRTC 媒体相关方法在 dynclib 环境下不支持，返回 `NotImplemented`

use actr_framework::{Context, Dest, MediaSample};
use actr_protocol::{ActorResult, ActrError, ActrId, ActrType, DataStream, PayloadType};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::future::BoxFuture;
use prost::Message as ProstMessage;

use crate::vtable::HostVTable;

/// cdylib guest 侧 actor 执行上下文
///
/// 在每次 `actr_handle` 调用时构建，持有本次调用的上下文数据。
/// 实现 `Context` trait，actor 业务代码无需感知底层 dynclib 环境。
#[derive(Clone)]
pub struct DynclibContext {
    vtable: *const HostVTable,
    self_id: ActrId,
    caller_id: Option<ActrId>,
    request_id: String,
}

// Safety: DynclibContext 仅在同一线程内使用（宿主保证同一 actor 实例不会被并发调用），
// vtable 指针在 actor 生命周期内有效。Send + Sync 是 Context trait 的约束。
unsafe impl Send for DynclibContext {}
unsafe impl Sync for DynclibContext {}

impl DynclibContext {
    /// 从 HostVTable 获取上下文数据，构建 DynclibContext
    ///
    /// # Safety
    ///
    /// `vtable` 必须指向有效的 HostVTable，且在返回的 DynclibContext 生命周期内有效。
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

    /// 获取 vtable 引用
    fn vt(&self) -> &HostVTable {
        unsafe { &*self.vtable }
    }
}

// ── 上下文数据获取辅助函数 ────────────────────────────────────────────────────

fn fetch_self_id(vt: &HostVTable) -> Result<ActrId, ActrError> {
    let mut ptr: *mut u8 = std::ptr::null_mut();
    let mut len: usize = 0;
    let err = unsafe { (vt.self_id)(&mut ptr, &mut len) };
    if err != 0 {
        return Err(ActrError::Internal(format!(
            "actr_host_self_id 返回错误码 {err}"
        )));
    }
    if ptr.is_null() || len == 0 {
        return Err(ActrError::Internal("actr_host_self_id 返回空".into()));
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let result = ActrId::decode(bytes)
        .map_err(|e| ActrError::Internal(format!("self_id decode 失败: {e}")));
    unsafe { (vt.free_host_buf)(ptr, len) };
    result
}

fn fetch_caller_id(vt: &HostVTable) -> Result<Option<ActrId>, ActrError> {
    let mut ptr: *mut u8 = std::ptr::null_mut();
    let mut len: usize = 0;
    let code = unsafe { (vt.caller_id)(&mut ptr, &mut len) };
    if code == 1 {
        // 无调用方（系统内部调用，如生命周期钩子）
        return Ok(None);
    }
    if ptr.is_null() || len == 0 {
        return Err(ActrError::Internal(
            "actr_host_caller_id 返回空缓冲区".into(),
        ));
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let result = ActrId::decode(bytes)
        .map_err(|e| ActrError::Internal(format!("caller_id decode 失败: {e}")));
    unsafe { (vt.free_host_buf)(ptr, len) };
    result.map(Some)
}

fn fetch_request_id(vt: &HostVTable) -> Result<String, ActrError> {
    let mut ptr: *mut u8 = std::ptr::null_mut();
    let mut len: usize = 0;
    let err = unsafe { (vt.request_id)(&mut ptr, &mut len) };
    if err != 0 {
        return Err(ActrError::Internal(format!(
            "actr_host_request_id 返回错误码 {err}"
        )));
    }
    if ptr.is_null() || len == 0 {
        // request_id 可以为空
        return Ok(String::new());
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let result = String::from_utf8(bytes.to_vec())
        .map_err(|e| ActrError::Internal(format!("request_id UTF-8 解码失败: {e}")));
    unsafe { (vt.free_host_buf)(ptr, len) };
    result
}

// ── Dest 序列化 ───────────────────────────────────────────────────────────────

/// 将 Dest 编码为字节序列，供 vtable 函数指针使用
///
/// 格式与 WASM guest 完全一致：
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

// ── Context impl ─────────────────────────────────────────────────────────────

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
            return Err(ActrError::Internal("call 返回空响应".into()));
        }

        let resp_bytes = unsafe { std::slice::from_raw_parts(resp_ptr, resp_len) };
        let result = R::Response::decode(resp_bytes)
            .map_err(|e| ActrError::DecodeFailure(format!("响应 decode 失败: {e}")));
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
            "register_stream 在 dynclib 环境下暂不支持".into(),
        ))
    }

    async fn unregister_stream(&self, _stream_id: &str) -> ActorResult<()> {
        Err(ActrError::NotImplemented(
            "unregister_stream 在 dynclib 环境下暂不支持".into(),
        ))
    }

    async fn send_data_stream(
        &self,
        _target: &Dest,
        _chunk: DataStream,
        _payload_type: PayloadType,
    ) -> ActorResult<()> {
        Err(ActrError::NotImplemented(
            "send_data_stream 在 dynclib 环境下暂不支持".into(),
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
                "discover 未返回候选节点".into(),
            ));
        }

        let bytes = unsafe { std::slice::from_raw_parts(out_ptr, out_len) };
        let result = ActrId::decode(bytes)
            .map_err(|e| ActrError::DecodeFailure(format!("discover 结果 decode 失败: {e}")));
        unsafe { (self.vt().free_host_buf)(out_ptr, out_len) };
        result
    }

    async fn call_raw(
        &self,
        target: &ActrId,
        route_key: &str,
        payload: Bytes,
    ) -> ActorResult<Bytes> {
        // call_raw 使用 Dest::Actor 编码将 ActrId 作为目标传递
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
            return Err(ActrError::Internal("call_raw 返回空响应".into()));
        }

        let resp_bytes = unsafe { std::slice::from_raw_parts(resp_ptr, resp_len) };
        let result = Bytes::copy_from_slice(resp_bytes);
        unsafe { (self.vt().free_host_buf)(resp_ptr, resp_len) };
        Ok(result)
    }

    // ── WebRTC 媒体方法（dynclib 环境不支持）────────────────────────────────────

    async fn register_media_track<F>(&self, _track_id: String, _callback: F) -> ActorResult<()>
    where
        F: Fn(MediaSample, ActrId) -> BoxFuture<'static, ActorResult<()>> + Send + Sync + 'static,
    {
        Err(ActrError::NotImplemented(
            "WebRTC 媒体轨道在 dynclib 环境下不支持".into(),
        ))
    }

    async fn unregister_media_track(&self, _track_id: &str) -> ActorResult<()> {
        Err(ActrError::NotImplemented(
            "WebRTC 媒体轨道在 dynclib 环境下不支持".into(),
        ))
    }

    async fn send_media_sample(
        &self,
        _target: &Dest,
        _track_id: &str,
        _sample: MediaSample,
    ) -> ActorResult<()> {
        Err(ActrError::NotImplemented(
            "WebRTC 媒体轨道在 dynclib 环境下不支持".into(),
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
            "WebRTC 媒体轨道在 dynclib 环境下不支持".into(),
        ))
    }

    async fn remove_media_track(&self, _target: &Dest, _track_id: &str) -> ActorResult<()> {
        Err(ActrError::NotImplemented(
            "WebRTC 媒体轨道在 dynclib 环境下不支持".into(),
        ))
    }
}

// ── 错误码转换 ────────────────────────────────────────────────────────────────

/// 将 ABI 错误码转换为 `ActrError`
fn abi_error_to_actr(code: i32) -> ActrError {
    use crate::abi::code as c;
    match code {
        c::GENERIC_ERROR => ActrError::Internal("host 返回通用错误".into()),
        c::INIT_FAILED => ActrError::Internal("host 初始化失败".into()),
        c::HANDLE_FAILED => ActrError::Internal("host 消息处理失败".into()),
        c::PROTOCOL_ERROR => ActrError::DecodeFailure("host 协议错误".into()),
        n if n < 0 => ActrError::Internal(format!("host 未知错误码 {n}")),
        _ => ActrError::Internal(format!("host 未知错误码 {code}")),
    }
}
