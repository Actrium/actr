//! WasmContext - WASM 客体侧 Context 实现
//!
//! 通过 host imports 实现 `Context` trait，使 actor 业务代码在 wasm32 目标下
//! 以与 native 完全相同的接口运行。
//!
//! # 设计要点
//!
//! - 在 `actr_handle` 入口处构建，通过 host import 获取并缓存上下文数据
//! - `call/tell` 等通信方法通过 host import 实现，asyncify 保证调用对上层透明
//! - WebRTC 媒体相关方法在 WASM 环境下不支持，返回 `NotImplemented`

use actr_framework::{Context, Dest, MediaSample};
use actr_protocol::{ActorResult, ActrError, ActrId, ActrType, DataStream, PayloadType};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::future::BoxFuture;
use prost::Message as ProstMessage;

use crate::imports;

/// WASM 客体侧 actor 执行上下文
///
/// 在每次 `actr_handle` 调用时构建，持有本次调用的上下文数据。
/// 实现 `Context` trait，actor 业务代码无需感知底层 WASM 环境。
#[derive(Clone)]
pub struct WasmContext {
    self_id: ActrId,
    caller_id: Option<ActrId>,
    request_id: String,
}

impl WasmContext {
    /// 在 `actr_handle` 入口处调用，通过 host import 获取本次调用的上下文数据
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

// ── 上下文数据获取辅助函数 ────────────────────────────────────────────────────

fn fetch_self_id() -> Result<ActrId, ActrError> {
    let mut buf = vec![0u8; 256];
    let n = unsafe { imports::actr_host_self_id(buf.as_mut_ptr() as i32, buf.len() as i32) };
    if n <= 0 {
        return Err(ActrError::Internal("actr_host_self_id 返回空".into()));
    }
    ActrId::decode(&buf[..n as usize])
        .map_err(|e| ActrError::Internal(format!("self_id decode 失败: {e}")))
}

fn fetch_caller_id() -> Result<Option<ActrId>, ActrError> {
    let mut buf = vec![0u8; 256];
    let n = unsafe { imports::actr_host_caller_id(buf.as_mut_ptr() as i32, buf.len() as i32) };
    if n < 0 {
        // -1 表示无调用方（系统内部调用，如生命周期钩子）
        return Ok(None);
    }
    if n == 0 {
        return Err(ActrError::Internal("actr_host_caller_id 返回 0".into()));
    }
    let id = ActrId::decode(&buf[..n as usize])
        .map_err(|e| ActrError::Internal(format!("caller_id decode 失败: {e}")))?;
    Ok(Some(id))
}

fn fetch_request_id() -> Result<String, ActrError> {
    let mut buf = vec![0u8; 128];
    let n = unsafe { imports::actr_host_request_id(buf.as_mut_ptr() as i32, buf.len() as i32) };
    if n <= 0 {
        return Ok(String::new());
    }
    String::from_utf8(buf[..n as usize].to_vec())
        .map_err(|e| ActrError::Internal(format!("request_id UTF-8 解码失败: {e}")))
}

// ── Dest 序列化 ───────────────────────────────────────────────────────────────

/// 将 Dest 编码为字节序列，供 host import 使用
///
/// 格式：`[tag: u8] [ActrId protobuf bytes (仅 Actor 变体)]`
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

// ── Context impl ─────────────────────────────────────────────────────────────

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

        // 预分配响应缓冲区（最大 64 KB，足够绝大多数 RPC 响应）
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
            .map_err(|e| ActrError::DecodeFailure(format!("响应 decode 失败: {e}")))
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
        // DataStream 回调注册在 WASM 环境下暂不支持
        // 需要宿主侧额外协议支持，待后续版本实现
        Err(ActrError::NotImplemented(
            "register_stream 在 WASM 环境下暂不支持".into(),
        ))
    }

    async fn unregister_stream(&self, _stream_id: &str) -> ActorResult<()> {
        Err(ActrError::NotImplemented(
            "unregister_stream 在 WASM 环境下暂不支持".into(),
        ))
    }

    async fn send_data_stream(
        &self,
        _target: &Dest,
        _chunk: DataStream,
        _payload_type: PayloadType,
    ) -> ActorResult<()> {
        Err(ActrError::NotImplemented(
            "send_data_stream 在 WASM 环境下暂不支持".into(),
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
            .map_err(|e| ActrError::DecodeFailure(format!("discover 结果 decode 失败: {e}")))
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

    // ── WebRTC 媒体方法（WASM 环境不支持）──────────────────────────────────────

    async fn register_media_track<F>(&self, _track_id: String, _callback: F) -> ActorResult<()>
    where
        F: Fn(MediaSample, ActrId) -> BoxFuture<'static, ActorResult<()>> + Send + Sync + 'static,
    {
        Err(ActrError::NotImplemented(
            "WebRTC 媒体轨道在 WASM 环境下不支持".into(),
        ))
    }

    async fn unregister_media_track(&self, _track_id: &str) -> ActorResult<()> {
        Err(ActrError::NotImplemented(
            "WebRTC 媒体轨道在 WASM 环境下不支持".into(),
        ))
    }

    async fn send_media_sample(
        &self,
        _target: &Dest,
        _track_id: &str,
        _sample: MediaSample,
    ) -> ActorResult<()> {
        Err(ActrError::NotImplemented(
            "WebRTC 媒体轨道在 WASM 环境下不支持".into(),
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
            "WebRTC 媒体轨道在 WASM 环境下不支持".into(),
        ))
    }

    async fn remove_media_track(&self, _target: &Dest, _track_id: &str) -> ActorResult<()> {
        Err(ActrError::NotImplemented(
            "WebRTC 媒体轨道在 WASM 环境下不支持".into(),
        ))
    }
}

// ── 错误码转换 ────────────────────────────────────────────────────────────────

/// 将 ABI 错误码（来自 host import 返回值）转换为 `ActrError`
fn abi_error_to_actr(code: i32) -> ActrError {
    use crate::abi::code;
    match code {
        code::GENERIC_ERROR => ActrError::Internal("WASM host 返回通用错误".into()),
        code::INIT_FAILED => ActrError::Internal("WASM host 初始化失败".into()),
        code::HANDLE_FAILED => ActrError::Internal("WASM host 消息处理失败".into()),
        code::ALLOC_FAILED => ActrError::Internal("WASM host 内存分配失败".into()),
        code::PROTOCOL_ERROR => ActrError::DecodeFailure("WASM host 协议错误".into()),
        // discover 相关：返回 0 或负值均表示未找到
        n if n <= 0 => ActrError::NotFound(format!("discover 无可用候选（code={n}）")),
        _ => ActrError::Internal(format!("WASM host 未知错误码 {code}")),
    }
}
