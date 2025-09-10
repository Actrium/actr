//! 信令服务器测试工具
//! 
//! 用于测试信令服务器的功能和性能

use std::time::Duration;
use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::Colorize;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tracing::{info, warn, error, Level};
use url::Url;
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "signaling-test")]
#[command(about = "Signaling Server Test Utility")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    
    #[arg(short, long, default_value = "ws://localhost:8080")]
    server_url: String,
}

#[derive(Subcommand)]
enum Commands {
    /// 测试连接到信令服务器
    Connect,
    /// 测试多个客户端连接
    MultiConnect {
        #[arg(short, long, default_value = "5")]
        count: usize,
    },
    /// 测试 Actor 注册和发现
    Discovery {
        #[arg(short, long, default_value = "3")]
        actors: usize,
    },
    /// 测试信令消息转发
    MessageRelay,
    /// 负载测试
    LoadTest {
        #[arg(short, long, default_value = "10")]
        concurrent: usize,
        #[arg(short, long, default_value = "100")]
        messages: usize,
    },
}

struct SignalingTester {
    server_url: String,
}

impl SignalingTester {
    fn new(server_url: String) -> Self {
        Self { server_url }
    }

    async fn test_connection(&self) -> Result<()> {
        println!("{}", "🔌 Testing connection to signaling server...".cyan().bold());
        
        let url = Url::parse(&self.server_url)?;
        let (ws_stream, _) = connect_async(url).await?;
        let (mut ws_sink, mut ws_stream) = ws_stream.split();

        // 发送注册消息
        let actor_id = rand::random::<u64>();
        let register_message = serde_json::json!({
            "messageType": "register",
            "actorId": {
                "serialNumber": actor_id.to_string(),
                "type": {
                    "code": 2,
                    "name": "test_actor",
                    "manufacturer": "test"
                }
            }
        });

        ws_sink.send(Message::Text(register_message.to_string())).await?;
        println!("{} Sent registration for Actor ID: {}", "📝".green(), actor_id.to_string().cyan());

        // 等待响应
        let timeout = Duration::from_secs(5);
        let start_time = std::time::Instant::now();

        while let Some(message) = ws_stream.next().await {
            if start_time.elapsed() > timeout {
                println!("{}", "⏰ Timeout waiting for response".red());
                break;
            }

            match message {
                Ok(Message::Text(text)) => {
                    if let Ok(json) = serde_json::from_str::<Value>(&text) {
                        let message_type = json["messageType"].as_str().unwrap_or("unknown");
                        
                        match message_type {
                            "registerSuccess" => {
                                println!("{} Registration successful!", "✅".green());
                                break;
                            },
                            "error" => {
                                let error = &json["error"];
                                let code = error["code"].as_u64().unwrap_or(0);
                                let msg = error["message"].as_str().unwrap_or("Unknown error");
                                println!("{} Registration failed [{}]: {}", "❌".red(), code, msg);
                                break;
                            },
                            _ => {
                                println!("{} Unexpected message: {}", "❓".yellow(), message_type);
                            }
                        }
                    }
                },
                Ok(Message::Close(_)) => {
                    println!("{}", "Connection closed by server".yellow());
                    break;
                },
                Err(e) => {
                    error!("WebSocket error: {}", e);
                    break;
                },
                _ => {}
            }
        }

        // 关闭连接
        ws_sink.send(Message::Close(None)).await.ok();
        println!("{}", "✅ Connection test completed".green());

        Ok(())
    }

    async fn test_multi_connect(&self, count: usize) -> Result<()> {
        println!("{}", format!("🔗 Testing {} concurrent connections...", count).cyan().bold());

        let mut handles = Vec::new();
        
        for i in 0..count {
            let server_url = self.server_url.clone();
            let handle = tokio::spawn(async move {
                let client_id = 3000 + i as u64;
                Self::single_connection_test(server_url, client_id).await
            });
            handles.push(handle);
        }

        // 等待所有连接完成
        let mut success_count = 0;
        for handle in handles {
            match handle.await {
                Ok(Ok(_)) => success_count += 1,
                Ok(Err(e)) => warn!("Connection failed: {}", e),
                Err(e) => warn!("Task failed: {}", e),
            }
        }

        println!("{}", format!("✅ Multi-connection test completed: {}/{} successful", success_count, count).green());
        Ok(())
    }

    async fn single_connection_test(server_url: String, actor_id: u64) -> Result<()> {
        let url = Url::parse(&server_url)?;
        let (ws_stream, _) = connect_async(url).await?;
        let (mut ws_sink, mut ws_stream) = ws_stream.split();

        // 注册 Actor
        let register_message = serde_json::json!({
            "messageType": "register",
            "actorId": {
                "serialNumber": actor_id.to_string(),
                "type": {
                    "code": 2,
                    "name": format!("test_actor_{}", actor_id),
                    "manufacturer": "test"
                }
            }
        });

        ws_sink.send(Message::Text(register_message.to_string())).await?;

        // 等待注册确认
        if let Some(Ok(Message::Text(text))) = ws_stream.next().await {
            if let Ok(json) = serde_json::from_str::<Value>(&text) {
                if json["messageType"] == "registerSuccess" {
                    println!("{} Actor {} registered", "✅".green(), actor_id.to_string().cyan());
                }
            }
        }

        // 保持连接一小段时间
        sleep(Duration::from_millis(100)).await;

        // 关闭连接
        ws_sink.send(Message::Close(None)).await.ok();
        Ok(())
    }

    async fn test_discovery(&self, actor_count: usize) -> Result<()> {
        println!("{}", format!("🔍 Testing actor discovery with {} actors...", actor_count).cyan().bold());

        let mut connections = Vec::new();

        // 创建多个 Actor 连接
        for i in 0..actor_count {
            let actor_id = 4000 + i as u64;
            let url = Url::parse(&self.server_url)?;
            let (ws_stream, _) = connect_async(url).await?;
            let (mut ws_sink, ws_stream) = ws_stream.split();

            // 注册 Actor
            let register_message = serde_json::json!({
                "messageType": "register",
                "actorId": {
                    "serialNumber": actor_id.to_string(),
                    "type": {
                        "code": 2,
                        "name": format!("discovery_test_{}", i),
                        "manufacturer": "test"
                    }
                }
            });

            ws_sink.send(Message::Text(register_message.to_string())).await?;
            connections.push((actor_id, ws_sink, ws_stream));
            
            // 稍微延迟，让服务器处理注册
            sleep(Duration::from_millis(50)).await;
        }

        println!("{} Created {} actor connections", "✅".green(), actor_count);

        // 监听新 Actor 发现消息
        let discovery_timeout = Duration::from_secs(2);
        let mut total_discoveries = 0;

        for (actor_id, _, mut ws_stream) in connections {
            println!("{} Listening for discoveries on Actor {}...", "👂".blue(), actor_id.to_string().cyan());
            
            let start_time = std::time::Instant::now();
            let mut discoveries = 0;

            while let Some(message) = ws_stream.next().await {
                if start_time.elapsed() > discovery_timeout {
                    break;
                }

                match message {
                    Ok(Message::Text(text)) => {
                        if let Ok(json) = serde_json::from_str::<Value>(&text) {
                            match json["messageType"].as_str() {
                                Some("registerSuccess") => {
                                    println!("  {} Registration confirmed", "✅".green());
                                },
                                Some("newActor") => {
                                    let discovered_id = json["actorId"]["serialNumber"].as_str().unwrap_or("unknown");
                                    println!("  {} Discovered Actor: {}", "🎭".yellow(), discovered_id.cyan());
                                    discoveries += 1;
                                },
                                _ => {}
                            }
                        }
                    },
                    Ok(Message::Close(_)) => break,
                    Err(e) => {
                        warn!("WebSocket error on Actor {}: {}", actor_id, e);
                        break;
                    },
                    _ => {}
                }
            }

            total_discoveries += discoveries;
            println!("  {} Actor {} discovered {} peers", "📊".blue(), actor_id.to_string().cyan(), discoveries.to_string().yellow());
        }

        println!("{}", format!("✅ Discovery test completed. Total discoveries: {}", total_discoveries).green());
        Ok(())
    }

    async fn test_message_relay(&self) -> Result<()> {
        println!("{}", "📡 Testing message relay between actors...".cyan().bold());

        // 创建两个 Actor 连接
        let actor_a_id = 5001;
        let actor_b_id = 5002;

        // Actor A
        let url_a = Url::parse(&self.server_url)?;
        let (ws_stream_a, _) = connect_async(url_a).await?;
        let (mut ws_sink_a, mut ws_stream_a) = ws_stream_a.split();

        // Actor B
        let url_b = Url::parse(&self.server_url)?;
        let (ws_stream_b, _) = connect_async(url_b).await?;
        let (mut ws_sink_b, mut ws_stream_b) = ws_stream_b.split();

        // 注册 Actor A
        let register_a = serde_json::json!({
            "messageType": "register",
            "actorId": {
                "serialNumber": actor_a_id.to_string(),
                "type": { "code": 2, "name": "relay_test_a", "manufacturer": "test" }
            }
        });
        ws_sink_a.send(Message::Text(register_a.to_string())).await?;

        // 注册 Actor B
        let register_b = serde_json::json!({
            "messageType": "register",
            "actorId": {
                "serialNumber": actor_b_id.to_string(),
                "type": { "code": 2, "name": "relay_test_b", "manufacturer": "test" }
            }
        });
        ws_sink_b.send(Message::Text(register_b.to_string())).await?;

        // 等待注册完成
        sleep(Duration::from_millis(500)).await;

        // Actor A 向 Actor B 发送消息
        let test_payload = Uuid::new_v4().to_string();
        let relay_message = serde_json::json!({
            "messageType": "webrtcSignal",
            "sourceActorId": {
                "serialNumber": actor_a_id.to_string(),
                "type": { "code": 2, "name": "relay_test_a", "manufacturer": "test" }
            },
            "targetActorId": {
                "serialNumber": actor_b_id.to_string(),
                "type": { "code": 2, "name": "relay_test_b", "manufacturer": "test" }
            },
            "payload": {
                "test_data": test_payload
            }
        });

        ws_sink_a.send(Message::Text(relay_message.to_string())).await?;
        println!("{} Sent test message from Actor {} to Actor {}", "📤".green(), actor_a_id.to_string().cyan(), actor_b_id.to_string().cyan());
        println!("  Test payload: {}", test_payload.yellow());

        // 等待 Actor B 接收消息
        let timeout = Duration::from_secs(3);
        let start_time = std::time::Instant::now();
        let mut message_received = false;

        while let Some(message) = ws_stream_b.next().await {
            if start_time.elapsed() > timeout {
                break;
            }

            match message {
                Ok(Message::Text(text)) => {
                    if let Ok(json) = serde_json::from_str::<Value>(&text) {
                        if json["messageType"] == "webrtcSignal" {
                            let received_payload = json["payload"]["test_data"].as_str().unwrap_or("");
                            if received_payload == test_payload {
                                println!("{} Message successfully relayed!", "✅".green());
                                println!("  Received payload: {}", received_payload.yellow());
                                message_received = true;
                                break;
                            }
                        }
                    }
                },
                Ok(Message::Close(_)) => break,
                Err(e) => {
                    error!("WebSocket error: {}", e);
                    break;
                },
                _ => {}
            }
        }

        if !message_received {
            println!("{}", "❌ Message relay test failed - no message received".red());
        }

        // 关闭连接
        ws_sink_a.send(Message::Close(None)).await.ok();
        ws_sink_b.send(Message::Close(None)).await.ok();

        Ok(())
    }

    async fn load_test(&self, concurrent: usize, messages: usize) -> Result<()> {
        println!("{}", format!("🏋️ Running load test: {} concurrent connections, {} messages each", concurrent, messages).cyan().bold());

        let start_time = std::time::Instant::now();
        let mut handles = Vec::new();

        for i in 0..concurrent {
            let server_url = self.server_url.clone();
            let handle = tokio::spawn(async move {
                let actor_id = 6000 + i as u64;
                Self::load_test_worker(server_url, actor_id, messages).await
            });
            handles.push(handle);
        }

        // 等待所有工作线程完成
        let mut total_messages = 0;
        let mut successful_connections = 0;

        for handle in handles {
            match handle.await {
                Ok(Ok(count)) => {
                    total_messages += count;
                    successful_connections += 1;
                },
                Ok(Err(e)) => warn!("Load test worker failed: {}", e),
                Err(e) => warn!("Task failed: {}", e),
            }
        }

        let elapsed = start_time.elapsed();
        let messages_per_second = total_messages as f64 / elapsed.as_secs_f64();

        println!("{}", "✅ Load test completed!".green().bold());
        println!("  {} successful connections", successful_connections);
        println!("  {} total messages sent", total_messages);
        println!("  {:.2?} elapsed time", elapsed);
        println!("  {:.2} messages/second", messages_per_second);

        Ok(())
    }

    async fn load_test_worker(server_url: String, actor_id: u64, message_count: usize) -> Result<usize> {
        let url = Url::parse(&server_url)?;
        let (ws_stream, _) = connect_async(url).await?;
        let (mut ws_sink, _) = ws_stream.split();

        // 注册
        let register_message = serde_json::json!({
            "messageType": "register",
            "actorId": {
                "serialNumber": actor_id.to_string(),
                "type": { "code": 2, "name": format!("load_test_{}", actor_id), "manufacturer": "test" }
            }
        });
        ws_sink.send(Message::Text(register_message.to_string())).await?;

        // 等待注册完成
        sleep(Duration::from_millis(10)).await;

        // 发送测试消息
        let mut sent_count = 0;
        for i in 0..message_count {
            let test_message = serde_json::json!({
                "messageType": "webrtcSignal",
                "sourceActorId": {
                    "serialNumber": actor_id.to_string(),
                    "type": { "code": 2, "name": format!("load_test_{}", actor_id), "manufacturer": "test" }
                },
                "targetActorId": {
                    "serialNumber": (actor_id + 1).to_string(),
                    "type": { "code": 2, "name": "dummy_target", "manufacturer": "test" }
                },
                "payload": { "message_index": i }
            });

            if ws_sink.send(Message::Text(test_message.to_string())).await.is_ok() {
                sent_count += 1;
            }
        }

        // 关闭连接
        ws_sink.send(Message::Close(None)).await.ok();
        Ok(sent_count)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let tester = SignalingTester::new(cli.server_url.clone());

    println!("{}", "🧪 Signaling Server Test Utility".blue().bold());
    println!("{}", format!("   Server URL: {}", cli.server_url).yellow());

    match cli.command {
        Commands::Connect => {
            tester.test_connection().await?;
        },
        Commands::MultiConnect { count } => {
            tester.test_multi_connect(count).await?;
        },
        Commands::Discovery { actors } => {
            tester.test_discovery(actors).await?;
        },
        Commands::MessageRelay => {
            tester.test_message_relay().await?;
        },
        Commands::LoadTest { concurrent, messages } => {
            tester.load_test(concurrent, messages).await?;
        },
    }

    Ok(())
}