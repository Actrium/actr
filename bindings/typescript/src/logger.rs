use actr_config::ObservabilityConfig;

/// Initialize logging and observability (tracing).
pub fn init_observability(config: ObservabilityConfig) {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.filter_level));

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_thread_ids(true);

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .init();
}
