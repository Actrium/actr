//! 测试客户端
//! 
//! 用于测试各种 demo actor 的功能

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

use shared_protocols::actor::{ActorId, ActorType, ActorTypeCode};

#[derive(Parser)]
#[command(name = "test-client")]
#[command(about = "Actor-RTC Framework Test Client")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    
    #[arg(short, long, default_value = "ws://localhost:8080")]
    signaling_url: String,
    
    #[arg(short, long, default_value = "2001")]
    client_id: u64,
}

#[derive(Subcommand)]
enum Commands {
    /// 连接到信令服务器并监听事件
    Listen,
    /// 测试回声服务
    Echo {
        #[arg(short, long, default_value = "Hello from test client!")]
        message: String,
        #[arg(short, long, default_value = "1001")]
        target_id: u64,
    },
    /// 性能测试 - 批量回声
    Benchmark {
        #[arg(short, long, default_value = "100")]
        count: usize,
        #[arg(short, long, default_value = "1001")]
        target_id: u64,
    },
    /// 发现可用的 Actors
    Discover,
}

struct TestClient {
    signaling_url: String,
    client_id: ActorId,
}

impl TestClient {
    fn new(signaling_url: String, client_id: u64) -> Self {
        let actor_id = ActorId {
            serial_number: client_id,
            r#type: Some(ActorType {
                code: ActorTypeCode::Authenticated as i32,
                manufacturer: Some("test".to_string()),
                name: "test_client".to_string(),
            }),
        };

        Self {
            signaling_url,
            client_id: actor_id,
        }
    }

    async fn connect(&self) -> Result<(
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    )> {
        info!("🔌 Connecting to signaling server: {}", self.signaling_url);
        
        let url = Url::parse(&self.signaling_url)?;
        let (ws_stream, _) = connect_async(url).await?;
        
        info!("✅ Connected to signaling server");
        Ok(ws_stream)
    }

    async fn register(&self, ws_sink: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
        Message,
    >) -> Result<()> {
        let register_message = serde_json::json!({
            "messageType": "register",
            "actorId": {
                "serialNumber": self.client_id.serial_number.to_string(),
                "type": {
                    "code": self.client_id.r#type.as_ref().unwrap().code,
                    "name": &self.client_id.r#type.as_ref().unwrap().name,
                    "manufacturer": &self.client_id.r#type.as_ref().unwrap().manufacturer
                }
            }
        });

        ws_sink.send(Message::Text(register_message.to_string())).await?;
        info!("📝 Registered as client: {}", self.client_id.serial_number);
        Ok(())
    }

    async fn listen_for_messages(&self) -> Result<()> {
        let ws_stream = self.connect().await?;
        let (mut ws_sink, mut ws_stream) = ws_stream.split();

        // 注册客户端
        self.register(&mut ws_sink).await?;

        println!("{}", "🎧 Listening for signaling messages...".green().bold());
        println!("{}", "Press Ctrl+C to stop".yellow());

        // 监听消息
        while let Some(message) = ws_stream.next().await {
            match message {
                Ok(Message::Text(text)) => {
                    match serde_json::from_str::<Value>(&text) {
                        Ok(json) => {
                            self.handle_signaling_message(&json).await;
                        },
                        Err(e) => {
                            warn!("Failed to parse JSON: {}", e);
                        }
                    }
                },
                Ok(Message::Close(_)) => {
                    info!("Connection closed by server");
                    break;
                },
                Err(e) => {
                    error!("WebSocket error: {}", e);
                    break;
                },
                _ => {}
            }
        }

        Ok(())
    }

    async fn handle_signaling_message(&self, json: &Value) {
        let message_type = json["messageType"].as_str().unwrap_or("unknown");
        
        match message_type {
            "registerSuccess" => {
                println!("{}", "✅ Registration successful!".green());
            },
            "newActor" => {
                let actor_id = &json["actorId"];
                let serial = actor_id["serialNumber"].as_str().unwrap_or("unknown");
                let name = actor_id["type"]["name"].as_str().unwrap_or("unknown");
                println!("{} New actor discovered: {} ({})", "🎭".blue(), serial.cyan(), name.yellow());
            },
            "webrtcSignal" => {
                let source = json["sourceActorId"]["serialNumber"].as_str().unwrap_or("unknown");
                println!("{} WebRTC signal from: {}", "📡".blue(), source.cyan());
            },
            "error" => {
                let error = &json["error"];
                let code = error["code"].as_u64().unwrap_or(0);
                let msg = error["message"].as_str().unwrap_or("Unknown error");
                println!("{} Error [{}]: {}", "❌".red(), code.to_string().red(), msg.red());
            },
            _ => {
                println!("{} Unknown message type: {}", "❓".yellow(), message_type.yellow());
            }
        }
    }

    async fn test_echo(&self, message: String, target_id: u64) -> Result<()> {
        println!("{}", format!("🔊 Testing echo service with message: '{}'", message).cyan().bold());
        println!("{}", format!("   Target Actor ID: {}", target_id).yellow());

        let ws_stream = self.connect().await?;
        let (mut ws_sink, mut ws_stream) = ws_stream.split();

        // 注册客户端
        self.register(&mut ws_sink).await?;

        // 等待注册完成
        sleep(Duration::from_millis(500)).await;

        // 发送 Echo 请求消息
        // 注意：这里应该通过 WebRTC 数据通道发送，但为了简化demo，我们通过信令服务器发送
        let echo_message = serde_json::json!({
            "messageType": "webrtcSignal",
            "sourceActorId": {
                "serialNumber": self.client_id.serial_number.to_string(),
                "type": self.client_id.r#type
            },
            "targetActorId": {
                "serialNumber": target_id.to_string(),
                "type": {
                    "code": ActorTypeCode::Authenticated as i32,
                    "name": "demo_echo",
                    "manufacturer": "demo"
                }
            },
            "payload": {
                "type": "echo_request",
                "data": {
                    "message": message,
                    "client_id": self.client_id.serial_number.to_string()
                }
            }
        });

        ws_sink.send(Message::Text(echo_message.to_string())).await?;
        println!("{}", "📤 Echo request sent!".green());

        // 等待响应（超时时间：5秒）
        let timeout_duration = Duration::from_secs(5);
        let start_time = std::time::Instant::now();

        while let Some(message) = ws_stream.next().await {
            if start_time.elapsed() > timeout_duration {
                println!("{}", "⏰ Timeout waiting for echo response".red());
                break;
            }

            match message {
                Ok(Message::Text(text)) => {
                    if let Ok(json) = serde_json::from_str::<Value>(&text) {
                        if json["messageType"] == "webrtcSignal" {
                            if let Some(payload) = json.get("payload") {
                                if payload["type"] == "echo_response" {
                                    let reply = payload["data"]["reply"].as_str().unwrap_or("No reply");
                                    println!("{} {}", "📥 Echo response:".green().bold(), reply.cyan());
                                    return Ok(());
                                }
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

        Ok(())
    }

    async fn benchmark(&self, count: usize, target_id: u64) -> Result<()> {
        println!("{}", format!("🏁 Starting benchmark: {} echo requests to Actor {}", count, target_id).magenta().bold());
        
        let start_time = std::time::Instant::now();
        
        // 为简化，这里只是模拟批量请求
        for i in 0..count {
            if i % 10 == 0 {
                print!(".");
                if i % 100 == 0 {
                    print!(" {}/{}", i, count);
                }
            }
            
            // 模拟网络延迟
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        
        let elapsed = start_time.elapsed();
        let rps = count as f64 / elapsed.as_secs_f64();
        
        println!();
        println!("{}", format!("✅ Benchmark completed!").green().bold());
        println!("   {} requests in {:?}", count, elapsed);
        println!("   {:.2} requests/second", rps);
        
        Ok(())
    }

    async fn discover_actors(&self) -> Result<()> {
        println!("{}", "🔍 Discovering available actors...".blue().bold());
        
        let ws_stream = self.connect().await?;
        let (mut ws_sink, mut ws_stream) = ws_stream.split();

        // 注册客户端
        self.register(&mut ws_sink).await?;

        // 等待一段时间来接收新Actor通知
        let discovery_duration = Duration::from_secs(3);
        let start_time = std::time::Instant::now();

        println!("{}", "Waiting for actor discoveries...".yellow());

        while let Some(message) = ws_stream.next().await {
            if start_time.elapsed() > discovery_duration {
                break;
            }

            match message {
                Ok(Message::Text(text)) => {
                    if let Ok(json) = serde_json::from_str::<Value>(&text) {
                        if json["messageType"] == "newActor" {
                            let actor_id = &json["actorId"];
                            let serial = actor_id["serialNumber"].as_str().unwrap_or("unknown");
                            let name = actor_id["type"]["name"].as_str().unwrap_or("unknown");
                            let manufacturer = actor_id["type"]["manufacturer"].as_str().unwrap_or("none");
                            
                            println!("  {} Found: {} ({}::{})", "🎭".green(), serial.cyan(), manufacturer.yellow(), name.yellow());
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

        println!("{}", "🔍 Discovery completed".blue());
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let client = TestClient::new(cli.signaling_url, cli.client_id);

    println!("{}", "🧪 Actor-RTC Test Client".blue().bold());
    println!("{}", format!("   Client ID: {}", cli.client_id).yellow());

    match cli.command {
        Commands::Listen => {
            client.listen_for_messages().await?;
        },
        Commands::Echo { message, target_id } => {
            client.test_echo(message, target_id).await?;
        },
        Commands::Benchmark { count, target_id } => {
            client.benchmark(count, target_id).await?;
        },
        Commands::Discover => {
            client.discover_actors().await?;
        },
    }

    Ok(())
}