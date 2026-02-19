//! Service information
//!
//! Defines the basic information structure for services

use crate::config::ActrixConfig;
use crate::monitoring::{ServiceState, service_type::ServiceType};
use actrix_proto::{ResourceType, ServiceStatus as ProtoServiceStatus};
use serde::{Deserialize, Serialize};
use tracing::{error, info};
use url::Url;

/// Basic service information
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

impl ServiceInfo {
    pub fn new(
        name: impl Into<String>,
        service_type: ServiceType,
        description: Option<String>,
        config: &ActrixConfig,
    ) -> Self {
        let (port_info, domain_name) = match service_type {
            ServiceType::Signaling => {
                let (port_info, domain_name) = if config.env == "dev" {
                    // Development environment prefers HTTP
                    if let Some(ref http_config) = config.bind.http {
                        (
                            http_config.port.to_string(),
                            format!("ws://{}", http_config.domain_name),
                        )
                    } else if let Some(ref https_config) = config.bind.https {
                        (
                            https_config.port.to_string(),
                            format!("wss://{}", https_config.domain_name),
                        )
                    } else {
                        ("0".to_string(), "ws://localhost".to_string())
                    }
                } else {
                    // Production environment uses HTTPS
                    if let Some(ref https_config) = config.bind.https {
                        (
                            https_config.port.to_string(),
                            format!("wss://{}", https_config.domain_name),
                        )
                    } else {
                        ("0".to_string(), "wss://localhost".to_string())
                    }
                };
                (port_info, domain_name)
            }
            ServiceType::Turn => {
                let (port_info, domain_name) = (
                    config.bind.ice.port.to_string(),
                    format!("turn:{}", config.bind.ice.domain_name),
                );
                (port_info, domain_name)
            }
            ServiceType::Stun => {
                let (port_info, domain_name) = (
                    config.bind.ice.port.to_string(),
                    format!("stun:{}", config.bind.ice.domain_name),
                );
                (port_info, domain_name)
            }
            ServiceType::Ais => {
                let (port_info, domain_name) = if config.env == "dev" {
                    // Development environment prefers HTTP
                    if let Some(ref http_config) = config.bind.http {
                        (
                            http_config.port.to_string(),
                            format!("http://{}", http_config.domain_name),
                        )
                    } else if let Some(ref https_config) = config.bind.https {
                        (
                            https_config.port.to_string(),
                            format!("https://{}", https_config.domain_name),
                        )
                    } else {
                        ("0".to_string(), "http://localhost".to_string())
                    }
                } else {
                    // Production environment uses HTTPS
                    if let Some(ref https_config) = config.bind.https {
                        (
                            https_config.port.to_string(),
                            format!("https://{}", https_config.domain_name),
                        )
                    } else {
                        ("0".to_string(), "https://localhost".to_string())
                    }
                };
                (port_info, domain_name)
            }
            ServiceType::Ks => {
                let (port_info, domain_name) = if config.env == "dev" {
                    // Development environment prefers HTTP
                    if let Some(ref http_config) = config.bind.http {
                        (
                            http_config.port.to_string(),
                            format!("http://{}", http_config.domain_name),
                        )
                    } else if let Some(ref https_config) = config.bind.https {
                        (
                            https_config.port.to_string(),
                            format!("https://{}", https_config.domain_name),
                        )
                    } else {
                        ("0".to_string(), "http://localhost".to_string())
                    }
                } else {
                    // Production environment uses HTTPS
                    if let Some(ref https_config) = config.bind.https {
                        (
                            https_config.port.to_string(),
                            format!("https://{}", https_config.domain_name),
                        )
                    } else {
                        ("0".to_string(), "https://localhost".to_string())
                    }
                };
                (port_info, domain_name)
            }
        };
        Self {
            name: name.into(),
            service_type,
            port_info,
            domain_name,
            status: ServiceState::Unknown,
            description,
        }
    }

    /// Set service status to running
    pub fn set_running(&mut self, url: Url) {
        self.status = ServiceState::Running(url.to_string());
        info!(
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
        error!(
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

        // Note: Current version of ServiceInfo does not include connection and request statistics
        // These fields can be extended in future versions
        // Currently returning default values: 0 connections, 0 requests, 0ms latency
        Self {
            name: service_info.name.clone(),
            r#type: ResourceType::from(&service_info.service_type).into(),
            is_healthy,
            active_connections: 0,
            total_requests: 0,
            failed_requests: 0,
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
