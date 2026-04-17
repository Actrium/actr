//! Runtime wrappers for UniFFI export

use crate::error::{ActrError, ActrResult};
use crate::types::{ActrId, ActrType, NetworkEventResult, PayloadType};
use actr_framework::{Bytes, Dest};
use actr_hyper::{
    ActrRef, Hyper, HyperConfig, NetworkEventHandle, Registered, TrustMode, WorkloadPackage,
};
use actr_protocol::{ActrIdExt, ActrTypeExt};
use parking_lot::Mutex;
use std::sync::Arc;
use tracing::{debug, error, info};

/// Wrapper for a package-backed runtime before startup.
#[derive(uniffi::Object)]
pub struct ActrSystemWrapper {
    inner: Mutex<Option<Hyper<Registered>>>,
    network_event_handle: Mutex<Option<NetworkEventHandle>>,
}

#[uniffi::export]
impl ActrSystemWrapper {
    /// Create a new runtime wrapper from config and a verified `.actr` package file.
    #[uniffi::constructor(async_runtime = "tokio")]
    pub async fn new_from_package_file(
        config_path: String,
        package_path: String,
    ) -> ActrResult<Arc<Self>> {
        let manifest_raw =
            actr_pack::read_manifest_raw(&std::fs::read(&package_path).map_err(|e| {
                ActrError::ConfigError {
                    msg: format!("Failed to read package at {}: {}", package_path, e),
                }
            })?)
            .map_err(|e| ActrError::ConfigError {
                msg: format!("Failed to read manifest.toml from {}: {}", package_path, e),
            })?;
        let manifest: actr_config::ManifestRawConfig =
            manifest_raw
                .parse()
                .map_err(|e: actr_config::ConfigError| ActrError::ConfigError {
                    msg: format!(
                        "Failed to parse package manifest from {}: {}",
                        package_path, e
                    ),
                })?;
        let package_info =
            manifest
                .package
                .clone()
                .into_package_info()
                .map_err(|e| ActrError::ConfigError {
                    msg: format!(
                        "Failed to extract package info from {}: {}",
                        package_path, e
                    ),
                })?;
        let config = actr_config::ConfigParser::from_runtime_file(
            &config_path,
            package_info,
            manifest.package.tags,
        )
        .map_err(|e| ActrError::ConfigError {
            msg: format!("Failed to parse config file at {}: {}", config_path, e),
        })?;

        crate::logger::init_observability(config.observability.clone());

        info!(
            signaling_url = config.signaling_url.as_str(),
            realm_id = config.realm.realm_id,
            package_path = %package_path,
            "Creating package-backed runtime wrapper",
        );

        let hyper_data_dir = actr_config::user_config::resolve_hyper_data_dir()
            .map_err(|e| ActrError::ConfigError { msg: e.to_string() })?;
        let hyper = Hyper::new(HyperConfig::new(&hyper_data_dir).with_trust_mode(
            TrustMode::Development {
                self_signed_pubkey: vec![0u8; 32],
            },
        ))
        .await
        .map_err(|e| {
            error!("Failed to initialize Hyper shell: {}", e);
            ActrError::InternalError {
                msg: format!("Failed to initialize Hyper shell: {e}"),
            }
        })?;

        let package_bytes = std::fs::read(&package_path).map_err(|e| {
            error!("Failed to read package at {}: {}", package_path, e);
            ActrError::InternalError {
                msg: format!("Failed to read package at {}: {}", package_path, e),
            }
        })?;
        let package = WorkloadPackage::new(package_bytes);
        let ais_endpoint = config.ais_endpoint.clone();

        let attached = hyper.attach(&package, config).await.map_err(|e| {
            error!("Failed to attach package-backed node: {}", e);
            ActrError::InternalError {
                msg: format!("Failed to attach package-backed node: {e}"),
            }
        })?;
        let registered = attached.register(&ais_endpoint).await.map_err(|e| {
            error!("AIS registration failed: {}", e);
            ActrError::InternalError {
                msg: format!("AIS registration failed: {e}"),
            }
        })?;

        Ok(Arc::new(Self {
            inner: Mutex::new(Some(registered)),
            network_event_handle: Mutex::new(None),
        }))
    }

    /// Create a network event handle for platform callbacks.
    ///
    /// This must be called before `start()`.
    pub fn create_network_event_handle(&self) -> ActrResult<Arc<NetworkEventHandleWrapper>> {
        let mut handle_guard = self.network_event_handle.lock();
        if let Some(handle) = handle_guard.as_ref() {
            return Ok(Arc::new(NetworkEventHandleWrapper {
                inner: handle.clone(),
            }));
        }

        let mut node_guard = self.inner.lock();
        let node = node_guard.as_mut().ok_or_else(|| ActrError::StateError {
            msg: "runtime node is no longer available".to_string(),
        })?;

        let handle = node.create_network_event_handle(0);
        *handle_guard = Some(handle.clone());

        Ok(Arc::new(NetworkEventHandleWrapper { inner: handle }))
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl ActrSystemWrapper {
    /// Start the package-backed node and return a running actor reference.
    pub async fn start(self: Arc<Self>) -> ActrResult<Arc<ActrRefWrapper>> {
        let hyper = self
            .inner
            .lock()
            .take()
            .ok_or_else(|| ActrError::StateError {
                msg: "ActrSystem already started".to_string(),
            })?;

        let actr_ref = hyper.start().await.map_err(|e| {
            error!("Failed to start package-backed actor: {}", e);
            ActrError::ConnectionError {
                msg: format!("Failed to start actor: {e}"),
            }
        })?;

        Ok(Arc::new(ActrRefWrapper { inner: actr_ref }))
    }
}

/// Wrapper for `NetworkEventHandle` - network lifecycle callbacks.
#[derive(uniffi::Object)]
pub struct NetworkEventHandleWrapper {
    inner: NetworkEventHandle,
}

#[uniffi::export(async_runtime = "tokio")]
impl NetworkEventHandleWrapper {
    /// Handle network available event.
    pub async fn handle_network_available(&self) -> ActrResult<NetworkEventResult> {
        let result = self
            .inner
            .handle_network_available()
            .await
            .map_err(|e| ActrError::InternalError { msg: e })?;
        Ok(result.into())
    }

    /// Handle network lost event.
    pub async fn handle_network_lost(&self) -> ActrResult<NetworkEventResult> {
        let result = self
            .inner
            .handle_network_lost()
            .await
            .map_err(|e| ActrError::InternalError { msg: e })?;
        Ok(result.into())
    }

    /// Handle network type changed event.
    pub async fn handle_network_type_changed(
        &self,
        is_wifi: bool,
        is_cellular: bool,
    ) -> ActrResult<NetworkEventResult> {
        let result = self
            .inner
            .handle_network_type_changed(is_wifi, is_cellular)
            .await
            .map_err(|e| ActrError::InternalError { msg: e })?;
        Ok(result.into())
    }

    /// Cleanup all connections.
    pub async fn cleanup_connections(&self) -> ActrResult<NetworkEventResult> {
        let result = self
            .inner
            .cleanup_connections()
            .await
            .map_err(|e| ActrError::InternalError { msg: e })?;
        Ok(result.into())
    }
}

/// Wrapper for a running actor reference.
#[derive(uniffi::Object)]
pub struct ActrRefWrapper {
    inner: ActrRef,
}

#[uniffi::export(async_runtime = "tokio")]
impl ActrRefWrapper {
    /// Get the actor's ID.
    pub fn actor_id(&self) -> ActrId {
        self.inner.actor_id().clone().into()
    }

    /// Discover actors of the specified type.
    pub async fn discover(&self, target_type: ActrType, count: u32) -> ActrResult<Vec<ActrId>> {
        let proto_type: actr_protocol::ActrType = target_type.into();
        info!(
            "discover: looking for {} (count={count})",
            proto_type.to_string_repr(),
        );

        match self
            .inner
            .discover_route_candidates(&proto_type, count as usize)
            .await
        {
            Ok(ids) => {
                info!("discover: found {} candidates", ids.len());
                for id in &ids {
                    debug!("candidate: {}", id.to_string_repr());
                }
                Ok(ids.into_iter().map(Into::into).collect())
            }
            Err(e) => {
                error!("discover failed: {}", e);
                Err(ActrError::RpcError {
                    msg: format!("Discovery failed: {e}"),
                })
            }
        }
    }

    /// Trigger shutdown.
    pub fn shutdown(&self) {
        self.inner.shutdown();
    }

    /// Wait for shutdown to complete.
    pub async fn wait_for_shutdown(&self) {
        self.inner.wait_for_shutdown().await;
    }

    /// Check if shutdown is already in progress.
    pub fn is_shutting_down(&self) -> bool {
        self.inner.is_shutting_down()
    }

    /// Call the local guest workload via RPC.
    pub async fn call(
        &self,
        route_key: String,
        payload_type: PayloadType,
        request_payload: Vec<u8>,
        timeout_ms: i64,
    ) -> ActrResult<Vec<u8>> {
        let proto_payload_type: actr_protocol::PayloadType = payload_type.into();
        let ctx = self.inner.app_context().await;

        let response_bytes = ctx
            .call_raw(
                &Dest::Local,
                route_key,
                proto_payload_type,
                Bytes::from(request_payload),
                timeout_ms,
            )
            .await?;

        Ok(response_bytes.to_vec())
    }

    /// Send a one-way message to the local guest workload.
    pub async fn tell(
        &self,
        route_key: String,
        payload_type: PayloadType,
        message_payload: Vec<u8>,
    ) -> ActrResult<()> {
        let proto_payload_type: actr_protocol::PayloadType = payload_type.into();
        let ctx = self.inner.app_context().await;

        ctx.tell_raw(
            &Dest::Local,
            route_key,
            proto_payload_type,
            Bytes::from(message_payload),
        )
        .await?;

        Ok(())
    }
}
