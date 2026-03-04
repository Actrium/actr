//! Data Stream Client for peer-to-peer bidirectional streaming with concurrent support.

mod stream_client_service;

use data_stream_peer_concurrent_shared::generated::data_stream_peer::*;
use data_stream_peer_concurrent_shared::generated::stream_client_actor::StreamClientWorkload;
use stream_client_service::StreamClientService;

use actr_protocol::ActrIdExt;
use actr_runtime::prelude::*;
use anyhow::Result;
use std::env;
use std::path::PathBuf;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let client_id = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "client-1".to_string());
    let message_count: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(3);

    let stream_id = format!("{}-stream", client_id);

    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Actr.toml");
    let config = actr_config::ConfigParser::from_file(&config_path)?;

    let _obs_guard = actr_runtime::init_observability(&config.observability)?;

    info!(
        "data-stream-peer-concurrent client starting: client_id={}, message_count={}",
        client_id, message_count
    );

    let system = ActrSystem::new(config).await?;
    let service = StreamClientService::new();
    let workload = StreamClientWorkload::new(service);
    let node = system.attach(workload);

    let actr_ref = node.start().await?;
    info!(
        "client started: actor_id={}",
        actr_ref.actor_id().to_string_repr()
    );

    let start_req = StartStreamRequest {
        client_id: client_id.clone(),
        stream_id,
        message_count,
    };

    let start_resp: StartStreamResponse = actr_ref.call(start_req).await?;
    info!(
        "start_stream response: accepted={}, message={}",
        start_resp.accepted, start_resp.message
    );

    let err = actr_ref.wait_for_ctrl_c_and_shutdown().await;
    if let Err(e) = err {
        eprintln!("Error: {}", e);
    }

    Ok(())
}
