//! ActrSystem - Generic-free infrastructure

use std::path::Path;
use std::sync::Arc;

use actr_config::Config;
use actr_framework::Workload;
use actr_protocol::ActorResult;

use super::ActrNode;
use crate::context_factory::ContextFactory;
use crate::observability::ObservabilityGuard;

// Use types from sub-crates
use crate::wire::webrtc::{
    ReconnectConfig, SignalingClient, SignalingConfig, WebSocketSignalingClient,
};
use actr_runtime_mailbox::{DeadLetterQueue, Mailbox};

/// Network event channels tuple type: (event receiver, result sender, optional debounce config)
type NetworkEventChannels = std::sync::Mutex<
    Option<(
        tokio::sync::mpsc::Receiver<crate::lifecycle::network_event::NetworkEvent>,
        tokio::sync::mpsc::Sender<crate::lifecycle::network_event::NetworkEventResult>,
        Option<crate::lifecycle::network_event::DebounceConfig>,
    )>,
>;

/// ActrSystem - Runtime infrastructure (generic-free)
///
/// # Design Philosophy
/// - Phase 1: Create pure runtime framework
/// - Knows nothing about business logic types
/// - Transforms into ActrNode<W> via attach()
pub struct ActrSystem {
    /// Runtime configuration
    config: Config,

    /// Observability guard (keeps logging alive while system exists)
    _observability_guard: Option<ObservabilityGuard>,

    /// SQLite persistent mailbox
    mailbox: Arc<dyn Mailbox>,

    /// Dead Letter Queue for poison messages
    dlq: Arc<dyn DeadLetterQueue>,

    /// Context factory (with inproc_gate ready, outproc_gate deferred)
    context_factory: ContextFactory,

    /// Signaling client
    signaling_client: Arc<dyn SignalingClient>,

    /// Network event channels (lazily created in create_network_event_handle())
    /// Taken and passed to ActrNode during attach()
    network_event_channels: NetworkEventChannels,
}

impl ActrSystem {
    /// Create a fully initialized system from a config file path.
    ///
    /// Combines config loading, observability initialization, and system creation.
    /// The returned `ActrSystem` owns the observability guard internally.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let system = ActrSystem::from_config("actr.toml").await?;
    /// let _ref = system.attach(MyWorkload).start().await?;
    /// ```
    pub async fn from_config(config_path: impl AsRef<Path>) -> ActorResult<Self> {
        let path = config_path.as_ref();

        let config = actr_config::ConfigParser::from_file(path).map_err(|e| {
            actr_protocol::ActrError::InvalidArgument(format!(
                "failed to load config from `{}`: {e}",
                path.display()
            ))
        })?;

        let obs_guard = crate::init_observability(&config.observability)?;

        tracing::info!(config_path = %path.display(), "ActrSystem initializing from config file");

        let mut system = Self::create(config).await?;
        system._observability_guard = Some(obs_guard);
        Ok(system)
    }

    /// Create new ActrSystem from an already-parsed config.
    ///
    /// Use this when you need custom observability setup or have already
    /// parsed the config yourself. For most cases, prefer [`Self::boot`].
    ///
    /// # Errors
    /// - Mailbox initialization failed
    /// - Transport initialization failed
    pub async fn new(config: Config) -> ActorResult<Self> {
        Self::create(config).await
    }

    /// Internal creation logic shared by `boot` and `new`.
    async fn create(config: Config) -> ActorResult<Self> {
        tracing::info!("🚀 Initializing ActrSystem");

        // Initialize Mailbox (using SqliteMailbox implementation)
        let mailbox_path = config
            .mailbox_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ":memory:".to_string());

        tracing::info!("📂 Mailbox database path: {}", mailbox_path);

        let mailbox: Arc<dyn Mailbox> = Arc::new(
            actr_runtime_mailbox::SqliteMailbox::new(&mailbox_path)
                .await
                .map_err(|e| {
                    actr_protocol::ActrError::Unavailable(format!("Mailbox init failed: {e}"))
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
            actr_runtime_mailbox::SqliteDeadLetterQueue::new_standalone(&dlq_path)
                .await
                .map_err(|e| {
                    actr_protocol::ActrError::Unavailable(format!("DLQ init failed: {e}"))
                })?,
        );
        tracing::info!("✅ Dead Letter Queue initialized");

        // Initialize signaling client (using WebSocketSignalingClient implementation)
        let webrtc_role = if config.webrtc.advanced.prefer_answerer() {
            Some("answer".to_string())
        } else {
            None
        };

        let signaling_config = SignalingConfig {
            server_url: config.signaling_url.clone(),
            connection_timeout: 30,
            heartbeat_interval: 30,
            reconnect_config: ReconnectConfig::default(),
            auth_config: None,
            webrtc_role,
        };

        let client = Arc::new(WebSocketSignalingClient::new(signaling_config));
        client.start_reconnect_manager(); // Start if reconnect_config.enabled = true
        let signaling_client: Arc<dyn SignalingClient> = client;

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // Initialize inproc infrastructure (Shell/Local communication - immediately available)
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        use crate::outbound::{Gate, HostGate};
        use crate::transport::HostTransport;

        // Create TWO separate HostTransport instances for bidirectional communication
        // This ensures Shell's pending_requests and Workload's pending_requests are separate

        // Direction 1: Shell → Workload (REQUEST)
        let shell_to_workload = Arc::new(HostTransport::new());
        tracing::debug!("✨ Created shell_to_workload HostTransport");

        // Direction 2: Workload → Shell (RESPONSE)
        let workload_to_shell = Arc::new(HostTransport::new());
        tracing::debug!("✨ Created workload_to_shell HostTransport");

        // Shell uses shell_to_workload for sending
        let inproc_gate = Gate::Host(Arc::new(HostGate::new(shell_to_workload.clone())));

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
            signaling_client.clone(),
        );

        tracing::info!("✅ Inproc infrastructure initialized (bidirectional Shell ↔ Workload)");

        tracing::info!("✅ ActrSystem initialized");

        Ok(Self {
            config,
            _observability_guard: None,
            mailbox,
            dlq,
            context_factory,
            signaling_client,
            network_event_channels: std::sync::Mutex::new(None),
        })
    }

    /// Create network event processing infrastructure (called on demand)
    ///
    /// Creates NetworkEventHandle and internal channels.
    /// Channels are stored in the struct for use by attach().
    ///
    /// # Parameters
    /// - `debounce_ms`: Debounce window time in milliseconds. If 0, uses default.
    ///
    /// # Notes
    /// - Can only be called once
    /// - If this method is not called, network event functionality will be unavailable
    ///
    /// # Panics
    /// Panics if this method has already been called
    pub fn create_network_event_handle(
        &self,
        debounce_ms: u64,
    ) -> crate::lifecycle::NetworkEventHandle {
        // Create bidirectional channels
        let (event_tx, event_rx) = tokio::sync::mpsc::channel(100);
        let (result_tx, result_rx) = tokio::sync::mpsc::channel(100);

        // Store channels (for use by attach())
        let mut channels = self
            .network_event_channels
            .lock()
            .expect("Failed to lock network_event_channels");

        if channels.is_some() {
            panic!("create_network_event_handle() can only be called once");
        }

        // Build debounce config
        let debounce_config = if debounce_ms > 0 {
            Some(crate::lifecycle::network_event::DebounceConfig {
                window: std::time::Duration::from_millis(debounce_ms),
            })
        } else {
            None
        };

        *channels = Some((event_rx, result_tx, debounce_config));

        // Create and return NetworkEventHandle
        crate::lifecycle::NetworkEventHandle::new(event_tx, result_rx)
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

        // Try to load Actr.lock.toml from config directory (optional)
        let actr_lock_path = self.config.config_dir.join("Actr.lock.toml");
        let actr_lock = match actr_config::lock::LockFile::from_file(&actr_lock_path) {
            Ok(lock) => {
                tracing::info!(
                    "📋 Loaded Actr.lock.toml with {} dependencies",
                    lock.dependencies.len()
                );
                Some(lock)
            }
            Err(e) => {
                // If lock file is missing or invalid, continue without dependency fingerprints
                tracing::warn!(
                    "⚠️ Actr.lock.toml not loaded (path: {:?}, ERR: {}). Continuing without dependency fingerprints.",
                    actr_lock_path,
                    e
                );
                None
            }
        };
        // Take channels from network_event_channels (if present)
        let (network_event_rx, network_event_result_tx, network_event_debounce_config) = self
            .network_event_channels
            .lock()
            .expect("Failed to lock network_event_channels")
            .take()
            .map(|(rx, tx, config)| (Some(rx), Some(tx), config))
            .unwrap_or((None, None, None));

        ActrNode {
            config: self.config,
            workload: Arc::new(workload),
            mailbox: self.mailbox,
            dlq: self.dlq,
            context_factory: Some(self.context_factory), // Initialized with inproc_gate ready
            signaling_client: self.signaling_client,
            actor_id: None,              // Obtained after startup
            credential_state: None,      // Obtained after startup (includes TurnCredential)
            webrtc_coordinator: None,    // Pass shared coordinator
            webrtc_gate: None,           // Created after startup
            websocket_gate: None,        // Created after startup (if websocket_listen_port is set)
            inproc_mgr: None,            // Set after startup
            workload_to_shell_mgr: None, // Set after startup
            shutdown_token: tokio_util::sync::CancellationToken::new(),
            actr_lock,
            network_event_rx,
            network_event_result_tx,
            network_event_debounce_config,
            dedup_state: std::sync::Arc::new(tokio::sync::Mutex::new(
                crate::lifecycle::dedup::DedupState::new(),
            )),
            discovered_ws_addresses: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            injected_registration: None, // Set by inject_credential() before start() in Process/Wasm mode
            executor: None, // Set by with_executor() / with_wasm_instance() after build()
        }
    }
}
