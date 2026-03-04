use std::sync::OnceLock;

use actr_config::ObservabilityConfig;
use actr_runtime::init_observability;

// Global guard to keep observability initialized
pub static OBSERVABILITY_GUARD: OnceLock<actr_runtime::ObservabilityGuard> = OnceLock::new();

pub fn ensure_observability_initialized(config: Option<ObservabilityConfig>) {
    OBSERVABILITY_GUARD.get_or_init(|| {
        let config = config.unwrap_or_else(|| {
            let filter_level = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
            ObservabilityConfig {
                filter_level: filter_level.clone(),
                tracing_enabled: false,
                tracing_endpoint: String::new(),
                tracing_service_name: "actr-runtime-py".to_string(),
            }
        });

        init_observability(&config).unwrap_or_else(|e| {
            eprintln!("[warn] Failed to initialize observability: {}", e);
            actr_runtime::ObservabilityGuard::default()
        })
    });
}
