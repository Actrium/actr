//! ACTR-CLI entry point — thin wrapper over [`actr_cli::cli::run`].

use anyhow::Result;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
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
