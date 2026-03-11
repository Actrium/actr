//! Context factory
//!
//! Creates `RuntimeContext` instances and injects gates plus related dependencies.

use crate::context::RuntimeContext;
use crate::inbound::{DataStreamRegistry, MediaFrameRegistry};
use crate::outbound::Gate;
use crate::transport::HostTransport;
use crate::wire::webrtc::SignalingClient;
use actr_config::lock::LockFile;
use actr_protocol::{AIdCredential, ActrId};
use std::sync::Arc;

/// Context factory
///
/// # Responsibilities
///
/// - create `RuntimeContext` instances
/// - inject gates with enum dispatch and no virtual calls
/// - manage default factory configuration
#[derive(Clone)]
pub struct ContextFactory {
    /// In-process communication gate for local calls, available immediately.
    pub(crate) inproc_gate: Gate,

    /// Cross-process communication gate for remote calls, initialized lazily.
    pub(crate) outproc_gate: Option<Gate>,

    /// Transport manager for the Shell-to-Workload direction.
    pub(crate) shell_to_workload: Arc<HostTransport>,

    /// Transport manager for the Workload-to-Shell direction.
    pub(crate) workload_to_shell: Arc<HostTransport>,

    /// Callback registry for `DataStream`.
    pub(crate) data_stream_registry: Arc<DataStreamRegistry>,

    /// Callback registry for `MediaTrack`.
    pub(crate) media_frame_registry: Arc<MediaFrameRegistry>,

    /// Signaling client for discovery
    pub(crate) signaling_client: Arc<dyn SignalingClient>,

    /// Actr.lock.toml for dependency fingerprint lookups
    pub(crate) actr_lock: Option<LockFile>,
}

impl ContextFactory {
    /// Create a new `ContextFactory`.
    ///
    /// # Parameters
    ///
    /// - `inproc_gate`: in-process gate for `Dest::Shell` and `Dest::Local`
    /// - `shell_to_workload`: transport manager for the Shell-to-Workload direction
    /// - `workload_to_shell`: transport manager for the Workload-to-Shell direction
    /// - `data_stream_registry`: callback registry for `DataStream`
    /// - `media_frame_registry`: callback registry for `MediaTrack`
    ///
    /// # Design Notes
    ///
    /// - **inproc_gate**: created during `ActrSystem::new()` and available immediately
    /// - **outproc_gate**: starts as `None` and is set after WebRTC initialization in `ActrNode::start()`
    /// - **bidirectional HostTransport**: keeps pending request state fully separated between Shell and Workload
    /// - **data_stream_registry**: manages callbacks for application data streams
    /// - **media_frame_registry**: manages callbacks for native WebRTC media tracks
    pub fn new(
        inproc_gate: Gate,
        shell_to_workload: Arc<HostTransport>,
        workload_to_shell: Arc<HostTransport>,
        data_stream_registry: Arc<DataStreamRegistry>,
        media_frame_registry: Arc<MediaFrameRegistry>,
        signaling_client: Arc<dyn SignalingClient>,
    ) -> Self {
        Self {
            inproc_gate,
            outproc_gate: None, // Lazily initialized once WebRTC is ready.
            shell_to_workload,
            workload_to_shell,
            data_stream_registry,
            media_frame_registry,
            signaling_client,
            actr_lock: None, // Set later during `ActrNode::start()`.
        }
    }

    /// Set the cross-process communication gate.
    ///
    /// # Usage
    ///
    /// Called after `ActrNode::start()` completes WebRTC initialization.
    pub fn set_outproc_gate(&mut self, gate: Gate) {
        tracing::debug!("🔄 Setting outproc Gate in ContextFactory");
        self.outproc_gate = Some(gate);
    }

    /// Set the loaded `Actr.lock.toml`.
    ///
    /// # Usage
    ///
    /// Called from `ActrNode::start()` so route discovery can read dependency fingerprints.
    pub fn set_actr_lock(&mut self, actr_lock: LockFile) {
        tracing::debug!("🔄 Setting actr_lock in ContextFactory");
        self.actr_lock = Some(actr_lock);
    }

    /// Get the transport manager for the Shell-to-Workload direction.
    pub fn shell_to_workload(&self) -> Arc<HostTransport> {
        self.shell_to_workload.clone()
    }

    /// Get the transport manager for the Workload-to-Shell direction.
    pub fn workload_to_shell(&self) -> Arc<HostTransport> {
        self.workload_to_shell.clone()
    }

    /// Create a context for message handling.
    ///
    /// # Parameters
    ///
    /// - `self_id`: current actor ID
    /// - `caller_id`: optional caller actor ID
    /// - `request_id`: unique request ID
    ///
    /// # Returns
    ///
    /// A `RuntimeContext` instance implementing the `Context` trait.
    pub fn create(
        &self,
        self_id: &ActrId,
        caller_id: Option<&ActrId>,
        request_id: &str,
        credential: &AIdCredential,
    ) -> RuntimeContext {
        RuntimeContext::new(
            self_id.clone(),
            caller_id.cloned(),
            request_id.to_string(),
            self.inproc_gate.clone(), // Clone the gate enum; the inner `Arc` keeps the cost low.
            self.outproc_gate.clone(), // Clone Option<Gate>
            self.data_stream_registry.clone(), // Clone Arc<DataStreamRegistry>
            self.media_frame_registry.clone(), // Clone Arc<MediaFrameRegistry>
            self.signaling_client.clone(),
            credential.clone(),
            self.actr_lock.clone(), // Clone Option<LockFile>
        )
    }

    /// Create a bootstrap context for lifecycle hooks.
    ///
    /// # Usage
    ///
    /// Used by `on_start` and `on_stop` hooks where there is no caller ID.
    pub fn create_bootstrap(&self, self_id: &ActrId, credential: &AIdCredential) -> RuntimeContext {
        RuntimeContext::new(
            self_id.clone(),
            None,
            uuid::Uuid::new_v4().to_string(),
            self.inproc_gate.clone(),
            self.outproc_gate.clone(),
            self.data_stream_registry.clone(),
            self.media_frame_registry.clone(),
            self.signaling_client.clone(),
            credential.clone(),
            self.actr_lock.clone(),
        )
    }
}
