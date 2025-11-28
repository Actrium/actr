use crate::error::RuntimeResult;
use actr_config::ObservabilityConfig;
#[cfg(feature = "opentelemetry")]
use opentelemetry::{KeyValue, trace::TracerProvider as _};
#[cfg(feature = "opentelemetry")]
use opentelemetry_otlp::WithExportConfig;
#[cfg(feature = "opentelemetry")]
use opentelemetry_sdk::{
    propagation::TraceContextPropagator, resource::Resource, trace::SdkTracerProvider,
};
use tracing_subscriber::{filter::EnvFilter, fmt, layer::SubscriberExt, prelude::*};

/// Guard for observability resources. Shuts down tracing exporter on drop.
#[derive(Default)]
pub struct ObservabilityGuard {
    #[cfg(feature = "opentelemetry")]
    tracer_provider: Option<SdkTracerProvider>,
}

impl Drop for ObservabilityGuard {
    fn drop(&mut self) {
        #[cfg(feature = "opentelemetry")]
        if let Some(provider) = self.tracer_provider.take() {
            if let Err(err) = provider.shutdown() {
                tracing::warn!("Failed to shutdown tracer provider: {err:?}");
            }
        }
    }
}

/// Initialize logging + (optional) tracing subscriber.
///
/// - `RUST_LOG` wins over configured level; fallback to `info` if unset.
/// - Tracing exporter only activates when both the `opentelemetry` feature is enabled and
///   `cfg.tracing_enabled` is true.
/// - Invalid endpoints fail fast; runtime delivery errors log but do not abort.
pub fn init_observability(
    cfg: &actr_config::ObservabilityConfig,
) -> RuntimeResult<ObservabilityGuard> {
    let level_directive = std::env::var("RUST_LOG")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| cfg.filter_level.clone());
    let env_filter =
        EnvFilter::try_new(level_directive.clone()).unwrap_or_else(|_| EnvFilter::new("info"));

    init_subscriber(cfg, env_filter)
}

#[cfg(not(feature = "opentelemetry"))]
fn init_subscriber(
    _cfg: &ObservabilityConfig,
    env_filter: EnvFilter,
) -> RuntimeResult<ObservabilityGuard> {
    let fmt_layer = fmt::layer()
        .with_target(true)
        .with_level(true)
        .with_line_number(true)
        .with_file(true)
        .with_filter(env_filter);

    let _ = tracing_subscriber::registry().with(fmt_layer).try_init();
    Ok(ObservabilityGuard::default())
}

#[cfg(feature = "opentelemetry")]
fn init_subscriber(
    cfg: &ObservabilityConfig,
    env_filter: EnvFilter,
) -> RuntimeResult<ObservabilityGuard> {
    if cfg.tracing_enabled {
        let fmt_layer = fmt::layer()
            .with_target(true)
            .with_level(true)
            .with_line_number(true)
            .with_file(true)
            .with_filter(env_filter.clone());

        let (otel_layer, provider) = build_otel_layer(cfg, env_filter.clone())?;
        let _ = tracing_subscriber::registry()
            .with(otel_layer)
            .with(fmt_layer)
            .try_init();
        Ok(ObservabilityGuard {
            tracer_provider: Some(provider),
        })
    } else {
        let fmt_layer = fmt::layer()
            .with_target(true)
            .with_level(true)
            .with_line_number(true)
            .with_file(true)
            .with_filter(env_filter.clone());

        let _ = tracing_subscriber::registry().with(fmt_layer).try_init();
        Ok(ObservabilityGuard::default())
    }
}

#[cfg(feature = "opentelemetry")]
fn build_otel_layer(
    config: &ObservabilityConfig,
    env_filter: EnvFilter,
) -> RuntimeResult<(
    impl tracing_subscriber::Layer<tracing_subscriber::Registry>,
    SdkTracerProvider,
)> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(config.tracing_endpoint.clone())
        .build()
        .map_err(|e| {
            crate::error::RuntimeError::InitializationError(format!(
                "OTLP exporter build failed: {e}"
            ))
        })?;

    let resource = Resource::builder()
        .with_service_name(config.tracing_service_name.clone())
        .with_attributes([KeyValue::new("telemetry.sdk.language", "rust")])
        .build();

    let tracer_provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_resource(resource)
        .with_batch_exporter(exporter)
        .build();

    opentelemetry::global::set_tracer_provider(tracer_provider.clone());
    opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

    let tracer = tracer_provider.tracer("actr-runtime");
    let otel_layer = tracing_opentelemetry::layer()
        .with_tracer(tracer)
        .with_filter(env_filter);

    Ok((otel_layer, tracer_provider))
}
