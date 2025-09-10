//! 信令适配器接口和 WebSocket 实现

use crate::error::{ActorResult, SignalingError};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use shared_protocols::actor::ActorId;
use shared_protocols::signaling::SignalingMessage;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};
use url::Url;

/// 信令适配器 trait
///
/// 实现此 trait 以支持不同的信令协议。
/// 框架通过此接口与信令服务器通信。
#[async_trait]
pub trait SignalingAdapter: Send + Sync {
    /// 连接到信令服务器
    async fn connect(&mut self) -> ActorResult<()>;

    /// 注册 Actor 到信令服务器
    async fn register_actor(&mut self, actor_id: &ActorId) -> ActorResult<()>;

    /// 发送信令消息
    async fn send_signal(&mut self, message: SignalingMessage) -> ActorResult<()>;

    /// 接收信令消息流
    ///
    /// 返回一个接收器，框架会监听此接收器来处理信令消息。
    /// 注意：此方法只能调用一次，后续调用会返回错误。
    async fn receive_signals(&mut self) -> ActorResult<mpsc::UnboundedReceiver<SignalingMessage>>;

    /// 断开连接
    async fn disconnect(&mut self) -> ActorResult<()>;

    /// 获取连接状态
    fn is_connected(&self) -> bool;
}

/// WebSocket 信令适配器
pub struct WebSocketSignaling {
    server_url: String,
    ws_tx: Arc<Mutex<Option<mpsc::UnboundedSender<Message>>>>,
    signal_rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<SignalingMessage>>>>,
    connected: Arc<AtomicBool>,
}

impl WebSocketSignaling {
    /// 创建新的 WebSocket 信令适配器
    pub fn new(server_url: impl Into<String>) -> ActorResult<Self> {
        let server_url = server_url.into();

        // 验证 URL 格式
        Url::parse(&server_url)
            .map_err(|e| SignalingError::ConnectionFailed(format!("Invalid URL: {}", e)))?;

        Ok(Self {
            server_url,
            ws_tx: Arc::new(Mutex::new(None)),
            signal_rx: Arc::new(Mutex::new(None)),
            connected: Arc::new(AtomicBool::new(false)),
        })
    }

    /// 将 ActorId 转换为 JSON
    fn actor_id_to_json(&self, actor_id: &ActorId) -> Value {
        serde_json::json!({
            "serialNumber": actor_id.serial_number.to_string(),
            "type": {
                "code": actor_id.r#type.as_ref().map(|t| t.code).unwrap_or(0),
                "name": actor_id.r#type.as_ref().map(|t| t.name.as_str()).unwrap_or("unknown"),
                "manufacturer": actor_id.r#type.as_ref().and_then(|t| t.manufacturer.as_ref())
            }
        })
    }

    /// 将 JSON 转换为 ActorId
    #[allow(dead_code)]
    fn json_to_actor_id(&self, json: &Value) -> ActorResult<ActorId> {
        let serial_number = json["serialNumber"]
            .as_str()
            .ok_or_else(|| SignalingError::ParseError("Missing serialNumber".to_string()))?
            .parse::<u64>()
            .map_err(|e| SignalingError::ParseError(format!("Invalid serialNumber: {}", e)))?;

        let type_info = &json["type"];
        let actor_type = Some(shared_protocols::actor::ActorType {
            code: type_info["code"].as_i64().unwrap_or(0) as i32,
            name: type_info["name"].as_str().unwrap_or("unknown").to_string(),
            manufacturer: type_info["manufacturer"].as_str().map(|s| s.to_string()),
        });

        Ok(ActorId {
            serial_number,
            r#type: actor_type,
        })
    }

    /// 解析并发送信令消息
    async fn parse_and_send_signaling(
        signal_tx: &mpsc::UnboundedSender<SignalingMessage>,
        json: Value,
    ) -> ActorResult<()> {
        let message_type = json["messageType"]
            .as_str()
            .ok_or_else(|| SignalingError::ParseError("Missing messageType".to_string()))?;

        match message_type {
            "registerSuccess" => {
                debug!("Registration successful");
                // 可以发送一个特殊的系统消息，但现在简单忽略
            }
            "newActor" => {
                let actor_id_json = &json["actorId"];
                let actor_id = Self::static_json_to_actor_id(actor_id_json)?;

                let signaling_msg = SignalingMessage {
                    message_type: Some(
                        shared_protocols::signaling::signaling_message::MessageType::NewActor(
                            shared_protocols::signaling::NewActor {
                                actor_id: Some(actor_id),
                            },
                        ),
                    ),
                };

                signal_tx.send(signaling_msg).map_err(|_| {
                    SignalingError::NetworkError("Signal channel closed".to_string())
                })?;
            }
            "webrtcSignal" => {
                debug!("Received WebRTC signal message");
                // TODO: 完整实现 WebRTC 信令解析
                let source_actor_json = &json["sourceActorId"];
                let target_actor_json = &json["targetActorId"];

                let source_actor = Self::static_json_to_actor_id(source_actor_json)?;
                let target_actor = Self::static_json_to_actor_id(target_actor_json)?;

                let webrtc_signal = shared_protocols::signaling::WebRtcSignal {
                    source_actor_id: Some(source_actor),
                    target_actor_id: Some(target_actor),
                    payload: None, // TODO: 解析 payload
                };

                let signaling_msg = SignalingMessage {
                    message_type: Some(
                        shared_protocols::signaling::signaling_message::MessageType::WebrtcSignal(
                            webrtc_signal,
                        ),
                    ),
                };

                signal_tx.send(signaling_msg).map_err(|_| {
                    SignalingError::NetworkError("Signal channel closed".to_string())
                })?;
            }
            "error" => {
                let error = &json["error"];
                let error_msg = shared_protocols::signaling::SignalingError {
                    code: error["code"].as_u64().unwrap_or(0) as u32,
                    message: error["message"]
                        .as_str()
                        .unwrap_or("Unknown error")
                        .to_string(),
                    request_id: None,
                };

                let signaling_msg = SignalingMessage {
                    message_type: Some(
                        shared_protocols::signaling::signaling_message::MessageType::Error(
                            error_msg,
                        ),
                    ),
                };

                signal_tx.send(signaling_msg).map_err(|_| {
                    SignalingError::NetworkError("Signal channel closed".to_string())
                })?;
            }
            _ => {
                debug!("Unknown message type: {}", message_type);
            }
        }

        Ok(())
    }

    /// 静态版本的 JSON 到 ActorId 转换
    fn static_json_to_actor_id(json: &Value) -> ActorResult<ActorId> {
        let serial_number = json["serialNumber"]
            .as_str()
            .ok_or_else(|| SignalingError::ParseError("Missing serialNumber".to_string()))?
            .parse::<u64>()
            .map_err(|e| SignalingError::ParseError(format!("Invalid serialNumber: {}", e)))?;

        let type_info = &json["type"];
        let actor_type = Some(shared_protocols::actor::ActorType {
            code: type_info["code"].as_i64().unwrap_or(0) as i32,
            name: type_info["name"].as_str().unwrap_or("unknown").to_string(),
            manufacturer: type_info["manufacturer"].as_str().map(|s| s.to_string()),
        });

        Ok(ActorId {
            serial_number,
            r#type: actor_type,
        })
    }

    /// 将 SignalingMessage 转换为 JSON
    fn signaling_message_to_json(&self, message: &SignalingMessage) -> ActorResult<Value> {
        match &message.message_type {
            Some(shared_protocols::signaling::signaling_message::MessageType::WebrtcSignal(
                signal,
            )) => {
                Ok(serde_json::json!({
                    "messageType": "webrtcSignal",
                    "sourceActorId": signal.source_actor_id.as_ref().map(|id| self.actor_id_to_json(id)),
                    "targetActorId": signal.target_actor_id.as_ref().map(|id| self.actor_id_to_json(id)),
                    "payload": {} // TODO: 实现 payload 序列化
                }))
            }
            Some(shared_protocols::signaling::signaling_message::MessageType::PullAcl(_)) => {
                Ok(serde_json::json!({
                    "messageType": "pullAcl"
                }))
            }
            _ => Err(
                SignalingError::ParseError("Unsupported signaling message type".to_string()).into(),
            ),
        }
    }
}

#[async_trait]
impl SignalingAdapter for WebSocketSignaling {
    async fn connect(&mut self) -> ActorResult<()> {
        info!("Connecting to signaling server: {}", self.server_url);

        let url = Url::parse(&self.server_url)
            .map_err(|e| SignalingError::ConnectionFailed(format!("Invalid URL: {}", e)))?;

        let (ws_stream, _) = connect_async(url).await.map_err(|e| {
            SignalingError::ConnectionFailed(format!("WebSocket connection failed: {}", e))
        })?;

        let (ws_sink, ws_stream) = ws_stream.split();

        // 创建消息通道
        let (tx, rx) = mpsc::unbounded_channel::<Message>();
        let (signal_tx, signal_rx) = mpsc::unbounded_channel::<SignalingMessage>();

        // 保存发送端和接收端
        *self.ws_tx.lock().await = Some(tx);
        *self.signal_rx.lock().await = Some(signal_rx);

        // 启动发送任务
        let ws_sink = Arc::new(Mutex::new(ws_sink));
        let ws_sink_clone = ws_sink.clone();
        let connected_flag = self.connected.clone();

        tokio::spawn(async move {
            let mut rx = rx;
            while let Some(message) = rx.recv().await {
                let mut sink = ws_sink_clone.lock().await;
                if let Err(e) = sink.send(message).await {
                    error!("Failed to send WebSocket message: {}", e);
                    connected_flag.store(false, std::sync::atomic::Ordering::Relaxed);
                    break;
                }
            }
        });

        // 启动接收任务
        let signal_tx_clone = signal_tx.clone();
        let connected_flag_clone = self.connected.clone();

        tokio::spawn(async move {
            let mut ws_stream = ws_stream;
            while let Some(message) = ws_stream.next().await {
                match message {
                    Ok(Message::Text(text)) => {
                        debug!("Received signaling message: {}", text);

                        match serde_json::from_str::<Value>(&text) {
                            Ok(json) => {
                                if let Err(e) =
                                    Self::parse_and_send_signaling(&signal_tx_clone, json).await
                                {
                                    warn!("Failed to parse signaling message: {}", e);
                                }
                            }
                            Err(e) => {
                                warn!("Failed to parse JSON: {}", e);
                            }
                        }
                    }
                    Ok(Message::Close(_)) => {
                        info!("WebSocket connection closed by server");
                        connected_flag_clone.store(false, std::sync::atomic::Ordering::Relaxed);
                        break;
                    }
                    Err(e) => {
                        error!("WebSocket error: {}", e);
                        connected_flag_clone.store(false, std::sync::atomic::Ordering::Relaxed);
                        break;
                    }
                    _ => {}
                }
            }
        });

        self.connected
            .store(true, std::sync::atomic::Ordering::Relaxed);
        info!("Successfully connected to signaling server");
        Ok(())
    }

    async fn register_actor(&mut self, actor_id: &ActorId) -> ActorResult<()> {
        let register_message = serde_json::json!({
            "messageType": "register",
            "actorId": self.actor_id_to_json(actor_id)
        });

        let ws_tx = self.ws_tx.lock().await;
        if let Some(ref tx) = *ws_tx {
            tx.send(Message::Text(register_message.to_string()))
                .map_err(|_| {
                    SignalingError::NetworkError("WebSocket channel closed".to_string())
                })?;
            info!("Registered actor: {}", actor_id.serial_number);
        } else {
            return Err(
                SignalingError::ConnectionFailed("WebSocket not connected".to_string()).into(),
            );
        }

        Ok(())
    }

    async fn send_signal(&mut self, message: SignalingMessage) -> ActorResult<()> {
        let json_message = self.signaling_message_to_json(&message)?;

        let ws_tx = self.ws_tx.lock().await;
        if let Some(ref tx) = *ws_tx {
            tx.send(Message::Text(json_message.to_string()))
                .map_err(|_| {
                    SignalingError::NetworkError("WebSocket channel closed".to_string())
                })?;
            debug!("Sent signaling message");
        } else {
            return Err(
                SignalingError::ConnectionFailed("WebSocket not connected".to_string()).into(),
            );
        }

        Ok(())
    }

    async fn receive_signals(&mut self) -> ActorResult<mpsc::UnboundedReceiver<SignalingMessage>> {
        let mut signal_rx = self.signal_rx.lock().await;
        signal_rx.take().ok_or_else(|| {
            SignalingError::ProtocolError("Signal receiver already taken".to_string()).into()
        })
    }

    async fn disconnect(&mut self) -> ActorResult<()> {
        let ws_tx = self.ws_tx.lock().await;
        if let Some(ref tx) = *ws_tx {
            let _ = tx.send(Message::Close(None));
        }

        self.connected
            .store(false, std::sync::atomic::Ordering::Relaxed);
        info!("Disconnected from signaling server");
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(std::sync::atomic::Ordering::Relaxed)
    }
}
