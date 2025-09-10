//! WebRTC 集成模块

use crate::error::{ActorResult, WebRTCError};
use bytes::Bytes;
use shared_protocols::actor::ActorId;
use shared_protocols::webrtc::{IceCandidate, SessionDescription};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::APIBuilder;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;

/// WebRTC 连接管理器
pub struct WebRTCManager {
    /// WebRTC API 实例
    api: webrtc::api::API,
    /// 活跃的对等连接
    peer_connections: Arc<Mutex<HashMap<String, PeerConnectionWrapper>>>,
    /// 配置
    config: RTCConfiguration,
}

impl WebRTCManager {
    /// 创建新的 WebRTC 管理器
    pub fn new() -> ActorResult<Self> {
        // 创建媒体引擎
        let mut media_engine = MediaEngine::default();

        // 注册默认编解码器
        media_engine.register_default_codecs().map_err(|e| {
            WebRTCError::PeerConnectionFailed(format!("Failed to register codecs: {}", e))
        })?;

        // 创建拦截器注册表
        let mut registry = Registry::new();

        // 注册默认拦截器
        registry = register_default_interceptors(registry, &mut media_engine).map_err(|e| {
            WebRTCError::PeerConnectionFailed(format!("Failed to register interceptors: {}", e))
        })?;

        // 构建 API
        let api = APIBuilder::new()
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .build();

        // 创建配置（使用公共 STUN 服务器）
        let config = RTCConfiguration {
            ice_servers: vec![RTCIceServer {
                urls: vec!["stun:stun.l.google.com:19302".to_owned()],
                ..Default::default()
            }],
            ..Default::default()
        };

        Ok(Self {
            api,
            peer_connections: Arc::new(Mutex::new(HashMap::new())),
            config,
        })
    }

    /// 创建到指定 Actor 的连接
    pub async fn create_connection(
        &self,
        target_actor: &ActorId,
        is_offerer: bool,
    ) -> ActorResult<Arc<RTCPeerConnection>> {
        let key = self.actor_key(target_actor);

        // 检查是否已存在连接
        {
            let connections = self.peer_connections.lock().await;
            if connections.contains_key(&key) {
                return Err(WebRTCError::PeerConnectionFailed(format!(
                    "Connection to {} already exists",
                    key
                ))
                .into());
            }
        }

        // 创建对等连接
        let peer_connection = Arc::new(
            self.api
                .new_peer_connection(self.config.clone())
                .await
                .map_err(|e| {
                    WebRTCError::PeerConnectionFailed(format!(
                        "Failed to create peer connection: {}",
                        e
                    ))
                })?,
        );

        info!("Created peer connection to {}", key);

        // 设置连接状态回调
        let pc_clone = Arc::downgrade(&peer_connection);
        let key_clone = key.clone();
        peer_connection.on_peer_connection_state_change(Box::new(
            move |state: RTCPeerConnectionState| {
                let key = key_clone.clone();
                let pc_clone = pc_clone.clone();
                Box::pin(async move {
                    info!("Peer connection state changed for {}: {:?}", key, state);
                    match state {
                        RTCPeerConnectionState::Failed | RTCPeerConnectionState::Disconnected => {
                            if let Some(pc) = pc_clone.upgrade() {
                                warn!("Peer connection to {} failed or disconnected", key);
                                let _ = pc.close().await;
                            }
                        }
                        RTCPeerConnectionState::Connected => {
                            info!("Successfully connected to peer {}", key);
                        }
                        _ => {}
                    }
                })
            },
        ));

        // 设置 ICE 连接状态回调
        let key_clone = key.clone();
        peer_connection.on_ice_connection_state_change(Box::new(
            move |state: RTCIceConnectionState| {
                let key = key_clone.clone();
                Box::pin(async move {
                    debug!("ICE connection state changed for {}: {:?}", key, state);
                })
            },
        ));

        // 如果是 offerer，创建数据通道
        let data_channel = if is_offerer {
            Some(
                self.create_data_channel(&peer_connection, "reliable")
                    .await?,
            )
        } else {
            None
        };

        // 如果不是 offerer，监听数据通道创建
        if !is_offerer {
            let key_clone = key.clone();
            peer_connection.on_data_channel(Box::new(move |dc: Arc<RTCDataChannel>| {
                let key = key_clone.clone();
                Box::pin(async move {
                    info!("Received data channel from {}: {}", key, dc.label());
                    // TODO: 处理接收到的数据通道
                })
            }));
        }

        // 创建包装器
        let wrapper = PeerConnectionWrapper {
            peer_connection: peer_connection.clone(),
            target_actor: target_actor.clone(),
            data_channels: Arc::new(Mutex::new(HashMap::new())),
            is_offerer,
            created_at: std::time::Instant::now(),
        };

        // 如果有数据通道，保存它
        if let Some(dc) = data_channel {
            wrapper
                .data_channels
                .lock()
                .await
                .insert("reliable".to_string(), dc);
        }

        // 保存连接
        {
            let mut connections = self.peer_connections.lock().await;
            connections.insert(key.clone(), wrapper);
        }

        Ok(peer_connection)
    }

    /// 创建数据通道
    async fn create_data_channel(
        &self,
        pc: &RTCPeerConnection,
        label: &str,
    ) -> ActorResult<Arc<RTCDataChannel>> {
        let data_channel = pc.create_data_channel(label, None).await.map_err(|e| {
            WebRTCError::DataChannelError(format!("Failed to create data channel: {}", e))
        })?;

        info!("Created data channel: {}", label);

        // 设置数据通道回调
        let _dc_clone = Arc::downgrade(&data_channel);
        let label_clone = label.to_string();
        data_channel.on_open(Box::new(move || {
            let label = label_clone.clone();
            Box::pin(async move {
                info!("Data channel {} opened", label);
            })
        }));

        let label_clone = label.to_string();
        data_channel.on_close(Box::new(move || {
            let label = label_clone.clone();
            Box::pin(async move {
                info!("Data channel {} closed", label);
            })
        }));

        let label_clone = label.to_string();
        data_channel.on_message(Box::new(move |msg: DataChannelMessage| {
            let label = label_clone.clone();
            Box::pin(async move {
                debug!(
                    "Received message on channel {}: {} bytes",
                    label,
                    msg.data.len()
                );
                // TODO: 将消息路由到消息处理系统
            })
        }));

        Ok(data_channel)
    }

    /// 创建 Offer
    pub async fn create_offer(&self, target_actor: &ActorId) -> ActorResult<SessionDescription> {
        let key = self.actor_key(target_actor);

        let connections = self.peer_connections.lock().await;
        let wrapper = connections.get(&key).ok_or_else(|| {
            WebRTCError::PeerConnectionFailed(format!("No connection to {}", key))
        })?;

        let offer = wrapper
            .peer_connection
            .create_offer(None)
            .await
            .map_err(|e| {
                WebRTCError::SdpNegotiationFailed(format!("Failed to create offer: {}", e))
            })?;

        wrapper
            .peer_connection
            .set_local_description(offer.clone())
            .await
            .map_err(|e| {
                WebRTCError::SdpNegotiationFailed(format!("Failed to set local description: {}", e))
            })?;

        info!("Created offer for {}", key);

        Ok(SessionDescription {
            r#type: shared_protocols::webrtc::session_description::Type::Offer as i32,
            sdp: offer.sdp,
        })
    }

    /// 创建 Answer
    pub async fn create_answer(&self, target_actor: &ActorId) -> ActorResult<SessionDescription> {
        let key = self.actor_key(target_actor);

        let connections = self.peer_connections.lock().await;
        let wrapper = connections.get(&key).ok_or_else(|| {
            WebRTCError::PeerConnectionFailed(format!("No connection to {}", key))
        })?;

        let answer = wrapper
            .peer_connection
            .create_answer(None)
            .await
            .map_err(|e| {
                WebRTCError::SdpNegotiationFailed(format!("Failed to create answer: {}", e))
            })?;

        wrapper
            .peer_connection
            .set_local_description(answer.clone())
            .await
            .map_err(|e| {
                WebRTCError::SdpNegotiationFailed(format!("Failed to set local description: {}", e))
            })?;

        info!("Created answer for {}", key);

        Ok(SessionDescription {
            r#type: shared_protocols::webrtc::session_description::Type::Answer as i32,
            sdp: answer.sdp,
        })
    }

    /// 设置远程 SDP
    pub async fn set_remote_description(
        &self,
        target_actor: &ActorId,
        sdp: &SessionDescription,
    ) -> ActorResult<()> {
        let key = self.actor_key(target_actor);

        let connections = self.peer_connections.lock().await;
        let wrapper = connections.get(&key).ok_or_else(|| {
            WebRTCError::PeerConnectionFailed(format!("No connection to {}", key))
        })?;

        let sdp_type = match sdp.r#type {
            x if x == shared_protocols::webrtc::session_description::Type::Offer as i32 => {
                webrtc::peer_connection::sdp::sdp_type::RTCSdpType::Offer
            }
            x if x == shared_protocols::webrtc::session_description::Type::Answer as i32 => {
                webrtc::peer_connection::sdp::sdp_type::RTCSdpType::Answer
            }
            _ => {
                return Err(
                    WebRTCError::SdpNegotiationFailed("Unknown SDP type".to_string()).into(),
                );
            }
        };

        let session_description = match sdp_type {
            webrtc::peer_connection::sdp::sdp_type::RTCSdpType::Offer => {
                RTCSessionDescription::offer(sdp.sdp.clone()).map_err(|e| {
                    WebRTCError::SdpNegotiationFailed(format!(
                        "Failed to create offer description: {}",
                        e
                    ))
                })?
            }
            webrtc::peer_connection::sdp::sdp_type::RTCSdpType::Answer => {
                RTCSessionDescription::answer(sdp.sdp.clone()).map_err(|e| {
                    WebRTCError::SdpNegotiationFailed(format!(
                        "Failed to create answer description: {}",
                        e
                    ))
                })?
            }
            _ => {
                return Err(
                    WebRTCError::SdpNegotiationFailed("Unsupported SDP type".to_string()).into(),
                )
            }
        };

        wrapper
            .peer_connection
            .set_remote_description(session_description)
            .await
            .map_err(|e| {
                WebRTCError::SdpNegotiationFailed(format!(
                    "Failed to set remote description: {}",
                    e
                ))
            })?;

        info!("Set remote description for {}", key);
        Ok(())
    }

    /// 添加 ICE 候选
    pub async fn add_ice_candidate(
        &self,
        target_actor: &ActorId,
        candidate: &IceCandidate,
    ) -> ActorResult<()> {
        let key = self.actor_key(target_actor);

        let connections = self.peer_connections.lock().await;
        let wrapper = connections.get(&key).ok_or_else(|| {
            WebRTCError::PeerConnectionFailed(format!("No connection to {}", key))
        })?;

        let ice_candidate = webrtc::ice_transport::ice_candidate::RTCIceCandidateInit {
            candidate: candidate.candidate.clone(),
            sdp_mid: candidate.sdp_mid.clone(),
            sdp_mline_index: candidate.sdp_mline_index.map(|x| x as u16),
            username_fragment: candidate.username_fragment.clone(),
        };

        wrapper
            .peer_connection
            .add_ice_candidate(ice_candidate)
            .await
            .map_err(|e| {
                WebRTCError::IceGatheringFailed(format!("Failed to add ICE candidate: {}", e))
            })?;

        debug!("Added ICE candidate for {}", key);
        Ok(())
    }

    /// 发送数据
    pub async fn send_data(
        &self,
        target_actor: &ActorId,
        channel_name: &str,
        data: Vec<u8>,
    ) -> ActorResult<()> {
        let key = self.actor_key(target_actor);

        let connections = self.peer_connections.lock().await;
        let wrapper = connections.get(&key).ok_or_else(|| {
            WebRTCError::PeerConnectionFailed(format!("No connection to {}", key))
        })?;

        let data_channels = wrapper.data_channels.lock().await;
        let data_channel = data_channels.get(channel_name).ok_or_else(|| {
            WebRTCError::DataChannelError(format!("No channel named {}", channel_name))
        })?;

        let data_len = data.len();
        data_channel
            .send(&Bytes::from(data))
            .await
            .map_err(|e| WebRTCError::DataChannelError(format!("Failed to send data: {}", e)))?;

        debug!(
            "Sent {} bytes to {} on channel {}",
            data_len, key, channel_name
        );
        Ok(())
    }

    /// 关闭到指定 Actor 的连接
    pub async fn close_connection(&self, target_actor: &ActorId) -> ActorResult<()> {
        let key = self.actor_key(target_actor);

        let mut connections = self.peer_connections.lock().await;
        if let Some(wrapper) = connections.remove(&key) {
            let _ = wrapper.peer_connection.close().await;
            info!("Closed connection to {}", key);
        }

        Ok(())
    }

    /// 生成 Actor 键
    fn actor_key(&self, actor_id: &ActorId) -> String {
        format!(
            "{}_{}",
            actor_id.serial_number,
            actor_id
                .r#type
                .as_ref()
                .map(|t| t.name.as_str())
                .unwrap_or("unknown")
        )
    }

    /// 获取连接统计
    pub async fn get_connection_stats(&self) -> HashMap<String, ConnectionStats> {
        let connections = self.peer_connections.lock().await;
        let mut stats = HashMap::new();

        for (key, wrapper) in connections.iter() {
            stats.insert(
                key.clone(),
                ConnectionStats {
                    target_actor: wrapper.target_actor.clone(),
                    is_offerer: wrapper.is_offerer,
                    created_at: wrapper.created_at,
                    state: wrapper.peer_connection.connection_state(),
                },
            );
        }

        stats
    }
}

/// 对等连接包装器
struct PeerConnectionWrapper {
    peer_connection: Arc<RTCPeerConnection>,
    target_actor: ActorId,
    data_channels: Arc<Mutex<HashMap<String, Arc<RTCDataChannel>>>>,
    is_offerer: bool,
    created_at: std::time::Instant,
}

/// 连接统计信息
#[derive(Debug, Clone)]
pub struct ConnectionStats {
    pub target_actor: ActorId,
    pub is_offerer: bool,
    pub created_at: std::time::Instant,
    pub state: RTCPeerConnectionState,
}
