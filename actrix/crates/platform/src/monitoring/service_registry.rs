//! Service registry for managing service statuses

use super::ServiceInfo;
use actrix_proto::ServiceStatus;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Service registry that provides service statuses
///
/// This registry stores ServiceInfo entries and can convert them to ServiceStatus.
#[derive(Clone, Debug)]
pub struct ServiceCollector {
    inner: Arc<RwLock<HashMap<String, ServiceInfo>>>,
}

impl Default for ServiceCollector {
    fn default() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl ServiceCollector {
    /// Create a new service registry
    pub fn new() -> Self {
        Self::default()
    }

    /// Get all service info values
    pub async fn values(&self) -> Vec<ServiceInfo> {
        self.inner
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>()
    }

    /// Insert a service info entry
    pub async fn insert(&self, key: String, value: ServiceInfo) {
        self.inner.write().await.insert(key, value);
    }

    /// Get all service statuses as proto ServiceStatus
    ///
    /// Converts all ServiceInfo entries to ServiceStatus using the From trait.
    pub async fn all_statuses(&self) -> Vec<ServiceStatus> {
        let map = self.inner.read().await;
        map.values()
            .map(|info| ServiceStatus::from(info.clone()))
            .collect()
    }

    /// Get all values directly (returns ServiceInfo)
    pub async fn all_values(&self) -> Vec<ServiceInfo> {
        self.values().await
    }
}
