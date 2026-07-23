//! Network Event Handling Architecture
//!
//! This module defines the network event handling infrastructure.
//!
//! # Architecture Overview
//!
//! ```text
//!        ┌─────────────────────────────────────────────┐
//!        │ (FFI Path - Implemented)  (Actor Path - TODO)
//!        ▼                                             ▼
//! ┌──────────────────────────┐      ┌──────────────────────────┐
//! │ NetworkEventHandle       │      │ Direct Proto Message     │
//! │ • Platform FFI calls     │      │ • Actor call/tell        │
//! │ • Send via channel       │      │ • Send to actor mailbox  │
//! │ • Await result           │      │ • No handle needed       │
//! └────────┬─────────────────┘      └──────┬───────────────────┘
//!          │                               │
//!          └───────────────┬───────────────┘
//!                          │ Both trigger
//!                          ▼
//! ┌─────────────────────────────────────────────────────────┐
//! │  ActrNode::network_event_loop()                         │
//! │  • Receive event from channel (FFI path)                │
//! │  • Or handle message directly (Actor path - TODO)       │
//! │  • Delegate to NetworkEventProcessor                    │
//! │  • Send result back via channel                         │
//! └──────────────────────┬──────────────────────────────────┘
//!                        │ Delegate
//!                        ▼
//! ┌─────────────────────────────────────────────────────────┐
//! │  NetworkEventProcessor (Trait)                          │
//! │                                                          │
//! │  DefaultNetworkEventProcessor:                          │
//! │  • feed normalized network/app inputs into policy FSM   │
//! │  • execute one recovery action                          │
//! │    └─► Offline / Probe / Restore / Cleanup / Reconnect  │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! # Key Components
//!
//! - **NetworkEvent**: Unified mobile network/app/command events
//! - **NetworkEventResult**: Processing result with success/error/duration
//! - **NetworkEventProcessor**: Trait for custom event handling logic
//! - **DefaultNetworkEventProcessor**: Default implementation with signaling + WebRTC recovery
//!
//! # Usage Patterns
//!
//! ## 1. Platform FFI Call (Primary, Implemented)
//! ```ignore
//! // Platform layer calls NetworkEventHandle via FFI
//! let network_handle = system.create_network_event_handle();
//! let result = network_handle.handle_network_path_changed(snapshot).await?;
//! if result.success {
//!     println!("Processed in {}ms", result.duration_ms);
//! }
//! ```
//!
//! ## 2. Actor Proto Message (Optional, TODO)
//! ```ignore
//! // TODO: actors send proto message directly (not yet implemented)
//! actor_ref.call(NetworkPathChangedMessage { snapshot }).await?;
//! ```
//!
//! **Key Differences:**
//! - FFI path: Uses NetworkEventHandle + channel (implemented)
//! - Actor path: Direct proto message to mailbox (TODO, future enhancement)

use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use crate::transport::{NetworkError, PeerTransport};
use crate::wire::webrtc::{CleanupGuard, SignalingClient, WebRtcCoordinator};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

use super::recovery_policy::diagnosis::{AbortCause, EffectDiagnosis, EffectOutcome};
use super::recovery_policy::translate as tp;
use super::recovery_supervisor::{
    RecoverySupervisor, StartedEffect, TimerOp, event_to_input, network_action_of,
};

const NETWORK_EVENT_RESULT_TIMEOUT: Duration = Duration::from_secs(5);
const SIGNALING_PROBE_TIMEOUT: Duration = Duration::from_secs(1);
pub(super) const LONG_BACKGROUND_RECONNECT_THRESHOLD_MS: u64 = 30_000;
static NEXT_NETWORK_EVENT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_NETWORK_RECOVERY_ACTION_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_NETWORK_SOURCE_EPOCH: AtomicU64 = AtomicU64::new(1);

/// Keeps outbound sends behind a network-event lifecycle barrier while the
/// reconciler evaluates and processes a queued decision cycle.
pub struct NetworkEventBarrier {
    _cleanup_guard: CleanupGuard,
}

/// Mobile network path snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NetworkSnapshot {
    pub sequence: u64,
    pub availability: NetworkAvailability,
    pub transport: NetworkTransportFlags,
    pub is_expensive: bool,
    pub is_constrained: bool,
}

impl NetworkSnapshot {
    pub fn is_offline(&self) -> bool {
        matches!(self.availability, NetworkAvailability::Unavailable)
    }

    pub fn should_restore(&self) -> bool {
        matches!(self.availability, NetworkAvailability::Available)
    }
}

/// Whether the platform currently has a usable network path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NetworkAvailability {
    Unknown,
    Available,
    Unavailable,
}

/// Active network transport flags. Multiple flags can be true at the same time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct NetworkTransportFlags {
    pub wifi: bool,
    pub cellular: bool,
    pub ethernet: bool,
    pub vpn: bool,
    pub other: bool,
}

/// App lifecycle state relevant to connection recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppLifecycleState {
    Background,
    Foreground { background_duration_ms: u64 },
}

/// Reason for a cleanup-only operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CleanupReason {
    AppTerminating,
    UserLogout,
    StaleConnectionSuspected,
    ManualReset,
}

/// Reason for a forced cleanup + restore operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReconnectReason {
    NetworkPathChanged,
    LongBackground,
    ProbeFailed,
    ManualReconnect,
    StaleConnectionSuspected,
}

/// Network event type
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NetworkEvent {
    /// Full mobile network path changed.
    NetworkPathChanged { snapshot: NetworkSnapshot },

    /// App lifecycle changed.
    AppLifecycleChanged { state: AppLifecycleState },

    /// Proactively clean up all connections
    ///
    /// Used when the caller explicitly wants teardown without reconnection,
    /// such as user logout, app termination, or a manual reset. Merely entering
    /// the background is represented by `AppLifecycleChanged::Background` and
    /// does not tear down a healthy connection.
    CleanupConnections { reason: CleanupReason },

    /// Proactively clean up and restore connections.
    ForceReconnect { reason: ReconnectReason },
}

/// Final action selected from one network-event decision cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkRecoveryAction {
    Noop,
    Offline,
    Probe,
    Restore,
    CleanupOnly,
    ForceReconnect,
}

/// Typed failure reported by a network recovery effect.
///
/// This is deliberately smaller than [`NetworkError`]: an effect only reports
/// observations that the recovery policy can classify without inspecting an
/// opaque error string. In particular, generic transport failures remain
/// availability-family `PathUnreachable` unless the effect has independently
/// verified a stronger fact.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum NetworkRecoveryError {
    #[error("path unreachable: {stage}")]
    PathUnreachable { stage: String },
    #[error("operation timed out: {stage}")]
    Timeout { stage: String },
    #[error("resource exhausted: {resource}")]
    ResourceExhausted { resource: String },
    #[error("authentication rejected: {kind}")]
    AuthRejected { kind: String },
    #[error("configuration rejected: {detail}")]
    ConfigRejected { detail: String },
    #[error("recovery invariant violated: {detail}")]
    InvariantViolation { detail: String },
}

impl NetworkRecoveryError {
    fn from_opaque(stage: &str, detail: impl std::fmt::Display) -> Self {
        Self::PathUnreachable {
            stage: format!("{stage}: {detail}"),
        }
    }

    fn from_network_error(stage: &str, error: NetworkError) -> Self {
        let detail = format!("{stage}: {error}");
        match error {
            NetworkError::TimeoutError(_) => Self::Timeout { stage: detail },
            NetworkError::ResourceExhaustedError(_) => Self::ResourceExhausted { resource: detail },
            NetworkError::AuthenticationError(_)
            | NetworkError::CredentialExpired(_)
            | NetworkError::PermissionError(_) => Self::AuthRejected { kind: detail },
            NetworkError::ProtocolError(_)
            | NetworkError::ConfigurationError(_)
            | NetworkError::ServiceDiscoveryError(_)
            | NetworkError::NotImplemented(_)
            | NetworkError::NoRoute(_)
            | NetworkError::InvalidOperation(_)
            | NetworkError::InvalidArgument(_)
            | NetworkError::UrlParseError(_) => Self::ConfigRejected { detail },
            NetworkError::SerializationError(_)
            | NetworkError::DeserializationError(_)
            | NetworkError::BroadcastError(_)
            | NetworkError::JsonError(_)
            | NetworkError::Other(_) => Self::InvariantViolation { detail },
            NetworkError::ConnectionError(_)
            | NetworkError::SignalingError(_)
            | NetworkError::WebRtcError(_)
            | NetworkError::NetworkUnreachableError(_)
            | NetworkError::NatTraversalError(_)
            | NetworkError::DataChannelError(_)
            | NetworkError::DataChannelClosed(_)
            | NetworkError::DataChannelNotOpen(_)
            | NetworkError::IceError(_)
            | NetworkError::DtlsError(_)
            | NetworkError::StunTurnError(_)
            | NetworkError::WebSocketError(_)
            | NetworkError::WebSocketClosed(_)
            | NetworkError::ConnectionNotFound(_)
            | NetworkError::ConnectionClosed(_)
            | NetworkError::PeerConnectionClosed(_)
            | NetworkError::ChannelClosed(_)
            | NetworkError::SendError(_)
            | NetworkError::ChannelNotFound(_)
            | NetworkError::IoError(_) => Self::PathUnreachable { stage: detail },
        }
    }

    fn into_diagnosis(self) -> EffectDiagnosis {
        match self {
            Self::PathUnreachable { stage } => EffectDiagnosis::PathUnreachable { stage },
            Self::Timeout { stage } => EffectDiagnosis::Timeout { stage },
            Self::ResourceExhausted { resource } => EffectDiagnosis::ResourceExhausted { resource },
            Self::AuthRejected { kind } => EffectDiagnosis::AuthRejected { kind },
            Self::ConfigRejected { detail } => EffectDiagnosis::ConfigRejected { detail },
            Self::InvariantViolation { detail } => EffectDiagnosis::InvariantViolation { detail },
        }
    }
}

/// The structured result of a bounded teardown effect (cleanup or confirmed
/// offline disconnect), per the RFC-0400 bounded-completion contract.
///
/// A teardown never reports a plain success/failure boolean: it reports whether
/// the logical teardown goal was reached, whether its overall deadline aborted
/// remaining steps, and the residual diagnostics accumulated from best-effort
/// physical steps. Remote-notification failures are recorded as residuals and
/// never block local completion.
#[derive(Debug, Clone)]
pub struct TeardownReport {
    /// Whether the logical teardown goal (local resources released) was reached.
    pub reached_goal: bool,
    /// Whether the overall teardown deadline aborted or detached remaining steps.
    pub deadline_reached: bool,
    /// One residual per best-effort step that left an error behind.
    pub residuals: Vec<String>,
}

impl TeardownReport {
    /// The teardown met its full contract with no residuals.
    pub fn succeeded() -> Self {
        Self {
            reached_goal: true,
            deadline_reached: false,
            residuals: Vec::new(),
        }
    }

    /// The teardown reached its goal but left the given residual diagnostics.
    pub fn completed_with_residuals(residuals: Vec<String>) -> Self {
        Self {
            reached_goal: true,
            deadline_reached: false,
            residuals,
        }
    }

    /// The overall deadline aborted remaining steps before the goal was reached.
    pub fn abandoned(residuals: Vec<String>) -> Self {
        Self {
            reached_goal: false,
            deadline_reached: true,
            residuals,
        }
    }
}

fn network_event_needs_lifecycle_barrier(event: &NetworkEvent) -> bool {
    match event {
        NetworkEvent::NetworkPathChanged { snapshot } => {
            snapshot.is_offline() || snapshot.should_restore()
        }
        NetworkEvent::AppLifecycleChanged { state } => match state {
            AppLifecycleState::Background => false,
            AppLifecycleState::Foreground {
                background_duration_ms,
            } => *background_duration_ms >= LONG_BACKGROUND_RECONNECT_THRESHOLD_MS,
        },
        NetworkEvent::CleanupConnections { .. } | NetworkEvent::ForceReconnect { .. } => true,
    }
}

/// Network event processing result
#[derive(Debug, Clone)]
pub struct NetworkEventResult {
    /// Event type
    pub event: NetworkEvent,

    /// Whether processing succeeded
    pub success: bool,

    /// Error message (if failed)
    pub error: Option<String>,

    /// Processing duration (milliseconds)
    pub duration_ms: u64,
}

impl NetworkEventResult {
    pub fn success(event: NetworkEvent, duration_ms: u64) -> Self {
        Self {
            event,
            success: true,
            error: None,
            duration_ms,
        }
    }

    pub fn failure(event: NetworkEvent, error: String, duration_ms: u64) -> Self {
        Self {
            event,
            success: false,
            error: Some(error),
            duration_ms,
        }
    }
}

/// Network event processor trait
///
/// Defines the processing logic for network events; can be custom-implemented by users
#[async_trait::async_trait]
pub trait NetworkEventProcessor: Send + Sync {
    /// Perform synchronous, non-blocking preparation as soon as an event is
    /// dequeued, before the reconciler optionally waits through offline grace.
    fn prepare_network_event(&self, _event: &NetworkEvent) {}

    /// Invalidate commit rights for every in-flight signaling connection
    /// attempt and keep automatic reconnect paused. The supervisor shell calls
    /// this synchronously before cancelling a superseded recovery effect.
    fn invalidate_signaling_connection_attempts(&self) {}

    /// Invalidate in-flight automatic reconnect and keep new attempts paused.
    ///
    /// Executed by the shell when policy translation emits
    /// `SignalingDirective::SuppressAutoReconnect`. The default is a no-op for
    /// custom processors that own no signaling socket.
    fn suppress_auto_reconnect(&self) {}

    /// Re-enable future automatic reconnects without starting one immediately.
    ///
    /// Executed by the shell when policy translation emits
    /// `SignalingDirective::ResumeAutoReconnect`. The default is a no-op.
    fn resume_auto_reconnect(&self) {}

    /// Enter a lifecycle barrier as soon as a queued event is observed by the
    /// reconciler. The default is no barrier for custom processors.
    fn begin_network_event_barrier(&self, _event: &NetworkEvent) -> Option<NetworkEventBarrier> {
        None
    }

    /// Process network available event
    ///
    /// # Returns
    /// - `Ok(())`: processing succeeded
    /// - `Err(String)`: processing failed, contains error message
    async fn process_network_available(&self) -> Result<(), String>;

    /// Process network lost event
    ///
    /// # Returns
    /// - `Ok(())`: processing succeeded
    /// - `Err(String)`: processing failed, contains error message
    async fn process_network_lost(&self) -> Result<(), String>;

    /// Process network type changed event
    ///
    /// # Returns
    /// - `Ok(())`: processing succeeded
    /// - `Err(String)`: processing failed, contains error message
    async fn process_network_type_changed(
        &self,
        is_wifi: bool,
        is_cellular: bool,
    ) -> Result<(), String>;

    /// Proactively clean up all connections
    ///
    /// This method proactively cleans up all network connections. Applicable scenarios:
    /// - User actively logging out
    /// - App about to exit
    /// - Need to reset network state
    ///
    /// # FFI Binding Note
    ///
    /// This method is specifically designed for FFI bindings, allowing upper-layer
    /// platform code (Swift/Kotlin) to proactively manage connection lifecycle
    /// through the unified `NetworkEventProcessor` interface.
    ///
    /// # Difference from Event Response
    ///
    /// - `process_network_lost()`: passively responds to network disconnection events
    /// - `cleanup_connections()`: proactively cleans up connections (independent of network events)
    ///
    /// # Returns
    /// - `Ok(())`: cleanup succeeded
    /// - `Err(String)`: cleanup failed, contains error message
    async fn cleanup_connections(&self) -> Result<(), String>;

    /// Probe existing connectivity without forcing cleanup.
    async fn probe_connectivity(&self) -> Result<(), String> {
        Ok(())
    }

    /// Proactively clean up and restore connections.
    async fn force_reconnect(&self) -> Result<(), String> {
        self.cleanup_connections().await?;
        self.process_network_available().await
    }

    /// Process the final action selected by the connection policy FSM.
    ///
    /// Custom processors can rely on the default mapping. The default runtime
    /// processor overrides this to coordinate signaling and WebRTC recovery.
    async fn process_network_recovery_action(
        &self,
        action: NetworkRecoveryAction,
    ) -> Result<(), String> {
        match action {
            NetworkRecoveryAction::Noop => Ok(()),
            NetworkRecoveryAction::Offline => self.process_network_lost().await,
            NetworkRecoveryAction::Probe => self.probe_connectivity().await,
            NetworkRecoveryAction::Restore => self.process_network_available().await,
            NetworkRecoveryAction::CleanupOnly => self.cleanup_connections().await,
            NetworkRecoveryAction::ForceReconnect => self.force_reconnect().await,
        }
    }

    /// Execute a policy-selected recovery action while preserving a typed
    /// diagnosis for the supervisor.
    ///
    /// The compatibility default wraps legacy `String` errors as
    /// `PathUnreachable`; the runtime processor overrides this method so native
    /// [`NetworkError`] variants reach the policy without string parsing.
    #[doc(hidden)]
    async fn process_network_recovery_effect(
        &self,
        action: NetworkRecoveryAction,
    ) -> Result<(), NetworkRecoveryError> {
        self.process_network_recovery_action(action)
            .await
            .map_err(|detail| NetworkRecoveryError::from_opaque("custom recovery effect", detail))
    }

    /// Run a bounded teardown effect (cleanup or confirmed offline disconnect).
    ///
    /// Implementations must synchronously invalidate the scoped generations,
    /// commit rights, and new-work admission before any physical step, bound the
    /// remaining physical steps by `budget`, and never let a remote-notification
    /// failure block local completion. The default delegates to the unbounded
    /// action so custom processors keep working; the runtime processor overrides
    /// it with the real bounded-completion contract.
    async fn run_bounded_teardown(
        &self,
        action: NetworkRecoveryAction,
        _budget: Duration,
    ) -> TeardownReport {
        match self.process_network_recovery_action(action).await {
            Ok(()) => TeardownReport::succeeded(),
            Err(e) => TeardownReport::completed_with_residuals(vec![e]),
        }
    }
}

/// Optional legacy time-throttling configuration for direct processor calls.
///
/// The persistent connection supervisor performs structural duplicate
/// suppression from snapshot sequence and route state, so the production
/// default is zero. A non-zero value is retained only as an explicit
/// compatibility/performance knob for callers that invoke processor methods
/// directly.
#[derive(Debug, Clone)]
pub struct DebounceConfig {
    /// Debounce time window (duplicate events within this window are ignored)
    pub window: Duration,
}

impl Default for DebounceConfig {
    fn default() -> Self {
        Self {
            // Correctness and duplicate suppression belong to the persistent
            // supervisor. Time-based throttling is legacy opt-in only.
            window: Duration::ZERO,
        }
    }
}

/// Debounce state tracking
#[derive(Debug)]
struct DebounceState {
    last_available: tokio::sync::Mutex<Option<Instant>>,
    last_lost: tokio::sync::Mutex<Option<Instant>>,
    last_type_changed: tokio::sync::Mutex<Option<Instant>>,
}

impl DebounceState {
    fn new() -> Self {
        Self {
            last_available: tokio::sync::Mutex::new(None),
            last_lost: tokio::sync::Mutex::new(None),
            last_type_changed: tokio::sync::Mutex::new(None),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DebounceEvent {
    Available,
    Lost,
    TypeChanged,
}

#[derive(Debug)]
struct SignalingRecoveryState {
    connect_lock: tokio::sync::Mutex<()>,
    last_successful_connect: tokio::sync::Mutex<Option<Instant>>,
}

impl SignalingRecoveryState {
    fn new() -> Self {
        Self {
            connect_lock: tokio::sync::Mutex::new(()),
            last_successful_connect: tokio::sync::Mutex::new(None),
        }
    }
}

/// Default network event processor implementation
pub struct DefaultNetworkEventProcessor {
    signaling_client: Arc<dyn SignalingClient>,
    webrtc_coordinator: Option<Arc<WebRtcCoordinator>>,
    peer_transport: Option<Arc<PeerTransport>>,
    debounce_config: DebounceConfig,
    debounce_state: Option<Arc<DebounceState>>,
    recovery_state: Arc<SignalingRecoveryState>,
}

impl DefaultNetworkEventProcessor {
    pub fn new(
        signaling_client: Arc<dyn SignalingClient>,
        webrtc_coordinator: Option<Arc<WebRtcCoordinator>>,
    ) -> Self {
        Self::new_with_debounce_and_peer_transport(
            signaling_client,
            webrtc_coordinator,
            DebounceConfig::default(),
            None,
        )
    }

    pub fn new_with_debounce(
        signaling_client: Arc<dyn SignalingClient>,
        webrtc_coordinator: Option<Arc<WebRtcCoordinator>>,
        debounce_config: DebounceConfig,
    ) -> Self {
        Self::new_with_debounce_and_peer_transport(
            signaling_client,
            webrtc_coordinator,
            debounce_config,
            None,
        )
    }

    pub(crate) fn new_with_peer_transport(
        signaling_client: Arc<dyn SignalingClient>,
        webrtc_coordinator: Option<Arc<WebRtcCoordinator>>,
        peer_transport: Option<Arc<PeerTransport>>,
    ) -> Self {
        Self::new_with_debounce_and_peer_transport(
            signaling_client,
            webrtc_coordinator,
            DebounceConfig::default(),
            peer_transport,
        )
    }

    pub(crate) fn new_with_debounce_and_peer_transport(
        signaling_client: Arc<dyn SignalingClient>,
        webrtc_coordinator: Option<Arc<WebRtcCoordinator>>,
        debounce_config: DebounceConfig,
        peer_transport: Option<Arc<PeerTransport>>,
    ) -> Self {
        let debounce_state =
            (!debounce_config.window.is_zero()).then(|| Arc::new(DebounceState::new()));
        Self {
            signaling_client,
            webrtc_coordinator,
            peer_transport,
            debounce_config,
            debounce_state,
            recovery_state: Arc::new(SignalingRecoveryState::new()),
        }
    }

    fn lifecycle_barrier(&self) -> Option<NetworkEventBarrier> {
        self.webrtc_coordinator
            .as_ref()
            .map(|coordinator| NetworkEventBarrier {
                _cleanup_guard: coordinator.cleanup_guard(),
            })
    }

    /// Check whether an event should be filtered by debounce
    ///
    /// # Returns
    /// - `true`: the event should be processed
    /// - `false`: the event is within the debounce window and should be ignored
    async fn should_process_event(&self, event: DebounceEvent) -> bool {
        if self.debounce_config.window.is_zero() {
            return true;
        }

        let debounce_state = self
            .debounce_state
            .as_ref()
            .expect("nonzero debounce window must have state");
        let now = Instant::now();

        match event {
            DebounceEvent::Available => {
                let mut last = debounce_state.last_available.lock().await;
                if let Some(last_time) = *last {
                    if now.duration_since(last_time) < self.debounce_config.window {
                        tracing::debug!(
                            "⏸️  Debouncing Network Available event (last event was {:?} ago)",
                            now.duration_since(last_time)
                        );
                        return false;
                    }
                }
                *last = Some(now);
                true
            }
            DebounceEvent::Lost => {
                let mut last = debounce_state.last_lost.lock().await;
                if let Some(last_time) = *last {
                    if now.duration_since(last_time) < self.debounce_config.window {
                        tracing::debug!(
                            "⏸️  Debouncing Network Lost event (last event was {:?} ago)",
                            now.duration_since(last_time)
                        );
                        return false;
                    }
                }
                *last = Some(now);
                true
            }
            DebounceEvent::TypeChanged => {
                let mut last = debounce_state.last_type_changed.lock().await;
                if let Some(last_time) = *last {
                    if now.duration_since(last_time) < self.debounce_config.window {
                        tracing::debug!(
                            "⏸️  Debouncing Network TypeChanged event (last event was {:?} ago)",
                            now.duration_since(last_time)
                        );
                        return false;
                    }
                }
                *last = Some(now);
                true
            }
        }
    }

    async fn ensure_signaling_healthy_once_typed(
        &self,
        reason: &str,
    ) -> Result<(), NetworkRecoveryError> {
        let _guard = self.recovery_state.connect_lock.lock().await;

        if !self.signaling_client.is_connected() {
            tracing::info!(
                reason = reason,
                "Network recovery event resetting signaling reconnect backoff before connect"
            );
            self.signaling_client
                .schedule_auto_reconnect_reset_backoff();
            tracing::info!(reason = reason, "🔄 Connecting signaling");
            self.signaling_client.connect_once().await.map_err(|e| {
                let error = NetworkRecoveryError::from_network_error("WebSocket connect failed", e);
                tracing::error!("❌ {}", error);
                error
            })?;

            *self.recovery_state.last_successful_connect.lock().await = Some(Instant::now());
            tracing::info!(reason = reason, "✅ Signaling connected");
            return Ok(());
        }

        tracing::debug!(
            reason = reason,
            timeout_ms = SIGNALING_PROBE_TIMEOUT.as_millis() as u64,
            "🔎 Probing existing signaling WebSocket"
        );

        match self
            .signaling_client
            .probe_alive(SIGNALING_PROBE_TIMEOUT)
            .await
        {
            Ok(()) => {
                tracing::debug!(
                    reason = reason,
                    "✅ Signaling probe succeeded; keeping existing WebSocket"
                );
                Ok(())
            }
            Err(e) => {
                tracing::warn!(
                    reason = reason,
                    "⚠️ Signaling probe failed; rebuilding WebSocket: {}",
                    e
                );

                if let Err(disconnect_err) = self.signaling_client.disconnect().await {
                    tracing::warn!(
                        reason = reason,
                        "⚠️ Failed to disconnect unhealthy signaling before rebuild: {}",
                        disconnect_err
                    );
                }

                tracing::info!(
                    reason = reason,
                    "Network recovery event resetting signaling reconnect backoff before rebuild"
                );
                self.signaling_client
                    .schedule_auto_reconnect_reset_backoff();
                tracing::info!(reason = reason, "🔄 Rebuilding signaling: connecting");
                self.signaling_client
                    .connect_once()
                    .await
                    .map_err(|connect_err| {
                        let error = NetworkRecoveryError::from_network_error(
                            "WebSocket rebuild failed",
                            connect_err,
                        );
                        tracing::error!("❌ {}", error);
                        error
                    })?;

                *self.recovery_state.last_successful_connect.lock().await = Some(Instant::now());
                tracing::info!(reason = reason, "✅ Signaling rebuilt");
                Ok(())
            }
        }
    }

    async fn restore_signaling_and_webrtc_typed(
        &self,
        reason: &str,
    ) -> Result<(), NetworkRecoveryError> {
        let _cleanup_guard = self.lifecycle_barrier();
        let recovery_targets = if let Some(coordinator) = self.webrtc_coordinator.clone() {
            coordinator.begin_network_recovery(reason).await
        } else {
            Vec::new()
        };

        self.ensure_signaling_healthy_once_typed(reason).await?;

        let coordinator = self.webrtc_coordinator.clone();

        if let Some(coordinator) = coordinator {
            if recovery_targets.is_empty() {
                tracing::info!("♻️ Resuming ICE restart for peers already in network recovery");
            } else {
                tracing::info!("♻️ Triggering ICE restart for recovering connections...");
            }
            coordinator.restart_network_recovery_connections().await;
        }

        Ok(())
    }

    fn schedule_auto_reconnect_after_recovery_failure(&self, reason: &str, err: &str) {
        tracing::warn!(
            reason = reason,
            error = %err,
            "Network recovery failed; ensuring signaling auto-reconnect remains scheduled"
        );
        self.signaling_client.schedule_auto_reconnect();
    }

    async fn restore_signaling_and_webrtc_from_network_event_typed(
        &self,
        reason: &str,
    ) -> Result<(), NetworkRecoveryError> {
        let result = self.restore_signaling_and_webrtc_typed(reason).await;
        if let Err(err) = &result {
            self.schedule_auto_reconnect_after_recovery_failure(reason, &err.to_string());
        }
        result
    }

    async fn restore_signaling_and_webrtc_from_network_event(
        &self,
        reason: &str,
    ) -> Result<(), String> {
        self.restore_signaling_and_webrtc_from_network_event_typed(reason)
            .await
            .map_err(|error| error.to_string())
    }

    async fn probe_connectivity_typed(&self) -> Result<(), NetworkRecoveryError> {
        self.signaling_client
            .probe_alive(SIGNALING_PROBE_TIMEOUT)
            .await
            .map_err(|error| {
                NetworkRecoveryError::from_network_error("Signaling probe failed", error)
            })
    }

    async fn probe_or_restore_typed(&self, reason: &str) -> Result<(), NetworkRecoveryError> {
        match self.probe_connectivity_typed().await {
            Ok(()) => Ok(()),
            Err(e) => {
                tracing::warn!(
                    reason = reason,
                    "Connectivity probe failed; restoring connections: {}",
                    e
                );
                if let Err(disconnect_err) = self.signaling_client.disconnect().await {
                    tracing::warn!(
                        reason = reason,
                        "Failed to disconnect unhealthy signaling before restore: {}",
                        disconnect_err
                    );
                }
                self.restore_signaling_and_webrtc_from_network_event_typed(reason)
                    .await
            }
        }
    }

    async fn process_network_available_typed(&self) -> Result<(), NetworkRecoveryError> {
        let should_process = self.should_process_event(DebounceEvent::Available).await;
        if !should_process && self.signaling_client.is_connected() {
            return Ok(());
        }

        tracing::info!("📱 Processing: Network available");

        self.restore_signaling_and_webrtc_from_network_event_typed("NetworkAvailable")
            .await
    }

    async fn force_reconnect_typed(&self) -> Result<(), NetworkRecoveryError> {
        self.cleanup_connections()
            .await
            .map_err(|error| NetworkRecoveryError::from_opaque("ForceReconnect cleanup", error))?;
        self.restore_signaling_and_webrtc_from_network_event_typed("ForceReconnect")
            .await
    }

    async fn process_offline(&self) -> Result<(), String> {
        let _cleanup_guard = self.lifecycle_barrier();
        tracing::info!("📱 Processing: Network offline");

        if let Some(ref coordinator) = self.webrtc_coordinator {
            coordinator.begin_network_recovery("NetworkLost").await;
            tracing::info!("🧹 Clearing pending ICE restart attempts...");
            coordinator.clear_pending_restarts().await;
        }

        tracing::info!("🔌 Disconnecting WebSocket...");
        let _ = self.signaling_client.disconnect().await;

        Ok(())
    }

    /// The confirmed-offline disconnect steps, accumulating step residuals in
    /// order instead of collapsing them to a single boolean.
    async fn process_offline_collect(&self) -> Vec<String> {
        let _cleanup_guard = self.lifecycle_barrier();
        let mut residuals = Vec::new();
        tracing::info!("📱 Processing: Network offline (bounded)");

        if let Some(ref coordinator) = self.webrtc_coordinator {
            coordinator.begin_network_recovery("NetworkLost").await;
            coordinator.clear_pending_restarts().await;
        }

        if let Err(e) = self.signaling_client.disconnect().await {
            residuals.push(format!("offline signaling disconnect failed: {e}"));
        }

        residuals
    }

    /// The full cleanup step sequence, accumulating one residual per best-effort
    /// step that leaves an error behind. The logical teardown goal — releasing
    /// local signaling, coordinator, and transport resources — is always
    /// attempted to completion; residuals are diagnostics, not early returns.
    async fn cleanup_connections_collect(&self) -> Vec<String> {
        let _cleanup_guard = self.lifecycle_barrier();
        let mut residuals: Vec<String> = Vec::new();
        let mut initial_coordinator_close_failed = false;

        tracing::info!("🧹 Cleaning up all connections (bounded)...");

        // Step 1: Stop old signaling ingress before reopening any peer
        // lifecycle. Disconnect resets the inbound queue, so delayed Offer,
        // RoleAssignment, and ICE messages cannot repopulate state after drain.
        tracing::info!("🔌 Disconnecting WebSocket before peer cleanup...");
        match self.signaling_client.disconnect().await {
            Ok(_) => tracing::info!("✅ WebSocket disconnected successfully"),
            Err(e) => {
                let err_msg = format!("Failed to disconnect WebSocket before cleanup: {e}");
                tracing::warn!("⚠️  {}", err_msg);
                residuals.push(err_msg);
            }
        }

        // Step 2: Clear pending ICE restart attempts.
        if let Some(ref coordinator) = self.webrtc_coordinator {
            tracing::info!("♻️  Clearing pending ICE restart attempts...");
            coordinator.clear_pending_restarts().await;
        }

        // Step 3: Remove coordinator-owned peer sessions first and force-close
        // them without draining DataChannels.
        if let Some(ref coordinator) = self.webrtc_coordinator {
            tracing::info!("🔻 Force-closing all WebRTC peer connections...");
            if let Err(e) = coordinator.close_all_peers_immediately().await {
                let err_msg = format!("Failed to close all peers: {}", e);
                tracing::warn!("⚠️  {}", err_msg);
                initial_coordinator_close_failed = true;
                residuals.push(err_msg);
            } else {
                tracing::info!("✅ All WebRTC peer connections closed");
            }
        }

        // Step 4: Cancel PeerTransport singleflight and close any remaining
        // established transport handles after the coordinator sessions are gone.
        if let Some(ref peer_transport) = self.peer_transport {
            tracing::info!("🔻 Closing all PeerTransport connections...");
            if let Err(e) = peer_transport.close_all().await {
                let err_msg = format!("Failed to close peer transports: {}", e);
                tracing::warn!("⚠️  {}", err_msg);
                residuals.push(err_msg);
            } else {
                tracing::info!("✅ All PeerTransport connections closed");
            }
        }

        // Step 5: A cancelled PeerTransport creator may have crossed the first
        // drain before observing its token. The signaling socket is already
        // down, so this final authoritative sweep closes anything it handed to
        // the coordinator without opening another ingress window.
        if let Some(ref coordinator) = self.webrtc_coordinator {
            tracing::info!("🔻 Finalizing WebRTC coordinator cleanup...");
            if let Err(e) = coordinator.close_all_peers_immediately().await {
                let err_msg = format!("Failed to finalize peer cleanup: {e}");
                tracing::warn!("⚠️  {}", err_msg);
                residuals.push(err_msg);
            } else if initial_coordinator_close_failed {
                tracing::info!("✅ Final WebRTC cleanup recovered the initial close failure");
            }
        }

        residuals
    }

    /// Synchronously close signaling commit rights and new-work admission, then
    /// run the cleanup steps under the obligation's remaining `budget`. On
    /// budget expiry the remaining steps are detached and the teardown is
    /// `Abandoned`; otherwise it completes (with residuals if any step failed).
    async fn bounded_cleanup(&self, budget: Duration) -> TeardownReport {
        // Step 0: before any physical step, invalidate the scoped signaling
        // generation and suppress new attempts so no later commit can race the
        // teardown across its commit boundary.
        self.signaling_client.invalidate_generation();

        match crate::timer::timeout(
            crate::timer::ids::RECOVERY_CLEANUP_TEARDOWN,
            budget,
            self.cleanup_connections_collect(),
        )
        .await
        {
            Ok(residuals) => TeardownReport {
                reached_goal: true,
                deadline_reached: false,
                residuals,
            },
            Err(_) => {
                tracing::warn!(
                    budget_ms = budget.as_millis() as u64,
                    "network_event.cleanup.deadline_abandoned"
                );
                TeardownReport::abandoned(vec![format!(
                    "cleanup teardown abandoned remaining steps after {}ms budget",
                    budget.as_millis()
                )])
            }
        }
    }

    /// The confirmed-offline disconnect under the obligation's remaining budget.
    async fn bounded_offline(&self, budget: Duration) -> TeardownReport {
        self.signaling_client.invalidate_generation();

        match crate::timer::timeout(
            crate::timer::ids::RECOVERY_OFFLINE_TEARDOWN,
            budget,
            self.process_offline_collect(),
        )
        .await
        {
            Ok(residuals) => TeardownReport {
                reached_goal: true,
                deadline_reached: false,
                residuals,
            },
            Err(_) => {
                tracing::warn!(
                    budget_ms = budget.as_millis() as u64,
                    "network_event.offline.deadline_abandoned"
                );
                TeardownReport::abandoned(vec![format!(
                    "offline disconnect abandoned remaining steps after {}ms budget",
                    budget.as_millis()
                )])
            }
        }
    }
}

#[async_trait::async_trait]
impl NetworkEventProcessor for DefaultNetworkEventProcessor {
    fn invalidate_signaling_connection_attempts(&self) {
        self.signaling_client.invalidate_generation();
    }

    fn suppress_auto_reconnect(&self) {
        // Executed when policy translation emits `SuppressAutoReconnect`
        // (entering the background, or a long-background foreground rebuild).
        self.signaling_client.suppress_auto_reconnect();
    }

    fn resume_auto_reconnect(&self) {
        // Executed when policy translation emits `ResumeAutoReconnect` (a short
        // background stay that only probes the still-healthy socket).
        self.signaling_client.resume_auto_reconnect();
    }

    fn begin_network_event_barrier(&self, event: &NetworkEvent) -> Option<NetworkEventBarrier> {
        if network_event_needs_lifecycle_barrier(event) {
            self.lifecycle_barrier()
        } else {
            None
        }
    }

    /// Process network available event
    async fn process_network_available(&self) -> Result<(), String> {
        self.process_network_available_typed()
            .await
            .map_err(|error| error.to_string())
    }

    /// Process network lost event
    async fn process_network_lost(&self) -> Result<(), String> {
        // Debounce check
        if !self.should_process_event(DebounceEvent::Lost).await {
            return Ok(());
        }

        self.process_offline().await
    }

    /// Process network type changed event
    async fn process_network_type_changed(
        &self,
        is_wifi: bool,
        is_cellular: bool,
    ) -> Result<(), String> {
        // Debounce check
        let should_process = self.should_process_event(DebounceEvent::TypeChanged).await;
        if !should_process && self.signaling_client.is_connected() {
            return Ok(());
        }

        tracing::info!(
            "📱 Processing: Network type changed (WiFi={}, Cellular={})",
            is_wifi,
            is_cellular
        );

        self.restore_signaling_and_webrtc_from_network_event("NetworkTypeChanged")
            .await
    }

    /// Proactively clean up all connections
    ///
    /// Differs from `process_network_lost()`:
    /// - No debounce check (proactive calls always execute)
    /// - Intended for app lifecycle management, not network event response
    async fn cleanup_connections(&self) -> Result<(), String> {
        let residuals = self.cleanup_connections_collect().await;
        if let Some(err) = residuals.into_iter().next() {
            tracing::warn!(
                error = %err,
                "Connection cleanup released remaining resources but did not fully quiesce"
            );
            Err(err)
        } else {
            tracing::info!("✅ Connection cleanup completed");
            Ok(())
        }
    }

    async fn run_bounded_teardown(
        &self,
        action: NetworkRecoveryAction,
        budget: Duration,
    ) -> TeardownReport {
        match action {
            NetworkRecoveryAction::CleanupOnly => self.bounded_cleanup(budget).await,
            NetworkRecoveryAction::Offline => self.bounded_offline(budget).await,
            // Not a teardown action; keep the coarse mapping for safety.
            other => match self.process_network_recovery_action(other).await {
                Ok(()) => TeardownReport::succeeded(),
                Err(e) => TeardownReport::completed_with_residuals(vec![e]),
            },
        }
    }

    async fn probe_connectivity(&self) -> Result<(), String> {
        self.probe_connectivity_typed()
            .await
            .map_err(|error| error.to_string())
    }

    async fn force_reconnect(&self) -> Result<(), String> {
        self.force_reconnect_typed()
            .await
            .map_err(|error| error.to_string())
    }

    async fn process_network_recovery_action(
        &self,
        action: NetworkRecoveryAction,
    ) -> Result<(), String> {
        self.process_network_recovery_effect(action)
            .await
            .map_err(|error| error.to_string())
    }

    async fn process_network_recovery_effect(
        &self,
        action: NetworkRecoveryAction,
    ) -> Result<(), NetworkRecoveryError> {
        if action == NetworkRecoveryAction::Noop {
            return Ok(());
        }

        // The single-flight execution machine (`view.execution`) is owned by the
        // supervisor, which is its only writer. The effect task performs I/O and
        // reports a typed outcome; it holds no policy state of its own. Driving a
        // second execution tracker here would poison it whenever the supervisor
        // cancels the effect after `begin` but before `complete` — a cancelled
        // future's `complete` never runs — permanently wedging every later
        // recovery action (RFC-0400 "MUST NOT let a cancellation poison
        // single-flight ownership", invariant 8).
        let action_id = NEXT_NETWORK_RECOVERY_ACTION_ID.fetch_add(1, Ordering::Relaxed);
        tracing::info!(
            action_id,
            action = ?action,
            "network_event.execution.action.start"
        );
        let result = match action {
            NetworkRecoveryAction::Noop => Ok(()),
            NetworkRecoveryAction::Offline => self
                .process_offline()
                .await
                .map_err(|error| NetworkRecoveryError::from_opaque("Offline", error)),
            NetworkRecoveryAction::Probe => self.probe_or_restore_typed("Probe").await,
            // Supervisor-selected work has already passed structural duplicate
            // suppression and single-flight admission. Running it through the
            // legacy direct-call debounce can silently acknowledge required
            // Restore work without executing it.
            NetworkRecoveryAction::Restore => {
                self.restore_signaling_and_webrtc_from_network_event_typed("SupervisorRestore")
                    .await
            }
            NetworkRecoveryAction::CleanupOnly => self
                .cleanup_connections()
                .await
                .map_err(|error| NetworkRecoveryError::from_opaque("CleanupOnly", error)),
            NetworkRecoveryAction::ForceReconnect => self.force_reconnect_typed().await,
        };
        tracing::info!(
            action_id,
            action = ?action,
            success = result.is_ok(),
            "network_event.execution.action.completed"
        );
        result
    }
}

/// Select one recovery action from a batch of events with the legacy
/// synchronous selector.
///
/// Deprecated: this is a second policy source that does not go through the RFC
/// `translate` engine and has no timer owner. Prefer the responsive reconciler
/// ([`run_network_event_reconciler`]), which routes every decision through the
/// pure translation function. Retained only for the migration window.
#[deprecated(note = "second policy source without a timer owner; use \
            run_network_event_reconciler, which drives the RFC translate engine")]
#[allow(deprecated)]
pub fn select_network_recovery_action(events: &[NetworkEvent]) -> NetworkRecoveryAction {
    super::recovery_supervisor::legacy_select_action(events)
}

/// Process a batch of events by selecting one action and running it once.
///
/// Deprecated: prefer the responsive reconciler
/// ([`run_network_event_reconciler`]); this legacy batch path bypasses the RFC
/// `translate` engine and its single-flight/timer ownership. Retained only for
/// the migration window.
#[deprecated(note = "legacy batch path bypassing the RFC translate engine; use \
            run_network_event_reconciler")]
#[allow(deprecated)]
pub async fn process_network_event_batch(
    events: Vec<NetworkEvent>,
    processor: Arc<dyn NetworkEventProcessor>,
) -> Vec<NetworkEventResult> {
    if events.is_empty() {
        return Vec::new();
    }

    let action = super::recovery_supervisor::legacy_select_action(&events);
    process_network_event_batch_with_action(events, action, processor).await
}

async fn process_network_event_batch_with_action(
    events: Vec<NetworkEvent>,
    action: NetworkRecoveryAction,
    processor: Arc<dyn NetworkEventProcessor>,
) -> Vec<NetworkEventResult> {
    let start = Instant::now();

    tracing::info!(
        event_count = events.len(),
        action = ?action,
        "network_event.action.start"
    );

    let result = processor.process_network_recovery_action(action).await;

    let duration_ms = start.elapsed().as_millis() as u64;
    match &result {
        Ok(()) => tracing::info!(
            event_count = events.len(),
            action = ?action,
            duration_ms,
            "network_event.action.completed"
        ),
        Err(e) => tracing::warn!(
            event_count = events.len(),
            action = ?action,
            duration_ms,
            error = %e,
            "network_event.action.completed"
        ),
    }

    events
        .into_iter()
        .map(|event| match &result {
            Ok(()) => NetworkEventResult::success(event, duration_ms),
            Err(e) => NetworkEventResult::failure(event, e.clone(), duration_ms),
        })
        .collect()
}

pub struct NetworkEventRequest {
    pub event: NetworkEvent,
    pub result_tx: oneshot::Sender<NetworkEventResult>,
    /// The path-monitor incarnation epoch stamped by the originating handle;
    /// `(source_epoch, sequence)` lexicographic order decides snapshot acceptance.
    pub source_epoch: u64,
    /// Monotonic supervisor-ingress time captured before the request is sent.
    ///
    /// This stays out of [`NetworkSnapshot`] so the platform FFI schema does
    /// not need to expose a runtime-specific clock representation.
    pub observed_at: tokio::time::Instant,
}

/// A minimal observation of supervisor progress for tests and callers.
///
/// It exposes the causal `policy_revision`, the identity of the most recently
/// started action, and the most recent effect outcome — the final result a
/// caller would otherwise infer by racing an effect.
#[derive(Debug, Clone, Default)]
pub struct SupervisorStatus {
    pub policy_revision: u64,
    pub last_action_id: Option<u64>,
    pub last_outcome: Option<ObservedOutcome>,
}

/// The observable class of the most recent effect completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedOutcome {
    Succeeded,
    CompletedWithResiduals,
    Abandoned,
    Failed,
    Cancelled,
    Aborted,
}

fn observed_outcome(outcome: &EffectOutcome) -> ObservedOutcome {
    match outcome {
        EffectOutcome::Succeeded => ObservedOutcome::Succeeded,
        EffectOutcome::CompletedWithResiduals { .. } => ObservedOutcome::CompletedWithResiduals,
        EffectOutcome::Abandoned { .. } => ObservedOutcome::Abandoned,
        EffectOutcome::Failed { .. } => ObservedOutcome::Failed,
        EffectOutcome::Cancelled => ObservedOutcome::Cancelled,
        EffectOutcome::Aborted { .. } => ObservedOutcome::Aborted,
    }
}

/// The shell's handle on the single in-flight effect: its identity and the
/// cancellation token the supervisor toggles on a preemption request.
struct RunningEffect {
    action_id: u64,
    token: CancellationToken,
}

/// A narrow, cloneable handle for resource owners (signaling, session) to feed
/// normalized facts into the running supervisor without going through the
/// public [`NetworkEvent`] surface.
///
/// Facts share the supervisor's internal input channel, so they are processed
/// in the same one-input-at-a-time policy order as timer expiries and effect
/// completions. Delivery is non-blocking and lossless while the supervisor is
/// alive: the internal queue is unbounded because resource owners may emit
/// while holding a transport lock and therefore cannot await capacity.
#[derive(Clone)]
pub struct SupervisorFactSink {
    tx: mpsc::UnboundedSender<tp::Input>,
}

/// The origin of a normalized signaling-committed fact, as seen at the public
/// resource-owner boundary. Mirrors the supervisor's internal origin type but
/// keeps the recovery-policy internals out of the wire layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalingFactOrigin {
    /// Produced by the running lifecycle effect (carries its `action_id`).
    CurrentEffect { action_id: u64 },
    /// External news from the signaling resource owner itself.
    External,
}

/// Why a live signaling generation was lost. Diagnostic only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalingFactLostCause {
    /// A lifecycle disconnect or explicit invalidation invalidated the socket.
    Disconnected,
    /// A newer generation superseded this one.
    Superseded,
    /// The remote or transport reset the connection.
    RemoteReset,
}

impl SupervisorFactSink {
    pub(crate) fn new(tx: mpsc::UnboundedSender<tp::Input>) -> Self {
        Self { tx }
    }

    fn emit(&self, input: tp::Input) {
        if self.tx.send(input).is_err() {
            tracing::debug!("network_event.fact_sink.closed");
        }
    }

    /// Record that a signaling generation was authoritatively committed (its
    /// generation validated and its socket published under the commit facade).
    pub fn signaling_generation_committed(&self, generation: u64, origin: SignalingFactOrigin) {
        let origin = match origin {
            SignalingFactOrigin::CurrentEffect { action_id } => {
                tp::SignalingOrigin::CurrentEffect { action_id }
            }
            SignalingFactOrigin::External => tp::SignalingOrigin::External,
        };
        self.emit(tp::Input::SignalingGenerationCommitted { generation, origin });
    }

    /// Record that a live signaling generation was lost at a disconnect or
    /// invalidation point.
    pub fn signaling_generation_lost(&self, generation: u64, cause: SignalingFactLostCause) {
        let cause = match cause {
            SignalingFactLostCause::Disconnected => tp::SignalingLostCause::Disconnected,
            SignalingFactLostCause::Superseded => tp::SignalingLostCause::Superseded,
            SignalingFactLostCause::RemoteReset => tp::SignalingLostCause::RemoteReset,
        };
        self.emit(tp::Input::SignalingGenerationLost { generation, cause });
    }

    /// Record that a new session generation was authoritatively committed.
    pub fn session_activated(&self, session_generation: u64) {
        self.emit(tp::Input::SessionActivated { session_generation });
    }
}

/// An opaque handle on the supervisor's internal input channel, so the node can
/// hold and hand it to [`run_network_event_reconciler_with_channel`] without
/// naming the crate-private input type.
pub struct SupervisorInternalChannel {
    tx: mpsc::UnboundedSender<tp::Input>,
    rx: mpsc::UnboundedReceiver<tp::Input>,
    profile: tp::LifecycleProfile,
    clock_origin: tokio::time::Instant,
}

#[derive(Debug, Clone, Copy)]
struct ReconcilerConfig {
    profile: tp::LifecycleProfile,
    clock_origin: tokio::time::Instant,
}

/// Create the supervisor's internal input channel plus a [`SupervisorFactSink`]
/// over its sender. The node hands the sink to resource owners and passes the
/// channel into [`run_network_event_reconciler_with_channel`]. Rust and
/// headless callers default to an ungated lifecycle profile.
#[cfg(any(test, feature = "test-utils"))]
pub(crate) fn supervisor_internal_channel() -> (SupervisorFactSink, SupervisorInternalChannel) {
    supervisor_internal_channel_with_profile(tp::LifecycleProfile::Ungated)
}

/// Test-support constructor matching the lifecycle-gated mobile binding path.
#[cfg(feature = "test-utils")]
pub(crate) fn supervisor_internal_channel_gated() -> (SupervisorFactSink, SupervisorInternalChannel)
{
    supervisor_internal_channel_with_profile(tp::LifecycleProfile::Gated)
}

/// Create the supervisor channel with an explicit constructor-time lifecycle
/// profile. Mobile bindings use `Gated`; Rust and headless deployments use an
/// `Ungated` profile.
pub(crate) fn supervisor_internal_channel_with_profile(
    profile: tp::LifecycleProfile,
) -> (SupervisorFactSink, SupervisorInternalChannel) {
    let (tx, rx) = mpsc::unbounded_channel::<tp::Input>();
    let clock_origin = tokio::time::Instant::now();
    (
        SupervisorFactSink::new(tx.clone()),
        SupervisorInternalChannel {
            tx,
            rx,
            profile,
            clock_origin,
        },
    )
}

pub async fn run_network_event_reconciler(
    event_rx: mpsc::Receiver<NetworkEventRequest>,
    processor: Arc<dyn NetworkEventProcessor>,
    shutdown_token: CancellationToken,
) {
    let clock_origin = tokio::time::Instant::now();
    let (internal_tx, internal_rx) = mpsc::unbounded_channel::<tp::Input>();
    let (status_tx, _status_rx) = watch::channel(SupervisorStatus::default());
    reconcile_loop(
        event_rx,
        internal_tx,
        internal_rx,
        processor,
        shutdown_token,
        status_tx,
        ReconcilerConfig {
            profile: tp::LifecycleProfile::Ungated,
            clock_origin,
        },
    )
    .await;
}

/// The responsive reconciler with an observable status stream.
///
/// The supervisor stays responsive while an effect runs: effects execute in
/// separate tasks and report versioned completions back over the ordered
/// internal channel, timers are armed as absolute deadlines that deliver their
/// own expiry inputs, and event acceptance is decoupled from effect completion.
pub async fn run_network_event_reconciler_with_status(
    event_rx: mpsc::Receiver<NetworkEventRequest>,
    processor: Arc<dyn NetworkEventProcessor>,
    shutdown_token: CancellationToken,
    status_tx: watch::Sender<SupervisorStatus>,
) {
    let clock_origin = tokio::time::Instant::now();
    let (internal_tx, internal_rx) = mpsc::unbounded_channel::<tp::Input>();
    reconcile_loop(
        event_rx,
        internal_tx,
        internal_rx,
        processor,
        shutdown_token,
        status_tx,
        ReconcilerConfig {
            profile: tp::LifecycleProfile::Ungated,
            clock_origin,
        },
    )
    .await;
}

/// The reconciler entry point used by the node. The caller supplies the internal
/// input channel (see [`supervisor_internal_channel`]) so a [`SupervisorFactSink`]
/// over the same sender can feed normalized signaling and session facts into the
/// supervisor's policy-ordered input queue.
pub(crate) async fn run_network_event_reconciler_with_channel(
    event_rx: mpsc::Receiver<NetworkEventRequest>,
    channel: SupervisorInternalChannel,
    processor: Arc<dyn NetworkEventProcessor>,
    shutdown_token: CancellationToken,
) {
    let (status_tx, _status_rx) = watch::channel(SupervisorStatus::default());
    let SupervisorInternalChannel {
        tx,
        rx,
        profile,
        clock_origin,
    } = channel;
    reconcile_loop(
        event_rx,
        tx,
        rx,
        processor,
        shutdown_token,
        status_tx,
        ReconcilerConfig {
            profile,
            clock_origin,
        },
    )
    .await;
}

async fn reconcile_loop(
    mut event_rx: mpsc::Receiver<NetworkEventRequest>,
    internal_tx: mpsc::UnboundedSender<tp::Input>,
    mut internal_rx: mpsc::UnboundedReceiver<tp::Input>,
    processor: Arc<dyn NetworkEventProcessor>,
    shutdown_token: CancellationToken,
    status_tx: watch::Sender<SupervisorStatus>,
    config: ReconcilerConfig,
) {
    let ReconcilerConfig {
        profile,
        clock_origin,
    } = config;
    tracing::info!(?profile, "🔄 Network event reconciler started");

    let start = clock_origin;
    let mut supervisor = RecoverySupervisor::new(profile);

    let mut timers: HashMap<tp::TimerId, tokio::task::AbortHandle> = HashMap::new();
    let mut effect: Option<RunningEffect> = None;
    let mut last_action_id: Option<u64> = None;
    let mut last_outcome: Option<ObservedOutcome> = None;

    // Arm the gated bootstrap deadline (no-op under `Ungated`).
    if let Some(op) = supervisor.bootstrap_arm(start.elapsed()) {
        arm_timer(&mut timers, start, &internal_tx, op);
    }

    loop {
        tokio::select! {
            biased;
            _ = shutdown_token.cancelled() => {
                tracing::info!("🛑 Network event reconciler shutting down");
                break;
            }
            Some(input) = internal_rx.recv() => {
                let now = start.elapsed();
                let terminate = handle_supervisor_input(
                    &mut supervisor, input, now, start, &internal_tx, &mut timers,
                    &mut effect, &processor, &status_tx, &mut last_action_id, &mut last_outcome,
                );
                if terminate {
                    tracing::info!("🛑 Supervisor ended at shutdown deadline");
                    break;
                }
            }
            Some(request) = event_rx.recv() => {
                let event = request.event.clone();
                let epoch = request.source_epoch;
                let wait_started = Instant::now();
                tracing::debug!(event = ?event, "network_event.reconciler.received");
                processor.prepare_network_event(&event);
                let now = start.elapsed();
                let observed_at = request
                    .observed_at
                    .checked_duration_since(start)
                    .unwrap_or(Duration::ZERO);
                let input = event_to_input(&event, epoch, observed_at);
                let terminate = handle_supervisor_input(
                    &mut supervisor, input, now, start, &internal_tx, &mut timers,
                    &mut effect, &processor, &status_tx, &mut last_action_id, &mut last_outcome,
                );
                // Acceptance: reply as soon as the fact is accepted and one
                // synchronous reconcile round completes. The result does not
                // wait for effect completion (RFC-0400 invariant 11).
                let duration_ms = wait_started.elapsed().as_millis() as u64;
                if request.result_tx.send(NetworkEventResult::success(event, duration_ms)).is_err() {
                    tracing::debug!("Network event caller dropped before receiving acceptance");
                }
                if terminate {
                    tracing::info!("🛑 Supervisor ended at shutdown deadline");
                    break;
                }
            }
            else => break,
        }
    }

    for (_, handle) in timers.drain() {
        handle.abort();
    }
    if let Some(running) = effect.take() {
        running.token.cancel();
    }
}

fn arm_timer(
    timers: &mut HashMap<tp::TimerId, tokio::task::AbortHandle>,
    start: tokio::time::Instant,
    internal_tx: &mpsc::UnboundedSender<tp::Input>,
    op: TimerOp,
) {
    if let TimerOp::Arm { key, at, fire } = op {
        if let Some(existing) = timers.remove(&key) {
            existing.abort();
        }
        let itx = internal_tx.clone();
        let target = start + at;
        let handle = tokio::spawn(async move {
            crate::timer::sleep_until(crate::timer::ids::RECOVERY_POLICY_DEADLINE, target).await;
            let _ = itx.send(fire);
        });
        timers.insert(key, handle.abort_handle());
    }
}

/// Apply one supervisor input and drive its side effects. Returns `true` when
/// the supervisor has ended unconditionally (shutdown overall deadline) and the
/// reconcile loop must stop.
#[allow(clippy::too_many_arguments)]
#[must_use]
fn handle_supervisor_input(
    supervisor: &mut RecoverySupervisor,
    input: tp::Input,
    now: Duration,
    start: tokio::time::Instant,
    internal_tx: &mpsc::UnboundedSender<tp::Input>,
    timers: &mut HashMap<tp::TimerId, tokio::task::AbortHandle>,
    effect: &mut Option<RunningEffect>,
    processor: &Arc<dyn NetworkEventProcessor>,
    status_tx: &watch::Sender<SupervisorStatus>,
    last_action_id: &mut Option<u64>,
    last_outcome: &mut Option<ObservedOutcome>,
) -> bool {
    if let tp::Input::EffectCompleted { outcome, .. } = &input {
        *last_outcome = Some(observed_outcome(outcome));
    }

    let outcome = supervisor.accept(input, now);
    let terminate = outcome.terminate;

    tracing::debug!(
        advanced = outcome.advanced,
        revision = supervisor.view().policy_revision,
        send_policy = ?supervisor.send_policy(),
        pending_action = ?supervisor.composite_action(),
        "network_event.supervisor.reconciled"
    );

    for op in outcome.timer_ops {
        match op {
            TimerOp::Cancel { key } => {
                if let Some(handle) = timers.remove(&key) {
                    handle.abort();
                }
            }
            arm @ TimerOp::Arm { .. } => arm_timer(timers, start, internal_tx, arm),
        }
    }

    // Lower policy-derived signaling directives (auto-reconnect control) to the
    // processor before requesting effect cancellation. A preemption fence must
    // revoke the old effect's commit rights before cooperative cancellation.
    for signal in &outcome.signals {
        match signal {
            tp::SignalingDirective::InvalidateConnectionAttempts => {
                tracing::debug!("network_event.signaling.invalidate_connection_attempts");
                processor.invalidate_signaling_connection_attempts();
            }
            tp::SignalingDirective::SuppressAutoReconnect => {
                tracing::debug!("network_event.signaling.suppress_auto_reconnect");
                processor.suppress_auto_reconnect();
            }
            tp::SignalingDirective::ResumeAutoReconnect => {
                tracing::debug!("network_event.signaling.resume_auto_reconnect");
                processor.resume_auto_reconnect();
            }
        }
    }

    if outcome.cancel_effect
        && let Some(running) = effect.as_ref()
    {
        tracing::debug!(
            action_id = running.action_id,
            "network_event.effect.cancel_requested"
        );
        running.token.cancel();
    }

    for record in &outcome.status {
        match record {
            tp::StatusRecord::RecoveryRejected { mode, reason } => {
                tracing::info!(?mode, ?reason, "network_event.supervisor.recovery_rejected")
            }
            tp::StatusRecord::BootstrapDeadlineElapsed => {
                tracing::error!("network_event.supervisor.bootstrap_deadline_elapsed")
            }
            tp::StatusRecord::ShutdownAbandon => {
                tracing::warn!("network_event.supervisor.shutdown_abandon")
            }
        }
    }

    // A terminating supervisor starts no further work; its remaining teardown
    // obligations were detached with `Abandoned` residuals in translation.
    if !terminate {
        if let Some(started) = supervisor.maybe_start_effect(now) {
            *last_action_id = Some(started.action_id);
            let token = CancellationToken::new();
            tracing::info!(
                action_id = started.action_id,
                kind = ?started.kind,
                policy_revision = started.captured_revision,
                "network_event.effect.started"
            );
            spawn_effect(
                started,
                token.clone(),
                processor.clone(),
                internal_tx.clone(),
            );
            *effect = Some(RunningEffect {
                action_id: started.action_id,
                token,
            });
        } else if supervisor.view().execution == tp::ExecutionState::Idle {
            *effect = None;
        }
    }

    let _ = status_tx.send(SupervisorStatus {
        policy_revision: supervisor.view().policy_revision,
        last_action_id: *last_action_id,
        last_outcome: *last_outcome,
    });

    terminate
}

fn spawn_effect(
    started: StartedEffect,
    token: CancellationToken,
    processor: Arc<dyn NetworkEventProcessor>,
    internal_tx: mpsc::UnboundedSender<tp::Input>,
) {
    let action_id = started.action_id;
    let kind = started.kind;
    let policy_revision = started.captured_revision;
    let net_action = network_action_of(started.action);
    let teardown_budget = started.teardown_budget;

    // Outer join monitor: every termination path — normal return, cancellation,
    // or panic/abort — reports exactly one terminal completion.
    tokio::spawn(async move {
        let inner = tokio::spawn(async move {
            match teardown_budget {
                // Teardown kinds run under the bounded-completion contract and
                // report a structured residual outcome, not a plain success.
                Some(budget) => tokio::select! {
                    biased;
                    _ = token.cancelled() => EffectOutcome::Cancelled,
                    report = processor.run_bounded_teardown(net_action, budget) =>
                        teardown_outcome(report),
                },
                None => tokio::select! {
                    biased;
                    _ = token.cancelled() => EffectOutcome::Cancelled,
                    res = processor.process_network_recovery_effect(net_action) => match res {
                        Ok(()) => EffectOutcome::Succeeded,
                        Err(error) => EffectOutcome::Failed {
                            diagnosis: error.into_diagnosis(),
                        },
                    }
                },
            }
        });
        let outcome = match inner.await {
            Ok(outcome) => outcome,
            Err(join_err) => EffectOutcome::Aborted {
                cause: if join_err.is_cancelled() {
                    AbortCause::SupervisorCancellation
                } else {
                    AbortCause::PanicOrContractViolation
                },
            },
        };
        let _ = internal_tx.send(tp::Input::EffectCompleted {
            action_id,
            kind,
            policy_revision,
            outcome,
        });
    });
}

/// Map a bounded [`TeardownReport`] onto the effect completion vocabulary.
///
/// Reaching the logical teardown goal is success-class: no residuals →
/// `Succeeded`, residuals → `CompletedWithResiduals`. A deadline that aborted
/// remaining steps before the goal is `Abandoned` (still success-class, so the
/// obligation is extinguished). Otherwise the teardown reports `Failed` so the
/// policy retries inside the obligation deadline. Residual strings are recorded
/// as availability-family `PathUnreachable` diagnostics, which is inside the
/// producible set for teardown kinds.
fn teardown_outcome(report: TeardownReport) -> EffectOutcome {
    let residuals: Vec<EffectDiagnosis> = report
        .residuals
        .iter()
        .map(|stage| EffectDiagnosis::PathUnreachable {
            stage: stage.clone(),
        })
        .collect();

    if report.reached_goal {
        if residuals.is_empty() {
            EffectOutcome::Succeeded
        } else {
            EffectOutcome::CompletedWithResiduals { residuals }
        }
    } else if report.deadline_reached {
        EffectOutcome::Abandoned { residuals }
    } else {
        let stage = report
            .residuals
            .into_iter()
            .next()
            .unwrap_or_else(|| "teardown did not reach its goal".to_string());
        EffectOutcome::Failed {
            diagnosis: EffectDiagnosis::PathUnreachable { stage },
        }
    }
}

/// Network Event Handle
///
/// Lightweight handle for sending network events and receiving processing results.
/// Created before `ActrNode::start()` to bridge platform network events.
pub struct NetworkEventHandle {
    /// Event sender (to ActrNode)
    event_tx: mpsc::Sender<NetworkEventRequest>,
    result_timeout: Duration,
    /// The path-monitor incarnation epoch assigned when this handle is created.
    ///
    /// One handle is one authoritative monitor incarnation; every snapshot it
    /// forwards carries this epoch, and clones share it. A restarted monitor
    /// gets a fresh handle and therefore a newer epoch, so a stale replay from
    /// the previous incarnation is rejected by `(epoch, sequence)` ordering
    /// without any change to the existing sequence semantics callers rely on.
    source_epoch: u64,
    /// Optional fact sink so the upper layer can report a `SessionActivated`
    /// generation commit through this handle when the session/credential layer
    /// is not itself wired to the supervisor.
    fact_sink: Option<SupervisorFactSink>,
}

impl NetworkEventHandle {
    /// Create a new NetworkEventHandle
    pub fn new(event_tx: mpsc::Sender<NetworkEventRequest>) -> Self {
        Self::new_with_result_timeout(event_tx, NETWORK_EVENT_RESULT_TIMEOUT)
    }

    /// Create a new NetworkEventHandle with a custom result timeout.
    ///
    /// Production bindings use [`NetworkEventHandle::new`]. Tests can use this
    /// constructor to verify bounded waiting without sleeping for the full
    /// binding timeout.
    pub fn new_with_result_timeout(
        event_tx: mpsc::Sender<NetworkEventRequest>,
        result_timeout: Duration,
    ) -> Self {
        Self {
            event_tx,
            result_timeout,
            source_epoch: NEXT_NETWORK_SOURCE_EPOCH.fetch_add(1, Ordering::Relaxed),
            fact_sink: None,
        }
    }

    /// Create a new NetworkEventHandle carrying a supervisor fact sink so the
    /// upper layer can report `SessionActivated` through
    /// [`NetworkEventHandle::notify_session_activated`].
    pub(crate) fn new_with_fact_sink(
        event_tx: mpsc::Sender<NetworkEventRequest>,
        fact_sink: SupervisorFactSink,
    ) -> Self {
        Self {
            event_tx,
            result_timeout: NETWORK_EVENT_RESULT_TIMEOUT,
            source_epoch: NEXT_NETWORK_SOURCE_EPOCH.fetch_add(1, Ordering::Relaxed),
            fact_sink: Some(fact_sink),
        }
    }

    /// Create a handle for a newly attached or restarted path monitor.
    ///
    /// The returned handle shares the same supervisor channel and fact sink,
    /// but receives a fresh `source_epoch`; ordinary [`Clone`] keeps the current
    /// epoch and is therefore only for sharing one monitor incarnation.
    pub fn new_monitor_incarnation(&self) -> Self {
        Self {
            event_tx: self.event_tx.clone(),
            result_timeout: self.result_timeout,
            source_epoch: NEXT_NETWORK_SOURCE_EPOCH.fetch_add(1, Ordering::Relaxed),
            fact_sink: self.fact_sink.clone(),
        }
    }

    /// Report that a new session generation was authoritatively committed.
    ///
    /// This is the explicit path for an upper layer whose session/credential
    /// owner is not itself wired to the supervisor; the runtime session state
    /// reports the same fact directly at its hard-rebind commit point. Delivery
    /// is non-blocking and best-effort.
    pub fn notify_session_activated(&self, session_generation: u64) {
        match &self.fact_sink {
            Some(sink) => sink.session_activated(session_generation),
            None => tracing::debug!(
                session_generation,
                "network_event.handle.session_activated_no_sink"
            ),
        }
    }

    /// Handle full network path changes.
    pub async fn handle_network_path_changed(
        &self,
        snapshot: NetworkSnapshot,
    ) -> Result<NetworkEventResult, String> {
        self.send_event_and_await_result(NetworkEvent::NetworkPathChanged { snapshot })
            .await
    }

    /// Handle app lifecycle changes.
    pub async fn handle_app_lifecycle_changed(
        &self,
        state: AppLifecycleState,
    ) -> Result<NetworkEventResult, String> {
        self.send_event_and_await_result(NetworkEvent::AppLifecycleChanged { state })
            .await
    }

    /// Proactively clean up all connections with a reason. This never reconnects.
    pub async fn cleanup_connections(
        &self,
        reason: CleanupReason,
    ) -> Result<NetworkEventResult, String> {
        self.send_event_and_await_result(NetworkEvent::CleanupConnections { reason })
            .await
    }

    /// Force cleanup and reconnect.
    pub async fn force_reconnect(
        &self,
        reason: ReconnectReason,
    ) -> Result<NetworkEventResult, String> {
        self.send_event_and_await_result(NetworkEvent::ForceReconnect { reason })
            .await
    }

    /// Send event and await result (internal helper)
    async fn send_event_and_await_result(
        &self,
        event: NetworkEvent,
    ) -> Result<NetworkEventResult, String> {
        let event_request_id = NEXT_NETWORK_EVENT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        let start = Instant::now();
        let (result_tx, result_rx) = oneshot::channel();
        let request = NetworkEventRequest {
            event: event.clone(),
            result_tx,
            source_epoch: self.source_epoch,
            observed_at: tokio::time::Instant::now(),
        };

        tracing::info!(
            event_request_id,
            event = ?event,
            result_timeout_ms = self.result_timeout.as_millis() as u64,
            "network_event.handle.enqueue"
        );

        if let Err(e) = self.event_tx.send(request).await {
            let err = format!("Failed to send network event: {}", e);
            tracing::warn!(
                event_request_id,
                event = ?event,
                error = %err,
                "network_event.handle.enqueue_failed"
            );
            return Err(err);
        }

        let result = match crate::timer::timeout(
            crate::timer::ids::NETWORK_EVENT_ACCEPTANCE,
            self.result_timeout,
            result_rx,
        )
        .await
        {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(_)) => Err("Failed to receive network event result".to_string()),
            Err(_) => Err(format!(
                "Timed out waiting for network event result after {}ms",
                self.result_timeout.as_millis()
            )),
        };

        let wait_ms = start.elapsed().as_millis() as u64;
        match &result {
            Ok(result) if result.success => tracing::info!(
                event_request_id,
                event = ?event,
                result_event = ?result.event,
                duration_ms = result.duration_ms,
                wait_ms,
                "network_event.handle.result_received"
            ),
            Ok(result) => tracing::warn!(
                event_request_id,
                event = ?event,
                result_event = ?result.event,
                duration_ms = result.duration_ms,
                wait_ms,
                error = ?result.error,
                "network_event.handle.result_received"
            ),
            Err(e) => tracing::warn!(
                event_request_id,
                event = ?event,
                wait_ms,
                error = %e,
                "network_event.handle.result_failed"
            ),
        }

        result
    }
}

impl Clone for NetworkEventHandle {
    fn clone(&self) -> Self {
        Self {
            event_tx: self.event_tx.clone(),
            result_timeout: self.result_timeout,
            source_epoch: self.source_epoch,
            fact_sink: self.fact_sink.clone(),
        }
    }
}

#[cfg(test)]
#[path = "network_event_tests.rs"]
mod tests;
