use std::rc::Rc;

use actr_runtime_sw::actr_protocol::ActrIdExt;
use actr_runtime_sw::{RuntimeContext, WebContext};
use bytes::Bytes;
use gloo_timers::future::TimeoutFuture;
use serde::{Deserialize, Serialize};
use wasm_bindgen_futures::spawn_local;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PrepareServerStreamRequest {
    stream_id: String,
    expected_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PrepareStreamResponse {
    ready: bool,
    message: String,
}

pub async fn handle_request(
    method: &str,
    request_bytes: &[u8],
    ctx: Rc<RuntimeContext>,
) -> Result<Vec<u8>, String> {
    match method {
        "PrepareStream" => prepare_stream(request_bytes, ctx).await,
        _ => Err(format!("Unknown StreamServer method: {}", method)),
    }
}

async fn prepare_stream(request_bytes: &[u8], ctx: Rc<RuntimeContext>) -> Result<Vec<u8>, String> {
    let req: PrepareServerStreamRequest = serde_json::from_slice(request_bytes)
        .map_err(|e| format!("Failed to decode PrepareServerStreamRequest: {}", e))?;

    log::info!(
        "[DataStreamServer] prepare_stream: stream_id={} expected_count={}",
        req.stream_id,
        req.expected_count
    );

    let stream_id = req.stream_id.clone();
    let expected_count = req.expected_count;
    ctx.register_stream(
        stream_id.clone(),
        Box::new(move |data: Bytes| {
            let text = String::from_utf8_lossy(&data);
            let seq = text
                .rsplit(' ')
                .next()
                .and_then(|n| n.parse::<u32>().ok())
                .unwrap_or_default();
            log::info!(
                "[DataStreamServer] server: stream {} received {}/{}: {}",
                stream_id,
                seq,
                expected_count,
                text
            );
        }),
    )
    .await
    .map_err(|e| format!("register_stream failed: {}", e))?;

    let caller_id = ctx
        .caller_id()
        .cloned()
        .ok_or_else(|| "No caller_id available in RuntimeContext".to_string())?;

    let caller_for_stream = caller_id.clone();
    let stream_id_for_stream = req.stream_id.clone();
    let expected_for_stream = req.expected_count;
    let ctx_clone = Rc::clone(&ctx);

    spawn_local(async move {
        log::info!(
            "[DataStreamServer] sending data stream back to client: {}",
            caller_for_stream
        );

        for i in 1..=expected_for_stream {
            let message = format!("[server] message {}", i);
            log::info!(
                "[DataStreamServer] server sending {}/{} on {}: {}",
                i,
                expected_for_stream,
                stream_id_for_stream,
                message
            );
            if let Err(error) = ctx_clone
                .send_data_stream(
                    &caller_for_stream,
                    &stream_id_for_stream,
                    Bytes::from(message.into_bytes()),
                )
                .await
            {
                log::error!("[DataStreamServer] send_data_stream failed: {}", error);
                break;
            }
            TimeoutFuture::new(700).await;
        }
    });

    serde_json::to_vec(&PrepareStreamResponse {
        ready: true,
        message: format!(
            "registered stream {} for {} messages",
            req.stream_id, req.expected_count
        ),
    })
    .map_err(|e| e.to_string())
}
