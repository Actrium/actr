//! Service Worker System Module
//!
//! Service Worker 端的 ActorSystem 实现
//! 负责 State Path：Mailbox + Scheduler + Actor 执行
//!
//! # 架构设计
//!
//! ```text
//! DOM 侧
//!   RPC 请求
//!     ↓
//! ═══════════════════════════════════════════════════════
//! SW 侧
//!     ↓
//!   HostGate.send_request()
//!     ↓
//!   MessageHandler (由 System 设置)
//!     ↓
//!   ┌─────────────────────────────────────────────────┐
//!   │ System 判断目标:                                │
//!   │ - 本地 Actor? → Mailbox → Scheduler → Actor    │
//!   │ - 远程 Actor? → Gate → Transport → Remote   │
//!   └─────────────────────────────────────────────────┘
//!     ↓
//!   响应返回
//!     ↓
//!   HostGate.handle_response()
//!     ↓
//!   DOM 侧收到响应
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use actr_protocol::ActrId;
use bytes::Bytes;
use futures::channel::oneshot;
use wasm_bindgen::prelude::*;
use web_sys::MessagePort;

use crate::outbound::{Gate, HostGate, PeerGate};

/// Service Worker System
///
/// 消息处理的中心枢纽，连接 DOM 和远程 Actor
///
/// 注意：WASM/Service Worker 是单线程环境，使用 Rc/RefCell 而不是 Arc/Mutex
pub struct System {
    /// HostGate - 处理来自 DOM 的请求
    host_gate: Arc<HostGate>,

    /// Gate - 出站消息路由
    ///
    /// Peer（专用 MessagePort + 完整传输栈）
    /// PeerGate → PeerTransport → DestTransport → WirePool → DataLane::PostMessage
    outgate: Rc<RefCell<Option<Gate>>>,

    /// DOM 通信端口
    dom_port: Rc<RefCell<Option<MessagePort>>>,

    /// 本地 Actor ID（客户端模式下的自身 ID）
    local_actor_id: Rc<RefCell<Option<ActrId>>>,

    /// Pending requests 用于响应匹配
    pending_requests: Rc<RefCell<HashMap<String, oneshot::Sender<Bytes>>>>,
}

impl System {
    /// 创建新的 System
    pub fn new() -> Self {
        let host_gate = Arc::new(HostGate::new());

        Self {
            host_gate,
            outgate: Rc::new(RefCell::new(None)),
            dom_port: Rc::new(RefCell::new(None)),
            local_actor_id: Rc::new(RefCell::new(None)),
            pending_requests: Rc::new(RefCell::new(HashMap::new())),
        }
    }

    /// 获取 HostGate
    pub fn host_gate(&self) -> &Arc<HostGate> {
        &self.host_gate
    }

    /// 设置 Gate（统一出站路由）
    pub fn set_outgate(&self, gate: Gate) {
        *self.outgate.borrow_mut() = Some(gate);
    }

    /// 设置 PeerGate（便捷方法，内部转为 Gate::Peer）
    pub fn set_peer_gate(&self, gate: Arc<PeerGate>) {
        self.set_outgate(Gate::peer(gate));
    }

    /// 获取当前 Gate 的克隆
    pub fn outgate(&self) -> Option<Gate> {
        self.outgate.borrow().clone()
    }

    /// 设置 DOM 端口
    pub fn set_dom_port(&self, port: MessagePort) {
        *self.dom_port.borrow_mut() = Some(port);
    }

    /// 设置本地 Actor ID
    pub fn set_local_actor_id(&self, actor_id: ActrId) {
        *self.local_actor_id.borrow_mut() = Some(actor_id);
    }

    /// 注册一个 pending request
    pub fn register_pending_request(&self, request_id: String, sender: oneshot::Sender<Bytes>) {
        self.pending_requests
            .borrow_mut()
            .insert(request_id, sender);
    }

    /// 初始化消息处理器
    ///
    /// 设置 HostGate 的 MessageHandler，将消息路由到正确的目标：
    /// - 本地 Actor → TODO (Phase 2)
    /// - 远程 Actor → Gate.send_message()
    pub fn init_message_handler(&self) {
        let local_actor_id = Rc::clone(&self.local_actor_id);
        let outgate = Rc::clone(&self.outgate);

        self.host_gate
            .set_message_handler(move |target_id, envelope| {
                log::info!(
                    "[System] MessageHandler: routing request_id={} to target={:?}",
                    envelope.request_id,
                    target_id
                );

                let local_id = local_actor_id.borrow().clone();
                let gate = outgate.borrow().clone();
                let envelope = envelope.clone();

                wasm_bindgen_futures::spawn_local(async move {
                    // 判断是本地还是远程调用
                    let is_local = local_id
                        .as_ref()
                        .map(|id| id == &target_id)
                        .unwrap_or(false);

                    if is_local {
                        // TODO: 本地 Actor 调用（Phase 2）
                        log::warn!(
                            "[System] Local actor calls not yet implemented, request_id={}",
                            envelope.request_id
                        );
                    } else {
                        // 远程调用：通过 Gate 发送
                        match gate {
                            Some(ref g) => {
                                if let Err(e) = g.send_message(&target_id, envelope.clone()).await {
                                    log::error!("[System] Gate send_message failed: {:?}", e);
                                }
                            }
                            None => {
                                log::error!(
                                    "[System] Gate not set, cannot route remote message"
                                );
                            }
                        }
                    }
                });
            });
    }

    /// 处理来自远程的响应
    ///
    /// 路由顺序：
    /// 1. Gate（DomBridge/Peer 的 pending requests，用于 Actor 发起的调用）
    /// 2. System pending_requests
    /// 3. HostGate（用于 DOM 发起的调用）
    pub fn handle_remote_response(&self, request_id: &str, response: Bytes) {
        // 1. 尝试 Gate（Actor 主动发起的 call() 的响应）
        if let Some(ref gate) = *self.outgate.borrow() {
            if gate.try_handle_response(request_id, response.clone()) {
                return;
            }
        }

        // 2. 尝试 System pending_requests
        if let Some(tx) = self.pending_requests.borrow_mut().remove(request_id) {
            match tx.send(response.clone()) {
                Ok(()) => return, // Receiver alive, consumed
                Err(_) => {}      // Receiver dropped, fall through
            }
        }

        // 3. 转发到 HostGate（DOM 发起的调用）
        self.host_gate.handle_response(request_id, response);
    }

    /// 发送消息到 DOM
    pub fn send_to_dom(&self, msg: &JsValue) -> Result<(), String> {
        let dom_port = self.dom_port.borrow();
        if let Some(ref port) = *dom_port {
            port.post_message(msg)
                .map_err(|e| format!("Failed to send to DOM: {:?}", e))?;
            Ok(())
        } else {
            Err("DOM port not set".to_string())
        }
    }
}

impl Default for System {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_creation() {
        let _system = System::new();
    }
}
