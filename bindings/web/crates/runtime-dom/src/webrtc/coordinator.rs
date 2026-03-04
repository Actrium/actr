//! WebRTC Coordinator - WebRTC 连接协调器（DOM 端辅助）
//!
//! 负责：
//! - 接收 SW 的 P2P 创建请求
//! - 调用 WebRTC API 创建 PeerConnection
//! - 完成后通知 SW

use crate::error_reporter::get_global_error_reporter;
use actr_web_common::{
    ControlMessage, CreateP2PRequest, Dest, ErrorSeverity, P2PReadyEvent, WebError, WebResult,
};
use dashmap::DashMap;
use parking_lot::Mutex;
use std::sync::Arc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{RtcConfiguration, RtcDataChannel, RtcDataChannelInit, RtcPeerConnection};

/// WebRTC Coordinator - DOM 端的 WebRTC 协调器
///
/// **纯辅助角色**，不管理连接，只负责创建并通知 SW
pub struct WebRtcCoordinator {
    /// SW 通道（接收请求，发送就绪事件）
    sw_channel: Arc<Mutex<Option<super::super::transport::lane::DataLane>>>,

    /// 活跃的 PeerConnection（peer_id → RtcPeerConnection）
    peer_connections: Arc<DashMap<String, RtcPeerConnection>>,

    /// ICE 服务器配置
    ice_servers: Vec<String>,
}

impl WebRtcCoordinator {
    /// 创建新的 WebRTC Coordinator
    pub fn new(ice_servers: Vec<String>) -> Self {
        Self {
            sw_channel: Arc::new(Mutex::new(None)),
            peer_connections: Arc::new(DashMap::new()),
            ice_servers,
        }
    }

    /// 设置 SW 通道
    pub fn set_sw_channel(&self, channel: super::super::transport::lane::DataLane) {
        let mut sw = self.sw_channel.lock();
        *sw = Some(channel);
        log::info!("[WebRtcCoordinator] SW channel set");
    }

    /// 启动事件循环（接收 SW 请求）
    pub fn start(&self) {
        let sw_channel = Arc::clone(&self.sw_channel);
        let peer_connections = Arc::clone(&self.peer_connections);
        let ice_servers = self.ice_servers.clone();

        wasm_bindgen_futures::spawn_local(async move {
            log::info!("[WebRtcCoordinator] Event loop started");

            loop {
                // 获取通道
                let channel = {
                    let sw = sw_channel.lock();
                    sw.as_ref().cloned()
                };

                if let Some(lane) = channel {
                    match lane.recv().await {
                        Some(data) => {
                            // 解析控制消息
                            match ControlMessage::deserialize(&data) {
                                Ok(ControlMessage::CreateP2P(request)) => {
                                    log::info!(
                                        "[WebRtcCoordinator] Received P2P request: {:?}",
                                        request.dest
                                    );

                                    // 处理请求（异步）
                                    Self::handle_create_p2p(
                                        request,
                                        peer_connections.clone(),
                                        lane.clone(),
                                        ice_servers.clone(),
                                    );
                                }
                                Ok(ControlMessage::P2PReady(_)) => {
                                    // SW 发来的就绪事件，忽略（这是 SW→DOM 方向）
                                    log::trace!("[WebRtcCoordinator] Ignoring P2PReady from SW");
                                }
                                Ok(ControlMessage::ErrorReport(_)) => {
                                    // SW 发来的错误报告，忽略（这是 DOM→SW 方向）
                                    log::trace!("[WebRtcCoordinator] Ignoring ErrorReport from SW");
                                }
                                Err(e) => {
                                    log::error!(
                                        "[WebRtcCoordinator] Failed to parse control message: {}",
                                        e
                                    );
                                }
                            }
                        }
                        None => {
                            log::warn!("[WebRtcCoordinator] SW channel closed");
                            break;
                        }
                    }
                } else {
                    log::warn!("[WebRtcCoordinator] SW channel not available");
                    break;
                }
            }

            log::info!("[WebRtcCoordinator] Event loop stopped");
        });
    }

    /// 处理 P2P 创建请求（异步）
    fn handle_create_p2p(
        request: CreateP2PRequest,
        peer_connections: Arc<DashMap<String, RtcPeerConnection>>,
        sw_channel: super::super::transport::lane::DataLane,
        ice_servers: Vec<String>,
    ) {
        wasm_bindgen_futures::spawn_local(async move {
            let peer_id = match &request.dest {
                Dest::Peer(id) => id.clone(),
                _ => {
                    log::error!(
                        "[WebRtcCoordinator] Invalid dest for P2P: {:?}",
                        request.dest
                    );
                    Self::send_failure(
                        request.request_id,
                        request.dest,
                        "Invalid dest type".to_string(),
                        sw_channel,
                    )
                    .await;
                    return;
                }
            };

            log::info!("[WebRtcCoordinator] Creating P2P for: {}", peer_id);

            match Self::create_peer_connection(&peer_id, ice_servers).await {
                Ok(pc) => {
                    // 保存 PeerConnection
                    peer_connections.insert(peer_id.clone(), pc);

                    log::info!("[WebRtcCoordinator] P2P created successfully: {}", peer_id);

                    // 发送成功事件
                    Self::send_success(request.request_id, request.dest, sw_channel).await;
                }
                Err(e) => {
                    log::error!(
                        "[WebRtcCoordinator] P2P creation failed: {}: {}",
                        peer_id,
                        e
                    );

                    // 报告错误
                    if let Some(reporter) = get_global_error_reporter() {
                        reporter.report_webrtc_error(
                            &request.dest,
                            format!("Failed to create P2P connection: {}", e),
                            ErrorSeverity::Error,
                        );
                    }

                    // 发送失败事件
                    Self::send_failure(request.request_id, request.dest, e.to_string(), sw_channel)
                        .await;
                }
            }
        });
    }

    /// 创建 PeerConnection
    async fn create_peer_connection(
        peer_id: &str,
        ice_servers: Vec<String>,
    ) -> WebResult<RtcPeerConnection> {
        // 1. 创建配置
        let config = RtcConfiguration::new();

        // 设置 ICE 服务器（简化：使用公共 STUN）
        if !ice_servers.is_empty() {
            // TODO: 实际设置 ICE 服务器
            log::debug!("[WebRtcCoordinator] ICE servers: {:?}", ice_servers);
        }

        // 2. 创建 PeerConnection
        let pc = RtcPeerConnection::new_with_configuration(&config).map_err(|e| {
            WebError::Transport(format!("Failed to create PeerConnection: {:?}", e))
        })?;

        log::debug!("[WebRtcCoordinator] PeerConnection created: {}", peer_id);

        // 3. 创建 DataChannel
        let dc_config = RtcDataChannelInit::new();
        dc_config.set_ordered(true);

        let dc = pc.create_data_channel_with_data_channel_dict("data", &dc_config);

        log::debug!("[WebRtcCoordinator] DataChannel created: {}", peer_id);

        // 4. 设置事件处理器
        Self::setup_datachannel_handlers(&dc, peer_id);

        // 5. 创建 Offer（简化实现：实际需要信令交换）
        // TODO: 实际的 SDP 交换需要通过信令服务器
        log::warn!("[WebRtcCoordinator] SDP exchange not implemented yet (Phase 4)");

        Ok(pc)
    }

    /// 设置 DataChannel 事件处理器
    fn setup_datachannel_handlers(dc: &RtcDataChannel, peer_id: &str) {
        let peer_id = peer_id.to_string();

        // onopen
        let peer_id_clone = peer_id.clone();
        let onopen = Closure::wrap(Box::new(move || {
            log::info!("[WebRtcCoordinator] DataChannel opened: {}", peer_id_clone);
        }) as Box<dyn FnMut()>);
        dc.set_onopen(Some(onopen.as_ref().unchecked_ref()));
        onopen.forget();

        // onerror
        let peer_id_clone = peer_id.clone();
        let onerror = Closure::wrap(Box::new(move |e: web_sys::ErrorEvent| {
            let error_msg = e.message();
            log::error!(
                "[WebRtcCoordinator] DataChannel error: {}: {:?}",
                peer_id_clone,
                error_msg
            );

            // 报告错误
            if let Some(reporter) = get_global_error_reporter() {
                reporter.report_webrtc_error(
                    &Dest::Peer(peer_id_clone.clone()),
                    format!("DataChannel error: {:?}", error_msg),
                    ErrorSeverity::Error,
                );
            }
        }) as Box<dyn FnMut(_)>);
        dc.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        onerror.forget();

        // onclose
        let onclose = Closure::wrap(Box::new(move || {
            log::info!("[WebRtcCoordinator] DataChannel closed: {}", peer_id);
        }) as Box<dyn FnMut()>);
        dc.set_onclose(Some(onclose.as_ref().unchecked_ref()));
        onclose.forget();
    }

    /// 发送成功事件到 SW
    async fn send_success(
        request_id: String,
        dest: Dest,
        sw_channel: super::super::transport::lane::DataLane,
    ) {
        let event = P2PReadyEvent::success(request_id, dest);
        let msg = ControlMessage::P2PReady(event);

        match msg.serialize() {
            Ok(data) => {
                if let Err(e) = sw_channel.send(data).await {
                    log::error!("[WebRtcCoordinator] Failed to send success event: {}", e);
                }
            }
            Err(e) => {
                log::error!(
                    "[WebRtcCoordinator] Failed to serialize success event: {}",
                    e
                );
            }
        }
    }

    /// 发送失败事件到 SW
    async fn send_failure(
        request_id: String,
        dest: Dest,
        error: String,
        sw_channel: super::super::transport::lane::DataLane,
    ) {
        let event = P2PReadyEvent::failure(request_id, dest.clone(), error.clone());
        let msg = ControlMessage::P2PReady(event);

        match msg.serialize() {
            Ok(data) => {
                if let Err(e) = sw_channel.send(data).await {
                    log::error!("[WebRtcCoordinator] Failed to send failure event: {}", e);

                    // 报告 MessagePort 错误
                    if let Some(reporter) = get_global_error_reporter() {
                        reporter.report_messageport_error(
                            format!("Failed to send P2P failure event: {}", e),
                            ErrorSeverity::Warning,
                        );
                    }
                }
            }
            Err(e) => {
                log::error!(
                    "[WebRtcCoordinator] Failed to serialize failure event: {}",
                    e
                );

                // 报告序列化错误
                if let Some(reporter) = get_global_error_reporter() {
                    reporter.report_messageport_error(
                        format!("Failed to serialize P2P failure event: {}", e),
                        ErrorSeverity::Warning,
                    );
                }
            }
        }
    }

    /// 关闭指定对等端的连接
    pub fn close_peer(&self, peer_id: &str) -> WebResult<()> {
        if let Some((_, pc)) = self.peer_connections.remove(peer_id) {
            pc.close();
            log::info!("[WebRtcCoordinator] Closed peer connection: {}", peer_id);
        }
        Ok(())
    }

    /// 关闭所有连接
    pub fn close_all(&self) -> WebResult<()> {
        log::info!(
            "[WebRtcCoordinator] Closing all peer connections (count: {})",
            self.peer_connections.len()
        );

        for entry in self.peer_connections.iter() {
            entry.value().close();
        }

        self.peer_connections.clear();
        Ok(())
    }
}

impl Default for WebRtcCoordinator {
    fn default() -> Self {
        Self::new(vec!["stun:stun.l.google.com:19302".to_string()])
    }
}
