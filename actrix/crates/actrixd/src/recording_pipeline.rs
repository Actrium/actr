use platform::config::{ActrixConfig, RecordingConfig};
use std::fs;
use std::path::PathBuf;
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_subscriber::{
    filter::EnvFilter,
    fmt::{self, writer::BoxMakeWriter},
    prelude::*,
};
use url::Url;

use crate::error::{Error, Result};
#[cfg(feature = "opentelemetry")]
use opentelemetry::KeyValue;
#[cfg(feature = "opentelemetry")]
use opentelemetry_otlp::WithExportConfig;
#[cfg(feature = "opentelemetry")]
use opentelemetry_sdk::propagation::TraceContextPropagator;
#[cfg(feature = "opentelemetry")]
use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};
#[cfg(feature = "opentelemetry")]
use tracing_subscriber::filter::filter_fn;

const TARGET_OBSERVABILITY: &str = "actrix::observability";
const TARGET_AUDIT: &str = "actrix::audit";
const TARGET_SECURITY: &str = "actrix::security";
const TARGET_OPERATIONS: &str = "actrix::operations";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordingChannel {
    Observability,
    Audit,
    Security,
    Operations,
}

impl RecordingChannel {
    #[cfg(feature = "opentelemetry")]
    fn service_suffix(self) -> &'static str {
        match self {
            Self::Observability => "observability",
            Self::Audit => "audit",
            Self::Security => "security",
            Self::Operations => "operations",
        }
    }
}

fn recording_channel_from_target(target: &str) -> Option<RecordingChannel> {
    match target {
        TARGET_OBSERVABILITY => Some(RecordingChannel::Observability),
        TARGET_AUDIT => Some(RecordingChannel::Audit),
        TARGET_SECURITY => Some(RecordingChannel::Security),
        TARGET_OPERATIONS => Some(RecordingChannel::Operations),
        _ => None,
    }
}

fn normalize_optional_string(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
}

#[cfg(feature = "opentelemetry")]
fn effective_channel_sink_uri(
    config: &RecordingConfig,
    channel: RecordingChannel,
) -> Option<String> {
    let global = normalize_optional_string(&config.sink);
    match channel {
        RecordingChannel::Observability => {
            normalize_optional_string(&config.observability.sink).or(global)
        }
        RecordingChannel::Audit => normalize_optional_string(&config.audit.sink).or(global),
        RecordingChannel::Security => normalize_optional_string(&config.security.sink).or(global),
        RecordingChannel::Operations => {
            normalize_optional_string(&config.operations.sink).or(global)
        }
    }
}

#[derive(Debug, Clone)]
enum RecordingSink {
    File(PathBuf),
    OtlpHttp,
    OtlpGrpc,
}

#[cfg(not(feature = "opentelemetry"))]
fn has_any_otlp_sink(config: &RecordingConfig) -> bool {
    [
        &config.sink,
        &config.observability.sink,
        &config.audit.sink,
        &config.security.sink,
        &config.operations.sink,
    ]
    .into_iter()
    .filter_map(normalize_optional_string)
    .any(|sink| sink.starts_with("otlp+http://") || sink.starts_with("otlp+grpc://"))
}

fn parse_recording_sink(uri: &str) -> Result<RecordingSink> {
    let parsed = Url::parse(uri)
        .map_err(|error| Error::custom(format!("Invalid recording sink URI '{uri}': {error}")))?;

    match parsed.scheme() {
        "file" => parsed
            .to_file_path()
            .map(RecordingSink::File)
            .map_err(|_| Error::custom(format!("Invalid file URI path '{uri}'"))),
        "otlp+http" => Ok(RecordingSink::OtlpHttp),
        "otlp+grpc" => Ok(RecordingSink::OtlpGrpc),
        other => Err(Error::custom(format!(
            "Unsupported recording sink scheme '{other}', expected file://, otlp+http:// or otlp+grpc://"
        ))),
    }
}

fn parse_optional_sink(uri: Option<&str>) -> Result<Option<RecordingSink>> {
    match uri {
        Some(uri) => parse_recording_sink(uri).map(Some),
        None => Ok(None),
    }
}

#[cfg(feature = "opentelemetry")]
fn build_sink_writer(guard: &mut RecordingPipelineGuard) -> NonBlocking {
    let (writer, worker_guard) = tracing_appender::non_blocking(std::io::sink());
    guard.log_guards.push(worker_guard);
    writer
}

fn build_stdout_writer(guard: &mut RecordingPipelineGuard) -> NonBlocking {
    let (writer, worker_guard) = tracing_appender::non_blocking(std::io::stdout());
    guard.log_guards.push(worker_guard);
    writer
}

fn build_file_writer(path: &PathBuf, guard: &mut RecordingPipelineGuard) -> Result<NonBlocking> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let (writer, worker_guard) = tracing_appender::non_blocking(file);
    guard.log_guards.push(worker_guard);
    Ok(writer)
}

fn build_writer_from_sink(
    sink: &RecordingSink,
    guard: &mut RecordingPipelineGuard,
) -> Result<NonBlocking> {
    match sink {
        RecordingSink::File(path) => build_file_writer(path, guard),
        RecordingSink::OtlpHttp | RecordingSink::OtlpGrpc => {
            #[cfg(feature = "opentelemetry")]
            {
                Ok(build_sink_writer(guard))
            }
            #[cfg(not(feature = "opentelemetry"))]
            {
                Ok(build_stdout_writer(guard))
            }
        }
    }
}

#[cfg(feature = "opentelemetry")]
fn otlp_endpoint_from_sink_uri(uri: &str) -> Result<String> {
    if let Some(rest) = uri.strip_prefix("otlp+http://") {
        return Ok(format!("http://{rest}"));
    }
    if let Some(rest) = uri.strip_prefix("otlp+grpc://") {
        return Ok(format!("http://{rest}"));
    }

    Err(Error::custom(format!("Invalid OTLP sink URI '{uri}'")))
}

#[cfg(feature = "opentelemetry")]
#[derive(Debug, Clone, Copy)]
enum OtlpTransport {
    Http,
    Grpc,
}

#[cfg(feature = "opentelemetry")]
#[derive(Debug, Clone)]
struct OtlpEndpoint {
    endpoint: String,
    transport: OtlpTransport,
}

#[cfg(feature = "opentelemetry")]
fn effective_channel_otlp_endpoint(
    config: &RecordingConfig,
    channel: RecordingChannel,
) -> Result<Option<OtlpEndpoint>> {
    let Some(uri) = effective_channel_sink_uri(config, channel) else {
        return Ok(None);
    };

    match parse_recording_sink(&uri)? {
        RecordingSink::File(_) => Ok(None),
        RecordingSink::OtlpHttp => Ok(Some(OtlpEndpoint {
            endpoint: otlp_endpoint_from_sink_uri(&uri)?,
            transport: OtlpTransport::Http,
        })),
        RecordingSink::OtlpGrpc => Ok(Some(OtlpEndpoint {
            endpoint: otlp_endpoint_from_sink_uri(&uri)?,
            transport: OtlpTransport::Grpc,
        })),
    }
}

/// Guard for recording pipeline resources (tracer provider and log writer).
#[derive(Default)]
pub struct RecordingPipelineGuard {
    #[cfg(feature = "opentelemetry")]
    tracer_providers: Vec<SdkTracerProvider>,
    log_guards: Vec<WorkerGuard>,
}

impl Drop for RecordingPipelineGuard {
    fn drop(&mut self) {
        #[cfg(feature = "opentelemetry")]
        for provider in self.tracer_providers.drain(..) {
            if let Err(error) = provider.shutdown() {
                eprintln!("Failed to shutdown tracer provider: {error:?}");
            }
        }
    }
}

#[derive(Clone)]
struct ChannelRoutingMakeWriter {
    default: NonBlocking,
    observability: Option<NonBlocking>,
    audit: Option<NonBlocking>,
    security: Option<NonBlocking>,
    operations: Option<NonBlocking>,
}

impl<'a> fmt::MakeWriter<'a> for ChannelRoutingMakeWriter {
    type Writer = NonBlocking;

    fn make_writer(&'a self) -> Self::Writer {
        self.default.clone()
    }

    fn make_writer_for(&'a self, metadata: &tracing::Metadata<'_>) -> Self::Writer {
        let writer = match recording_channel_from_target(metadata.target()) {
            Some(RecordingChannel::Observability) => self.observability.as_ref(),
            Some(RecordingChannel::Audit) => self.audit.as_ref(),
            Some(RecordingChannel::Security) => self.security.as_ref(),
            Some(RecordingChannel::Operations) => self.operations.as_ref(),
            None => None,
        };

        writer.cloned().unwrap_or_else(|| self.default.clone())
    }
}

/// Initialize unified recording pipeline (logging + tracing) based on configuration.
pub fn init_recording_pipeline(config: &ActrixConfig) -> Result<RecordingPipelineGuard> {
    let mut guard = RecordingPipelineGuard::default();
    let recording_config = config.recording_config();

    let (writer, use_ansi) = build_log_writer(recording_config, &mut guard)?;
    init_subscriber(writer, use_ansi, &mut guard, config)?;

    Ok(guard)
}

/// Create an EnvFilter from config, with RUST_LOG taking precedence.
fn create_env_filter(config: &RecordingConfig) -> EnvFilter {
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

fn build_log_writer(
    recording_config: &RecordingConfig,
    guard: &mut RecordingPipelineGuard,
) -> Result<(BoxMakeWriter, bool)> {
    let global_sink_uri = normalize_optional_string(&recording_config.sink);
    let observability_override_uri =
        normalize_optional_string(&recording_config.observability.sink);
    let audit_override_uri = normalize_optional_string(&recording_config.audit.sink);
    let security_override_uri = normalize_optional_string(&recording_config.security.sink);
    let operations_override_uri = normalize_optional_string(&recording_config.operations.sink);

    println!("日志写入模式: sink");
    if let Some(uri) = &global_sink_uri {
        println!("  - recording.sink: {uri}");
    }
    if let Some(uri) = &observability_override_uri {
        println!("  - recording.observability.sink: {uri}");
    }
    if let Some(uri) = &audit_override_uri {
        println!("  - recording.audit.sink: {uri}");
    }
    if let Some(uri) = &security_override_uri {
        println!("  - recording.security.sink: {uri}");
    }
    if let Some(uri) = &operations_override_uri {
        println!("  - recording.operations.sink: {uri}");
    }

    let global_sink = parse_optional_sink(global_sink_uri.as_deref())?;
    let observability_override_sink = parse_optional_sink(observability_override_uri.as_deref())?;
    let audit_override_sink = parse_optional_sink(audit_override_uri.as_deref())?;
    let security_override_sink = parse_optional_sink(security_override_uri.as_deref())?;
    let operations_override_sink = parse_optional_sink(operations_override_uri.as_deref())?;

    let default_writer = match &global_sink {
        Some(sink) => build_writer_from_sink(sink, guard)?,
        None => build_stdout_writer(guard),
    };

    let observability_writer = match &observability_override_sink {
        Some(sink) => Some(build_writer_from_sink(sink, guard)?),
        None => None,
    };

    let audit_writer = match &audit_override_sink {
        Some(sink) => Some(build_writer_from_sink(sink, guard)?),
        None => None,
    };

    let security_writer = match &security_override_sink {
        Some(sink) => Some(build_writer_from_sink(sink, guard)?),
        None => None,
    };

    let operations_writer = match &operations_override_sink {
        Some(sink) => Some(build_writer_from_sink(sink, guard)?),
        None => None,
    };

    let writer = ChannelRoutingMakeWriter {
        default: default_writer,
        observability: observability_writer,
        audit: audit_writer,
        security: security_writer,
        operations: operations_writer,
    };

    let use_ansi = match &global_sink {
        Some(RecordingSink::File(_)) => false,
        Some(RecordingSink::OtlpHttp | RecordingSink::OtlpGrpc) => {
            #[cfg(feature = "opentelemetry")]
            {
                false
            }
            #[cfg(not(feature = "opentelemetry"))]
            {
                true
            }
        }
        None => true,
    };
    Ok((BoxMakeWriter::new(writer), use_ansi))
}

fn init_subscriber(
    writer: BoxMakeWriter,
    use_ansi: bool,
    #[cfg_attr(not(feature = "opentelemetry"), allow(unused_variables))]
    guard: &mut RecordingPipelineGuard,
    config: &ActrixConfig,
) -> Result<()> {
    let fmt_layer = fmt::layer()
        .with_target(true)
        .with_level(true)
        .with_line_number(true)
        .with_file(true)
        .with_ansi(use_ansi)
        .with_writer(writer);

    let recording_config = config.recording_config();

    #[cfg(feature = "opentelemetry")]
    {
        let observability_otlp =
            effective_channel_otlp_endpoint(recording_config, RecordingChannel::Observability)?;
        let audit_otlp =
            effective_channel_otlp_endpoint(recording_config, RecordingChannel::Audit)?;
        let security_otlp =
            effective_channel_otlp_endpoint(recording_config, RecordingChannel::Security)?;
        let operations_otlp =
            effective_channel_otlp_endpoint(recording_config, RecordingChannel::Operations)?;

        if observability_otlp.is_none()
            && audit_otlp.is_none()
            && security_otlp.is_none()
            && operations_otlp.is_none()
        {
            println!("📊 OpenTelemetry tracing is disabled (no otlp+* sink configured)");
            tracing_subscriber::registry()
                .with(create_env_filter(recording_config))
                .with(fmt_layer)
                .try_init()
                .ok();
            return Ok(());
        }

        println!(
            "📊 Initializing OpenTelemetry tracing: service_name={}",
            config.recording_service_name()
        );

        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());
        use opentelemetry::trace::TracerProvider as _;

        let mut observability_layer = None;
        let mut audit_layer = None;
        let mut security_layer = None;
        let mut operations_layer = None;

        if let Some(otlp) = &observability_otlp {
            let provider =
                build_tracing_provider(config, otlp, Some(RecordingChannel::Observability))?;
            let tracer = provider.tracer(format!(
                "{}.{}",
                config.recording_service_name(),
                RecordingChannel::Observability.service_suffix()
            ));

            if guard.tracer_providers.is_empty() {
                opentelemetry::global::set_tracer_provider(provider.clone());
            }

            guard.tracer_providers.push(provider);
            observability_layer = Some(
                tracing_opentelemetry::layer()
                    .with_tracer(tracer)
                    .with_filter(filter_fn(otlp_observability_filter)),
            );
        }

        if let Some(otlp) = &audit_otlp {
            let provider = build_tracing_provider(config, otlp, Some(RecordingChannel::Audit))?;
            let tracer = provider.tracer(format!(
                "{}.{}",
                config.recording_service_name(),
                RecordingChannel::Audit.service_suffix()
            ));

            if guard.tracer_providers.is_empty() {
                opentelemetry::global::set_tracer_provider(provider.clone());
            }

            guard.tracer_providers.push(provider);
            audit_layer = Some(
                tracing_opentelemetry::layer()
                    .with_tracer(tracer)
                    .with_filter(filter_fn(otlp_audit_filter)),
            );
        }

        if let Some(otlp) = &security_otlp {
            let provider = build_tracing_provider(config, otlp, Some(RecordingChannel::Security))?;
            let tracer = provider.tracer(format!(
                "{}.{}",
                config.recording_service_name(),
                RecordingChannel::Security.service_suffix()
            ));

            if guard.tracer_providers.is_empty() {
                opentelemetry::global::set_tracer_provider(provider.clone());
            }

            guard.tracer_providers.push(provider);
            security_layer = Some(
                tracing_opentelemetry::layer()
                    .with_tracer(tracer)
                    .with_filter(filter_fn(otlp_security_filter)),
            );
        }

        if let Some(otlp) = &operations_otlp {
            let provider =
                build_tracing_provider(config, otlp, Some(RecordingChannel::Operations))?;
            let tracer = provider.tracer(format!(
                "{}.{}",
                config.recording_service_name(),
                RecordingChannel::Operations.service_suffix()
            ));

            if guard.tracer_providers.is_empty() {
                opentelemetry::global::set_tracer_provider(provider.clone());
            }

            guard.tracer_providers.push(provider);
            operations_layer = Some(
                tracing_opentelemetry::layer()
                    .with_tracer(tracer)
                    .with_filter(filter_fn(otlp_operations_filter)),
            );
        }

        tracing_subscriber::registry()
            .with(create_env_filter(recording_config))
            .with(fmt_layer)
            .with(observability_layer)
            .with(audit_layer)
            .with(security_layer)
            .with(operations_layer)
            .try_init()
            .ok();

        println!("✅ OpenTelemetry tracing initialized successfully");
    }

    #[cfg(not(feature = "opentelemetry"))]
    {
        if has_any_otlp_sink(recording_config) {
            println!("⚠️ otlp+* sink is configured but this build does not enable `opentelemetry`");
        }

        tracing_subscriber::registry()
            .with(create_env_filter(recording_config))
            .with(fmt_layer)
            .try_init()
            .ok();
    }

    Ok(())
}

#[cfg(feature = "opentelemetry")]
fn build_tracing_provider(
    config: &ActrixConfig,
    otlp: &OtlpEndpoint,
    channel: Option<RecordingChannel>,
) -> Result<SdkTracerProvider> {
    let exporter = match otlp.transport {
        OtlpTransport::Grpc => opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(&otlp.endpoint)
            .build(),
        OtlpTransport::Http => opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_endpoint(&otlp.endpoint)
            .build(),
    }
    .map_err(|error| Error::custom(format!("Failed to build OTLP exporter: {error}")))?;

    let mut attributes = vec![
        KeyValue::new("service.instance.id", config.name.clone()),
        KeyValue::new("service.environment", config.env.clone()),
        KeyValue::new("service.location", config.location_tag.clone()),
    ];

    let service_name = if let Some(channel) = channel {
        attributes.push(KeyValue::new("recording.channel", channel.service_suffix()));
        format!(
            "{}.{}",
            config.recording_service_name(),
            channel.service_suffix()
        )
    } else {
        config.recording_service_name().to_string()
    };

    let resource = Resource::builder()
        .with_service_name(service_name)
        .with_attributes(attributes)
        .build();

    Ok(SdkTracerProvider::builder()
        .with_resource(resource)
        .with_batch_exporter(exporter)
        .build())
}

#[cfg(feature = "opentelemetry")]
fn otlp_observability_filter(metadata: &tracing::Metadata<'_>) -> bool {
    if metadata.is_span() {
        return true;
    }

    match recording_channel_from_target(metadata.target()) {
        Some(RecordingChannel::Audit)
        | Some(RecordingChannel::Security)
        | Some(RecordingChannel::Operations) => false,
        Some(RecordingChannel::Observability) | None => true,
    }
}

#[cfg(feature = "opentelemetry")]
fn otlp_audit_filter(metadata: &tracing::Metadata<'_>) -> bool {
    metadata.is_event()
        && matches!(
            recording_channel_from_target(metadata.target()),
            Some(RecordingChannel::Audit)
        )
}

#[cfg(feature = "opentelemetry")]
fn otlp_security_filter(metadata: &tracing::Metadata<'_>) -> bool {
    metadata.is_event()
        && matches!(
            recording_channel_from_target(metadata.target()),
            Some(RecordingChannel::Security)
        )
}

#[cfg(feature = "opentelemetry")]
fn otlp_operations_filter(metadata: &tracing::Metadata<'_>) -> bool {
    metadata.is_event()
        && matches!(
            recording_channel_from_target(metadata.target()),
            Some(RecordingChannel::Operations)
        )
}
