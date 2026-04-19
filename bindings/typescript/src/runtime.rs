use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::types::{ActrId, ActrType, PayloadType};
use actr_config::ConfigParser;
use actr_framework::Dest;
use actr_hyper::{ActrRef as RuntimeActrRef, Node, Registered};

#[napi]
pub struct ActrNode {
    inner: Option<Node<Registered>>,
}

#[napi]
impl ActrNode {
    /// Create a client-only ActrNode from manifest.toml and the sibling actr.toml.
    #[napi(factory)]
    pub async fn from_file(config_path: String) -> Result<ActrNode> {
        // Accept the manifest.toml path, resolve its sibling actr.toml,
        // and let Node::from_config_file own config + trust + Hyper
        // construction. TypeScript bindings are client-only so we finish
        // with attach_none.
        let manifest =
            ConfigParser::from_manifest_file(&config_path).map_err(crate::error::config_error_to_napi)?;
        let runtime_path = manifest.config_dir.join("actr.toml");

        let init = Node::from_config_file(&runtime_path)
            .await
            .map_err(crate::error::hyper_error_to_napi)?;
        crate::logger::init_observability(init.runtime_config().observability.clone());
        let attached = init
            .attach_none()
            .await
            .map_err(crate::error::hyper_error_to_napi)?;
        let ais_endpoint = attached.ais_endpoint().to_string();
        let registered = attached
            .register(&ais_endpoint)
            .await
            .map_err(crate::error::hyper_error_to_napi)?;

        Ok(ActrNode {
            inner: Some(registered),
        })
    }
    /// Start the node and return ActrRef.
    ///
    /// One-shot: consumes the internal Hyper handle. A second call resolves
    /// with `Node already started`.
    ///
    /// The `unsafe` marker is required by napi-rs for async methods taking
    /// `&mut self` — it is not surfaced to JavaScript callers.
    #[napi]
    pub async unsafe fn start(&mut self) -> Result<ActrRef> {
        let hyper = self
            .inner
            .take()
            .ok_or_else(|| Error::from_reason("Node already started"))?;

        let actr_ref = hyper
            .start()
            .await
            .map_err(crate::error::protocol_error_to_napi)?;

        Ok(ActrRef { inner: actr_ref })
    }
}

#[napi]
pub struct ActrRef {
    inner: RuntimeActrRef,
}

#[napi]
impl ActrRef {
    /// Get the actor ID.
    #[napi]
    pub fn actor_id(&self) -> ActrId {
        self.inner.actor_id().clone().into()
    }

    /// Discover actors of the given type.
    #[napi]
    pub async fn discover(&self, target_type: ActrType, count: u32) -> Result<Vec<ActrId>> {
        let proto_type: actr_protocol::ActrType = target_type.into();
        let ids = self
            .inner
            .discover_route_candidates(&proto_type, count as usize)
            .await
            .map_err(crate::error::protocol_error_to_napi)?;

        Ok(ids.into_iter().map(|id| id.into()).collect())
    }

    /// Call remote actor (RPC).
    #[napi]
    pub async fn call(
        &self,
        target: ActrId,
        route_key: String,
        payload_type: PayloadType,
        request_payload: Buffer,
        timeout_ms: i64,
    ) -> Result<Buffer> {
        let target_id: actr_protocol::ActrId = target.into();
        let proto_payload_type: actr_protocol::PayloadType = payload_type.into();
        let ctx = self.inner.app_context().await;
        let response = ctx
            .call_raw(
                &Dest::Actor(target_id),
                route_key,
                proto_payload_type,
                bytes::Bytes::from(request_payload.to_vec()),
                timeout_ms,
            )
            .await
            .map_err(crate::error::protocol_error_to_napi)?;

        Ok(response.to_vec().into())
    }

    /// Send one-way message (fire-and-forget).
    #[napi]
    pub async fn tell(
        &self,
        target: ActrId,
        route_key: String,
        payload_type: PayloadType,
        message_payload: Buffer,
    ) -> Result<()> {
        let target_id: actr_protocol::ActrId = target.into();
        let proto_payload_type: actr_protocol::PayloadType = payload_type.into();
        let ctx = self.inner.app_context().await;
        ctx.tell_raw(
            &Dest::Actor(target_id),
            route_key,
            proto_payload_type,
            bytes::Bytes::from(message_payload.to_vec()),
        )
            .await
            .map_err(crate::error::protocol_error_to_napi)?;

        Ok(())
    }

    /// Trigger shutdown.
    #[napi]
    pub fn shutdown(&self) {
        self.inner.shutdown();
    }

    /// Wait for shutdown to complete.
    #[napi]
    pub async fn wait_for_shutdown(&self) {
        self.inner.wait_for_shutdown().await;
    }

    /// Check if shutdown is in progress.
    #[napi]
    pub fn is_shutting_down(&self) -> bool {
        self.inner.is_shutting_down()
    }
}
