//! Runtime wrappers for UniFFI export

use crate::error::{ActrError, ActrResult};
use crate::types::NetworkEventResult;
use actr_config::Config;
use actr_hyper::{ActrNode, Hyper, HyperConfig, NetworkEventHandle, TrustMode};
use parking_lot::Mutex;
use std::sync::Arc;
use tracing::{error, info};

/// Wrapper for pre-start actor runtime state.
#[derive(uniffi::Object)]
pub struct ActrSystemWrapper {
    inner: Mutex<Option<ActrNode>>,
    #[allow(dead_code)]
    config: Config,
    network_event_handle: Mutex<Option<NetworkEventHandle>>,
}

#[uniffi::export]
impl ActrSystemWrapper {
    /// Create a new runtime wrapper from configuration file.
    #[uniffi::constructor(async_runtime = "tokio")]
    pub async fn new_from_file(config_path: String) -> ActrResult<Arc<Self>> {
        // Parse configuration first to get observability settings
        let config = actr_config::ConfigParser::from_file(&config_path).map_err(|e| {
            ActrError::ConfigError {
                msg: format!("Failed to parse config file at {}: {}", config_path, e),
            }
        })?;

        // Initialize logger based on configuration
        crate::logger::init_observability(config.observability.clone());

        info!(
            "Creating runtime wrapper with signaling_url={}, realm_id={}",
            config.signaling_url, config.realm.realm_id
        );

        let hyper_data_dir = config.config_dir.join(".hyper");
        let hyper = Hyper::init(HyperConfig::new(&hyper_data_dir).with_trust_mode(
            TrustMode::Development {
                // Pure runtime setup does not verify packages, so a placeholder key is sufficient.
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

        let node = hyper.attach_none(config.clone()).await.map_err(|e| {
            error!("Failed to create runtime node: {}", e);
            ActrError::InternalError {
                msg: format!("Failed to create runtime node: {e}"),
            }
        })?;

        info!("Runtime wrapper created successfully");

        Ok(Arc::new(Self {
            inner: Mutex::new(Some(node)),
            config,
            network_event_handle: Mutex::new(None),
        }))
    }

    /// Create a network event handle for platform callbacks.
    ///
    /// This must be called before host startup.
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

        // Use default debounce behavior (0 = default).
        let handle = node.create_network_event_handle(0);
        *handle_guard = Some(handle.clone());

        Ok(Arc::new(NetworkEventHandleWrapper { inner: handle }))
    }
}

/// Wrapper for `NetworkEventHandle` - network lifecycle callbacks.
#[derive(uniffi::Object)]
pub struct NetworkEventHandleWrapper {
    inner: NetworkEventHandle,
}

#[uniffi::export(async_runtime = "tokio")]
impl NetworkEventHandleWrapper {
    /// Handle network available event
    pub async fn handle_network_available(&self) -> ActrResult<NetworkEventResult> {
        let result = self
            .inner
            .handle_network_available()
            .await
            .map_err(|e| ActrError::InternalError { msg: e })?;
        Ok(result.into())
    }

    /// Handle network lost event
    pub async fn handle_network_lost(&self) -> ActrResult<NetworkEventResult> {
        let result = self
            .inner
            .handle_network_lost()
            .await
            .map_err(|e| ActrError::InternalError { msg: e })?;
        Ok(result.into())
    }

    /// Handle network type changed event
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

    /// Cleanup all connections (does not depend on network events).
    pub async fn cleanup_connections(&self) -> ActrResult<NetworkEventResult> {
        let result = self
            .inner
            .cleanup_connections()
            .await
            .map_err(|e| ActrError::InternalError { msg: e })?;
        Ok(result.into())
    }
}
