//! StreamClient service implementation.

use actr_protocol::{ActorResult, ActrIdExt, ActrType, ActrTypeExt, DataStream};
use actr_runtime::prelude::*;
use async_trait::async_trait;
use data_stream_peer_concurrent_shared::generated::data_stream_peer::*;
use data_stream_peer_concurrent_shared::generated::stream_client_actor::StreamClientHandler;

/// Starts a stream by calling the remote server and sending DataStream chunks.
pub struct StreamClientService {
    server_type: ActrType,
}

impl StreamClientService {
    pub fn new() -> Self {
        Self {
            server_type: ActrType {
                manufacturer: "acme".to_string(),
                name: "DataStreamPeerConcurrentServer".to_string(),
            },
        }
    }
}

#[async_trait]
impl StreamClientHandler for StreamClientService {
    async fn prepare_client_stream<C: Context>(
        &self,
        req: PrepareClientStreamRequest,
        ctx: &C,
    ) -> ActorResult<PrepareStreamResponse> {
        tracing::info!(
            "prepare_client_stream: stream_id={}, expected_count={}",
            req.stream_id,
            req.expected_count
        );

        let stream_id = req.stream_id.clone();
        let expected_count = req.expected_count;

        ctx.register_stream(
            stream_id.clone(),
            move |data_stream: DataStream, sender_id| {
                Box::pin(async move {
                    let text = String::from_utf8_lossy(&data_stream.payload);
                    tracing::info!(
                        "client received {}/{} from {}: {}",
                        data_stream.sequence,
                        expected_count,
                        sender_id.to_string_repr(),
                        text
                    );
                    Ok(())
                })
            },
        )
        .await?;

        Ok(PrepareStreamResponse {
            ready: true,
            message: format!(
                "client ready to receive {} messages on {}",
                req.expected_count, req.stream_id
            ),
        })
    }

    async fn start_stream<C: Context>(
        &self,
        req: StartStreamRequest,
        ctx: &C,
    ) -> ActorResult<StartStreamResponse> {
        tracing::info!(
            "start_stream: client_id={}, stream_id={}, message_count={}",
            req.client_id,
            req.stream_id,
            req.message_count
        );

        tracing::info!(
            "discovering server type: {}",
            self.server_type.to_string_repr()
        );
        let server_id = ctx.discover_route_candidate(&self.server_type).await?;
        tracing::info!("discovered server: {}", server_id.to_string_repr());

        let prepare_req = PrepareServerStreamRequest {
            stream_id: req.stream_id.clone(),
            expected_count: req.message_count,
        };

        let prepare_resp: PrepareStreamResponse = ctx
            .call(&Dest::Actor(server_id.clone()), prepare_req)
            .await?;

        if !prepare_resp.ready {
            return Ok(StartStreamResponse {
                accepted: false,
                message: prepare_resp.message,
            });
        }

        let client_id = req.client_id.clone();
        let stream_id = req.stream_id.clone();
        let message_count = req.message_count;
        let server_id_clone = server_id.clone();
        let ctx_clone = ctx.clone();

        tokio::spawn(async move {
            for i in 1..=message_count {
                let message = format!("[client {}] message {}", client_id, i);
                let data_stream = DataStream {
                    stream_id: stream_id.clone(),
                    sequence: i as u64,
                    payload: bytes::Bytes::from(message.clone()),
                    metadata: vec![],
                    timestamp_ms: None,
                };

                tracing::info!("client sending {}/{}: {}", i, message_count, message);
                let res = ctx_clone
                    .send_data_stream(&Dest::Actor(server_id_clone.clone()), data_stream, actr_protocol::PayloadType::StreamReliable)
                    .await;
                if let Err(e) = res {
                    tracing::error!("client send_data_stream error: {}", e);
                }
                tokio::time::sleep(Duration::from_millis(1000)).await;
            }
        });

        Ok(StartStreamResponse {
            accepted: true,
            message: format!(
                "started sending {} messages to {}",
                req.message_count,
                server_id.to_string_repr()
            ),
        })
    }
}
