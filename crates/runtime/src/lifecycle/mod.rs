//! Lifecycle management layer (non-architectural layer)
//!
//! Responsible for Actor system lifecycle management:
//! - ActrSystem: Initialization and configuration
//! - ActrNode<W>: Generic node (bound to Workload Type)

mod actr_node;
mod actr_system;

pub use actr_node::{ActrNode, CredentialState};
pub use actr_system::ActrSystem;
