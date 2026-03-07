//! Unified runtime recording model.
//!
//! This module provides a typed `Record<{...}>` model that can be routed to
//! observability/audit/security/operations channels with one entry point.

use chrono::Utc;
use once_cell::sync::Lazy;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::fmt;
use std::sync::OnceLock;
use thiserror::Error;
use uuid::Uuid;

/// Recording channel mask: observability.
pub const CHANNEL_OBSERVABILITY: u8 = 0b0001;
/// Recording channel mask: audit.
pub const CHANNEL_AUDIT: u8 = 0b0010;
/// Recording channel mask: security.
pub const CHANNEL_SECURITY: u8 = 0b0100;
/// Recording channel mask: operations.
pub const CHANNEL_OPERATIONS: u8 = 0b1000;

const CHANNEL_MASK_ALL: u8 =
    CHANNEL_OBSERVABILITY | CHANNEL_AUDIT | CHANNEL_SECURITY | CHANNEL_OPERATIONS;

static FALLBACK_SOURCE_NODE: Lazy<Option<String>> = Lazy::new(detect_source_node);

/// Stable identifier for each recording item.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct RecordId(pub String);

impl RecordId {
    /// Create a new random record id.
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl Default for RecordId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for RecordId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Unified severity level for runtime recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RecordLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

/// Result/outcome of one runtime action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Success,
    Failure,
    Denied,
    Timeout,
    #[default]
    Unknown,
}

/// Common metadata shared by all channel payloads.
#[derive(Debug, Clone, Serialize)]
pub struct Common {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    pub outcome: Outcome,
    pub level: RecordLevel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_service: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_node: Option<String>,
}

impl Default for Common {
    fn default() -> Self {
        Self {
            actor: None,
            outcome: Outcome::Unknown,
            level: RecordLevel::Info,
            trace_id: None,
            span_id: None,
            request_id: None,
            source_service: None,
            source_node: None,
        }
    }
}

/// Observability-specific payload.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ObservabilityPayload {
    /// Human readable summary for operators.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Component/subsystem that emitted the record.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    /// Logical operation name, such as `http.route.add`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    /// Network/runtime protocol context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<Protocol>,
    /// HTTP/gRPC/ws route or endpoint pattern.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    /// HTTP/gRPC method verb.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Response status code when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
    /// End-to-end duration in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Runtime/request protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    #[default]
    Internal,
    Http,
    Grpc,
    #[serde(rename = "websocket")]
    WebSocket,
    Stun,
    Turn,
    Tcp,
    Udp,
}

impl ObservabilityPayload {
    fn validate(&self) -> Result<(), RecordingError> {
        if self.summary.is_none()
            && self.component.is_none()
            && self.operation.is_none()
            && self.protocol.is_none()
            && self.route.is_none()
            && self.method.is_none()
            && self.status_code.is_none()
            && self.duration_ms.is_none()
        {
            return Err(RecordingError::EmptyPayload {
                channel: "observability",
            });
        }

        validate_optional_non_empty("observability", "summary", &self.summary)?;
        validate_optional_non_empty("observability", "component", &self.component)?;
        validate_optional_non_empty("observability", "operation", &self.operation)?;
        validate_optional_non_empty("observability", "route", &self.route)?;
        validate_optional_non_empty("observability", "method", &self.method)?;

        Ok(())
    }
}

/// Audit-specific payload.
#[derive(Debug, Clone, Serialize)]
pub struct AuditPayload {
    /// Action verb, e.g. `config.update`.
    pub action: String,
    /// Target resource class, e.g. `service.binding`.
    pub resource: String,
    /// Stable resource id when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    /// Optional business/operator reason for the action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Optional source address for the action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_addr: Option<String>,
    /// Optional session/correlation id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Before snapshot (JSON) for change actions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    /// After snapshot (JSON) for change actions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
}

impl AuditPayload {
    fn validate(&self) -> Result<(), RecordingError> {
        validate_required_non_empty("audit", "action", &self.action)?;
        validate_required_non_empty("audit", "resource", &self.resource)?;
        validate_optional_non_empty("audit", "resource_id", &self.resource_id)?;
        validate_optional_non_empty("audit", "reason", &self.reason)?;
        validate_optional_non_empty("audit", "remote_addr", &self.remote_addr)?;
        validate_optional_non_empty("audit", "session_id", &self.session_id)?;

        Ok(())
    }
}

/// Security impact level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SecuritySeverity {
    Low,
    #[default]
    Medium,
    High,
    Critical,
}

// ── Per-channel semantic filters ────────────────────────────────────────

/// Observability channel filter — controls resolution (how much detail).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ObservabilityFilter {
    Off,
    #[default]
    Digest,
    Detailed,
    Full,
}

/// Audit channel filter — controls scope (which actions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuditFilter {
    Off,
    #[default]
    Mutations,
    All,
}

/// Security channel filter — controls severity threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SecurityFilter {
    Off,
    Critical,
    High,
    Medium,
    #[default]
    All,
}

/// Operations channel filter — controls detail level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OperationsFilter {
    Off,
    #[default]
    Lifecycle,
    Detailed,
}

/// Holds the active filter for each channel.
#[derive(Debug, Clone, Default)]
pub struct ChannelFilters {
    pub observability: ObservabilityFilter,
    pub audit: AuditFilter,
    pub security: SecurityFilter,
    pub operations: OperationsFilter,
}

static CHANNEL_FILTERS: OnceLock<ChannelFilters> = OnceLock::new();

/// Install channel filters (call once at pipeline init).
pub fn set_channel_filters(filters: ChannelFilters) {
    let _ = CHANNEL_FILTERS.set(filters);
}

/// Read the active channel filters (returns defaults if never set).
pub fn get_channel_filters() -> &'static ChannelFilters {
    static DEFAULT: Lazy<ChannelFilters> = Lazy::new(ChannelFilters::default);
    CHANNEL_FILTERS.get().unwrap_or(&DEFAULT)
}

/// Parse a config string into an `ObservabilityFilter`.
pub fn parse_observability_filter(s: &str) -> ObservabilityFilter {
    match s {
        "off" => ObservabilityFilter::Off,
        "digest" => ObservabilityFilter::Digest,
        "detailed" => ObservabilityFilter::Detailed,
        "full" => ObservabilityFilter::Full,
        _ => ObservabilityFilter::default(),
    }
}

/// Parse a config string into an `AuditFilter`.
pub fn parse_audit_filter(s: &str) -> AuditFilter {
    match s {
        "off" => AuditFilter::Off,
        "mutations" => AuditFilter::Mutations,
        "all" => AuditFilter::All,
        _ => AuditFilter::default(),
    }
}

/// Parse a config string into a `SecurityFilter`.
pub fn parse_security_filter(s: &str) -> SecurityFilter {
    match s {
        "off" => SecurityFilter::Off,
        "critical" => SecurityFilter::Critical,
        "high" => SecurityFilter::High,
        "medium" => SecurityFilter::Medium,
        "all" => SecurityFilter::All,
        _ => SecurityFilter::default(),
    }
}

/// Parse a config string into an `OperationsFilter`.
pub fn parse_operations_filter(s: &str) -> OperationsFilter {
    match s {
        "off" => OperationsFilter::Off,
        "lifecycle" => OperationsFilter::Lifecycle,
        "detailed" => OperationsFilter::Detailed,
        _ => OperationsFilter::default(),
    }
}

// ── Gate functions ──────────────────────────────────────────────────────

/// Read-only verbs for audit mutation detection.
const AUDIT_READ_PREFIXES: &[&str] = &["read", "get", "list", "query", "check", "view", "describe"];

fn is_audit_read_action(action: &str) -> bool {
    let lower = action.to_ascii_lowercase();
    AUDIT_READ_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

fn should_emit_observability(common: &Common, filter: ObservabilityFilter) -> bool {
    match filter {
        ObservabilityFilter::Off => false,
        ObservabilityFilter::Digest => matches!(
            common.level,
            RecordLevel::Info | RecordLevel::Warn | RecordLevel::Error
        ),
        ObservabilityFilter::Detailed => matches!(
            common.level,
            RecordLevel::Debug | RecordLevel::Info | RecordLevel::Warn | RecordLevel::Error
        ),
        ObservabilityFilter::Full => true,
    }
}

fn should_emit_audit(payload: &AuditPayload, filter: AuditFilter) -> bool {
    match filter {
        AuditFilter::Off => false,
        AuditFilter::Mutations => !is_audit_read_action(&payload.action),
        AuditFilter::All => true,
    }
}

fn should_emit_security(payload: &SecurityPayload, filter: SecurityFilter) -> bool {
    match filter {
        SecurityFilter::Off => false,
        SecurityFilter::Critical => matches!(payload.severity, SecuritySeverity::Critical),
        SecurityFilter::High => matches!(
            payload.severity,
            SecuritySeverity::High | SecuritySeverity::Critical
        ),
        SecurityFilter::Medium => matches!(
            payload.severity,
            SecuritySeverity::Medium | SecuritySeverity::High | SecuritySeverity::Critical
        ),
        SecurityFilter::All => true,
    }
}

fn should_emit_operations(common: &Common, filter: OperationsFilter) -> bool {
    match filter {
        OperationsFilter::Off => false,
        OperationsFilter::Lifecycle => matches!(
            common.level,
            RecordLevel::Info | RecordLevel::Warn | RecordLevel::Error
        ),
        OperationsFilter::Detailed => true,
    }
}

/// Security-specific payload.
#[derive(Debug, Clone, Serialize)]
pub struct SecurityPayload {
    /// Security control/policy/rule that evaluated this action.
    pub control: String,
    /// Severity for triage/alerting.
    pub severity: SecuritySeverity,
    /// Security category, e.g. `authn`, `authz`, `abuse`, `network`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Subject under evaluation (actor/user/device/service).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Source address when network-related.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_addr: Option<String>,
    /// Destination address when network-related.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_addr: Option<String>,
    /// Extra evidence/context (JSON).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Value>,
}

impl SecurityPayload {
    fn validate(&self) -> Result<(), RecordingError> {
        validate_required_non_empty("security", "control", &self.control)?;
        validate_optional_non_empty("security", "category", &self.category)?;
        validate_optional_non_empty("security", "subject", &self.subject)?;
        validate_optional_non_empty("security", "source_addr", &self.source_addr)?;
        validate_optional_non_empty("security", "destination_addr", &self.destination_addr)?;

        Ok(())
    }
}

/// Operations-specific payload.
#[derive(Debug, Clone, Serialize)]
pub struct OperationsPayload {
    /// Operation name, e.g. `service.restart`.
    pub operation: String,
    /// Component/service affected by the operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    /// Runbook name/id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runbook: Option<String>,
    /// Ticket/incident id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
    /// Deployment/change id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_id: Option<String>,
    /// Retry/attempt number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    /// Execution window in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

impl OperationsPayload {
    fn validate(&self) -> Result<(), RecordingError> {
        validate_required_non_empty("operations", "operation", &self.operation)?;
        validate_optional_non_empty("operations", "component", &self.component)?;
        validate_optional_non_empty("operations", "runbook", &self.runbook)?;
        validate_optional_non_empty("operations", "ticket", &self.ticket)?;
        validate_optional_non_empty("operations", "change_id", &self.change_id)?;

        Ok(())
    }
}

#[derive(Debug, Clone)]
struct Callsite {
    file: &'static str,
    line: u32,
    module_path: &'static str,
}

fn enrich_common(mut common: Common, callsite: &Callsite) -> Common {
    common.actor = sanitize_optional_string(common.actor);
    common.request_id = sanitize_optional_string(common.request_id);

    let explicit_trace_id = sanitize_optional_string(common.trace_id);
    let explicit_span_id = sanitize_optional_string(common.span_id);
    let (auto_trace_id, auto_span_id) = if explicit_trace_id.is_none() || explicit_span_id.is_none()
    {
        detect_current_trace_context()
    } else {
        (None, None)
    };
    common.trace_id = explicit_trace_id.or(auto_trace_id);
    common.span_id = explicit_span_id.or(auto_span_id);

    let explicit_source_service = sanitize_optional_string(common.source_service);
    common.source_service = explicit_source_service.or_else(|| detect_source_service(callsite));

    let explicit_source_node = sanitize_optional_string(common.source_node);
    common.source_node = explicit_source_node.or_else(|| FALLBACK_SOURCE_NODE.as_ref().cloned());

    common
}

fn sanitize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(sanitize_string)
}

fn sanitize_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_string())
}

fn detect_source_service(callsite: &Callsite) -> Option<String> {
    let source_service = std::env::var("ACTRIX_SOURCE_SERVICE")
        .ok()
        .and_then(sanitize_string);
    if source_service.is_some() {
        return source_service;
    }

    if callsite.module_path == "unknown" {
        return None;
    }

    callsite
        .module_path
        .split("::")
        .next()
        .and_then(|value| sanitize_string(value.to_string()))
}

fn detect_source_node() -> Option<String> {
    std::env::var("ACTRIX_SOURCE_NODE")
        .ok()
        .and_then(sanitize_string)
        .or_else(|| std::env::var("HOSTNAME").ok().and_then(sanitize_string))
        .or_else(|| std::env::var("COMPUTERNAME").ok().and_then(sanitize_string))
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .and_then(sanitize_string)
        })
}

#[cfg(feature = "opentelemetry")]
fn detect_current_trace_context() -> (Option<String>, Option<String>) {
    use opentelemetry::trace::TraceContextExt;
    use tracing_opentelemetry::OpenTelemetrySpanExt;

    let context = tracing::Span::current().context();
    let span_ref = context.span();
    let span_context = span_ref.span_context();
    if !span_context.is_valid() {
        return (None, None);
    }

    (
        Some(span_context.trace_id().to_string()),
        Some(span_context.span_id().to_string()),
    )
}

#[cfg(not(feature = "opentelemetry"))]
fn detect_current_trace_context() -> (Option<String>, Option<String>) {
    (None, None)
}

/// Full recording item, with channel mask on type level.
#[derive(Debug, Clone)]
pub struct Record<const CHANNEL_MASK: u8> {
    pub id: RecordId,
    pub kind: String,
    pub common: Common,
    pub observability: Option<ObservabilityPayload>,
    pub audit: Option<AuditPayload>,
    pub security: Option<SecurityPayload>,
    pub operations: Option<OperationsPayload>,
    pub attributes: Map<String, Value>,
}

impl<const CHANNEL_MASK: u8> Record<CHANNEL_MASK> {
    /// Create a new record with defaults.
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            id: RecordId::new(),
            kind: kind.into(),
            common: Common::default(),
            observability: None,
            audit: None,
            security: None,
            operations: None,
            attributes: Map::new(),
        }
    }

    fn validate(&self) -> Result<(), RecordingError> {
        validate_channel_mask::<CHANNEL_MASK>()?;

        if self.kind.trim().is_empty() {
            return Err(RecordingError::EmptyKind);
        }

        validate_channel_payload(
            CHANNEL_MASK,
            CHANNEL_OBSERVABILITY,
            self.observability.is_some(),
            "observability",
        )?;
        validate_channel_payload(CHANNEL_MASK, CHANNEL_AUDIT, self.audit.is_some(), "audit")?;
        validate_channel_payload(
            CHANNEL_MASK,
            CHANNEL_SECURITY,
            self.security.is_some(),
            "security",
        )?;
        validate_channel_payload(
            CHANNEL_MASK,
            CHANNEL_OPERATIONS,
            self.operations.is_some(),
            "operations",
        )?;

        if let Some(payload) = &self.observability {
            payload.validate()?;
        }
        if let Some(payload) = &self.audit {
            payload.validate()?;
        }
        if let Some(payload) = &self.security {
            payload.validate()?;
        }
        if let Some(payload) = &self.operations {
            payload.validate()?;
        }

        Ok(())
    }
}

/// Runtime recording validation/emit error.
#[derive(Debug, Error)]
pub enum RecordingError {
    #[error("recording channel mask cannot be empty")]
    EmptyChannelMask,
    #[error("recording channel mask has unsupported bits: {mask:#010b}")]
    UnsupportedChannelMask { mask: u8 },
    #[error("recording kind cannot be empty")]
    EmptyKind,
    #[error("channel '{channel}' was selected but payload is missing")]
    MissingSelectedChannelPayload { channel: &'static str },
    #[error("channel '{channel}' payload is provided but channel was not selected")]
    UnexpectedChannelPayload { channel: &'static str },
    #[error("channel '{channel}' field '{field}' cannot be empty")]
    EmptyPayloadField {
        channel: &'static str,
        field: &'static str,
    },
    #[error("channel '{channel}' payload cannot be empty")]
    EmptyPayload { channel: &'static str },
    #[error("failed to serialize '{channel}' payload: {error}")]
    SerializePayload {
        channel: &'static str,
        #[source]
        error: serde_json::Error,
    },
}

/// Emit a typed recording through current tracing backend.
pub fn emit<const CHANNEL_MASK: u8>(record: Record<CHANNEL_MASK>) -> Result<(), RecordingError> {
    emit_with_callsite(record, "unknown", 0, "unknown")
}

/// Emit a typed recording with explicit callsite metadata.
pub fn emit_with_callsite<const CHANNEL_MASK: u8>(
    mut record: Record<CHANNEL_MASK>,
    file: &'static str,
    line: u32,
    module_path: &'static str,
) -> Result<(), RecordingError> {
    let callsite = Callsite {
        file,
        line,
        module_path,
    };
    record.common = enrich_common(record.common, &callsite);
    record.validate()?;

    let filters = get_channel_filters();
    let level = record.common.level;
    let common = record.common;
    let Record {
        id,
        kind,
        common: _,
        observability,
        audit,
        security,
        operations,
        attributes,
    } = record;

    if let Some(payload) = observability
        && should_emit_observability(&common, filters.observability)
    {
        let value = build_channel_record(
            "observability",
            &id,
            &kind,
            &common,
            payload,
            &attributes,
            &callsite,
        )?;
        emit_channel(level, Channel::Observability, &value);
    }

    if let Some(payload) = audit
        && should_emit_audit(&payload, filters.audit)
    {
        let value = build_channel_record(
            "audit",
            &id,
            &kind,
            &common,
            payload,
            &attributes,
            &callsite,
        )?;
        emit_channel(level, Channel::Audit, &value);
    }

    if let Some(payload) = security
        && should_emit_security(&payload, filters.security)
    {
        let value = build_channel_record(
            "security",
            &id,
            &kind,
            &common,
            payload,
            &attributes,
            &callsite,
        )?;
        emit_channel(level, Channel::Security, &value);
    }

    if let Some(payload) = operations
        && should_emit_operations(&common, filters.operations)
    {
        let value = build_channel_record(
            "operations",
            &id,
            &kind,
            &common,
            payload,
            &attributes,
            &callsite,
        )?;
        emit_channel(level, Channel::Operations, &value);
    }

    Ok(())
}

/// Emit one serialized channel record through the unified tracing pipeline.
#[derive(Debug, Clone, Copy)]
enum Channel {
    Observability,
    Audit,
    Security,
    Operations,
}

/// Emit one serialized channel record through the unified tracing pipeline.
#[allow(clippy::disallowed_macros)]
fn emit_channel(level: RecordLevel, channel: Channel, record: &Value) {
    match channel {
        Channel::Observability => match level {
            RecordLevel::Trace => {
                tracing::event!(target: "actrix::observability", tracing::Level::TRACE, recording = %record)
            }
            RecordLevel::Debug => {
                tracing::event!(target: "actrix::observability", tracing::Level::DEBUG, recording = %record)
            }
            RecordLevel::Info => {
                tracing::event!(target: "actrix::observability", tracing::Level::INFO, recording = %record)
            }
            RecordLevel::Warn => {
                tracing::event!(target: "actrix::observability", tracing::Level::WARN, recording = %record)
            }
            RecordLevel::Error => {
                tracing::event!(target: "actrix::observability", tracing::Level::ERROR, recording = %record)
            }
        },
        Channel::Audit => match level {
            RecordLevel::Trace => {
                tracing::event!(target: "actrix::audit", tracing::Level::TRACE, recording = %record)
            }
            RecordLevel::Debug => {
                tracing::event!(target: "actrix::audit", tracing::Level::DEBUG, recording = %record)
            }
            RecordLevel::Info => {
                tracing::event!(target: "actrix::audit", tracing::Level::INFO, recording = %record)
            }
            RecordLevel::Warn => {
                tracing::event!(target: "actrix::audit", tracing::Level::WARN, recording = %record)
            }
            RecordLevel::Error => {
                tracing::event!(target: "actrix::audit", tracing::Level::ERROR, recording = %record)
            }
        },
        Channel::Security => match level {
            RecordLevel::Trace => {
                tracing::event!(target: "actrix::security", tracing::Level::TRACE, recording = %record)
            }
            RecordLevel::Debug => {
                tracing::event!(target: "actrix::security", tracing::Level::DEBUG, recording = %record)
            }
            RecordLevel::Info => {
                tracing::event!(target: "actrix::security", tracing::Level::INFO, recording = %record)
            }
            RecordLevel::Warn => {
                tracing::event!(target: "actrix::security", tracing::Level::WARN, recording = %record)
            }
            RecordLevel::Error => {
                tracing::event!(target: "actrix::security", tracing::Level::ERROR, recording = %record)
            }
        },
        Channel::Operations => match level {
            RecordLevel::Trace => {
                tracing::event!(target: "actrix::operations", tracing::Level::TRACE, recording = %record)
            }
            RecordLevel::Debug => {
                tracing::event!(target: "actrix::operations", tracing::Level::DEBUG, recording = %record)
            }
            RecordLevel::Info => {
                tracing::event!(target: "actrix::operations", tracing::Level::INFO, recording = %record)
            }
            RecordLevel::Warn => {
                tracing::event!(target: "actrix::operations", tracing::Level::WARN, recording = %record)
            }
            RecordLevel::Error => {
                tracing::event!(target: "actrix::operations", tracing::Level::ERROR, recording = %record)
            }
        },
    }
}

fn validate_required_non_empty(
    channel: &'static str,
    field: &'static str,
    value: &str,
) -> Result<(), RecordingError> {
    if value.trim().is_empty() {
        return Err(RecordingError::EmptyPayloadField { channel, field });
    }

    Ok(())
}

fn validate_optional_non_empty(
    channel: &'static str,
    field: &'static str,
    value: &Option<String>,
) -> Result<(), RecordingError> {
    if let Some(v) = value
        && v.trim().is_empty()
    {
        return Err(RecordingError::EmptyPayloadField { channel, field });
    }

    Ok(())
}

const fn validate_channel_mask<const CHANNEL_MASK: u8>() -> Result<(), RecordingError> {
    if CHANNEL_MASK == 0 {
        return Err(RecordingError::EmptyChannelMask);
    }

    if CHANNEL_MASK & !CHANNEL_MASK_ALL != 0 {
        return Err(RecordingError::UnsupportedChannelMask { mask: CHANNEL_MASK });
    }

    Ok(())
}

fn validate_channel_payload(
    selected_mask: u8,
    channel_mask: u8,
    has_payload: bool,
    channel_name: &'static str,
) -> Result<(), RecordingError> {
    let selected = selected_mask & channel_mask != 0;

    if selected && !has_payload {
        return Err(RecordingError::MissingSelectedChannelPayload {
            channel: channel_name,
        });
    }

    if !selected && has_payload {
        return Err(RecordingError::UnexpectedChannelPayload {
            channel: channel_name,
        });
    }

    Ok(())
}

fn build_channel_record<P: Serialize>(
    channel_name: &'static str,
    record_id: &RecordId,
    kind: &str,
    common: &Common,
    payload: P,
    attributes: &Map<String, Value>,
    callsite: &Callsite,
) -> Result<Value, RecordingError> {
    let payload_value =
        serde_json::to_value(payload).map_err(|error| RecordingError::SerializePayload {
            channel: channel_name,
            error,
        })?;

    Ok(json!({
        "recording_id": record_id,
        "kind": kind,
        "channel": channel_name,
        "timestamp": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "common": common,
        "callsite": {
            "file": callsite.file,
            "line": callsite.line,
            "module": callsite.module_path,
        },
        "process": {
            "pid": std::process::id(),
            "thread": format!("{:?}", std::thread::current().id()),
        },
        "attributes": attributes,
        "payload": payload_value,
    }))
}

/// Macro entry-point.
///
/// Usage:
/// `platform::recording::log!(Record::<{ CHANNEL_OBSERVABILITY }> { ... });`
#[macro_export]
macro_rules! recording_log {
    ($record:expr $(,)?) => {{ $crate::recording::emit_with_callsite($record, file!(), line!(), module_path!()) }};
}

/// Re-export macro as `recording::log!`.
pub use crate::recording_log as log;

/// Emit a minimal observability record from format-style log arguments.
///
/// This is a migration bridge for legacy `info!/warn!/error!/debug!/trace!`
/// callsites. New code should prefer explicit `recording::log!(Record::<...> { ... })`.
#[macro_export]
macro_rules! recording_observability_log {
    ($level:expr, $($arg:tt)+) => {{
        let _ = $crate::recording::emit_with_callsite(
            $crate::recording::Record::<{ $crate::recording::CHANNEL_OBSERVABILITY }> {
                id: $crate::recording::RecordId::new(),
                kind: "runtime.log".to_string(),
                common: $crate::recording::Common {
                    level: $level,
                    ..Default::default()
                },
                observability: Some($crate::recording::ObservabilityPayload {
                    summary: Some(format!($($arg)+)),
                    ..Default::default()
                }),
                audit: None,
                security: None,
                operations: None,
                attributes: Default::default(),
            },
            file!(),
            line!(),
            module_path!(),
        );
    }};
}

#[macro_export]
macro_rules! recording_trace {
    ($($arg:tt)+) => {{
        $crate::recording_observability_log!($crate::recording::RecordLevel::Trace, $($arg)+);
    }};
}

#[macro_export]
macro_rules! recording_debug {
    ($($arg:tt)+) => {{
        $crate::recording_observability_log!($crate::recording::RecordLevel::Debug, $($arg)+);
    }};
}

#[macro_export]
macro_rules! recording_info {
    ($($arg:tt)+) => {{
        $crate::recording_observability_log!($crate::recording::RecordLevel::Info, $($arg)+);
    }};
}

#[macro_export]
macro_rules! recording_warn {
    ($($arg:tt)+) => {{
        $crate::recording_observability_log!($crate::recording::RecordLevel::Warn, $($arg)+);
    }};
}

#[macro_export]
macro_rules! recording_error {
    ($($arg:tt)+) => {{
        $crate::recording_observability_log!($crate::recording::RecordLevel::Error, $($arg)+);
    }};
}

/// Re-export migration bridge macros under `recording::*`.
pub use crate::recording_debug as debug;
pub use crate::recording_error as error;
pub use crate::recording_info as info;
pub use crate::recording_trace as trace;
pub use crate::recording_warn as warn;

#[cfg(test)]
mod tests {
    use super::*;

    fn base_record<const CHANNEL_MASK: u8>() -> Record<CHANNEL_MASK> {
        Record::new("test.recording")
    }

    fn make_callsite(module_path: &'static str) -> Callsite {
        Callsite {
            file: "test.rs",
            line: 42,
            module_path,
        }
    }

    #[test]
    fn enrich_common_should_preserve_explicit_fields() {
        let common = Common {
            actor: Some("actor:1".to_string()),
            outcome: Outcome::Success,
            level: RecordLevel::Info,
            trace_id: Some("trace-explicit".to_string()),
            span_id: Some("span-explicit".to_string()),
            request_id: Some("request-1".to_string()),
            source_service: Some("service-explicit".to_string()),
            source_node: Some("node-explicit".to_string()),
        };

        let enriched = enrich_common(common, &make_callsite("actrix::service::manager"));

        assert_eq!(enriched.actor.as_deref(), Some("actor:1"));
        assert_eq!(enriched.trace_id.as_deref(), Some("trace-explicit"));
        assert_eq!(enriched.span_id.as_deref(), Some("span-explicit"));
        assert_eq!(enriched.request_id.as_deref(), Some("request-1"));
        assert_eq!(enriched.source_service.as_deref(), Some("service-explicit"));
        assert_eq!(enriched.source_node.as_deref(), Some("node-explicit"));
    }

    #[test]
    fn enrich_common_should_fill_source_service_from_module_path() {
        let common = Common::default();
        let enriched = enrich_common(common, &make_callsite("actrix::service::manager"));

        assert_eq!(enriched.source_service.as_deref(), Some("actrix"));
    }

    #[test]
    fn enrich_common_should_treat_blank_explicit_values_as_missing() {
        let common = Common {
            source_service: Some("  ".to_string()),
            request_id: Some("   ".to_string()),
            ..Default::default()
        };

        let enriched = enrich_common(common, &make_callsite("actrix::service::manager"));

        assert_eq!(enriched.source_service.as_deref(), Some("actrix"));
        assert!(enriched.request_id.is_none());
    }

    #[test]
    fn emit_observability_record_should_succeed() {
        let mut record = base_record::<{ CHANNEL_OBSERVABILITY }>();
        record.observability = Some(ObservabilityPayload {
            summary: Some("ready".to_string()),
            component: Some("health.check".to_string()),
            operation: Some("http.health".to_string()),
            protocol: Some(Protocol::Http),
            route: Some("/health".to_string()),
            method: Some("GET".to_string()),
            ..Default::default()
        });

        assert!(emit(record).is_ok());
    }

    #[test]
    fn emit_should_fail_when_selected_channel_payload_is_missing() {
        let record = base_record::<{ CHANNEL_AUDIT }>();
        let error = emit(record).expect_err("audit payload should be required");

        assert!(matches!(
            error,
            RecordingError::MissingSelectedChannelPayload { channel: "audit" }
        ));
    }

    #[test]
    fn emit_should_fail_when_unselected_channel_payload_is_present() {
        let mut record = base_record::<{ CHANNEL_OBSERVABILITY }>();
        record.observability = Some(ObservabilityPayload {
            summary: Some("http route mounted".to_string()),
            ..Default::default()
        });
        record.audit = Some(AuditPayload {
            action: "config.update".to_string(),
            resource: "bind.http.port".to_string(),
            resource_id: None,
            reason: None,
            remote_addr: None,
            session_id: None,
            before: Some(json!("8080")),
            after: Some(json!("8443")),
        });

        let error = emit(record).expect_err("audit payload should not be accepted");

        assert!(matches!(
            error,
            RecordingError::UnexpectedChannelPayload { channel: "audit" }
        ));
    }

    #[test]
    fn emit_should_fail_on_empty_channel_mask() {
        let record = base_record::<0>();
        let error = emit(record).expect_err("empty mask should fail");

        assert!(matches!(error, RecordingError::EmptyChannelMask));
    }

    #[test]
    fn emit_should_fail_when_required_payload_field_is_empty() {
        let mut record = base_record::<{ CHANNEL_AUDIT }>();
        record.audit = Some(AuditPayload {
            action: String::new(),
            resource: "service.binding".to_string(),
            resource_id: None,
            reason: None,
            remote_addr: None,
            session_id: None,
            before: None,
            after: None,
        });

        let error = emit(record).expect_err("empty action should fail validation");

        assert!(matches!(
            error,
            RecordingError::EmptyPayloadField {
                channel: "audit",
                field: "action"
            }
        ));
    }

    #[test]
    fn emit_should_fail_when_optional_payload_field_is_blank() {
        let mut record = base_record::<{ CHANNEL_SECURITY }>();
        record.security = Some(SecurityPayload {
            control: "rate_limit.connection".to_string(),
            severity: SecuritySeverity::High,
            category: Some(String::new()),
            subject: None,
            source_addr: None,
            destination_addr: None,
            evidence: None,
        });

        let error = emit(record).expect_err("blank optional field should fail validation");

        assert!(matches!(
            error,
            RecordingError::EmptyPayloadField {
                channel: "security",
                field: "category"
            }
        ));
    }

    #[test]
    fn emit_should_fail_when_observability_payload_is_empty() {
        let mut record = base_record::<{ CHANNEL_OBSERVABILITY }>();
        record.observability = Some(ObservabilityPayload::default());

        let error = emit(record).expect_err("empty observability payload should fail validation");

        assert!(matches!(
            error,
            RecordingError::EmptyPayload {
                channel: "observability"
            }
        ));
    }

    // ── Filter parse tests ──

    #[test]
    fn parse_observability_filter_values() {
        assert_eq!(parse_observability_filter("off"), ObservabilityFilter::Off);
        assert_eq!(
            parse_observability_filter("digest"),
            ObservabilityFilter::Digest
        );
        assert_eq!(
            parse_observability_filter("detailed"),
            ObservabilityFilter::Detailed
        );
        assert_eq!(
            parse_observability_filter("full"),
            ObservabilityFilter::Full
        );
        assert_eq!(
            parse_observability_filter("bogus"),
            ObservabilityFilter::Digest
        );
    }

    #[test]
    fn parse_audit_filter_values() {
        assert_eq!(parse_audit_filter("off"), AuditFilter::Off);
        assert_eq!(parse_audit_filter("mutations"), AuditFilter::Mutations);
        assert_eq!(parse_audit_filter("all"), AuditFilter::All);
        assert_eq!(parse_audit_filter("bogus"), AuditFilter::Mutations);
    }

    #[test]
    fn parse_security_filter_values() {
        assert_eq!(parse_security_filter("off"), SecurityFilter::Off);
        assert_eq!(parse_security_filter("critical"), SecurityFilter::Critical);
        assert_eq!(parse_security_filter("high"), SecurityFilter::High);
        assert_eq!(parse_security_filter("medium"), SecurityFilter::Medium);
        assert_eq!(parse_security_filter("all"), SecurityFilter::All);
        assert_eq!(parse_security_filter("bogus"), SecurityFilter::All);
    }

    #[test]
    fn parse_operations_filter_values() {
        assert_eq!(parse_operations_filter("off"), OperationsFilter::Off);
        assert_eq!(
            parse_operations_filter("lifecycle"),
            OperationsFilter::Lifecycle
        );
        assert_eq!(
            parse_operations_filter("detailed"),
            OperationsFilter::Detailed
        );
        assert_eq!(
            parse_operations_filter("bogus"),
            OperationsFilter::Lifecycle
        );
    }

    // ── Gate logic tests ──

    #[test]
    fn observability_gate_digest_passes_info_blocks_debug() {
        let info = Common {
            level: RecordLevel::Info,
            ..Default::default()
        };
        let debug = Common {
            level: RecordLevel::Debug,
            ..Default::default()
        };
        let trace = Common {
            level: RecordLevel::Trace,
            ..Default::default()
        };
        assert!(should_emit_observability(
            &info,
            ObservabilityFilter::Digest
        ));
        assert!(!should_emit_observability(
            &debug,
            ObservabilityFilter::Digest
        ));
        assert!(!should_emit_observability(
            &trace,
            ObservabilityFilter::Digest
        ));
    }

    #[test]
    fn observability_gate_detailed_passes_debug_blocks_trace() {
        let debug = Common {
            level: RecordLevel::Debug,
            ..Default::default()
        };
        let trace = Common {
            level: RecordLevel::Trace,
            ..Default::default()
        };
        assert!(should_emit_observability(
            &debug,
            ObservabilityFilter::Detailed
        ));
        assert!(!should_emit_observability(
            &trace,
            ObservabilityFilter::Detailed
        ));
    }

    #[test]
    fn observability_gate_full_passes_all() {
        let trace = Common {
            level: RecordLevel::Trace,
            ..Default::default()
        };
        assert!(should_emit_observability(&trace, ObservabilityFilter::Full));
    }

    #[test]
    fn observability_gate_off_blocks_all() {
        let error = Common {
            level: RecordLevel::Error,
            ..Default::default()
        };
        assert!(!should_emit_observability(&error, ObservabilityFilter::Off));
    }

    #[test]
    fn audit_gate_mutations_filters_reads() {
        let write_payload = AuditPayload {
            action: "config.update".to_string(),
            resource: "bind".to_string(),
            resource_id: None,
            reason: None,
            remote_addr: None,
            session_id: None,
            before: None,
            after: None,
        };
        let read_payload = AuditPayload {
            action: "read.config".to_string(),
            resource: "bind".to_string(),
            resource_id: None,
            reason: None,
            remote_addr: None,
            session_id: None,
            before: None,
            after: None,
        };
        let list_payload = AuditPayload {
            action: "list.realms".to_string(),
            resource: "realm".to_string(),
            resource_id: None,
            reason: None,
            remote_addr: None,
            session_id: None,
            before: None,
            after: None,
        };
        assert!(should_emit_audit(&write_payload, AuditFilter::Mutations));
        assert!(!should_emit_audit(&read_payload, AuditFilter::Mutations));
        assert!(!should_emit_audit(&list_payload, AuditFilter::Mutations));
        assert!(should_emit_audit(&read_payload, AuditFilter::All));
    }

    #[test]
    fn security_gate_severity_threshold() {
        let low = SecurityPayload {
            control: "test".to_string(),
            severity: SecuritySeverity::Low,
            category: None,
            subject: None,
            source_addr: None,
            destination_addr: None,
            evidence: None,
        };
        let high = SecurityPayload {
            control: "test".to_string(),
            severity: SecuritySeverity::High,
            category: None,
            subject: None,
            source_addr: None,
            destination_addr: None,
            evidence: None,
        };
        let critical = SecurityPayload {
            control: "test".to_string(),
            severity: SecuritySeverity::Critical,
            category: None,
            subject: None,
            source_addr: None,
            destination_addr: None,
            evidence: None,
        };
        assert!(!should_emit_security(&low, SecurityFilter::High));
        assert!(should_emit_security(&high, SecurityFilter::High));
        assert!(should_emit_security(&critical, SecurityFilter::High));
        assert!(should_emit_security(&critical, SecurityFilter::Critical));
        assert!(!should_emit_security(&high, SecurityFilter::Critical));
        assert!(should_emit_security(&low, SecurityFilter::All));
    }

    #[test]
    fn operations_gate_lifecycle_passes_info() {
        let info = Common {
            level: RecordLevel::Info,
            ..Default::default()
        };
        let debug = Common {
            level: RecordLevel::Debug,
            ..Default::default()
        };
        assert!(should_emit_operations(&info, OperationsFilter::Lifecycle));
        assert!(!should_emit_operations(&debug, OperationsFilter::Lifecycle));
        assert!(should_emit_operations(&debug, OperationsFilter::Detailed));
    }
}
