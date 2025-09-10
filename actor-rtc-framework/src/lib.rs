//! # Actor-RTC Framework
//!
//! 基于 WebRTC 和 Actor 模型的分布式实时通信框架。
//!
//! ## 核心特性
//!
//! - **宏观 Actor 模型**: 进程级别的 Actor 抽象
//! - **WebRTC 原生支持**: 内置 NAT 穿透和点对点通信
//! - **双路径处理**: 状态路径(可靠) + 快车道(低延迟)
//! - **类型安全**: 基于 Protobuf 的契约驱动开发
//! - **ACL 感知**: 访问控制的安全发现机制
//!
//! ## 快速开始
//!
//! ```rust,no_run
//! use actor_rtc_framework::prelude::*;
//! use actor_rtc_framework::routing::{RouteProvider, Route};
//! use std::sync::Arc;
//!
//! // 定义你的 Actor
//! #[derive(Default)]
//! struct MyActor;
//!
//! #[async_trait::async_trait]
//! impl ILifecycle for MyActor {
//!     async fn on_start(&self, ctx: Arc<Context>) {
//!         println!("Actor started!");
//!     }
//!     
//!     async fn on_stop(&self, ctx: Arc<Context>) {
//!         println!("Actor stopped!");
//!     }
//!     
//!     async fn on_actor_discovered(&self, _ctx: Arc<Context>, _actor_id: &ActorId) -> bool {
//!         false
//!     }
//! }
//!
//! // 实现 RouteProvider (通常由 protoc-gen-actorframework 自动生成)
//! struct MyActorAdapter;
//! impl RouteProvider<dyn ILifecycle> for MyActorAdapter {
//!     fn get_routes(_actor: Arc<dyn ILifecycle>) -> Vec<Route> {
//!         vec![]
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let actor = Arc::new(MyActor::default()) as Arc<dyn ILifecycle>;
//!     let actor_id = ActorId::new(1001, ActorTypeCode::Authenticated, "my_actor".to_string());
//!     
//!     let signaling = WebSocketSignaling::new("ws://localhost:8080")?;
//!     
//!     ActorSystem::new(actor_id)
//!         .with_signaling(Box::new(signaling))
//!         .attach::<MyActorAdapter, _>(actor)
//!         .start()
//!         .await?;
//!         
//!     Ok(())
//! }
//! ```

pub mod actor;
pub mod concurrent_handle;
pub mod context;
pub mod error;
pub mod input_handler;
pub mod lifecycle;
pub mod local_actor;
pub mod messaging;
pub mod persistent_mailbox;
pub mod remote_actor;
pub mod routing;
pub mod signaling;
pub mod webrtc;

// 重新导出核心类型
pub use actor::*;
pub use context::*;
pub use error::*;
pub use lifecycle::*;
pub use local_actor::*;
pub use messaging::*;
pub use remote_actor::*;
pub use routing::{Route, RouteProvider};
pub use signaling::*;

// 重新导出来自 shared-protocols 的类型
pub use shared_protocols::actor::{ActorId, ActorType, ActorTypeCode};
pub use shared_protocols::signaling::{NewActor, SignalingMessage};

/// 预导入模块，包含最常用的类型和 trait
pub mod prelude {
    pub use crate::actor::*;
    pub use crate::context::*;
    pub use crate::error::*;
    pub use crate::lifecycle::*;
    pub use crate::local_actor::*;
    pub use crate::messaging::*;
    pub use crate::remote_actor::*;
    pub use crate::routing::{Route, RouteProvider};
    pub use crate::signaling::*;

    pub use anyhow::Result;
    pub use async_trait::async_trait;
    pub use shared_protocols::actor::{ActorId, ActorType, ActorTypeCode};
    pub use std::sync::Arc;
}
