use anyhow::{Context, Result};
use std::path::PathBuf;
use tracing::{error, info};

mod client_workload;
mod generated;

use actr_protocol::ActrType;
use actr_hyper::prelude::*;
use client_workload::ClientWorkload;

#[tokio::main]
async fn main() -> Result<()> {
    // loadconfig[...]
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("actr.toml");
    let config = actr_config::ConfigParser::from_file(&config_path)?;

    // initialize[...]（log/[...]）
    let _obs_guard = actr_hyper::init_observability(&config.observability)?;

    info!("🚀 Echo Client App - [...]");

    // [...] workload [...]configservice[...]
    let workload = ClientWorkload::new();

    info!("🌐 via/throughsignalingservice[...]discover echo server...");
    let target_type = ActrType {
        manufacturer: "acme".to_string(),
        name: "EchoService".to_string(),
        version: "1.0.0".to_string(),
    };

    let node = ActrNode::new(config, workload.clone()).await?;
    info!("✅ ActrNode createsuccess");

    info!("🚀 start ActrNode...");
    let actr_ref = node.start().await?;
    let mut candidates = actr_ref
        .discover_route_candidates(&target_type, 1)
        .await
        .context("[...]signalingservice[...] echo [...]nodefailed")?;

    let server_id = candidates
        .pop()
        .ok_or_else(|| anyhow::anyhow!("[...] echo server [...]"))?;

    info!("🎯 [...]service[...]: {:?}", server_id);
    workload.set_server_id(server_id).await;
    let local_id = actr_ref.actor_id().clone();
    info!("✅ ActrNode startsuccess，ID: {:?}", local_id);

    // [...]client ID [...]requestmessage
    use generated::echo::{EchoRequest, EchoResponse};

    let client_id_str = format!("{:?}", local_id);
    let message = format!("Client[{}] says hello!", client_id_str);

    let request = EchoRequest {
        message: message.clone(),
    };

    info!("📤 send echo request: {}", request.message);
    println!("\n🔹 Client ID: {}", client_id_str);
    println!("🔹 sendmessage: {}", message);

    match actr_ref.call(request).await {
        Ok(response) => {
            let response: EchoResponse = response;
            info!("📥 [...] echo response: {}", response.reply);
            info!("⏰ [...]: {}", response.timestamp);

            // [...]response[...]client ID
            if response.reply.contains(&client_id_str) {
                println!("\n✅ success！response[...]client");
                println!("  client ID:  {}", client_id_str);
                println!("  sendmessage:   {}", message);
                println!("  service[...]response: {}", response.reply);
                println!("  [...]:     {}", response.timestamp);
                println!("  ✓ [...]via/through: response[...]client ID");
            } else {
                println!("\n⚠️  warning！response[...]");
                println!("  [...]:   {}", client_id_str);
                println!("  [...]response:   {}", response.reply);
            }
        }
        Err(e) => {
            error!("❌ [...] echo servicefailed: {:?}", e);
            println!("\n❌ error: {}", e);
        }
    }

    info!("👋 [...]close/shutdown");
    Ok(())
}
