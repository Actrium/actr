//! ActrSystem - Generic-free infrastructure

use actr_config::Config;
use actr_framework::Workload;
use actr_protocol::ActorResult;
use std::sync::Arc;

use super::ActrNode;
use crate::context_factory::ContextFactory;

// Use types from sub-crates
use crate::wire::webrtc::{
    ReconnectConfig, SignalingClient, SignalingConfig, WebSocketSignalingClient,
};
use actr_mailbox::{DeadLetterQueue, Mailbox};

/// ActrSystem - Runtime infrastructure (generic-free)
///
/// # Design Philosophy
/// - Phase 1: Create pure runtime framework
/// - Knows nothing about business logic types
/// - Transforms into ActrNode<W> via attach()
pub struct ActrSystem {
    /// Runtime configuration
    config: Config,

    /// SQLite persistent mailbox
    mailbox: Arc<dyn Mailbox>,

    /// Dead Letter Queue for poison messages
    dlq: Arc<dyn DeadLetterQueue>,

    /// Context factory (with inproc_gate ready, outproc_gate deferred)
    context_factory: ContextFactory,

    /// Signaling client
    signaling_client: Arc<dyn SignalingClient>,
}

impl ActrSystem {
    /// Create new ActrSystem
    ///
    /// # Errors
    /// - Mailbox initialization failed
    /// - Transport initialization failed
    pub async fn new(config: Config) -> ActorResult<Self> {
        tracing::info!("🚀 Initializing ActrSystem");

        // Initialize Mailbox (using SqliteMailbox implementation)
        let mailbox_path = config
            .mailbox_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ":memory:".to_string());

        tracing::info!("📂 Mailbox database path: {}", mailbox_path);

        let mailbox: Arc<dyn Mailbox> = Arc::new(
            actr_mailbox::SqliteMailbox::new(&mailbox_path)
                .await
                .map_err(|e| {
                    actr_protocol::ProtocolError::TransportError(format!(
                        "Mailbox init failed: {e}"
                    ))
                })?,
        );

        // Initialize Dead Letter Queue
        // Use same path as mailbox, DLQ will create separate table in same database
        let dlq_path = if mailbox_path == ":memory:" {
            ":memory:".to_string() // Separate in-memory DB for DLQ
        } else {
            format!("{mailbox_path}.dlq") // Separate file for DLQ
        };

        let dlq: Arc<dyn DeadLetterQueue> = Arc::new(
            actr_mailbox::SqliteDeadLetterQueue::new_standalone(&dlq_path)
                .await
                .map_err(|e| {
                    actr_protocol::ProtocolError::TransportError(format!("DLQ init failed: {e}"))
                })?,
        );
        tracing::info!("✅ Dead Letter Queue initialized");

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // Initialize inproc infrastructure (Shell/Local communication - immediately available)
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        use crate::outbound::{InprocOutGate, OutGate};
        use crate::transport::InprocTransportManager;

        // Create TWO separate InprocTransportManager instances for bidirectional communication
        // This ensures Shell's pending_requests and Workload's pending_requests are separate

        // Direction 1: Shell → Workload (REQUEST)
        let shell_to_workload = Arc::new(InprocTransportManager::new());
        tracing::debug!("✨ Created shell_to_workload InprocTransportManager");

        // Direction 2: Workload → Shell (RESPONSE)
        let workload_to_shell = Arc::new(InprocTransportManager::new());
        tracing::debug!("✨ Created workload_to_shell InprocTransportManager");

        // Shell uses shell_to_workload for sending
        let inproc_gate =
            OutGate::InprocOut(Arc::new(InprocOutGate::new(shell_to_workload.clone())));

        // Create DataStreamRegistry for DataStream callbacks
        let data_stream_registry = Arc::new(crate::inbound::DataStreamRegistry::new());
        tracing::debug!("✨ Created DataStreamRegistry");

        // Create MediaFrameRegistry for MediaTrack callbacks
        let media_frame_registry = Arc::new(crate::inbound::MediaFrameRegistry::new());
        tracing::debug!("✨ Created MediaFrameRegistry");

        // ContextFactory holds both managers and registries
        let context_factory = ContextFactory::new(
            inproc_gate,
            shell_to_workload.clone(),
            workload_to_shell.clone(),
            data_stream_registry,
            media_frame_registry,
        );

        tracing::info!("✅ Inproc infrastructure initialized (bidirectional Shell ↔ Workload)");

        // Initialize signaling client (using WebSocketSignalingClient implementation)
        let signaling_config = SignalingConfig {
            server_url: config.signaling_url.clone(),
            connection_timeout: 30,
            heartbeat_interval: 30,
            reconnect_config: ReconnectConfig::default(),
            auth_config: None,
        };
        let signaling_client: Arc<dyn SignalingClient> =
            Arc::new(WebSocketSignalingClient::new(signaling_config));

        tracing::info!("✅ ActrSystem initialized");

        Ok(Self {
            config,
            mailbox,
            dlq,
            context_factory,
            signaling_client,
        })
    }

    /// Attach Workload, transform into ActrNode<W>
    ///
    /// # Type Inference
    /// - Infer W::Dispatcher from W
    /// - Compiler monomorphizes ActrNode<W>
    /// - Completely zero-dyn, full inline chain
    ///
    /// # Consumes self
    /// - Move ensures can only be called once
    /// - Embodies one-actor-per-instance principle
    pub fn attach<W: Workload>(self, workload: W) -> ActrNode<W> {
        tracing::info!("📦 Attaching workload");

        ActrNode {
            config: self.config,
            workload: Arc::new(workload),
            mailbox: self.mailbox,
            dlq: self.dlq,
            context_factory: Some(self.context_factory), // Initialized with inproc_gate ready
            signaling_client: self.signaling_client,
            actor_id: None,              // Obtained after startup
            credential: None,            // Obtained after startup
            webrtc_coordinator: None,    // Created after startup
            webrtc_gate: None,           // Created after startup
            inproc_mgr: None,            // Set after startup
            workload_to_shell_mgr: None, // Set after startup
            shutdown_token: tokio_util::sync::CancellationToken::new(),
        }
    }
}
