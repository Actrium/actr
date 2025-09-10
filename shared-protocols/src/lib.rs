//! 共享协议定义
//!
//! 此 crate 包含了所有 demo 程序使用的 protobuf 协议定义。

// 框架核心协议
pub mod webrtc {
    tonic::include_proto!("webrtc");
}

pub mod actor {
    tonic::include_proto!("actor");
}

pub mod signaling {
    tonic::include_proto!("signaling");
}

// Demo 服务协议
pub mod echo {
    tonic::include_proto!("echo");
}

pub mod media_streaming {
    tonic::include_proto!("media_streaming");
}

pub mod file_transfer {
    tonic::include_proto!("file_transfer");
}

pub mod stream_test {
    tonic::include_proto!("stream_test");
}

// 重新导出常用类型
pub use actor::{ActorId, ActorType, ActorTypeCode};
pub use signaling::{NewActor, SignalingMessage, WebRtcSignal};
pub use webrtc::{IceCandidate, SessionDescription};

// 工具函数
impl ActorId {
    /// 创建一个新的 Actor ID
    pub fn new(serial_number: u64, type_code: ActorTypeCode, name: String) -> Self {
        Self {
            serial_number,
            r#type: Some(ActorType {
                code: type_code as i32,
                manufacturer: None,
                name,
            }),
        }
    }

    /// 创建带厂商前缀的 Actor ID
    pub fn new_with_manufacturer(
        serial_number: u64,
        type_code: ActorTypeCode,
        manufacturer: String,
        name: String,
    ) -> Self {
        Self {
            serial_number,
            r#type: Some(ActorType {
                code: type_code as i32,
                manufacturer: Some(manufacturer),
                name,
            }),
        }
    }

    /// 创建默认的匿名 Actor ID
    pub fn anonymous() -> Self {
        Self::new(0, ActorTypeCode::Anonymous, "unknown".to_string())
    }
}
