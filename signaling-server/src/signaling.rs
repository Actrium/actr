use serde::{Deserialize, Serialize};
use shared_protocols::actor::ActorId;

/// WebRTC 信令消息类型  
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "messageType")]
pub enum SignalingMessage {
    /// 注册 Actor ID
    #[serde(rename = "register")]
    Register {
        #[serde(rename = "actorId")]
        actor_id: ActorId,
    },

    /// WebRTC Offer
    Offer {
        target: ActorId,
        source: ActorId,
        sdp: String,
    },

    /// WebRTC Answer
    Answer {
        target: ActorId,
        source: ActorId,
        sdp: String,
    },

    /// ICE Candidate
    IceCandidate {
        target: ActorId,
        source: ActorId,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    },

    /// Actor 消息路由
    ActorMessage {
        target: ActorId,
        source: ActorId,
        payload: String,
        message_type: String,
    },

    /// 连接状态通知
    ConnectionStatus {
        actor_id: ActorId,
        status: ConnectionStatus,
    },

    /// 错误消息
    Error {
        code: u32,
        message: String,
    },

    /// 心跳消息
    Ping,
    Pong,
}

/// 连接状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConnectionStatus {
    Connected,
    Disconnected,
    Connecting,
    Failed,
}

impl SignalingMessage {
    /// 获取消息类型的字符串表示
    pub fn message_type(&self) -> &'static str {
        match self {
            SignalingMessage::Register { .. } => "register",
            SignalingMessage::Offer { .. } => "offer",
            SignalingMessage::Answer { .. } => "answer",
            SignalingMessage::IceCandidate { .. } => "ice-candidate",
            SignalingMessage::ActorMessage { .. } => "actor-message",
            SignalingMessage::ConnectionStatus { .. } => "connection-status",
            SignalingMessage::Error { .. } => "error",
            SignalingMessage::Ping => "ping",
            SignalingMessage::Pong => "pong",
        }
    }

    /// 获取目标 Actor ID（如果有）
    #[allow(dead_code)]
    pub fn target_actor(&self) -> Option<&ActorId> {
        match self {
            SignalingMessage::Offer { target, .. }
            | SignalingMessage::Answer { target, .. }
            | SignalingMessage::IceCandidate { target, .. }
            | SignalingMessage::ActorMessage { target, .. } => Some(target),
            _ => None,
        }
    }

    /// 获取源 Actor ID（如果有）
    #[allow(dead_code)]
    pub fn source_actor(&self) -> Option<&ActorId> {
        match self {
            SignalingMessage::Offer { source, .. }
            | SignalingMessage::Answer { source, .. }
            | SignalingMessage::IceCandidate { source, .. }
            | SignalingMessage::ActorMessage { source, .. } => Some(source),
            SignalingMessage::Register { actor_id }
            | SignalingMessage::ConnectionStatus { actor_id, .. } => Some(actor_id),
            _ => None,
        }
    }
}

/// 信令服务器错误代码
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum SignalingError {
    /// 目标 Actor 未找到
    ActorNotFound(u64),
    /// 消息格式错误
    InvalidMessage(String),
    /// 连接错误
    ConnectionError(String),
    /// 认证错误
    AuthenticationError,
}

impl SignalingError {
    #[allow(dead_code)]
    pub fn code(&self) -> u32 {
        match self {
            SignalingError::ActorNotFound(_) => 404,
            SignalingError::InvalidMessage(_) => 400,
            SignalingError::ConnectionError(_) => 500,
            SignalingError::AuthenticationError => 401,
        }
    }

    #[allow(dead_code)]
    pub fn message(&self) -> String {
        match self {
            SignalingError::ActorNotFound(id) => format!("Actor {} not found", id),
            SignalingError::InvalidMessage(msg) => format!("Invalid message: {}", msg),
            SignalingError::ConnectionError(msg) => format!("Connection error: {}", msg),
            SignalingError::AuthenticationError => "Authentication required".to_string(),
        }
    }

    #[allow(dead_code)]
    pub fn into_message(self) -> SignalingMessage {
        SignalingMessage::Error {
            code: self.code(),
            message: self.message(),
        }
    }
}
