//! Lifecycle management layer (non-architectural layer)
//!
//! Responsible for Actor system lifecycle management:
//! - `node::Inner`: internal running-state struct used by `Node<S>` / `ActrRef`.

pub(crate) mod compat_lock;
pub(crate) mod dedup;
mod heartbeat;
pub(crate) mod hooks;
mod network_event;
pub(crate) mod node;

pub use network_event::{
    DebounceConfig, DefaultNetworkEventProcessor, NetworkEvent, NetworkEventHandle,
    NetworkEventProcessor, NetworkEventResult, NetworkRecoveryAction, process_network_event_batch,
    run_network_event_reconciler, select_network_recovery_action,
};
pub use node::CredentialState;
