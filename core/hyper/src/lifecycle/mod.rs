//! Lifecycle management layer (non-architectural layer)
//!
//! Responsible for Actor system lifecycle management:
//! - `node::Inner`: internal running-state struct used by `Node<S>` / `ActrRef`.

pub(crate) mod compat_lock;
pub(crate) mod credential_manager;
pub(crate) mod dedup;
mod heartbeat;
pub(crate) mod hooks;
mod machine_docs;
mod network_event;
pub(crate) mod node;
mod recovery_execution;
mod recovery_policy;
mod recovery_supervisor;
pub(crate) mod session_state;

pub use recovery_supervisor::ConnectionFact;
// `process_network_event_batch` and `select_network_recovery_action` are the
// deprecated legacy batch path; they are still re-exported for the migration
// window, so this re-export intentionally allows the deprecation lint.
#[allow(deprecated)]
pub use network_event::{
    AppLifecycleState, CleanupReason, DebounceConfig, DefaultNetworkEventProcessor,
    NetworkAvailability, NetworkEvent, NetworkEventHandle, NetworkEventProcessor,
    NetworkEventRequest, NetworkEventResult, NetworkRecoveryAction, NetworkSnapshot,
    NetworkTransportFlags, ObservedOutcome, ReconnectReason, SignalingFactLostCause,
    SignalingFactOrigin, SupervisorFactSink, SupervisorStatus, TeardownReport,
    process_network_event_batch, run_network_event_reconciler,
    run_network_event_reconciler_with_status, select_network_recovery_action,
};
#[cfg(feature = "test-utils")]
pub(crate) use network_event::{
    SupervisorInternalChannel, run_network_event_reconciler_with_channel,
    supervisor_internal_channel, supervisor_internal_channel_gated,
};
pub use node::CredentialState;
pub use session_state::{SessionPhase, SessionSnapshot, SessionState};
