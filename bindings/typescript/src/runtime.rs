use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::types::{ActrId, ActrType, PayloadType};
use crate::workload::DynamicWorkload;

#[napi]
pub struct ActrSystem {
    inner: Option<actr_runtime::ActrSystem>,
    #[allow(dead_code)]
    config: actr_config::Config,
}

#[napi]
impl ActrSystem {
    /// Create ActrSystem from a config file path.
    #[napi(factory)]
    pub async fn from_file(config_path: String) -> Result<ActrSystem> {
        let config = actr_config::ConfigParser::from_file(&config_path)
            .map_err(crate::error::config_error_to_napi)?;

        crate::logger::init_observability(config.observability.clone());

        let system = actr_runtime::ActrSystem::new(config.clone())
            .await
            .map_err(crate::error::protocol_error_to_napi)?;

        Ok(ActrSystem {
            inner: Some(system),
            config,
        })
    }

    /// Attach a workload and create ActrNode.
    #[napi]
    pub fn attach(&mut self, callback: Object) -> Result<ActrNode> {
        let system = self
            .inner
            .take()
            .ok_or_else(|| Error::from_reason("System already consumed"))?;

        let workload = DynamicWorkload::new(callback)?;
        let node = system.attach(workload);

        Ok(ActrNode { inner: Some(node) })
    }
}

#[napi]
pub struct ActrNode {
    inner: Option<actr_runtime::ActrNode<DynamicWorkload>>,
}

#[napi]
impl ActrNode {
    /// Start the node and return ActrRef.
    ///
    /// # Safety
    ///
    /// This function is unsafe because it takes ownership of the internal node and
    /// starts the actor runtime. It must only be called once.
    #[napi]
    pub async unsafe fn start(&mut self) -> Result<ActrRef> {
        let node = self
            .inner
            .take()
            .ok_or_else(|| Error::from_reason("Node already started"))?;

        let actr_ref = node
            .start()
            .await
            .map_err(crate::error::protocol_error_to_napi)?;

        Ok(ActrRef { inner: actr_ref })
    }
}

#[napi]
pub struct ActrRef {
    inner: actr_runtime::ActrRef<DynamicWorkload>,
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
            .discover_route_candidates(&proto_type, count)
            .await
            .map_err(crate::error::protocol_error_to_napi)?;

        Ok(ids.into_iter().map(|id| id.into()).collect())
    }

    /// Call remote actor (RPC).
    #[napi]
    pub async fn call(
        &self,
        route_key: String,
        payload_type: PayloadType,
        request_payload: Buffer,
        timeout_ms: i64,
    ) -> Result<Buffer> {
        let proto_payload_type: actr_protocol::PayloadType = payload_type.into();
        let response = self
            .inner
            .call_raw(
                route_key,
                bytes::Bytes::from(request_payload.to_vec()),
                timeout_ms,
                proto_payload_type,
            )
            .await
            .map_err(crate::error::protocol_error_to_napi)?;

        Ok(response.to_vec().into())
    }

    /// Send one-way message (fire-and-forget).
    #[napi]
    pub async fn tell(
        &self,
        route_key: String,
        payload_type: PayloadType,
        message_payload: Buffer,
    ) -> Result<()> {
        let proto_payload_type: actr_protocol::PayloadType = payload_type.into();
        self.inner
            .tell_raw(
                route_key,
                bytes::Bytes::from(message_payload.to_vec()),
                proto_payload_type,
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
