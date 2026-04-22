//! Lifecycle management layer (non-architectural layer)
//!
//! Responsible for Actor system lifecycle management:
//! - `node::Inner`: internal running-state struct used by `Node<S>` / `ActrRef`.

pub mod compat_lock;
pub mod dedup;
mod heartbeat;
pub(crate) mod hooks;
mod network_event;
pub(crate) mod node;

pub use compat_lock::{CompatLockFile, CompatLockManager, CompatibilityCheck, NegotiationEntry};
pub use heartbeat::heartbeat_task;
pub use network_event::{
    DebounceConfig, DefaultNetworkEventProcessor, NetworkEvent, NetworkEventHandle,
    NetworkEventProcessor, NetworkEventResult,
};
pub use node::CredentialState;
