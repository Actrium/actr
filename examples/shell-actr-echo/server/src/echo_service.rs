//! Echo service implementation

use crate::generated::echo::{EchoRequest, EchoResponse};
use crate::generated::echo_service_actor::EchoServiceHandler;
use actr_framework::Context;
use actr_protocol::ActorResult;
use async_trait::async_trait;
use tracing::info;

/// Echo service implementation
pub struct EchoService;

impl EchoService {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl EchoServiceHandler for EchoService {
    async fn echo<C: Context>(&self, req: EchoRequest, ctx: &C) -> ActorResult<EchoResponse> {
        info!(
            "📨 Received Echo request: message='{}', trace_id={}",
            req.message,
            ctx.trace_id()
        );

        let reply = format!("Echo: {}", req.message);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        info!("📤 Sending Echo response: reply='{}'", reply);

        Ok(EchoResponse { reply, timestamp })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_echo_logic() {
        let service = EchoService::new();

        // Create a minimal context for testing
        use actr_protocol::ActrId;
        use actr_runtime::ContextFactory;
        use actr_runtime::inbound::{DataStreamRegistry, MediaFrameRegistry};
        use actr_runtime::outbound::{InprocOutGate, OutGate};
        use actr_runtime::transport::InprocTransportManager;
        use std::sync::Arc;

        let test_id = ActrId::default();
        // Create two separate InprocTransportManager instances (bidirectional)
        let shell_to_workload = Arc::new(InprocTransportManager::new());
        let workload_to_shell = Arc::new(InprocTransportManager::new());
        let inproc_gate =
            OutGate::InprocOut(Arc::new(InprocOutGate::new(shell_to_workload.clone())));
        let data_stream_registry = Arc::new(DataStreamRegistry::new());
        let media_frame_registry = Arc::new(MediaFrameRegistry::new());
        let context_factory = ContextFactory::new(
            inproc_gate,
            shell_to_workload,
            workload_to_shell,
            data_stream_registry,
            media_frame_registry,
        );
        let ctx = context_factory.create_bootstrap(&test_id);

        let request = EchoRequest {
            message: "Hello, World!".to_string(),
        };

        let response = service.echo(request, &ctx).await.unwrap();

        assert_eq!(response.reply, "Echo: Hello, World!");
        assert!(response.timestamp > 0);
    }
}
