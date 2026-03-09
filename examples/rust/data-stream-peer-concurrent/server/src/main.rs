//! Data Stream Server for peer-to-peer bidirectional streaming with concurrent support.

mod stream_server_service;

use data_stream_peer_concurrent_shared::generated::stream_server_actor::StreamServerWorkload;
use stream_server_service::StreamServerService;

use actr_protocol::{ActrIdExt, ActrTypeExt};
use actr_runtime::prelude::*;
use std::path::PathBuf;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("actr.toml");
    let config = actr_config::ConfigParser::from_file(&config_path)?;

    let _obs_guard = actr_runtime::init_observability(&config.observability)?;

    info!(
        "data-stream-peer-concurrent server starting: type={}",
        config.package.actr_type.to_string_repr()
    );

    let system = match ActrSystem::new(config).await {
        Ok(sys) => sys,
        Err(e) => {
            error!("ActrSystem creation failed: {}", e);
            return Err(e.into());
        }
    };

    let service = StreamServerService::new();
    let workload = StreamServerWorkload::new(service);
    let node = system.attach(workload);

    let actr_ref = match node.start().await {
        Ok(actr) => actr,
        Err(e) => {
            error!("ActrNode start failed: {}", e);
            error!("Ensure signaling-server is running");
            return Err(e.into());
        }
    };

    info!(
        "server started: actor_id={}",
        actr_ref.actor_id().to_string_repr()
    );
    actr_ref.wait_for_ctrl_c_and_shutdown().await?;

    Ok(())
}
