use futures_util::{SinkExt, StreamExt};
use shared_protocols::actor::{ActorId, ActorType};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{RwLock, broadcast};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{error, info, warn};
use uuid::Uuid;

mod signaling;
use signaling::*;

/// 信令服务器状态
#[derive(Debug)]
pub struct SignalingServer {
    /// 已连接的客户端
    clients: Arc<RwLock<HashMap<String, ClientConnection>>>,
    /// 广播通道，用于向所有客户端发送消息
    broadcast_tx: broadcast::Sender<SignalingMessage>,
}

/// 客户端连接信息
#[derive(Debug)]
pub struct ClientConnection {
    pub id: String,
    pub actor_id: Option<ActorId>,
    pub addr: SocketAddr,
}

impl SignalingServer {
    pub fn new() -> Self {
        let (broadcast_tx, _) = broadcast::channel(1000);

        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
            broadcast_tx,
        }
    }

    /// 启动信令服务器
    pub async fn start(&self, addr: &str) -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(addr).await?;
        info!("🚀 Actor-RTC 信令服务器启动在 {}", addr);

        while let Ok((stream, addr)) = listener.accept().await {
            let clients = self.clients.clone();
            let broadcast_tx = self.broadcast_tx.clone();

            tokio::spawn(async move {
                if let Err(e) = handle_connection(stream, addr, clients, broadcast_tx).await {
                    error!("处理连接时发生错误: {}", e);
                }
            });
        }

        Ok(())
    }
}

/// 处理单个客户端连接
async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    clients: Arc<RwLock<HashMap<String, ClientConnection>>>,
    broadcast_tx: broadcast::Sender<SignalingMessage>,
) -> Result<(), Box<dyn std::error::Error>> {
    let client_id = Uuid::new_v4().to_string();
    info!("🔗 新客户端连接: {} ({})", client_id, addr);

    // 升级到 WebSocket 连接
    let ws_stream = accept_async(stream).await?;
    let mut broadcast_rx = broadcast_tx.subscribe();

    // 注册客户端
    {
        let mut clients_guard = clients.write().await;
        clients_guard.insert(
            client_id.clone(),
            ClientConnection {
                id: client_id.clone(),
                actor_id: None,
                addr,
            },
        );
    }

    // 分离读写流
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // 处理客户端消息的任务
    let clients_for_receive = clients.clone();
    let broadcast_tx_for_receive = broadcast_tx.clone();
    let client_id_for_receive = client_id.clone();

    let receive_task = tokio::spawn(async move {
        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Err(e) = handle_client_message(
                        &text,
                        &client_id_for_receive,
                        &clients_for_receive,
                        &broadcast_tx_for_receive,
                    )
                    .await
                    {
                        error!("处理客户端消息错误: {}", e);
                        break;
                    }
                }
                Ok(Message::Close(_)) => {
                    info!("客户端 {} 主动断开连接", client_id_for_receive);
                    break;
                }
                Err(e) => {
                    error!("WebSocket 错误: {}", e);
                    break;
                }
                _ => {}
            }
        }

        // 清理客户端
        cleanup_client(&client_id_for_receive, &clients_for_receive).await;
    });

    // 处理广播消息的任务
    let send_task = tokio::spawn(async move {
        while let Ok(msg) = broadcast_rx.recv().await {
            if let Ok(json) = serde_json::to_string(&msg) {
                if ws_sender.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
        }
    });

    // 等待任一任务完成
    tokio::select! {
        _ = receive_task => {},
        _ = send_task => {},
    }

    // 清理客户端连接
    cleanup_client(&client_id, &clients).await;
    info!("🔌 客户端 {} 已断开连接", client_id);

    Ok(())
}

/// 处理客户端发送的消息
async fn handle_client_message(
    text: &str,
    client_id: &str,
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
    broadcast_tx: &broadcast::Sender<SignalingMessage>,
) -> Result<(), Box<dyn std::error::Error>> {
    // 首先解析为 JSON 值来处理格式不匹配
    let json_value: serde_json::Value = serde_json::from_str(text)?;

    // 检查消息类型
    if let Some(message_type) = json_value.get("messageType").and_then(|v| v.as_str()) {
        match message_type {
            "register" => {
                // 手动解析注册消息
                if let Some(actor_id_json) = json_value.get("actorId") {
                    let actor_id = parse_actor_id_from_json(actor_id_json)?;

                    // 处理注册逻辑
                    let mut clients_guard = clients.write().await;
                    if let Some(client) = clients_guard.get_mut(client_id) {
                        client.actor_id = Some(actor_id.clone());
                        info!(
                            "📝 客户端 {} 注册为 Actor {}",
                            client_id, actor_id.serial_number
                        );
                    }
                }
            }
            _ => {
                // 对于其他消息类型，尝试标准反序列化
                let msg: SignalingMessage = serde_json::from_str(text)?;
                match &msg {
                    SignalingMessage::Offer { target, .. }
                    | SignalingMessage::Answer { target, .. }
                    | SignalingMessage::IceCandidate { target, .. } => {
                        route_message_to_target(&msg, target, clients).await?;
                    }
                    _ => {
                        let _ = broadcast_tx.send(msg);
                    }
                }
            }
        }
    }

    Ok(())
}

/// 从 JSON 解析 ActorId（处理格式不匹配）
fn parse_actor_id_from_json(
    json: &serde_json::Value,
) -> Result<ActorId, Box<dyn std::error::Error>> {
    let serial_number = json
        .get("serialNumber")
        .and_then(|v| v.as_str())
        .ok_or("Missing serialNumber")?
        .parse::<u64>()?;

    let actor_type = if let Some(type_json) = json.get("type") {
        Some(ActorType {
            code: type_json.get("code").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
            name: type_json
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            manufacturer: type_json
                .get("manufacturer")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        })
    } else {
        None
    };

    Ok(ActorId {
        serial_number,
        r#type: actor_type,
    })
}

/// 将消息路由到目标 Actor
async fn route_message_to_target(
    msg: &SignalingMessage,
    target: &ActorId,
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let clients_guard = clients.read().await;

    // 查找目标 Actor 的客户端连接
    let target_client = clients_guard.values().find(|client| {
        client
            .actor_id
            .as_ref()
            .map_or(false, |id| id.serial_number == target.serial_number)
    });

    if let Some(_target_client) = target_client {
        info!(
            "📤 路由消息到 Actor {}: {:?}",
            target.serial_number,
            msg.message_type()
        );
        // 这里需要直接发送给特定客户端，暂时记录日志
        // 在实际实现中，我们需要维护每个客户端的发送通道
    } else {
        warn!("⚠️ 未找到目标 Actor {}", target.serial_number);
    }

    Ok(())
}

/// 清理客户端连接
async fn cleanup_client(client_id: &str, clients: &Arc<RwLock<HashMap<String, ClientConnection>>>) {
    let mut clients_guard = clients.write().await;
    if let Some(client) = clients_guard.remove(client_id) {
        if let Some(actor_id) = client.actor_id {
            info!("🧹 清理 Actor {} 的连接", actor_id.serial_number);
        }
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

    let server = SignalingServer::new();
    let addr = std::env::var("SIGNALING_ADDR").unwrap_or_else(|_| "0.0.0.0:8081".to_string());

    server.start(&addr).await?;
    Ok(())
}
