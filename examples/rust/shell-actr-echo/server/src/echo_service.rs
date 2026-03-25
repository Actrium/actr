//! Echo service implementation

use crate::generated::echo::{EchoRequest, EchoResponse};
use crate::generated::echo_actor::EchoServiceHandler;
use actr::framework::Context;
use actr::protocol::ActorResult;
use async_trait::async_trait;
use tracing::info;

pub struct EchoService;

#[async_trait]
impl EchoServiceHandler for EchoService {
    async fn echo<C: Context>(&self, req: EchoRequest, ctx: &C) -> ActorResult<EchoResponse> {
        info!(message = %req.message, request_id = %ctx.request_id(), "received echo request");

        let reply = format!("Echo: {}", req.message);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Ok(EchoResponse { reply, timestamp })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actr_framework::test_support::DummyContext;
    use actr::protocol::ActrId;

    #[tokio::test]
    async fn test_echo_logic() {
        let service = EchoService;
        let ctx = DummyContext::new(ActrId::default());

        let request = EchoRequest {
            message: "Hello, World!".to_string(),
        };

        let response = service.echo(request, &ctx).await.unwrap();
        assert_eq!(response.reply, "Echo: Hello, World!");
        assert!(response.timestamp > 0);
    }
}
