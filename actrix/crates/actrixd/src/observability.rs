use platform::config::{ActrixConfig, ObservabilityConfig};
use std::fs;
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_subscriber::{filter::EnvFilter, fmt, prelude::*};

#[cfg(feature = "opentelemetry")]
use crate::error::Error;
use crate::error::Result;
#[cfg(feature = "opentelemetry")]
use opentelemetry::KeyValue;
#[cfg(feature = "opentelemetry")]
use opentelemetry_otlp::WithExportConfig;
#[cfg(feature = "opentelemetry")]
use opentelemetry_sdk::propagation::TraceContextPropagator;
#[cfg(feature = "opentelemetry")]
use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};

/// Guard for observability resources (tracer provider and log writer)
#[derive(Default)]
pub struct ObservabilityGuard {
    #[cfg(feature = "opentelemetry")]
    tracer_provider: Option<SdkTracerProvider>,
    log_guard: Option<WorkerGuard>,
}

impl Drop for ObservabilityGuard {
    fn drop(&mut self) {
        #[cfg(feature = "opentelemetry")]
        if let Some(provider) = self.tracer_provider.take()
            && let Err(e) = provider.shutdown()
        {
            eprintln!("Failed to shutdown tracer provider: {e:?}");
        }
    }
}

/// Initialize logging and tracing based on configuration
pub fn init_observability(config: &ActrixConfig) -> Result<ObservabilityGuard> {
    let mut guard = ObservabilityGuard::default();
    let observability_config = config.observability_config();

    match observability_config.log.output.as_str() {
        "file" => {
            fs::create_dir_all(&observability_config.log.path)?;
            let (non_blocking, worker_guard) =
                build_file_writer(&observability_config.log, observability_config.log.rotate)?;
            guard.log_guard = Some(worker_guard);

            init_subscriber_with_writer(non_blocking, false, &mut guard, config)?;
        }
        _ => {
            init_subscriber_with_writer(std::io::stdout, true, &mut guard, config)?;
        }
    }

    Ok(guard)
}

/// Create an EnvFilter from config, with RUST_LOG taking precedence
fn create_env_filter(config: &ObservabilityConfig) -> EnvFilter {
    let directive = std::env::var("RUST_LOG")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            println!(
                "RUST_LOG not set, using default filter level: {}",
                config.filter_level
            );
            config.filter_level.clone()
        });

    EnvFilter::try_new(&directive).unwrap_or_else(|_| {
        println!("Failed to parse filter directive: {directive}. Falling back to default: info");
        EnvFilter::new("info")
    })
}

fn init_subscriber_with_writer<W>(
    writer: W,
    use_ansi: bool,
    #[cfg_attr(not(feature = "opentelemetry"), allow(unused_variables))]
    guard: &mut ObservabilityGuard,
    config: &ActrixConfig,
) -> Result<()>
where
    W: for<'a> fmt::MakeWriter<'a> + Send + Sync + 'static,
{
    let fmt_layer = fmt::layer()
        .with_target(true)
        .with_level(true)
        .with_line_number(true)
        .with_file(true)
        .with_ansi(use_ansi)
        .with_writer(writer);

    let observability_config = config.observability_config();

    #[cfg(feature = "opentelemetry")]
    {
        let tracer_provider = build_tracing_provider(config)?;

        if let Some(provider) = tracer_provider {
            use opentelemetry::trace::TracerProvider as _;
            let tracer = provider.tracer(observability_config.tracing.service_name().to_string());
            guard.tracer_provider = Some(provider);

            // Global filter: events are filtered first, then passed to all layers
            tracing_subscriber::registry()
                .with(create_env_filter(observability_config))
                .with(fmt_layer)
                .with(tracing_opentelemetry::layer().with_tracer(tracer))
                .try_init()
                .ok();
        } else {
            tracing_subscriber::registry()
                .with(create_env_filter(observability_config))
                .with(fmt_layer)
                .try_init()
                .ok();
        }
    }

    #[cfg(not(feature = "opentelemetry"))]
    {
        tracing_subscriber::registry()
            .with(create_env_filter(observability_config))
            .with(fmt_layer)
            .try_init()
            .ok();
    }

    Ok(())
}

fn build_file_writer(
    log_config: &platform::config::LogConfig,
    rotate: bool,
) -> Result<(NonBlocking, WorkerGuard)> {
    if rotate {
        println!("日志写入模式: 文件");
        println!("  - 路径: {}", log_config.path);
        println!(
            "  - 轮转: {}",
            if rotate {
                "开启（按天）"
            } else {
                "关闭"
            }
        );
        let file_appender = tracing_appender::rolling::daily(&log_config.path, "actrix.log");
        Ok(tracing_appender::non_blocking(file_appender))
    } else {
        let log_file_path = std::path::Path::new(&log_config.path).join("actrix.log");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_file_path)?;
        Ok(tracing_appender::non_blocking(file))
    }
}

#[cfg(feature = "opentelemetry")]
fn build_tracing_provider(config: &ActrixConfig) -> Result<Option<SdkTracerProvider>> {
    let tracing_cfg = config.tracing_config();

    if !tracing_cfg.is_enabled() {
        println!("📊 OpenTelemetry tracing is disabled in config");
        return Ok(None);
    }

    if let Err(e) = tracing_cfg.validate() {
        return Err(Error::custom(e));
    }

    println!(
        "📊 Initializing OpenTelemetry tracing: service_name={}, endpoint={}",
        tracing_cfg.service_name(),
        tracing_cfg.endpoint()
    );

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(tracing_cfg.endpoint())
        .build()
        .map_err(|e| Error::custom(format!("Failed to build OTLP exporter: {e}")))?;

    let resource = Resource::builder()
        .with_service_name(tracing_cfg.service_name().to_string())
        .with_attributes([
            KeyValue::new("service.instance.id", config.name.clone()),
            KeyValue::new("service.environment", config.env.clone()),
            KeyValue::new("service.location", config.location_tag.clone()),
        ])
        .build();

    let tracer_provider = SdkTracerProvider::builder()
        .with_resource(resource)
        .with_batch_exporter(exporter)
        .build();

    opentelemetry::global::set_tracer_provider(tracer_provider.clone());
    opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

    println!("✅ OpenTelemetry tracing initialized successfully");

    Ok(Some(tracer_provider))
}
