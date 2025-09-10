//! Simple Echo Demo
//! 
//! 演示如何使用 Actor-RTC 框架创建一个简单的回声服务
//! 使用生成的代码和新的简化 attach API

use actor_rtc_framework::actor::ActorSystem;
use actor_rtc_framework::context::Context;
use actor_rtc_framework::local_actor::LocalActor;
use actor_rtc_framework::lifecycle::ILifecycle;
use actor_rtc_framework::error::ActorError;
use actor_rtc_framework::routing::AttachableActor;
use actor_rtc_framework::signaling::WebSocketSignaling;
use async_trait::async_trait;
use shared_protocols::actor::{ActorId, ActorType, ActorTypeCode};
use std::env;
use std::sync::Arc;
use tokio::signal;
use tracing::{error, info};

// 引入生成的代码
pub mod echo {
    tonic::include_proto!("echo");
}

// 引入自动生成的 Actor 代码
mod echo_service_actor {
    include!(concat!(env!("OUT_DIR"), "/echo_service_actor.rs"));
}

use echo::{EchoRequest, EchoResponse, BatchEchoRequest, BatchEchoResponse};
use echo_service_actor::{IEchoService, EchoServiceAdapter};

/// 简单的回声 Actor 实现
pub struct SimpleEchoActor {
    actor_id: ActorId,
    message_count: std::sync::atomic::AtomicU64,
}

impl SimpleEchoActor {
    pub fn new(actor_id: ActorId) -> Self {
        Self {
            actor_id,
            message_count: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

/// 实现 LocalActor trait
#[async_trait]
impl LocalActor for SimpleEchoActor {
    async fn initialize(&self, _ctx: Arc<Context>) -> Result<(), ActorError> {
        info!("🔧 SimpleEchoActor initialized");
        Ok(())
    }

    async fn start(&self, _ctx: Arc<Context>) -> Result<(), ActorError> {
        info!("🚀 SimpleEchoActor started");
        Ok(())
    }

    async fn stop(&self, _ctx: Arc<Context>) -> Result<(), ActorError> {
        info!("🛑 SimpleEchoActor stopped");
        Ok(())
    }

    async fn handle_state_message(&self, _ctx: Arc<Context>, _message: Vec<u8>) -> Result<(), ActorError> {
        info!("📨 Received state message");
        Ok(())
    }

    fn get_actor_id(&self) -> &ActorId {
        &self.actor_id
    }

    fn get_type_name(&self) -> &str {
        "SimpleEchoActor"
    }
}

/// 实现 AttachableActor trait，用于简化的 attach API
impl AttachableActor for SimpleEchoActor {
    type Adapter = EchoServiceAdapter;
}

/// 实现 ILifecycle trait
#[async_trait]
impl ILifecycle for SimpleEchoActor {
    async fn on_start(&self, ctx: Arc<Context>) {
        ctx.log_info("🚀 SimpleEchoActor lifecycle started");
    }

    async fn on_stop(&self, ctx: Arc<Context>) {
        ctx.log_info("🛑 SimpleEchoActor lifecycle stopped");
    }
}

/// 实现生成的 IEchoService trait
#[async_trait]
impl IEchoService for SimpleEchoActor {
    async fn send_echo(
        &self,
        request: EchoRequest,
        _context: Arc<Context>,
    ) -> Result<EchoResponse, ActorError> {
        let count = self
            .message_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;

        info!("📨 Received echo #{}: '{}'", count, request.message);

        let response = EchoResponse {
            reply: format!("Echo: {}", request.message),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        };

        info!("🔄 Sending response: '{}'", response.reply);
        Ok(response)
    }

    async fn batch_echo(
        &self,
        request: BatchEchoRequest,
        _context: Arc<Context>,
    ) -> Result<BatchEchoResponse, ActorError> {
        let _count = self.message_count.fetch_add(
            request.messages.len() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );

        info!("📦 Received batch with {} messages", request.messages.len());

        let replies: Vec<String> = request
            .messages
            .iter()
            .map(|msg| format!("Batch Echo: {}", msg))
            .collect();

        let response = BatchEchoResponse {
            replies,
            batch_id: request.batch_id.unwrap_or(0),
            processed_count: request.messages.len() as u32,
        };

        Ok(response)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // 获取配置
    let signaling_url = env::var("SIGNALING_URL")
        .unwrap_or_else(|_| "ws://localhost:8081".to_string());
    let actor_id_num: u64 = env::var("ACTOR_ID")
        .unwrap_or_else(|_| "1001".to_string())
        .parse()
        .unwrap_or(1001);

    info!("🚀 Starting Simple Echo Demo");
    info!("🔗 Signaling URL: {}", signaling_url);
    info!("🆔 Actor ID: {}", actor_id_num);

    // 创建 Actor ID
    let actor_id = ActorId {
        serial_number: actor_id_num,
        r#type: Some(ActorType {
            code: ActorTypeCode::Predefined as i32,
            name: "SimpleEchoActor".to_string(),
            manufacturer: Some("Actor-RTC Framework".to_string()),
        }),
    };

    // 创建 Actor 实例
    let echo_actor = SimpleEchoActor::new(actor_id.clone());

    // 创建信令适配器
    let signaling = WebSocketSignaling::new(signaling_url)?;

    // 创建 Actor 系统，使用简化的 attach API
    let actor_system = ActorSystem::new(actor_id.clone())
        .with_signaling(Box::new(signaling))
        .attach(echo_actor);  // 简化的 attach API！

    info!("✅ Actor system configured with simplified attach API");

    // 启动系统
    info!("🎬 Starting actor system...");
    actor_system.start().await?;
    info!("🎭 Actor system started successfully!");

    // 等待中断信号
    info!("🎭 Echo Demo is running. Press Ctrl+C to stop.");

    match signal::ctrl_c().await {
        Ok(()) => {
            info!("🛑 Received interrupt signal, shutting down...");
        }
        Err(err) => {
            error!("❌ Unable to listen for shutdown signal: {}", err);
        }
    }

    info!("🏁 Simple Echo Demo stopped");
    Ok(())
}