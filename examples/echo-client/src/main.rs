//! Echo Client Demo
//!
//! 演示如何创建一个客户端 Actor 来调用 echo 服务

use actor_rtc_framework::actor::ActorSystem;
use actor_rtc_framework::context::Context;
use actor_rtc_framework::error::ActorError;
use actor_rtc_framework::lifecycle::ILifecycle;
use actor_rtc_framework::local_actor::LocalActor;
use actor_rtc_framework::routing::{AttachableActor, Route, RouteProvider};
use actor_rtc_framework::signaling::WebSocketSignaling;
use async_trait::async_trait;
use shared_protocols::actor::{ActorId, ActorType, ActorTypeCode};

// 引入生成的代码
pub mod echo {
    tonic::include_proto!("echo");
}

use echo::{EchoRequest, EchoResponse};
use std::env;
use std::sync::Arc;
use tokio::signal;
use tokio::time::{sleep, Duration};
use tracing::{error, info};

/// Echo 客户端 Actor
pub struct EchoClientActor {
    actor_id: ActorId,
    target_actor_id: u64,
}

impl EchoClientActor {
    pub fn new(actor_id: ActorId, target_actor_id: u64) -> Self {
        Self {
            actor_id,
            target_actor_id,
        }
    }

    /// 发送 echo 请求
    async fn send_echo_request(&self, counter: u32) -> Result<(), ActorError> {
        info!(
            "📤 Sending echo request #{} to Actor {}",
            counter, self.target_actor_id
        );

        let request = EchoRequest {
            message: format!("Hello from Echo Client! (Request #{})", counter),
            client_id: Some(self.actor_id.serial_number.to_string()),
        };

        // 创建目标 Actor ID
        let target_id = ActorId {
            serial_number: self.target_actor_id,
            r#type: Some(ActorType {
                code: ActorTypeCode::Predefined as i32,
                name: "SimpleEchoActor".to_string(),
                manufacturer: Some("Actor-RTC Framework".to_string()),
            }),
        };

        info!("🔄 Sending echo request: '{}'", request.message);
        info!("📍 Target: Actor {}", target_id.serial_number);

        // 模拟一些处理时间
        sleep(Duration::from_millis(100)).await;

        // 模拟收到响应
        let response = EchoResponse {
            reply: format!("Echo: {}", request.message),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        };

        info!("📥 Received echo response: '{}'", response.reply);
        info!("⏰ Response timestamp: {}", response.timestamp);

        Ok(())
    }
}

/// 实现 LocalActor trait
#[async_trait]
impl LocalActor for EchoClientActor {
    async fn initialize(&self, _ctx: Arc<Context>) -> Result<(), ActorError> {
        info!("🔧 EchoClientActor initialized");
        Ok(())
    }

    async fn start(&self, _ctx: Arc<Context>) -> Result<(), ActorError> {
        info!("🚀 EchoClientActor started");

        // 直接在这里发送请求，不使用 spawn
        // 等待系统稳定
        tokio::time::sleep(Duration::from_millis(1000)).await;

        info!("🎯 Starting echo test sequence...");

        // 发送几个echo请求进行测试
        for i in 1..=3 {
            if let Err(e) = self.send_echo_request(i).await {
                error!("❌ Failed to send echo request #{}: {}", i, e);
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        info!("✅ Echo client demo completed!");
        Ok(())
    }

    async fn stop(&self, _ctx: Arc<Context>) -> Result<(), ActorError> {
        info!("🛑 EchoClientActor stopped");
        Ok(())
    }

    async fn handle_state_message(
        &self,
        _ctx: Arc<Context>,
        message: Vec<u8>,
    ) -> Result<(), ActorError> {
        let msg_str = String::from_utf8_lossy(&message);
        info!("📨 Received state message: {}", msg_str);
        Ok(())
    }

    fn get_actor_id(&self) -> &ActorId {
        &self.actor_id
    }

    fn get_type_name(&self) -> &str {
        "EchoClientActor"
    }
}

/// 实现 ILifecycle trait
#[async_trait]
impl ILifecycle for EchoClientActor {
    async fn on_start(&self, ctx: Arc<Context>) {
        ctx.log_info("🚀 EchoClientActor lifecycle started");

        // 启动 echo 测试序列
        let self_clone = Self::new(self.actor_id.clone(), self.target_actor_id);
        tokio::spawn(async move {
            // 等待系统稳定
            tokio::time::sleep(Duration::from_millis(1000)).await;

            info!("🎯 Starting echo test sequence...");

            // 发送几个echo请求进行测试
            for i in 1..=3 {
                if let Err(e) = self_clone.send_echo_request(i).await {
                    error!("❌ Failed to send echo request #{}: {}", i, e);
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }

            info!("✅ Echo client demo completed!");
        });
    }

    async fn on_stop(&self, ctx: Arc<Context>) {
        ctx.log_info("🛑 EchoClientActor lifecycle stopped");
    }
}

/// 简单的路由适配器
pub struct EchoClientAdapter;

impl RouteProvider<EchoClientActor> for EchoClientAdapter {
    fn get_routes(_actor: Arc<EchoClientActor>) -> Vec<Route> {
        // 这个客户端不需要接收路由的消息，它主动发送请求
        vec![]
    }
}

/// 实现 AttachableActor trait
impl AttachableActor for EchoClientActor {
    type Adapter = EchoClientAdapter;
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
    let signaling_url =
        env::var("SIGNALING_URL").unwrap_or_else(|_| "ws://localhost:8081".to_string());
    let actor_id_num: u64 = env::var("ACTOR_ID")
        .unwrap_or_else(|_| "2001".to_string())
        .parse()
        .unwrap_or(2001);
    let target_actor_id: u64 = env::var("TARGET_ACTOR_ID")
        .unwrap_or_else(|_| "1001".to_string())
        .parse()
        .unwrap_or(1001);

    info!("🚀 Starting Echo Client Demo");
    info!("🔗 Signaling URL: {}", signaling_url);
    info!("🆔 Client Actor ID: {}", actor_id_num);
    info!("🎯 Target Echo Actor ID: {}", target_actor_id);

    // 创建 Actor ID
    let actor_id = ActorId {
        serial_number: actor_id_num,
        r#type: Some(ActorType {
            code: ActorTypeCode::Predefined as i32,
            name: "EchoClientActor".to_string(),
            manufacturer: Some("Actor-RTC Framework".to_string()),
        }),
    };

    // 创建 Actor 实例
    let echo_client = EchoClientActor::new(actor_id.clone(), target_actor_id);

    // 创建信令适配器
    let signaling = WebSocketSignaling::new(signaling_url)?;

    // 创建 Actor 系统
    let actor_system = ActorSystem::new(actor_id.clone())
        .with_signaling(Box::new(signaling))
        .attach(echo_client);

    info!("✅ Echo client system configured");

    // 启动系统
    info!("🎬 Starting echo client system...");
    actor_system.start().await?;
    info!("🎭 Echo client system started successfully!");

    // 等待中断信号
    info!("🎭 Echo Client Demo is running. Press Ctrl+C to stop.");

    match signal::ctrl_c().await {
        Ok(()) => {
            info!("🛑 Received interrupt signal, shutting down...");
        }
        Err(err) => {
            error!("❌ Unable to listen for shutdown signal: {}", err);
        }
    }

    info!("🏁 Echo Client Demo stopped");
    Ok(())
}
