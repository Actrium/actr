//! WebSocketServer - inbound WebSocket connection listener.
//!
//! Binds a TCP port and accepts WebSocket connections initiated by peer nodes in direct-connect mode.
//! Each inbound connection is wrapped as `InboundWsConn` and passed to `WebSocketGate` through an `mpsc` channel.
//!
//! ## Sender Identification and Authentication
//!
//! Connecting peers should send the following headers in the HTTP upgrade request:
//! ```text
//! X-Actr-Source-ID:  <hex-encoded protobuf ActrId bytes>
//! X-Actr-Credential: <base64-encoded proto AIdCredential bytes> (optional)
//! ```
//! - If `X-Actr-Source-ID` is missing, `source_id_bytes` stays empty and response routing fails.
//! - `X-Actr-Credential` is used for Ed25519 signature verification in the gate layer; when missing, connection rejection depends on configuration.

use super::connection::WebSocketConnection;
use actr_protocol::AIdCredential;
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{ActorResult, ActrError};
use std::net::SocketAddr;
use std::sync::Mutex as StdMutex;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::MaybeTlsStream;
use tokio_util::sync::CancellationToken;

/// Capacity of the inbound WebSocket connection notification channel.
const ACCEPT_CHANNEL_CAPACITY: usize = 64;

/// Inbound connection info: connection instance, sender `ActrId` bytes, and optional sender `AIdCredential`.
pub(crate) type InboundWsConn = (WebSocketConnection, Vec<u8>, Option<AIdCredential>);

/// WebSocketServer listening for inbound WebSocket connections.
///
/// The channel carries `InboundWsConn = (WebSocketConnection, Vec<u8>, Option<AIdCredential>)`.
///
/// # Usage
/// ```rust,ignore
/// let (server, mut rx) = WebSocketServer::bind(8090).await?;
/// server.start(shutdown_token);
///
/// while let Some((conn, source_id, credential)) = rx.recv().await {
///     gate.handle_inbound(conn, source_id, credential).await;
/// }
/// ```
pub(crate) struct WebSocketServer {
    listener: TcpListener,
    conn_tx: mpsc::Sender<InboundWsConn>,
    local_addr: SocketAddr,
}

impl WebSocketServer {
    /// Bind to the given port and return the server plus the inbound receiver.
    pub async fn bind(port: u16) -> ActorResult<(Self, mpsc::Receiver<InboundWsConn>)> {
        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        let listener = TcpListener::bind(addr).await.map_err(|e| {
            ActrError::Internal(format!("WebSocketServer: failed to bind port {port}: {e}"))
        })?;
        let local_addr = listener.local_addr().map_err(|e| {
            ActrError::Internal(format!("WebSocketServer: failed to get local addr: {e}"))
        })?;

        let (conn_tx, conn_rx) = mpsc::channel(ACCEPT_CHANNEL_CAPACITY);

        tracing::info!("🔌 WebSocketServer bound on {}", local_addr);

        Ok((
            Self {
                listener,
                conn_tx,
                local_addr,
            },
            conn_rx,
        ))
    }

    /// Start the accept loop in a background task.
    ///
    /// Each accepted TCP connection is upgraded to WebSocket, then wrapped as
    /// `InboundWsConn` and sent through the channel for identity verification by `WebSocketGate`.
    ///
    /// Sender identity is conveyed through these HTTP headers:
    /// - `X-Actr-Source-ID`: hex-encoded protobuf `ActrId` bytes
    /// - `X-Actr-Credential`: base64-encoded protobuf `AIdCredential` bytes used for Ed25519 verification
    pub fn start(self, shutdown_token: CancellationToken) {
        tokio::spawn(async move {
            tracing::info!(
                "🚀 WebSocketServer accept loop started on {}",
                self.local_addr
            );

            loop {
                tokio::select! {
                    _ = shutdown_token.cancelled() => {
                        tracing::info!("🛑 WebSocketServer shutting down");
                        break;
                    }
                    accept_result = self.listener.accept() => {
                        match accept_result {
                            Ok((stream, peer_addr)) => {
                                tracing::debug!(
                                    "🔗 Incoming TCP connection from: {}",
                                    peer_addr
                                );

                                let conn_tx = self.conn_tx.clone();

                                // Complete the WebSocket handshake in a dedicated task to avoid blocking the accept loop.
                                tokio::spawn(async move {
                                    // Use Arc plus std Mutex to capture handshake headers from the synchronous callback.
                                    let captured_source_id: std::sync::Arc<StdMutex<Vec<u8>>> =
                                        std::sync::Arc::new(StdMutex::new(Vec::new()));
                                    let captured_credential: std::sync::Arc<StdMutex<Option<AIdCredential>>> =
                                        std::sync::Arc::new(StdMutex::new(None));

                                    let capture_src = captured_source_id.clone();
                                    let capture_cred = captured_credential.clone();

                                    #[allow(clippy::result_large_err)]
                                    let callback = move |req: &tokio_tungstenite::tungstenite::handshake::server::Request,
                                                         res: tokio_tungstenite::tungstenite::handshake::server::Response|
                                     -> Result<
                                        tokio_tungstenite::tungstenite::handshake::server::Response,
                                        tokio_tungstenite::tungstenite::handshake::server::ErrorResponse,
                                    > {
                                        // Extract `X-Actr-Source-ID`.
                                        if let Some(val) = req.headers().get("X-Actr-Source-ID") {
                                            if let Ok(hex_str) = val.to_str() {
                                                match hex::decode(hex_str) {
                                                    Ok(bytes) => {
                                                        *capture_src.lock().unwrap() = bytes;
                                                    }
                                                    Err(e) => {
                                                        tracing::warn!(
                                                            "⚠️ Invalid X-Actr-Source-ID hex from {}: {}",
                                                            peer_addr,
                                                            e
                                                        );
                                                    }
                                                }
                                            }
                                        } else {
                                            tracing::warn!(
                                                "⚠️ No X-Actr-Source-ID header from {} — response routing will fail",
                                                peer_addr
                                            );
                                        }

                                        // Extract `X-Actr-Credential` (base64 -> protobuf `AIdCredential`).
                                        if let Some(val) = req.headers().get("X-Actr-Credential") {
                                            if let Ok(b64_str) = val.to_str() {
                                                use base64::Engine as _;
                                                match base64::engine::general_purpose::STANDARD.decode(b64_str) {
                                                    Ok(cred_bytes) => {
                                                        match AIdCredential::decode(cred_bytes.as_slice()) {
                                                            Ok(credential) => {
                                                                *capture_cred.lock().unwrap() = Some(credential);
                                                            }
                                                            Err(e) => {
                                                                tracing::warn!(
                                                                    "⚠️ Invalid X-Actr-Credential proto from {}: {}",
                                                                    peer_addr, e
                                                                );
                                                            }
                                                        }
                                                    }
                                                    Err(e) => {
                                                        tracing::warn!(
                                                            "⚠️ Invalid X-Actr-Credential base64 from {}: {}",
                                                            peer_addr, e
                                                        );
                                                    }
                                                }
                                            }
                                        }

                                        Ok(res)
                                    };

                                    match tokio_tungstenite::accept_hdr_async(
                                        MaybeTlsStream::Plain(stream),
                                        callback,
                                    )
                                    .await
                                    {
                                        Ok(ws_stream) => {
                                            tracing::info!(
                                                "✅ WebSocket handshake completed from: {}",
                                                peer_addr
                                            );

                                            let source_id = captured_source_id.lock().unwrap().clone();
                                            let credential = captured_credential.lock().unwrap().take();

                                            let conn =
                                                WebSocketConnection::from_server_stream(ws_stream);

                                            if conn_tx.send((conn, source_id, credential)).await.is_err() {
                                                tracing::warn!(
                                                    "⚠️ WebSocketServer: conn_tx closed, dropping connection from {}",
                                                    peer_addr
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                "❌ WebSocket handshake failed from {}: {}",
                                                peer_addr,
                                                e
                                            );
                                        }
                                    }
                                });
                            }
                            Err(e) => {
                                tracing::error!("❌ WebSocketServer accept error: {}", e);
                                // Sleep briefly before retrying to avoid a tight error loop.
                                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                            }
                        }
                    }
                }
            }

            tracing::info!("🔌 WebSocketServer accept loop exited");
        });
    }
}
