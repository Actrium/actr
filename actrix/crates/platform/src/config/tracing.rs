//! OpenTelemetry tracing configuration
//!
//! Provides configuration for distributed tracing using OpenTelemetry and OTLP protocol.

use serde::{Deserialize, Serialize};

/// Default service name for tracing
fn default_service_name() -> String {
    "actrix".to_string()
}

/// Default OTLP endpoint (Jaeger/Grafana Tempo/etc.)
fn default_endpoint() -> String {
    "http://127.0.0.1:4317".to_string()
}

/// OpenTelemetry tracing configuration
///
/// Enables distributed tracing export to OTLP-compatible backends like Jaeger,
/// Grafana Tempo, or any OpenTelemetry collector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TracingConfig {
    /// Enable tracing (default: false)
    ///
    /// When enabled, requires the `opentelemetry` feature to be compiled.
    #[serde(default)]
    pub enable: bool,

    /// Service name for tracing spans
    ///
    /// Used to identify this service in the tracing backend.
    /// Default: "actrix"
    #[serde(default = "default_service_name")]
    pub service_name: String,

    /// OTLP gRPC endpoint
    ///
    /// Endpoint for OpenTelemetry Protocol export.
    /// Examples:
    /// - Jaeger: http://localhost:4317
    /// - Grafana Tempo: http://tempo:4317
    /// - OpenTelemetry Collector: http://otel-collector:4317
    ///
    /// Default: "http://127.0.0.1:4317"
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            enable: false,
            service_name: default_service_name(),
            endpoint: default_endpoint(),
        }
    }
}

impl TracingConfig {
    /// Validate tracing configuration
    pub fn validate(&self) -> Result<(), String> {
        if self.enable {
            if self.endpoint.trim().is_empty() {
                return Err("Tracing endpoint cannot be empty when tracing is enabled".to_string());
            }

            // Basic URL validation
            if !self.endpoint.starts_with("http://") && !self.endpoint.starts_with("https://") {
                return Err("Tracing endpoint must start with http:// or https://".to_string());
            }
        }
        Ok(())
    }

    /// Check if tracing is enabled
    pub fn is_enabled(&self) -> bool {
        self.enable
    }

    /// Get OTLP endpoint
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Get service name
    pub fn service_name(&self) -> &str {
        &self.service_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_tracing_config() {
        let config = TracingConfig::default();
        assert!(!config.is_enabled());
        assert_eq!(config.service_name(), "actrix");
        assert_eq!(config.endpoint(), "http://127.0.0.1:4317");
    }

    #[test]
    fn test_tracing_config_validation() {
        let mut config = TracingConfig::default();

        // Disabled tracing should validate even with empty endpoint
        assert!(config.validate().is_ok());

        // Enabled tracing with empty endpoint should fail
        config.enable = true;
        config.endpoint = "".to_string();
        assert!(config.validate().is_err());

        // Enabled tracing with invalid endpoint should fail
        config.endpoint = "invalid-url".to_string();
        assert!(config.validate().is_err());

        // Enabled tracing with valid endpoint should pass
        config.endpoint = "http://localhost:4317".to_string();
        assert!(config.validate().is_ok());

        config.endpoint = "https://tempo:4317".to_string();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_tracing_config_deserialize() {
        let toml = r#"
            enable = true
            service_name = "my-service"
            endpoint = "http://jaeger:4317"
        "#;

        let config: TracingConfig = toml::from_str(toml).unwrap();
        assert!(config.is_enabled());
        assert_eq!(config.service_name(), "my-service");
        assert_eq!(config.endpoint(), "http://jaeger:4317");
    }

    #[test]
    fn test_tracing_config_ignores_unknown_fields() {
        // Test that unknown fields in [observability.tracing] are silently ignored
        // This is important: location_tag and actrix_shared_key should NOT be here
        let toml = r#"
            enable = false
            service_name = "test"
            endpoint = "http://127.0.0.1:4317"
            location_tag = "wrong-location"
            actrix_shared_key = "wrong-key"
        "#;

        // Should parse successfully - unknown fields are ignored by serde
        let config: TracingConfig = toml::from_str(toml).unwrap();
        assert!(!config.is_enabled());
        assert_eq!(config.service_name(), "test");
        assert_eq!(config.endpoint(), "http://127.0.0.1:4317");
        // Note: location_tag and actrix_shared_key are ignored, not accessible
    }
}
