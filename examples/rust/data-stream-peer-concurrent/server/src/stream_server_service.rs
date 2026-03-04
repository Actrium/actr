//! StreamServer service implementation.

use actr_protocol::{ActorResult, ActrIdExt, DataStream};
use actr_runtime::prelude::*;
use async_trait::async_trait;
use data_stream_peer_concurrent_shared::generated::data_stream_peer::*;
use data_stream_peer_concurrent_shared::generated::stream_server_actor::StreamServerHandler;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Receives DataStream messages after the client prepares the stream.
pub struct StreamServerService {
    received_count: Arc<Mutex<u32>>,
}

impl StreamServerService {
    pub fn new() -> Self {
        Self {
            received_count: Arc::new(Mutex::new(0)),
        }
    }
}

#[async_trait]
impl StreamServerHandler for StreamServerService {
    async fn prepare_stream<C: Context>(
        &self,
        req: PrepareServerStreamRequest,
        ctx: &C,
    ) -> ActorResult<PrepareStreamResponse> {
        tracing::info!(
            "prepare_stream: stream_id={}, expected_count={}",
            req.stream_id,
            req.expected_count
        );

        *self.received_count.lock().await = 0;

        let stream_id = req.stream_id.clone();
        let expected_count = req.expected_count;
        let received_count = self.received_count.clone();

        ctx.register_stream(
            stream_id.clone(),
            move |data_stream: DataStream, sender_id| {
                let received_count = received_count.clone();

                Box::pin(async move {
                    let mut count = received_count.lock().await;
                    *count += 1;

                    let text = String::from_utf8_lossy(&data_stream.payload);
                    tracing::info!(
                        "server: stream {} received {}/{} from {}: {}",
                        data_stream.stream_id,
                        *count,
                        expected_count,
                        sender_id.to_string_repr(),
                        text
                    );

                    Ok(())
                })
            },
        )
        .await?;

        let caller_id = ctx.caller_id().cloned().ok_or_else(|| {
            actr_protocol::ProtocolError::TransportError("No caller ID found".to_string())
        })?;

        let prepare_client_resp: PrepareStreamResponse = ctx
            .call(
                &Dest::Actor(caller_id.clone()),
                PrepareClientStreamRequest {
                    stream_id: req.stream_id.clone(),
                    expected_count,
                },
            )
            .await?;

        if !prepare_client_resp.ready {
            return Ok(PrepareStreamResponse {
                ready: false,
                message: prepare_client_resp.message,
            });
        }

        let stream_id = req.stream_id.clone();
        let ctx_clone = ctx.clone();
        tokio::spawn(async move {
            tracing::info!(
                "sending data stream back to client: {}",
                caller_id.to_string_repr()
            );

            for i in 1..=expected_count {
                let message = format!("[server] message {}", i);
                let data_stream = DataStream {
                    stream_id: stream_id.clone(),
                    sequence: i as u64,
                    payload: bytes::Bytes::from(message.clone()),
                    metadata: vec![],
                    timestamp_ms: None,
                };

                tracing::info!("server sending {}/{}: {}", i, expected_count, message);
                if let Err(err) = ctx_clone
                    .send_data_stream(&Dest::Actor(caller_id.clone()), data_stream)
                    .await
                {
                    tracing::warn!("server send_data_stream failed: {}", err);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1000)).await;
            }
        });

        Ok(PrepareStreamResponse {
            ready: true,
            message: format!(
                "registered stream {} for {} messages",
                req.stream_id, req.expected_count
            ),
        })
    }
}
