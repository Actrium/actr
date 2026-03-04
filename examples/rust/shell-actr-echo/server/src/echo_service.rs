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
            "📨 Received Echo request: message='{}', request_id={}",
            req.message,
            ctx.request_id()
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
    use actr_framework::test_support::DummyContext;
    use actr_protocol::ActrId;

    #[tokio::test]
    async fn test_echo_logic() {
        let service = EchoService::new();

        // Create a dummy context for testing
        let ctx = DummyContext::new(ActrId::default());

        let request = EchoRequest {
            message: "Hello, World!".to_string(),
        };

        let response = service.echo(request, &ctx).await.unwrap();

        assert_eq!(response.reply, "Echo: Hello, World!");
        assert!(response.timestamp > 0);
    }
}
