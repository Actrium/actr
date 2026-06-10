//! ACTR-CLI entry point — thin wrapper over [`actr_cli::cli::run`].

use anyhow::Result;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[tokio::main]
async fn main() -> Result<()> {
    // Skip CLI-level tracing init in detached child mode — the runtime
    // sets up its own tracing subscriber via init_observability().
    // If we init here first, the global subscriber with "off" filter
    // prevents init_observability()'s try_init() from succeeding,
    // and all runtime logs are silently dropped.
    let is_detached_child = std::env::args().any(|a| a == "--internal-detached-child");
    if !is_detached_child {
        init_tracing();
    }
    actr_cli::cli::run().await
}

fn init_tracing() {
    let layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_level(true)
        .with_line_number(true)
        .with_file(true);
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("off"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .try_init();
}
