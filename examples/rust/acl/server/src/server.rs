use actr_hyper::prelude::*;
use std::path::PathBuf;
use tracing::info;

mod generated;
use generated::greeter::{GreetRequest, GreetResponse};
use generated::greeter_actor::{GreeterServiceHandler, GreeterServiceWorkload};

struct GreeterService;

#[async_trait::async_trait]
impl GreeterServiceHandler for GreeterService {
    async fn greet<C: actr_framework::Context>(
        &self,
        request: GreetRequest,
        _ctx: &C,
    ) -> actr_protocol::ActorResult<GreetResponse> {
        info!("✅ Server received greeting request from: {}", request.name);
        Ok(GreetResponse {
            message: format!("Hello, {}! (from ACL-protected server)", request.name),
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("🚀 Greeter Server (ACL Protected) starting...");

    // Load configuration
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("actr.toml");
    let config = actr_config::ConfigParser::from_file(&config_path)?;
    info!("✅ Configuration loaded");

    // Create workload
    let handler = GreeterService;
    let workload = GreeterServiceWorkload::new(handler);
    info!("📦 GreeterService workload created");

    // Build node with workload
    let node = ActrNode::new(config, workload).await?;
    info!("✅ ActrNode created with workload");

    // Start the node
    let _actr_ref = node.start().await?;
    info!("🚀 ActrNode started successfully!");
    info!("🎉 Greeter Server is running and registered with ACL protection");
    info!("   ACL: Only allowing 'allowed-greeter-client' actor type");

    // Keep running
    tokio::signal::ctrl_c().await?;
    info!("👋 Shutting down...");

    Ok(())
}
