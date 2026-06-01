use actr_framework::{Bytes, Context, MessageDispatcher, Workload};
use actr_protocol::{ActorResult, ActrError, ActrType, RpcEnvelope};
use async_trait::async_trait;

const ECHO_ROUTE: &str = "echo.EchoService.Echo";

#[derive(Default)]
pub(crate) struct EchoProxyWorkload;

#[async_trait]
impl Workload for EchoProxyWorkload {
    type Dispatcher = EchoProxyDispatcher;
}

pub(crate) struct EchoProxyDispatcher;

#[async_trait]
impl MessageDispatcher for EchoProxyDispatcher {
    type Workload = EchoProxyWorkload;

    async fn dispatch<C: Context>(
        _workload: &Self::Workload,
        envelope: RpcEnvelope,
        ctx: &C,
    ) -> ActorResult<Bytes> {
        if envelope.route_key != ECHO_ROUTE {
            return Err(ActrError::UnknownRoute(envelope.route_key));
        }

        let target_type = ActrType {
            manufacturer: "acme".to_string(),
            name: "EchoService".to_string(),
            version: "0.1.0".to_string(),
        };
        let target = ctx.discover_route_candidate(&target_type).await?;
        let payload = envelope.payload.unwrap_or_default();
        ctx.call_raw(&target, ECHO_ROUTE, payload).await
    }
}

#[cfg(test)]
mod tests {
    use actr_framework::{Context, MediaSample, MessageDispatcher};
    use actr_protocol::{
        ActorResult, ActrError, ActrId, ActrType, PayloadType, Realm, RpcEnvelope,
    };
    use async_trait::async_trait;
    use bytes::Bytes;
    use futures_util::future::BoxFuture;
    use parking_lot::Mutex;
    use std::sync::Arc;

    use super::{EchoProxyDispatcher, EchoProxyWorkload};

    #[derive(Clone)]
    struct RecordingContext {
        self_id: ActrId,
        discovered: ActrId,
        calls: Arc<Mutex<Vec<(ActrId, String, Bytes)>>>,
    }

    impl RecordingContext {
        fn new() -> Self {
            let self_id = actor_id(1, "EchoApp");
            let discovered = actor_id(2, "EchoService");
            Self {
                self_id,
                discovered,
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait]
    impl Context for RecordingContext {
        fn self_id(&self) -> &ActrId {
            &self.self_id
        }

        fn caller_id(&self) -> Option<&ActrId> {
            None
        }

        fn request_id(&self) -> &str {
            "test-request"
        }

        async fn call<R: actr_protocol::RpcRequest>(
            &self,
            _target: &actr_framework::Dest,
            _request: R,
        ) -> ActorResult<R::Response> {
            Err(ActrError::NotImplemented("typed call not used".to_string()))
        }

        async fn tell<R: actr_protocol::RpcRequest>(
            &self,
            _target: &actr_framework::Dest,
            _message: R,
        ) -> ActorResult<()> {
            Err(ActrError::NotImplemented("typed tell not used".to_string()))
        }

        async fn register_stream<F>(&self, _stream_id: String, _callback: F) -> ActorResult<()>
        where
            F: Fn(actr_protocol::DataStream, ActrId) -> BoxFuture<'static, ActorResult<()>>
                + Send
                + Sync
                + 'static,
        {
            Ok(())
        }

        async fn unregister_stream(&self, _stream_id: &str) -> ActorResult<()> {
            Ok(())
        }

        async fn send_data_stream(
            &self,
            _target: &actr_framework::Dest,
            _chunk: actr_protocol::DataStream,
            _payload_type: PayloadType,
        ) -> ActorResult<()> {
            Err(ActrError::NotImplemented("streaming not used".to_string()))
        }

        async fn discover_route_candidate(&self, target_type: &ActrType) -> ActorResult<ActrId> {
            assert_eq!(target_type.to_string_repr(), "acme:EchoService:0.1.0");
            Ok(self.discovered.clone())
        }

        async fn call_raw(
            &self,
            target: &ActrId,
            route_key: &str,
            payload: Bytes,
        ) -> ActorResult<Bytes> {
            self.calls
                .lock()
                .push((target.clone(), route_key.to_string(), payload));
            Ok(Bytes::from_static(b"remote-reply"))
        }

        async fn register_media_track<F>(&self, _track_id: String, _callback: F) -> ActorResult<()>
        where
            F: Fn(MediaSample, ActrId) -> BoxFuture<'static, ActorResult<()>>
                + Send
                + Sync
                + 'static,
        {
            Ok(())
        }

        async fn unregister_media_track(&self, _track_id: &str) -> ActorResult<()> {
            Ok(())
        }

        async fn send_media_sample(
            &self,
            _target: &actr_framework::Dest,
            _track_id: &str,
            _sample: MediaSample,
        ) -> ActorResult<()> {
            Err(ActrError::NotImplemented("media not used".to_string()))
        }

        async fn add_media_track(
            &self,
            _target: &actr_framework::Dest,
            _track_id: &str,
            _codec: &str,
            _media_type: &str,
        ) -> ActorResult<()> {
            Err(ActrError::NotImplemented("media not used".to_string()))
        }

        async fn remove_media_track(
            &self,
            _target: &actr_framework::Dest,
            _track_id: &str,
        ) -> ActorResult<()> {
            Err(ActrError::NotImplemented("media not used".to_string()))
        }
    }

    #[tokio::test]
    async fn echo_proxy_dispatcher_discovers_and_forwards_raw_echo_payload() {
        let ctx = RecordingContext::new();
        let envelope = RpcEnvelope {
            request_id: "r1".to_string(),
            route_key: "echo.EchoService.Echo".to_string(),
            payload: Some(Bytes::from_static(b"hello")),
            error: None,
            traceparent: None,
            tracestate: None,
            metadata: vec![],
            timeout_ms: 0,
        };

        let response = EchoProxyDispatcher::dispatch(&EchoProxyWorkload, envelope, &ctx)
            .await
            .expect("echo proxy should forward the request");

        assert_eq!(response, Bytes::from_static(b"remote-reply"));
        let calls = ctx.calls.lock();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, ctx.discovered);
        assert_eq!(calls[0].1, "echo.EchoService.Echo");
        assert_eq!(calls[0].2, Bytes::from_static(b"hello"));
    }

    #[tokio::test]
    async fn echo_proxy_dispatcher_rejects_unknown_route() {
        let ctx = RecordingContext::new();
        let envelope = RpcEnvelope {
            request_id: "r2".to_string(),
            route_key: "unknown.Route".to_string(),
            payload: Some(Bytes::new()),
            error: None,
            traceparent: None,
            tracestate: None,
            metadata: vec![],
            timeout_ms: 0,
        };

        let err = EchoProxyDispatcher::dispatch(&EchoProxyWorkload, envelope, &ctx)
            .await
            .expect_err("unknown route should fail");

        assert!(matches!(err, ActrError::UnknownRoute(_)));
    }

    fn actor_id(serial_number: u64, name: &str) -> ActrId {
        ActrId {
            realm: Realm { realm_id: 1 },
            serial_number,
            r#type: ActrType {
                manufacturer: "acme".to_string(),
                name: name.to_string(),
                version: "0.1.0".to_string(),
            },
        }
    }
}
