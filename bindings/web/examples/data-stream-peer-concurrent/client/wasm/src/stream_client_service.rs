use std::rc::Rc;
use std::sync::OnceLock;

use actr_runtime_sw::actr_protocol::{ActrIdExt, ActrType, ActrTypeExt};
use actr_runtime_sw::{RuntimeContext, WebContext};
use bytes::Bytes;
use gloo_timers::future::TimeoutFuture;
use serde::{Deserialize, Serialize};
use wasm_bindgen_futures::spawn_local;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StartStreamRequest {
    client_id: String,
    stream_id: String,
    message_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StartStreamResponse {
    accepted: bool,
    message: String,
}

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

static SERVER_TYPE: OnceLock<ActrType> = OnceLock::new();

fn server_type() -> &'static ActrType {
    SERVER_TYPE.get_or_init(|| ActrType {
        manufacturer: "acme".to_string(),
        name: "DataStreamPeerConcurrentServer".to_string(),
        version: "0.1.0".to_string(),
    })
}

pub async fn handle_request(
    method: &str,
    request_bytes: &[u8],
    ctx: Rc<RuntimeContext>,
) -> Result<Vec<u8>, String> {
    match method {
        "StartStream" => start_stream(request_bytes, ctx).await,
        _ => Err(format!("Unknown StreamClient method: {}", method)),
    }
}

async fn start_stream(request_bytes: &[u8], ctx: Rc<RuntimeContext>) -> Result<Vec<u8>, String> {
    let req: StartStreamRequest = serde_json::from_slice(request_bytes)
        .map_err(|e| format!("Failed to decode StartStreamRequest: {}", e))?;

    log::info!(
        "[DataStreamClient] start_stream: client_id={} stream_id={} message_count={}",
        req.client_id,
        req.stream_id,
        req.message_count
    );
    log::info!(
        "[DataStreamClient] discovering server type: {}",
        server_type().to_string_repr()
    );

    let receive_stream_id = req.stream_id.clone();
    let receive_expected_count = req.message_count;
    ctx.register_stream(
        receive_stream_id.clone(),
        Box::new(move |data: Bytes| {
            let text = String::from_utf8_lossy(&data);
            let seq = text
                .rsplit(' ')
                .next()
                .and_then(|n| n.parse::<u32>().ok())
                .unwrap_or_default();
            log::info!(
                "[DataStreamClient] client received {}/{} on {}: {}",
                seq,
                receive_expected_count,
                receive_stream_id,
                text
            );
        }),
    )
    .await
    .map_err(|e| format!("register_stream failed: {}", e))?;

    let server_id = ctx
        .discover(server_type())
        .await
        .map_err(|e| format!("discover failed: {}", e))?;

    log::info!(
        "[DataStreamClient] discovered server: {}",
        server_id.to_string_repr()
    );

    let prepare_req = serde_json::to_vec(&PrepareServerStreamRequest {
        stream_id: req.stream_id.clone(),
        expected_count: req.message_count,
    })
    .map_err(|e| e.to_string())?;

    let prepare_resp_bytes = ctx
        .call_raw(
            &server_id,
            "data_stream.StreamServer.PrepareStream",
            &prepare_req,
            30000,
        )
        .await
        .map_err(|e| format!("PrepareStream call failed: {}", e))?;

    let prepare_resp: PrepareStreamResponse = serde_json::from_slice(&prepare_resp_bytes)
        .map_err(|e| format!("Failed to decode PrepareStreamResponse: {}", e))?;

    if !prepare_resp.ready {
        return serde_json::to_vec(&StartStreamResponse {
            accepted: false,
            message: prepare_resp.message,
        })
        .map_err(|e| e.to_string());
    }

    let stream_id = req.stream_id.clone();
    let message_count = req.message_count;
    let client_id = req.client_id.clone();
    let ctx_clone = Rc::clone(&ctx);
    let server_id_clone = server_id.clone();

    spawn_local(async move {
        for i in 1..=message_count {
            let message = format!("[client {}] message {}", client_id, i);
            log::info!(
                "[DataStreamClient] client sending {}/{} on {}: {}",
                i,
                message_count,
                stream_id,
                message
            );
            if let Err(error) = ctx_clone
                .send_data_stream(
                    &server_id_clone,
                    &stream_id,
                    Bytes::from(message.into_bytes()),
                )
                .await
            {
                log::error!("[DataStreamClient] send_data_stream failed: {}", error);
                break;
            }
            TimeoutFuture::new(700).await;
        }
    });

    serde_json::to_vec(&StartStreamResponse {
        accepted: true,
        message: format!("started sending {} messages", req.message_count),
    })
    .map_err(|e| e.to_string())
}
