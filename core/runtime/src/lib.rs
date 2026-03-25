//! # actr-runtime -- Business Dispatch Layer
//!
//! The streamlined `actr-runtime` contains only pure business dispatch logic,
//! with **no dependency** on tokio, WebRTC, wasmtime or other platform-specific libraries,
//! and can be compiled for both native and `wasm32-unknown-unknown` targets.
//!
//! ## Responsibility Separation
//!
//! ```text
//! actr-hyper   <- Infrastructure layer (transport, wire, signaling, WASM engine ...)
//! actr-runtime <- Business dispatch layer (ACL + dispatch + lifecycle hooks)  <- you are here
//! actr-framework <- SDK interface layer (trait definitions: Workload, Context, MessageDispatcher)
//! actr-protocol  <- Data definition layer (protobuf types)
//! ```
//!
//! ## Core Types
//!
//! - [`ActrDispatch`] -- Holds `Arc<Workload>` + optional ACL, provides `dispatch()` entry point
//! - [`check_acl_permission`] -- Pure function for ACL permission evaluation
//!
//! ## Usage Example
//!
//! ```rust,ignore
//! use actr_runtime::ActrDispatch;
//!
//! let dispatch = ActrDispatch::new(Arc::new(workload), acl);
//!
//! // Lifecycle
//! dispatch.on_start(&ctx).await?;
//!
//! // Message dispatch
//! let response = dispatch.dispatch(&self_id, caller_id.as_ref(), envelope, &ctx).await?;
//!
//! // Shutdown
//! dispatch.on_stop(&ctx).await?;
//! ```

pub mod acl;
pub mod dispatch;

// -- Core exports --
pub use acl::check_acl_permission;
pub use dispatch::ActrDispatch;

// -- Re-export actr-framework core traits for convenient downstream imports --
pub use actr_framework::{Context, MessageDispatcher, Workload};

// -- Re-export commonly used actr-protocol types --
pub use actr_protocol::{Acl, ActorResult, ActrError, ActrId, ActrType, RpcEnvelope};
