//! Default NetworkValidator implementation

use anyhow::{Context, Result};
use async_trait::async_trait;
use std::net::ToSocketAddrs;
use std::time::Duration;
use tokio::net::TcpStream;
use url::Url;

use super::{
    ConnectivityStatus, HealthStatus, LatencyInfo, NetworkCheckOptions, NetworkCheckResult,
    NetworkValidator,
};

/// Default network validator
pub struct DefaultNetworkValidator;

impl DefaultNetworkValidator {
    pub fn new() -> Self {
        Self
    }

    /// Try to connect to a host and measure latency
    async fn ping_host(&self, host_port: &str, timeout: Duration) -> Result<Duration> {
        let start = std::time::Instant::now();

        // Attempt TCP connection as a proxy for reachability
        let _stream = tokio::time::timeout(timeout, TcpStream::connect(host_port))
            .await
            .context("Connection timeout")?
            .context("Failed to connect")?;

        Ok(start.elapsed())
    }

    /// Parse a service name or URL into a host:port string
    fn resolve_address(&self, address: &str) -> Result<String> {
        if let Ok(url) = Url::parse(address) {
            let host = url.host_str().context("No host in URL")?;
            let port = url.port_or_known_default().unwrap_or(80);
            Ok(format!("{}:{}", host, port))
        } else if address.contains(':') {
            Ok(address.to_string())
        } else {
            // Assume it's a hostname and try to resolve to verify it exists
            let addr = format!("{}:80", address);
            if addr.to_socket_addrs().is_ok() {
                Ok(addr)
            } else {
                anyhow::bail!("Invalid address format: {}", address)
            }
        }
    }
}

impl Default for DefaultNetworkValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl NetworkValidator for DefaultNetworkValidator {
    async fn check_connectivity(
        &self,
        service_address: &str,
        options: &NetworkCheckOptions,
    ) -> Result<ConnectivityStatus> {
        let timeout = options.timeout;
        match self.resolve_address(service_address) {
            Ok(addr) => match self.ping_host(&addr, timeout).await {
                Ok(latency) => Ok(ConnectivityStatus {
                    is_reachable: true,
                    response_time_ms: Some(latency.as_millis() as u64),
                    error: None,
                }),
                Err(e) => Ok(ConnectivityStatus {
                    is_reachable: false,
                    response_time_ms: None,
                    error: Some(e.to_string()),
                }),
            },
            Err(e) => Ok(ConnectivityStatus {
                is_reachable: false,
                response_time_ms: None,
                error: Some(format!("Address resolution failed: {}", e)),
            }),
        }
    }

    async fn verify_service_health(
        &self,
        service_name: &str,
        options: &NetworkCheckOptions,
    ) -> Result<HealthStatus> {
        let status = self.check_connectivity(service_name, options).await?;
        if status.is_reachable {
            Ok(HealthStatus::Healthy)
        } else {
            Ok(HealthStatus::Unhealthy)
        }
    }

    async fn test_latency(
        &self,
        service_name: &str,
        options: &NetworkCheckOptions,
    ) -> Result<LatencyInfo> {
        let mut samples = Vec::new();
        let timeout = options.timeout;
        let addr = self.resolve_address(service_name)?;

        for _ in 0..3 {
            if let Ok(latency) = self.ping_host(&addr, timeout).await {
                samples.push(latency.as_millis() as u64);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        if samples.is_empty() {
            anyhow::bail!("Failed to get latency samples for {}", service_name);
        }

        let min = *samples.iter().min().unwrap();
        let max = *samples.iter().max().unwrap();
        let avg = samples.iter().sum::<u64>() / samples.len() as u64;

        Ok(LatencyInfo {
            min_ms: min,
            max_ms: max,
            avg_ms: avg,
            samples: samples.len() as u32,
        })
    }

    async fn batch_check(
        &self,
        service_names: &[String],
        options: &NetworkCheckOptions,
    ) -> Result<Vec<NetworkCheckResult>> {
        let mut results = Vec::new();
        for name in service_names {
            let connectivity = self.check_connectivity(name, options).await?;
            let health = if connectivity.is_reachable {
                HealthStatus::Healthy
            } else {
                HealthStatus::Unhealthy
            };

            let latency = if connectivity.is_reachable {
                self.test_latency(name, options).await.ok()
            } else {
                None
            };

            results.push(NetworkCheckResult {
                connectivity,
                health,
                latency,
            });
        }
        Ok(results)
    }
}
