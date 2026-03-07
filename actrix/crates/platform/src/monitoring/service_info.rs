//! Service information
//!
//! Defines the basic information structure for services

use crate::config::ActrixConfig;
use crate::monitoring::{ServiceCounters, ServiceState, service_type::ServiceType};
use actrix_proto::{ResourceType, ServiceStatus as ProtoServiceStatus};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use url::Url;

/// Basic service information
#[derive(Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    /// Service name
    pub name: String,
    /// Service type. Turn service is a collection of STUN and TURN
    pub service_type: ServiceType,
    pub domain_name: String,
    pub port_info: String,
    /// Service status
    pub status: ServiceState,
    /// Service description
    pub description: Option<String>,
    /// Live counters for this service (not serialized).
    #[serde(skip)]
    counters: Option<Arc<ServiceCounters>>,
}

impl std::fmt::Debug for ServiceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceInfo")
            .field("name", &self.name)
            .field("service_type", &self.service_type)
            .field("domain_name", &self.domain_name)
            .field("port_info", &self.port_info)
            .field("status", &self.status)
            .field("description", &self.description)
            .field("counters", &self.counters.as_ref().map(|_| "..."))
            .finish()
    }
}

impl ServiceInfo {
    /// Create a ServiceInfo with explicit domain/port (no config needed).
    pub fn new_raw(
        name: impl Into<String>,
        service_type: ServiceType,
        domain_name: String,
        port_info: String,
        description: Option<String>,
    ) -> Self {
        Self {
            name: name.into(),
            service_type,
            port_info,
            domain_name,
            status: ServiceState::Unknown,
            description,
            counters: None,
        }
    }

    pub fn new(
        name: impl Into<String>,
        service_type: ServiceType,
        description: Option<String>,
        config: &ActrixConfig,
    ) -> Self {
        let (port_info, domain_name) = match service_type {
            ServiceType::Signaling => {
                if let Some(ref h) = config.bind.http {
                    (
                        h.port.to_string(),
                        format!("{}://{}", h.ws_scheme(), h.domain_name),
                    )
                } else {
                    ("0".to_string(), "ws://localhost".to_string())
                }
            }
            ServiceType::Turn => (
                config.bind.ice.port.to_string(),
                format!("turn:{}", config.bind.ice.ip),
            ),
            ServiceType::Stun => (
                config.bind.ice.port.to_string(),
                format!("stun:{}", config.bind.ice.ip),
            ),
            ServiceType::Ais | ServiceType::Ks => {
                if let Some(ref h) = config.bind.http {
                    (
                        h.port.to_string(),
                        format!("{}://{}", h.scheme(), h.domain_name),
                    )
                } else {
                    ("0".to_string(), "http://localhost".to_string())
                }
            }
        };
        Self {
            name: name.into(),
            service_type,
            port_info,
            domain_name,
            status: ServiceState::Unknown,
            description,
            counters: None,
        }
    }

    /// Attach live counters to this service.
    pub fn set_counters(&mut self, counters: Arc<ServiceCounters>) {
        self.counters = Some(counters);
    }

    /// Get the live counters (if set).
    pub fn counters(&self) -> Option<&Arc<ServiceCounters>> {
        self.counters.as_ref()
    }

    /// Set service status to running
    pub fn set_running(&mut self, url: Url) {
        self.status = ServiceState::Running(url.to_string());
        crate::recording::info!(
            "Service '{}' is now running at {}/{}",
            self.name,
            self.url(),
            self.domain_name
        );
    }

    /// Set service status to error
    pub fn set_error(&mut self, error: impl Into<String>) {
        let error_msg = error.into();
        self.status = ServiceState::Error(error_msg.clone());
        crate::recording::error!(
            "Service '{}' encountered error: {}/{}",
            self.name,
            self.url(),
            self.domain_name
        );
    }

    /// Check if service is running
    pub fn is_running(&self) -> bool {
        matches!(self.status, ServiceState::Running(_))
    }

    /// Get service status URL (if in running state)
    pub fn url(&self) -> String {
        match &self.status {
            ServiceState::Running(url) => url.to_string(),
            _ => "N/A".to_string(),
        }
    }
}

/// Convert ServiceInfo to proto ServiceStatus
impl From<&ServiceInfo> for ProtoServiceStatus {
    fn from(service_info: &ServiceInfo) -> Self {
        let is_healthy = matches!(service_info.status, ServiceState::Running(_));

        // Parse port number (extract digits from port_info)
        let port = service_info.port_info.parse::<u32>().unwrap_or(0);

        // Build URL
        let url = service_info.url();

        // Read live counters if available, otherwise return defaults.
        let (active_connections, total_requests, failed_requests) =
            if let Some(ctr) = &service_info.counters {
                (
                    ctr.active_conns.load(std::sync::atomic::Ordering::Relaxed),
                    ctr.total_requests
                        .load(std::sync::atomic::Ordering::Relaxed),
                    ctr.failed_requests
                        .load(std::sync::atomic::Ordering::Relaxed),
                )
            } else {
                (0, 0, 0)
            };

        Self {
            name: service_info.name.clone(),
            r#type: ResourceType::from(&service_info.service_type).into(),
            is_healthy,
            active_connections,
            total_requests,
            failed_requests,
            average_latency_ms: 0.0,
            url: Some(url),
            port: if port > 0 { Some(port) } else { None },
            domain: if service_info.domain_name != "N/A" {
                Some(service_info.domain_name.clone())
            } else {
                None
            },
        }
    }
}

/// Convert ServiceInfo to proto ServiceStatus (owned version)
impl From<ServiceInfo> for ProtoServiceStatus {
    fn from(service_info: ServiceInfo) -> Self {
        Self::from(&service_info)
    }
}
