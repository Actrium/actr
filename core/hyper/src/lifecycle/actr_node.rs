//! ActrNode - runtime node with optional workload

use crate::actr_ref::{ActrRef, ActrRefShared};
use crate::ais_client::AisClient;
use crate::context_factory::ContextFactory;
use crate::lifecycle::dedup::{DedupOutcome, DedupState};
use crate::transport::HostTransport;
use crate::wire::webrtc::SignalingClient;
#[cfg(feature = "opentelemetry")]
use crate::wire::webrtc::trace::{inject_span_context_to_rpc, set_parent_from_rpc_envelope};
use actr_framework::Bytes;
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{
    AIdCredential, ActorResult, ActrError, ActrId, PayloadType, RegisterRequest, RpcEnvelope,
    TurnCredential, register_response,
};
use actr_runtime::check_acl_permission;
use actr_runtime_mailbox::{DeadLetterQueue, Mailbox};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
#[cfg(feature = "opentelemetry")]
use tracing::Instrument as _;

/// ActrNode - runtime node with optional workload
pub struct ActrNode {
    /// Runtime configuration
    pub(crate) config: actr_config::RuntimeConfig,

    /// SQLite persistent mailbox
    pub(crate) mailbox: Arc<dyn Mailbox>,

    /// Dead Letter Queue for poison messages
    pub(crate) dlq: Arc<dyn DeadLetterQueue>,

    /// Context factory (created in start() after obtaining ActorId)
    pub(crate) context_factory: Option<ContextFactory>,

    /// Signaling client
    pub(crate) signaling_client: Arc<dyn SignalingClient>,

    /// Actor ID (obtained after startup)
    pub(crate) actor_id: Option<ActrId>,

    /// Actor Credential (obtained after startup, used for subsequent authentication messages)
    pub(crate) credential_state: Option<CredentialState>,

    /// WebRTC coordinator (created after startup)
    pub(crate) webrtc_coordinator: Option<Arc<crate::wire::webrtc::coordinator::WebRtcCoordinator>>,

    /// WebRTC Gate (created after startup)
    pub(crate) webrtc_gate: Option<Arc<crate::wire::webrtc::gate::WebRtcGate>>,

    /// WebSocket Gate (direct-connect mode inbound, optional)
    pub(crate) websocket_gate: Option<Arc<crate::wire::websocket::WebSocketGate>>,

    /// Shell → Guest Transport Manager
    ///
    /// Guest receives REQUEST from Shell (zero serialization, direct RpcEnvelope passing)
    pub(crate) inproc_mgr: Option<Arc<HostTransport>>,

    /// Guest → Shell Transport Manager
    ///
    /// Guest sends RESPONSE to Shell (separate pending_requests from Shell's)
    pub(crate) guest_to_shell_mgr: Option<Arc<HostTransport>>,

    /// Shutdown token for graceful shutdown
    pub(crate) shutdown_token: CancellationToken,

    /// Packaged manifest.lock.toml content loaded at startup for fingerprint lookups
    pub(crate) actr_lock: Option<actr_config::lock::LockFile>,
    /// Network event receiver (from NetworkEventHandle)
    pub(crate) network_event_rx:
        Option<tokio::sync::mpsc::Receiver<crate::lifecycle::network_event::NetworkEvent>>,

    /// Network event result sender (to NetworkEventHandle)
    pub(crate) network_event_result_tx:
        Option<tokio::sync::mpsc::Sender<crate::lifecycle::network_event::NetworkEventResult>>,

    /// Network event debounce configuration
    pub(crate) network_event_debounce_config:
        Option<crate::lifecycle::network_event::DebounceConfig>,

    /// Request deduplication state (15 s TTL response cache, prevents double-processing on retry)
    pub(crate) dedup_state: Arc<Mutex<DedupState>>,

    /// Verified package manifest for package-backed nodes.
    #[allow(dead_code)]
    pub(crate) package_manifest: Option<crate::verify::PackageManifest>,

    /// Pre-issued registration credential injected by the Hyper layer during
    /// the `Attached → Registered` state transition. `start()` uses it directly
    /// instead of re-registering with the signaling server.
    pub(crate) injected_registration: Option<actr_protocol::register_response::RegisterOk>,

    /// Shared WebSocket direct-connect address map populated by discovery
    ///
    /// Shared with `DefaultWireBuilder` so discovered ws:// URLs can be reused
    /// directly instead of relying on a static url_template
    /// The map is keyed by `ActrId`.
    pub(crate) discovered_ws_addresses:
        Arc<tokio::sync::RwLock<std::collections::HashMap<ActrId, String>>>,

    /// Runtime workload (WASM, dynclib, etc.)
    ///
    /// `handle_incoming` dispatches through this workload.
    ///
    /// The `Mutex` serializes dispatch into a single guest actor instance.
    pub(crate) workload: Mutex<crate::workload::Workload>,
}

/// Credential state for shared access between tasks
#[derive(Clone)]
pub struct CredentialState {
    inner: Arc<RwLock<CredentialStateInner>>,
}

#[derive(Clone)]
struct CredentialStateInner {
    credential: AIdCredential,
    expires_at: Option<prost_types::Timestamp>,
    /// HMAC time-limited TURN credential, updated together with credential on registration/renewal
    turn_credential: Option<TurnCredential>,
}

impl CredentialState {
    /// Create a new CredentialState with TURN credential
    pub fn new(
        credential: AIdCredential,
        expires_at: Option<prost_types::Timestamp>,
        turn_credential: Option<TurnCredential>,
    ) -> Self {
        Self {
            inner: Arc::new(RwLock::new(CredentialStateInner {
                credential,
                expires_at,
                turn_credential,
            })),
        }
    }

    pub async fn credential(&self) -> AIdCredential {
        self.inner.read().await.credential.clone()
    }

    pub async fn expires_at(&self) -> Option<prost_types::Timestamp> {
        self.inner.read().await.expires_at
    }

    /// Get TURN credential (HMAC time-limited credential)
    pub async fn turn_credential(&self) -> Option<TurnCredential> {
        self.inner.read().await.turn_credential.clone()
    }

    /// Update credential and TURN credential
    ///
    /// Called on credential renewal; only overwrites the old TURN credential when the new one is not empty
    pub(crate) async fn update(
        &self,
        credential: AIdCredential,
        expires_at: Option<prost_types::Timestamp>,
        turn_credential: Option<TurnCredential>,
    ) {
        let mut guard = self.inner.write().await;
        guard.credential = credential;
        guard.expires_at = expires_at;
        if turn_credential.is_some() {
            guard.turn_credential = turn_credential;
        }
    }
}

/// Host operation executor - routes guest outbound calls through RuntimeContext
///
/// Called by the workload dispatch path in `handle_incoming`.
async fn host_operation_handler(
    ctx: crate::context::RuntimeContext,
    pending: crate::workload::HostOperation,
) -> crate::workload::HostOperationResult {
    use crate::workload::{HostOperation, HostOperationResult, decode_dest};
    use actr_framework::guest::abi::code as abi_code;
    use actr_framework::{Context as _, Dest};
    use actr_protocol::PayloadType;

    /// Map `ActrError` to ABI error code, preserving semantics for guest-side discrimination
    fn actr_error_to_code(err: &ActrError) -> i32 {
        match err {
            ActrError::DecodeFailure(_) | ActrError::InvalidArgument(_) => abi_code::PROTOCOL_ERROR,
            _ => abi_code::GENERIC_ERROR,
        }
    }

    match pending {
        HostOperation::CallRaw(req) => {
            match ctx
                .call_raw(
                    &Dest::Actor(req.target),
                    req.route_key,
                    PayloadType::RpcReliable,
                    bytes::Bytes::from(req.payload),
                    30_000,
                )
                .await
            {
                Ok(resp) => HostOperationResult::Bytes(resp.to_vec()),
                Err(e) => {
                    tracing::error!("call_raw routing failed: {e:?}");
                    HostOperationResult::Error(actr_error_to_code(&e))
                }
            }
        }

        HostOperation::Call(req) => {
            let dest = match decode_dest(&req.dest) {
                Some(d) => d,
                None => {
                    tracing::error!(route_key = req.route_key, "call: dest decode failed");
                    return HostOperationResult::Error(abi_code::PROTOCOL_ERROR);
                }
            };
            match ctx
                .call_raw(
                    &dest,
                    req.route_key,
                    PayloadType::RpcReliable,
                    bytes::Bytes::from(req.payload),
                    30_000,
                )
                .await
            {
                Ok(resp) => HostOperationResult::Bytes(resp.to_vec()),
                Err(e) => {
                    tracing::error!("call routing failed: {e:?}");
                    HostOperationResult::Error(actr_error_to_code(&e))
                }
            }
        }

        HostOperation::Tell(req) => {
            let dest = match decode_dest(&req.dest) {
                Some(d) => d,
                None => {
                    tracing::error!(route_key = req.route_key, "tell: dest decode failed");
                    return HostOperationResult::Error(abi_code::PROTOCOL_ERROR);
                }
            };
            match ctx
                .tell_raw(
                    &dest,
                    req.route_key,
                    PayloadType::RpcReliable,
                    bytes::Bytes::from(req.payload),
                )
                .await
            {
                Ok(()) => HostOperationResult::Done,
                Err(e) => {
                    tracing::error!("tell routing failed: {e:?}");
                    HostOperationResult::Error(actr_error_to_code(&e))
                }
            }
        }

        HostOperation::Discover(req) => {
            match ctx.discover_route_candidate(&req.target_type).await {
                Ok(id) => HostOperationResult::Bytes(id.encode_to_vec()),
                Err(e) => {
                    tracing::error!("discover failed: {e:?}");
                    HostOperationResult::Error(actr_error_to_code(&e))
                }
            }
        }
    }
}

/// Map ActrError to error code for ErrorResponse
fn protocol_error_to_code(err: &ActrError) -> u32 {
    match err {
        ActrError::Unavailable(_) => 503,            // Service Unavailable
        ActrError::TimedOut => 504,                  // Gateway Timeout
        ActrError::NotFound(_) => 404,               // Not Found
        ActrError::PermissionDenied(_) => 403,       // Forbidden
        ActrError::InvalidArgument(_) => 400,        // Bad Request
        ActrError::UnknownRoute(_) => 404,           // Not Found - route not found
        ActrError::DependencyNotFound { .. } => 400, // Bad Request
        ActrError::DecodeFailure(_) => 400,          // Bad Request - decode failure
        ActrError::NotImplemented(_) => 501,         // Not Implemented
        ActrError::Internal(_) => 500,               // Internal Server Error
    }
}

impl ActrNode {
    #[allow(dead_code)]
    pub(crate) fn package_manifest(&self) -> Option<&crate::verify::PackageManifest> {
        self.package_manifest.as_ref()
    }

    /// Get Inproc Transport Manager
    ///
    /// # Returns
    /// - `Some(Arc<HostTransport>)`: Initialized manager
    /// - `None`: Not yet started (need to call start() first)
    ///
    /// # Use Cases
    /// - Guest internals need to communicate with Shell
    /// - Create custom LatencyFirst/MediaTrack channels
    pub fn inproc_mgr(&self) -> Option<Arc<HostTransport>> {
        self.inproc_mgr.clone()
    }

    /// Get ActorId (if registration has completed)
    pub fn actor_id(&self) -> Option<&ActrId> {
        self.actor_id.as_ref()
    }

    /// Get credential state (if registration has completed)
    pub fn credential_state(&self) -> Option<CredentialState> {
        self.credential_state.clone()
    }

    /// Get signaling client (for manual control such as UnregisterRequest)
    pub fn signaling_client(&self) -> Arc<dyn SignalingClient> {
        self.signaling_client.clone()
    }

    /// Get shutdown token for this node
    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown_token.clone()
    }

    /// Network event processing loop (background task)
    ///
    /// # Responsibilities
    /// - Receive network events from Channel
    /// - Delegate to NetworkEventProcessor for handling
    /// - Record processing time and send results
    async fn network_event_loop(
        mut event_rx: tokio::sync::mpsc::Receiver<crate::lifecycle::network_event::NetworkEvent>,
        result_tx: tokio::sync::mpsc::Sender<crate::lifecycle::network_event::NetworkEventResult>,
        event_processor: Arc<dyn crate::lifecycle::network_event::NetworkEventProcessor>,
        shutdown_token: CancellationToken,
    ) {
        use crate::lifecycle::network_event::{
            NetworkEvent, NetworkEventResult, deduplicate_network_events,
        };

        tracing::info!("🔄 Network event loop started");

        loop {
            tokio::select! {
                // Receive network events
                Some(event) = event_rx.recv() => {
                    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
                    // Queue cleanup: deduplicate by type, keeping the latest event of each type
                    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

                    // Collect all events from the queue
                    let mut all_events = vec![event];
                    while let Ok(next_event) = event_rx.try_recv() {
                        all_events.push(next_event);
                    }

                    let total_events = all_events.len();

                    // Deduplicate by type
                    let events_to_process = deduplicate_network_events(all_events);

                    let processed_count = events_to_process.len();
                    let discarded_count = total_events - processed_count;

                    if discarded_count > 0 {
                        tracing::info!(
                            total_events = total_events,
                            processed_count = processed_count,
                            discarded_count = discarded_count,
                            "🗑️ Deduplicated {} stale events (by type), processing {} unique events",
                            discarded_count,
                            processed_count
                        );
                    }

                    // Process deduplicated events
                    for latest_event in events_to_process {
                        let start = std::time::Instant::now();

                        let result = match &latest_event {
                            NetworkEvent::Available => {
                                event_processor.process_network_available().await
                            }
                            NetworkEvent::Lost => {
                                event_processor.process_network_lost().await
                            }
                            NetworkEvent::TypeChanged { is_wifi, is_cellular } => {
                                event_processor
                                    .process_network_type_changed(*is_wifi, *is_cellular)
                                    .await
                            }
                            NetworkEvent::CleanupConnections => {
                                event_processor.cleanup_connections().await
                            }
                        };

                        let duration_ms = start.elapsed().as_millis() as u64;

                        // Construct processing result
                        let event_result = match result {
                            Ok(_) => NetworkEventResult::success(latest_event.clone(), duration_ms),
                            Err(e) => {
                                NetworkEventResult::failure(latest_event.clone(), e, duration_ms)
                            }
                        };

                        // Send result (ignore send failures to avoid blocking)
                        if let Err(e) = result_tx.send(event_result).await {
                            tracing::warn!("Failed to send event result: {}", e);
                        }
                    }
                }

                // Listen for shutdown signal
                _ = shutdown_token.cancelled() => {
                    tracing::info!("🛑 Network event loop shutting down");
                    break;
                }
            }
        }
    }

    /// - Single-hop calls: effectively identical
    /// - Multi-hop calls: trace_id spans all hops, request_id per hop
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(
            skip_all,
            name = "ActrNode.handle_incoming",
            fields(
                actr_id = %self.actor_id.as_ref().map(|id| id.to_string()).unwrap_or_default(),
                route_key = %envelope.route_key,
                request_id = %envelope.request_id,
            )
        )
    )]
    pub async fn handle_incoming(
        &self,
        envelope: RpcEnvelope,
        caller_id: Option<&ActrId>,
    ) -> ActorResult<Bytes> {
        // Log received message
        if let Some(caller) = caller_id {
            tracing::debug!(
                "📨 Handling incoming message: route_key={}, caller={}, request_id={}",
                envelope.route_key,
                caller,
                envelope.request_id
            );
        } else {
            tracing::debug!(
                "📨 Handling incoming message: route_key={}, request_id={}",
                envelope.route_key,
                envelope.request_id
            );
        }

        // 0. Get actor_id early for ACL check
        let actor_id = self.actor_id.as_ref().ok_or_else(|| {
            ActrError::Internal(
                "Actor ID not set - node must be started before handling messages".to_string(),
            )
        })?;

        // 0.1. ACL Permission Check (before processing message)
        let acl_allowed = check_acl_permission(caller_id, actor_id, self.config.acl.as_ref())
            .map_err(|err_msg| ActrError::Internal(format!("ACL check failed: {}", err_msg)))?;

        if !acl_allowed {
            tracing::warn!(
                severity = 5,
                error_category = "acl_denied",
                request_id = %envelope.request_id,
                route_key = %envelope.route_key,
                caller = %caller_id
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "<none>".to_string()),
                "🚫 ACL: Permission denied"
            );

            return Err(ActrError::PermissionDenied(format!(
                "ACL denied: {} is not allowed to call {}",
                caller_id
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "<unknown>".to_string()),
                actor_id
            )));
        }

        // 0.2. Deduplication: return cached response for retried request_ids
        {
            let outcome = self
                .dedup_state
                .lock()
                .await
                .check_or_mark(&envelope.request_id);
            match outcome {
                DedupOutcome::Fresh => {} // proceed normally
                DedupOutcome::InFlight => {
                    tracing::warn!(
                        request_id = %envelope.request_id,
                        route_key = %envelope.route_key,
                        "⚠️ duplicate request in-flight, dropping concurrent copy"
                    );
                    return Err(ActrError::InvalidArgument(
                        "duplicate request already in-flight".to_string(),
                    ));
                }
                DedupOutcome::Duplicate(cached) => {
                    tracing::debug!(
                        request_id = %envelope.request_id,
                        route_key = %envelope.route_key,
                        "♻️ returning cached response for duplicate request_id"
                    );
                    return cached;
                }
            }
        }

        // 1. Create Context with caller_id from transport layer
        let credential_state = self.credential_state.clone().ok_or_else(|| {
            ActrError::Internal(
                "Credential not set - node must be started before handling messages".to_string(),
            )
        })?;
        let ctx = self
            .context_factory
            .as_ref()
            .expect("ContextFactory must be initialized in start()")
            .create(
                actor_id,
                caller_id, // caller_id from transport layer (MessageRecord.from)
                &envelope.request_id,
                &credential_state.credential().await,
            );

        // 2. Dispatch
        let dispatch_ctx = crate::workload::InvocationContext {
            self_id: actor_id.clone(),
            caller_id: caller_id.cloned(),
            request_id: envelope.request_id.clone(),
        };
        let ctx_for_executor = ctx.clone();
        let call_executor: crate::workload::HostAbiFn = Box::new(move |pending| {
            let ctx = ctx_for_executor.clone();
            Box::pin(async move { host_operation_handler(ctx, pending).await })
        });

        let mut guard = self.workload.lock().await;
        let result = guard
            .dispatch_envelope(envelope.clone(), ctx.clone(), dispatch_ctx, &call_executor)
            .await
            .map_err(|e| ActrError::Internal(format!("workload dispatch failed: {e:?}")));

        match &result {
            Ok(_) => tracing::debug!(
                request_id = %envelope.request_id,
                route_key = %envelope.route_key,
                "✅ Message handled successfully"
            ),
            Err(e) => tracing::error!(
                severity = 6,
                error_category = "handler_error",
                request_id = %envelope.request_id,
                route_key = %envelope.route_key,
                "❌ Message handling failed: {:?}", e
            ),
        }

        // 3. Store completed result in dedup cache before returning
        self.dedup_state
            .lock()
            .await
            .complete(&envelope.request_id, result.clone());

        result
    }

    /// Build a new ActrNode from config and runtime workload.
    ///
    /// This is the internal constructor behind the public node builders and
    /// Hyper package attach helpers.
    pub(crate) async fn build(
        config: actr_config::RuntimeConfig,
        workload: crate::workload::Workload,
        package_manifest: Option<crate::verify::PackageManifest>,
        packaged_lock: Option<actr_config::lock::LockFile>,
    ) -> ActorResult<Self> {
        use crate::outbound::{Gate, HostGate};
        use crate::wire::webrtc::{ReconnectConfig, SignalingConfig, WebSocketSignalingClient};

        tracing::info!("🚀 Initializing ActrNode");

        // Initialize Mailbox
        let mailbox_path = config
            .mailbox_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ":memory:".to_string());

        tracing::info!("📂 Mailbox database path: {}", mailbox_path);

        let mailbox: Arc<dyn actr_runtime_mailbox::Mailbox> = Arc::new(
            actr_runtime_mailbox::SqliteMailbox::new(&mailbox_path)
                .await
                .map_err(|e| {
                    actr_protocol::ActrError::Unavailable(format!("Mailbox init failed: {e}"))
                })?,
        );

        // Initialize Dead Letter Queue
        let dlq_path = if mailbox_path == ":memory:" {
            ":memory:".to_string()
        } else {
            format!("{mailbox_path}.dlq")
        };

        let dlq: Arc<dyn actr_runtime_mailbox::DeadLetterQueue> = Arc::new(
            actr_runtime_mailbox::SqliteDeadLetterQueue::new_standalone(&dlq_path)
                .await
                .map_err(|e| {
                    actr_protocol::ActrError::Unavailable(format!("DLQ init failed: {e}"))
                })?,
        );
        tracing::info!("✅ Dead Letter Queue initialized");

        // Initialize signaling client
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
        client.start_reconnect_manager();
        let signaling_client: Arc<dyn crate::wire::webrtc::SignalingClient> = client;

        // Initialize inproc infrastructure (Shell ↔ Guest)
        let shell_to_workload = Arc::new(HostTransport::new());
        let workload_to_shell = Arc::new(HostTransport::new());
        let inproc_gate = Gate::Host(Arc::new(HostGate::new(shell_to_workload.clone())));

        let data_stream_registry = Arc::new(crate::inbound::DataStreamRegistry::new());
        let media_frame_registry = Arc::new(crate::inbound::MediaFrameRegistry::new());

        let context_factory = ContextFactory::new(
            inproc_gate,
            shell_to_workload.clone(),
            workload_to_shell.clone(),
            data_stream_registry,
            media_frame_registry,
            signaling_client.clone(),
        );

        tracing::info!("✅ Inproc infrastructure initialized (bidirectional Shell ↔ Guest)");

        let actr_lock = if let Some(lock) = packaged_lock {
            tracing::info!(
                "📋 Loaded packaged manifest.lock.toml with {} dependencies",
                lock.dependencies.len()
            );
            Some(lock)
        } else {
            tracing::warn!(
                "⚠️ manifest.lock.toml not found in package. Continuing without dependency fingerprints."
            );
            None
        };

        tracing::info!("✅ ActrNode initialized");

        Ok(Self {
            config,
            mailbox,
            dlq,
            context_factory: Some(context_factory),
            signaling_client,
            actor_id: None,
            credential_state: None,
            webrtc_coordinator: None,
            webrtc_gate: None,
            websocket_gate: None,
            inproc_mgr: None,
            guest_to_shell_mgr: None,
            shutdown_token: CancellationToken::new(),
            actr_lock,
            network_event_rx: None,
            network_event_result_tx: None,
            network_event_debounce_config: None,
            dedup_state: Arc::new(Mutex::new(DedupState::new())),
            package_manifest,
            injected_registration: None,
            discovered_ws_addresses: Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            workload: Mutex::new(workload),
        })
    }

    /// Create network event processing infrastructure (called on demand, before `start()`).
    ///
    /// # Parameters
    /// - `debounce_ms`: Debounce window in milliseconds. If 0, no debounce.
    ///
    /// # Panics
    /// Panics if called more than once.
    pub fn create_network_event_handle(
        &mut self,
        debounce_ms: u64,
    ) -> crate::lifecycle::NetworkEventHandle {
        if self.network_event_rx.is_some() {
            panic!("create_network_event_handle() can only be called once");
        }

        let (event_tx, event_rx) = tokio::sync::mpsc::channel(100);
        let (result_tx, result_rx) = tokio::sync::mpsc::channel(100);

        let debounce_config = if debounce_ms > 0 {
            Some(crate::lifecycle::network_event::DebounceConfig {
                window: std::time::Duration::from_millis(debounce_ms),
            })
        } else {
            None
        };

        self.network_event_rx = Some(event_rx);
        self.network_event_result_tx = Some(result_tx);
        self.network_event_debounce_config = debounce_config;

        crate::lifecycle::NetworkEventHandle::new(event_tx, result_rx)
    }

    /// Inject a pre-issued registration credential
    ///
    /// Called by the Hyper layer before `start()`, writing the already-issued `RegisterOk`
    /// into ActrNode so that `start()` skips the signaling registration step.
    pub fn inject_credential(&mut self, register_ok: register_response::RegisterOk) {
        tracing::debug!("Injected pre-registration credential; start() will skip AIS registration");
        self.injected_registration = Some(register_ok);
    }

    /// Start the system
    pub async fn start(mut self) -> ActorResult<ActrRef> {
        tracing::info!("🚀 Starting ActrNode");
        println!("Actr Rust version: {}", env!("CARGO_PKG_VERSION"));

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // 1. Build RegisterRequest
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // Get ActrType from configuration
        let actr_type = self.config.actr_type().clone();
        tracing::info!("📋 Actor type: {}", actr_type);

        // Calculate ServiceSpec from config exports
        let service_spec = self.config.calculate_service_spec();
        if let Some(ref spec) = service_spec {
            tracing::info!("📦 Service fingerprint: {}", spec.fingerprint);
            tracing::info!("📦 Service tags: {:?}", spec.tags);
        } else {
            tracing::info!("📦 No proto exports, ServiceSpec is None");
        }

        // If a WebSocket listen port is configured, build the advertised ws:// address
        // to register with the signaling server so clients can discover it.
        let ws_address = if let Some(port) = self.config.websocket_listen_port {
            let host = self
                .config
                .websocket_advertised_host
                .as_deref()
                .unwrap_or("127.0.0.1");
            Some(format!("ws://{}:{}", host, port))
        } else {
            None
        };

        if let Some(ref addr) = ws_address {
            tracing::info!(
                "📡 Advertising WebSocket address to signaling server: {}",
                addr
            );
        }

        let register_request = RegisterRequest {
            actr_type: actr_type.clone(),
            realm: self.config.realm,
            service_spec,
            acl: self.config.acl.clone(),
            service: None,
            ws_address,
            ..Default::default()
        };

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // 1. Obtain registration info (Hyper pre-injected or AIS HTTP)
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        let register_ok = if let Some(injected) = self.injected_registration.take() {
            tracing::info!(
                "Using Hyper pre-injected registration credential; skipping AIS registration"
            );
            injected
        } else {
            let ais_endpoint = &self.config.ais_endpoint;
            tracing::info!(
                ais_endpoint = %ais_endpoint,
                "Registering actor with AIS via HTTP"
            );
            let mut ais = AisClient::new(ais_endpoint);
            if let Some(ref secret) = self.config.realm_secret {
                ais = ais.with_realm_secret(secret);
            }
            let resp = ais
                .register_with_manifest(register_request.clone())
                .await
                .map_err(|e| ActrError::Unavailable(format!("AIS registration failed: {e}")))?;
            match resp.result {
                Some(register_response::Result::Success(ok)) => {
                    tracing::info!("✅ AIS HTTP registration successful");
                    ok
                }
                Some(register_response::Result::Error(error)) => {
                    tracing::error!(
                        severity = 10,
                        error_category = "registration_error",
                        error_code = error.code,
                        "❌ AIS registration failed: code={}, message={}",
                        error.code,
                        error.message
                    );
                    return Err(ActrError::Unavailable(format!(
                        "AIS registration rejected: {} (code: {})",
                        error.message, error.code
                    )));
                }
                None => {
                    tracing::error!(
                        severity = 10,
                        error_category = "registration_error",
                        "❌ AIS registration response missing result"
                    );
                    return Err(ActrError::Unavailable(
                        "Invalid AIS registration response: missing result".to_string(),
                    ));
                }
            }
        };

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // 3. Set credential on signaling client, then connect signaling WS
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // The signaling server requires credential params in the WS URL for
        // authentication. We must set actor_id + credential BEFORE connecting
        // so that build_url_with_identity() includes them in the query string.
        {
            let actor_id = register_ok.actr_id.clone();
            let credential_state = CredentialState::new(
                register_ok.credential.clone(),
                register_ok.credential_expires_at,
                Some(register_ok.turn_credential.clone()),
            );
            self.signaling_client.set_actor_id(actor_id).await;
            self.signaling_client
                .set_credential_state(credential_state)
                .await;
        }

        tracing::info!("📡 Connecting to signaling server (with credential)");
        self.signaling_client
            .connect()
            .await
            .map_err(|e| ActrError::Unavailable(format!("Signaling connect failed: {e}")))?;
        tracing::info!("✅ Connected to signaling server");

        // Collect background task handles so they can be managed by ActrRefShared later.
        let mut task_handles = Vec::new();

        {
            let actor_id = register_ok.actr_id;
            let credential = register_ok.credential;

            tracing::info!("🆔 Assigned ActrId: {}", actor_id);
            tracing::info!("🔐 Received credential (key_id: {})", credential.key_id);
            tracing::info!(
                "💓 Signaling heartbeat interval: {} seconds",
                register_ok.signaling_heartbeat_interval_secs
            );

            // TurnCredential is a required field; should always be present under normal registration.
            tracing::debug!("TurnCredential received, TURN authentication ready");

            if let Some(expires_at) = &register_ok.credential_expires_at {
                tracing::debug!("⏰ Credential expires at: {}s", expires_at.seconds);
            }

            // Store ActrId and credential state
            self.actor_id = Some(actor_id.clone());
            let credential_state = CredentialState::new(
                credential,
                register_ok.credential_expires_at,
                Some(register_ok.turn_credential.clone()),
            );
            self.credential_state = Some(credential_state.clone());

            // Note: actor_id and credential_state were already set on signaling_client
            // before connect (step 3 above), so reconnect URLs already carry correct auth.

            // Persist identity into ContextFactory for later Context creation
            // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
            // 1.2. Set actr_lock in ContextFactory for fingerprint lookups
            // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
            if let Some(actr_lock) = self.actr_lock.clone() {
                self.context_factory
                    .as_mut()
                    .expect("ContextFactory must exist")
                    .set_actr_lock(actr_lock);
                tracing::info!(
                    "✅ manifest.lock.toml set in ContextFactory for fingerprint lookups"
                );
            }

            // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
            // 1.3. Store references to both inproc managers (created in ActrNode::build())
            // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
            let shell_to_guest = self
                .context_factory
                .as_ref()
                .expect("ContextFactory must exist")
                .shell_to_workload();
            let guest_to_shell = self
                .context_factory
                .as_ref()
                .expect("ContextFactory must exist")
                .workload_to_shell();
            self.inproc_mgr = Some(shell_to_guest);
            self.guest_to_shell_mgr = Some(guest_to_shell);
            tracing::info!("✅ Inproc infrastructure already ready (created in ActrNode::build())");

            // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
            // 1.5. Create WebRTC infrastructure
            // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
            tracing::info!("🌐 Initializing WebRTC infrastructure");

            // Get MediaFrameRegistry from ContextFactory
            let media_frame_registry = self
                .context_factory
                .as_ref()
                .expect("ContextFactory must exist")
                .media_frame_registry
                .clone();

            // Create WebRtcCoordinator
            let coordinator = Arc::new(crate::wire::webrtc::coordinator::WebRtcCoordinator::new(
                actor_id.clone(),
                credential_state.clone(),
                self.signaling_client.clone(),
                self.config.webrtc.clone(),
                media_frame_registry,
            ));

            // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
            // 1.6. Create PeerTransport + PeerGate (new architecture)
            // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
            tracing::info!("🏗️  Creating PeerTransport with WebRTC support");

            // Create DefaultWireBuilder with WebRTC coordinator
            use crate::transport::{DefaultWireBuilder, DefaultWireBuilderConfig};

            // WebSocket channel always enabled: target ws:// address is fully discovered at runtime
            // Direct-connect mode: encode local node ActrId as hex, sent as X-Actr-Node-Id
            let local_id_hex = hex::encode(actor_id.encode_to_vec());
            let wire_builder_config = DefaultWireBuilderConfig {
                local_id_hex,
                enable_webrtc: true,
                enable_websocket: true,
                // Share the discovered_ws_addresses map so that post-discovery calls
                // can use the signaling-provided ws:// URL for this actor node.
                discovered_ws_addresses: self.discovered_ws_addresses.clone(),
                // Pass credential_state so outbound WS handshake carries X-Actr-Credential,
                // enabling peer WebSocketGate to perform Ed25519 signature verification.
                credential_state: Some(credential_state.clone()),
            };
            let wire_builder = Arc::new(DefaultWireBuilder::new(
                Some(coordinator.clone()),
                wire_builder_config,
            ));

            // Create PeerTransport
            use crate::transport::PeerTransport;
            let transport_manager = Arc::new(PeerTransport::new(actor_id.clone(), wire_builder));

            // Create PeerGate with WebRTC coordinator for MediaTrack support
            use crate::outbound::{Gate, PeerGate};
            let outproc_gate =
                Arc::new(PeerGate::new(transport_manager, Some(coordinator.clone())));
            let outproc_gate_enum = Gate::Peer(outproc_gate.clone());
            tracing::info!("PeerTransport + PeerGate initialized");

            // Get DataStreamRegistry from ContextFactory
            let data_stream_registry = self
                .context_factory
                .as_ref()
                .expect("ContextFactory must exist")
                .data_stream_registry
                .clone();

            // Create WebRtcGate with shared pending_requests and DataStreamRegistry
            let pending_requests = outproc_gate.get_pending_requests();
            let gate = Arc::new(crate::wire::webrtc::gate::WebRtcGate::new(
                coordinator.clone(),
                pending_requests,
                data_stream_registry.clone(),
            ));
            // Set local_id
            gate.set_local_id(actor_id.clone()).await;
            tracing::info!(
                "✅ WebRtcGate created with shared pending_requests and DataStreamRegistry"
            );

            // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
            // 1.7. Set outproc_gate in ContextFactory (completing initialization)
            // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
            tracing::info!("🔧 Setting outproc_gate in ContextFactory");
            self.context_factory
                .as_mut()
                .expect("ContextFactory must exist")
                .set_outproc_gate(outproc_gate_enum);
            tracing::info!("✅ ContextFactory fully initialized (inproc + outproc gates ready)");

            // Save references
            self.webrtc_coordinator = Some(coordinator.clone());
            self.webrtc_gate = Some(gate.clone());
            tracing::info!("✅ WebRTC infrastructure initialized");

            // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
            // 1.7.6. WebSocket Server (direct-connect mode, optional)
            // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
            if let Some(listen_port) = self.config.websocket_listen_port {
                tracing::info!(
                    "🔌 WebSocket direct-connect mode enabled, binding port {}",
                    listen_port
                );
                use crate::key_cache::AisKeyCache;
                use crate::wire::websocket::gate::WsAuthContext;
                use crate::wire::websocket::{WebSocketGate, WebSocketServer};

                // Build AisKeyCache and seed it with the signing key from the registration response
                let ais_key_cache = AisKeyCache::new();
                if !register_ok.signing_pubkey.is_empty() {
                    match ais_key_cache
                        .seed(register_ok.signing_key_id, &register_ok.signing_pubkey)
                        .await
                    {
                        Ok(()) => tracing::info!(
                            key_id = register_ok.signing_key_id,
                            "🔑 AisKeyCache seeded from RegisterOk"
                        ),
                        Err(e) => tracing::warn!(
                            key_id = register_ok.signing_key_id,
                            error = ?e,
                            "AisKeyCache seed failed; WebSocket will reject all inbound connections"
                        ),
                    }
                } else {
                    tracing::warn!(
                        "RegisterOk missing signing_pubkey; WebSocket credential verification will degrade"
                    );
                }

                let auth_ctx = WsAuthContext {
                    ais_key_cache,
                    actor_id: actor_id.clone(),
                    credential_state: credential_state.clone(),
                    signaling_client: self.signaling_client.clone(),
                };

                match WebSocketServer::bind(listen_port).await {
                    Ok((ws_server, conn_rx)) => {
                        ws_server.start(self.shutdown_token.clone());
                        let ws_gate = Arc::new(WebSocketGate::new(
                            conn_rx,
                            outproc_gate.get_pending_requests(),
                            data_stream_registry.clone(),
                            Some(auth_ctx),
                        ));
                        self.websocket_gate = Some(ws_gate);
                        tracing::info!(
                            "✅ WebSocketServer + WebSocketGate initialized (credential auth enabled)"
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            "❌ Failed to bind WebSocket server on port {}: {:?}",
                            listen_port,
                            e
                        );
                    }
                }
            }

            // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
            // 1.7.5. Create shared state for credential management
            // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
            // Shared credential state initialized above; reused across tasks

            // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
            // 1.8. Spawn heartbeat task (periodic Ping to signaling server)
            // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
            {
                let shutdown = self.shutdown_token.clone();
                let client = self.signaling_client.clone();
                let actor_id_for_heartbeat = actor_id.clone();
                let credential_state_for_heartbeat = credential_state.clone();
                let mailbox_for_heartbeat = self.mailbox.clone();
                let register_request_for_heartbeat = register_request.clone();

                // Use interval from registration response, default to 30s
                let heartbeat_interval_secs = register_ok.signaling_heartbeat_interval_secs;
                let heartbeat_interval = if heartbeat_interval_secs > 0 {
                    Duration::from_secs(heartbeat_interval_secs as u64)
                } else {
                    Duration::from_secs(30)
                };
                let ais_endpoint_for_heartbeat = self.config.ais_endpoint.clone();
                let heartbeat_handle = tokio::spawn(crate::lifecycle::heartbeat::heartbeat_task(
                    shutdown,
                    client,
                    actor_id_for_heartbeat,
                    credential_state_for_heartbeat,
                    mailbox_for_heartbeat,
                    heartbeat_interval,
                    register_request_for_heartbeat,
                    ais_endpoint_for_heartbeat,
                ));
                task_handles.push(heartbeat_handle);
            }
            tracing::info!(
                "✅ Heartbeat task started (interval: {}s)",
                register_ok.signaling_heartbeat_interval_secs
            );

            // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
            // 1.8.5. Spawn network event processing loop
            // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
            if let (Some(event_rx), Some(result_tx)) = (
                self.network_event_rx.take(),
                self.network_event_result_tx.take(),
            ) {
                use crate::lifecycle::network_event::DefaultNetworkEventProcessor;

                // Create DefaultNetworkEventProcessor
                // If debounce config exists, use new_with_debounce
                let event_processor =
                    if let Some(config) = self.network_event_debounce_config.clone() {
                        Arc::new(DefaultNetworkEventProcessor::new_with_debounce(
                            self.signaling_client.clone(),
                            self.webrtc_coordinator.clone(),
                            config,
                        ))
                    } else {
                        Arc::new(DefaultNetworkEventProcessor::new(
                            self.signaling_client.clone(),
                            self.webrtc_coordinator.clone(),
                        ))
                    };

                let shutdown = self.shutdown_token.clone();
                let network_event_handle = tokio::spawn(async move {
                    Self::network_event_loop(event_rx, result_tx, event_processor, shutdown).await;
                });
                task_handles.push(network_event_handle);
                tracing::info!("✅ Network event loop started");
            }

            {
                // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
                // 1.9. Spawn dedicated Unregister task (best-effort, with timeout)
                // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
                //
                // This task:
                // - Waits for shutdown_token to be cancelled (e.g., wait_for_ctrl_c_and_shutdown)
                // - Then sends UnregisterRequest via signaling client with a timeout
                //
                // NOTE: we push its JoinHandle into task_handles so it can be aborted
                // by ActrRefShared::Drop if needed.
                let shutdown = self.shutdown_token.clone();
                let client = self.signaling_client.clone();
                let actor_id_for_unreg = actor_id.clone();
                let credential_state_for_unreg = credential_state.clone();
                let webrtc_coordinator = self.webrtc_coordinator.clone();

                let unregister_handle = tokio::spawn(async move {
                    // Wait for shutdown signal
                    shutdown.cancelled().await;
                    tracing::info!(
                        "📡 Shutdown signal received, sending UnregisterRequest for Actor {}",
                        actor_id_for_unreg
                    );

                    // 1. Close all WebRTC peer connections first (if any)
                    if let Some(coord) = webrtc_coordinator {
                        if let Err(e) = coord.close_all_peers().await {
                            tracing::warn!(
                                "⚠️ Failed to close all WebRTC peers before UnregisterRequest: {}",
                                e
                            );
                        } else {
                            tracing::info!("✅ All WebRTC peers closed before UnregisterRequest");
                        }
                    } else {
                        tracing::debug!(
                            "WebRTC coordinator not found before UnregisterRequest (no WebRTC?)"
                        );
                    }

                    // 2. Then send UnregisterRequest with a timeout (e.g. 5 seconds)
                    let result = tokio::time::timeout(
                        Duration::from_secs(5),
                        client.send_unregister_request(
                            actor_id_for_unreg.clone(),
                            credential_state_for_unreg.credential().await,
                            Some("Graceful shutdown".to_string()),
                        ),
                    )
                    .await;
                    tracing::info!("UnregisterRequest result: {:?}", result);
                    match result {
                        Ok(Ok(_)) => {
                            tracing::info!(
                                "✅ UnregisterRequest sent to signaling server for Actor {}",
                                actor_id_for_unreg
                            );
                        }
                        Ok(Err(e)) => {
                            tracing::warn!(
                                "⚠️ Failed to send UnregisterRequest for Actor {}: {}",
                                actor_id_for_unreg,
                                e
                            );
                        }
                        Err(_) => {
                            tracing::warn!(
                                "⚠️ UnregisterRequest timeout (5s) for Actor {}",
                                actor_id_for_unreg
                            );
                        }
                    }
                });

                task_handles.push(unregister_handle);
            }
        } // end registration setup block

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // 2. Transport layer initialization (completed via WebRTC infrastructure)
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        tracing::info!("✅ Transport layer initialized via WebRTC infrastructure");

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // 3.1 Convert to Arc (before starting background loops)
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // Clone actor_id before moving self into Arc
        let actor_id = self
            .actor_id
            .as_ref()
            .ok_or_else(|| ActrError::Internal("Actor ID not set".to_string()))?
            .clone();
        let context_factory = self
            .context_factory
            .clone()
            .expect("ContextFactory must be initialized in start()");
        let credential_state = self
            .credential_state
            .clone()
            .expect("CredentialState must be initialized in start()");
        let shutdown_token = self.shutdown_token.clone();
        let node_ref = Arc::new(self);

        {
            let startup_ctx =
                context_factory.create_bootstrap(&actor_id, &credential_state.credential().await);
            let workload = node_ref.workload.lock().await;
            workload.on_start(&startup_ctx).await?;
        }

        {
            let node = node_ref.clone();
            let actor_id = actor_id.clone();
            let credential_state = credential_state.clone();
            let shutdown = shutdown_token.clone();
            let on_stop_handle = tokio::spawn(async move {
                shutdown.cancelled().await;
                let stop_ctx = node
                    .context_factory
                    .as_ref()
                    .expect("ContextFactory must be initialized in start()")
                    .create_bootstrap(&actor_id, &credential_state.credential().await);
                let workload = node.workload.lock().await;
                if let Err(err) = workload.on_stop(&stop_ctx).await {
                    tracing::warn!("workload on_stop hook failed: {err:?}");
                }
            });
            task_handles.push(on_stop_handle);
        }

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // 3.5. Start WebRTC background loops
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        tracing::info!("🚀 Starting WebRTC background loops");

        // Start WebRtcCoordinator signaling loop
        if let Some(coordinator) = &node_ref.webrtc_coordinator {
            coordinator.clone().start().await.map_err(|e| {
                ActrError::Unavailable(format!("WebRtcCoordinator start failed: {e}"))
            })?;
            tracing::info!("✅ WebRtcCoordinator signaling loop started");
        }

        // Start WebRtcGate message receive loop (route to Mailbox)
        if let Some(gate) = &node_ref.webrtc_gate {
            gate.start_receive_loop(node_ref.mailbox.clone())
                .await
                .map_err(|e| {
                    ActrError::Unavailable(format!("WebRtcGate receive loop start failed: {e}"))
                })?;
            tracing::info!("✅ WebRtcGate → Mailbox routing started");
        }

        // Start WebSocketGate message receive loop (route to Mailbox, direct-connect mode)
        if let Some(ws_gate) = &node_ref.websocket_gate {
            ws_gate
                .start_receive_loop(node_ref.mailbox.clone())
                .await
                .map_err(|e| {
                    ActrError::Unavailable(format!("WebSocketGate receive loop start failed: {e}"))
                })?;
            tracing::info!("✅ WebSocketGate → Mailbox routing started");
        }
        tracing::info!("✅ WebRTC background loops started");

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // 4.6. Start Inproc receive loop (Shell → Guest)
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        if let Some(shell_to_guest) = &node_ref.inproc_mgr {
            tracing::info!("🔄 Starting Inproc receive loop (Shell → Guest)");
            // Start Guest receive loop (Shell → Guest REQUEST)
            if let Some(guest_to_shell) = &node_ref.guest_to_shell_mgr {
                let node = node_ref.clone();
                let request_rx_lane = shell_to_guest
                    .get_lane(PayloadType::RpcReliable, None)
                    .await
                    .map_err(|e| {
                        ActrError::Unavailable(format!("Failed to get guest receive lane: {e}"))
                    })?;
                let response_tx = guest_to_shell.clone();
                let shutdown = shutdown_token.clone();

                let inproc_handle = tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = shutdown.cancelled() => {
                                tracing::info!("📭 Guest receive loop (Shell → Guest) received shutdown signal");
                                break;
                            }
                            envelope_result = request_rx_lane.recv_envelope() => {
                                match envelope_result {
                                    Ok(envelope) => {
                                        let request_id = envelope.request_id.clone();
                                        tracing::debug!("📨 Guest received REQUEST from Shell: request_id={}", request_id);
                                        // Extract and set tracing context from envelope
                                        #[cfg(feature = "opentelemetry")]
                                        let span = {
                                            let actr_id_str = node.actor_id.as_ref().map(|id| id.to_string()).unwrap_or_default();
                                            let span = tracing::info_span!("ActrNode.lane_receive", actr_id = %actr_id_str, request_id = %request_id);
                                            set_parent_from_rpc_envelope(&span, &envelope);
                                            span
                                        };

                                        // Shell calls have no caller_id (local process communication)
                                        let handle_incoming_fut = node.handle_incoming(envelope.clone(), None);
                                        #[cfg(feature = "opentelemetry")]
                                        let handle_incoming_fut = handle_incoming_fut.instrument(span.clone());

                                        match handle_incoming_fut.await {
                                            Ok(response_bytes) => {
                                                // Send RESPONSE back via guest_to_shell
                                                // Keep same route_key (no prefix needed - separate channels!)
                                                #[cfg_attr(not(feature = "opentelemetry"), allow(unused_mut))]
                                                let mut response_envelope = RpcEnvelope {
                                                    route_key: envelope.route_key.clone(),
                                                    payload: Some(response_bytes),
                                                    error: None,
                                                    traceparent: None,
                                                    tracestate: None,
                                                    request_id: request_id.clone(),
                                                    metadata: Vec::new(),
                                                    timeout_ms: 30000,
                                                };
                                                // Inject tracing context
                                                #[cfg(feature = "opentelemetry")]
                                                inject_span_context_to_rpc(&span, &mut response_envelope);

                                                // Send via Guest → Shell channel
                                                let send_response_fut = response_tx.send_message(PayloadType::RpcReliable, None, response_envelope);
                                                #[cfg(feature = "opentelemetry")]
                                                let send_response_fut = send_response_fut.instrument(span.clone());
                                                if let Err(e) = send_response_fut.await {
                                                    tracing::error!(
                                                        severity = 7,
                                                        error_category = "transport_error",
                                                        request_id = %request_id,
                                                        "❌ Failed to send RESPONSE to Shell: {:?}",
                                                        e
                                                    );
                                                }
                                            }
                                            Err(e) => {
                                                tracing::error!(
                                                    severity = 6,
                                                    error_category = "handler_error",
                                                    request_id = %request_id,
                                                    route_key = %envelope.route_key,
                                                    "❌ Guest message handling failed: {:?}",
                                                    e
                                                );

                                                // Send error response (system-level error on envelope)
                                                let error_response = actr_protocol::ErrorResponse {
                                                    code: protocol_error_to_code(&e),
                                                    message: e.to_string(),
                                                };
                                                #[cfg_attr(not(feature = "opentelemetry"), allow(unused_mut))]
                                                let mut error_envelope = RpcEnvelope {
                                                    route_key: envelope.route_key.clone(),
                                                    payload: None,
                                                    error: Some(error_response),
                                                    traceparent: envelope.traceparent.clone(),
                                                    tracestate: envelope.tracestate.clone(),
                                                    request_id: request_id.clone(),
                                                    metadata: Vec::new(),
                                                    timeout_ms: 30000,
                                                };
                                                // Inject tracing context
                                                #[cfg(feature = "opentelemetry")]
                                                inject_span_context_to_rpc(&span, &mut error_envelope);

                                                let send_error_response_fut = response_tx.send_message(PayloadType::RpcReliable, None, error_envelope);
                                                #[cfg(feature = "opentelemetry")]
                                                let send_error_response_fut = send_error_response_fut.instrument(span);
                                                if let Err(send_err) = send_error_response_fut.await {
                                                    tracing::error!(
                                                        severity = 7,
                                                        error_category = "transport_error",
                                                        request_id = %request_id,
                                                        "❌ Failed to send ERROR response to Shell: {:?}",
                                                        send_err
                                                    );
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::error!(
                                            severity = 8,
                                            error_category = "transport_error",
                                            "❌ Failed to receive from Shell → Guest lane: {:?}",
                                            e
                                        );
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    tracing::info!("✅ Guest receive loop (Shell → Guest) terminated gracefully");
                });
                task_handles.push(inproc_handle);
            }
        }
        tracing::info!("✅ Guest receive loop (Shell → Guest REQUEST) started");

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // 4.7. Start Shell receive loop (Guest → Shell RESPONSE)
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        tracing::info!("🔄 Starting Shell receive loop (Guest → Shell RESPONSE)");
        if let Some(guest_to_shell) = &node_ref.guest_to_shell_mgr {
            // Start Shell receive loop (Guest → Shell RESPONSE)
            if let Some(shell_to_guest) = &node_ref.inproc_mgr {
                let response_rx_lane = guest_to_shell
                    .get_lane(PayloadType::RpcReliable, None)
                    .await
                    .map_err(|e| {
                        ActrError::Unavailable(format!("Failed to get shell receive lane: {e}"))
                    })?;
                let request_mgr = shell_to_guest.clone();
                let shutdown = shutdown_token.clone();

                let shell_receive_handle = tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = shutdown.cancelled() => {
                                tracing::info!("📭 Shell receive loop (Guest → Shell) received shutdown signal");
                                break;
                            }
                            envelope_result = response_rx_lane.recv_envelope() => {
                                match envelope_result {
                                    Ok(envelope) => {
                                        tracing::debug!(
                                            "📨 Shell received RESPONSE from Guest: request_id={}",
                                            envelope.request_id
                                        );

                                        // Check if response is success or error
                                        match (envelope.payload, envelope.error) {
                                            (Some(payload), None) => {
                                                // Success response
                                                if let Err(e) = request_mgr
                                                    .complete_response(&envelope.request_id, payload)
                                                    .await
                                                {
                                                    tracing::warn!(
                                                        severity = 4,
                                                        error_category = "orphan_response",
                                                        request_id = %envelope.request_id,
                                                        "⚠️  No pending request found for response: {:?}",
                                                        e
                                                    );
                                                }
                                            }
                                            (None, Some(error)) => {
                                                // Error response - convert to ActrError and complete with error
                                                let actr_err = ActrError::Unavailable(format!("RPC error {}: {}", error.code, error.message));
                                                if let Err(e) = request_mgr
                                                    .complete_error(&envelope.request_id, actr_err)
                                                    .await
                                                {
                                                    tracing::warn!(
                                                        severity = 4,
                                                        error_category = "orphan_response",
                                                        request_id = %envelope.request_id,
                                                        "⚠️  No pending request found for error response: {:?}",
                                                        e
                                                    );
                                                }
                                            }
                                            _ => {
                                                tracing::error!(
                                                    severity = 7,
                                                    error_category = "protocol_error",
                                                    request_id = %envelope.request_id,
                                                    "❌ Invalid RpcEnvelope: both payload and error are present or both absent"
                                                );
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::error!(
                                            severity = 8,
                                            error_category = "transport_error",
                                            "❌ Failed to receive from Guest → Shell lane: {:?}",
                                            e
                                        );
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    tracing::info!("✅ Shell receive loop (Guest → Shell) terminated gracefully");
                });
                task_handles.push(shell_receive_handle);
            }
        }
        tracing::info!("✅ Shell receive loop (Guest → Shell RESPONSE) started");

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // 5. Start Mailbox processing loop (State Path)
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        tracing::info!("🔄 Starting Mailbox processing loop (State Path)");
        {
            let node = node_ref.clone();
            let mailbox = node_ref.mailbox.clone();
            let gate = node_ref.webrtc_gate.clone();
            let shutdown = shutdown_token.clone();

            let mailbox_handle = tokio::spawn(async move {
                loop {
                    tokio::select! {
                        // Listen for shutdown signal
                        _ = shutdown.cancelled() => {
                            tracing::info!("📭 Mailbox loop received shutdown signal");
                            break;
                        }
                        // Dequeue messages (by priority)
                        result = mailbox.dequeue() => {
                            match result {
                                Ok(messages) => {
                                    if messages.is_empty() {
                                        // Queue empty, sleep briefly
                                        tokio::time::sleep(Duration::from_millis(10)).await;
                                        continue;
                                    }
                                    tracing::debug!("📬 Mailbox dequeue: {} messages", messages.len());

                                    // Process messages one by one
                                    for msg_record in messages {
                                        // Deserialize RpcEnvelope (Protobuf)
                                        match RpcEnvelope::decode(&msg_record.payload[..]) {
                                            Ok(envelope) => {
                                                let request_id = envelope.request_id.clone();
                                                let queue_latency_ms = (chrono::Utc::now() - msg_record.created_at).num_milliseconds();
                                                tracing::info!(request_id = %request_id, queue_latency_ms = queue_latency_ms, "rpc.mailbox.dequeued");

                                                tracing::debug!("📦 Processing message: request_id={}", request_id);
                                                #[cfg(feature = "opentelemetry")]
                                                let span = {
                                                    let actr_id_str = node.actor_id.as_ref().map(|id| id.to_string()).unwrap_or_default();
                                                    let span = tracing::info_span!("ActrNode.mailbox_receive", actr_id = %actr_id_str, request_id = %request_id, queue_wait_ms = queue_latency_ms);
                                                    set_parent_from_rpc_envelope(&span, &envelope);
                                                    span
                                                };

                                                // Decode caller_id from MessageRecord.from (transport layer)
                                                let caller_id_result = ActrId::decode(&msg_record.from[..]);
                                                let caller_id_ref = caller_id_result.as_ref().ok();

                                                if caller_id_ref.is_none() {
                                                    tracing::warn!(
                                                        request_id = %request_id,
                                                        "⚠️  Failed to decode caller_id from MessageRecord.from"
                                                    );
                                                }

                                                // Call handle_incoming with caller_id from transport layer
                                                let handle_incoming_fut = node.handle_incoming(envelope.clone(), caller_id_ref);
                                                #[cfg(feature = "opentelemetry")]
                                                let handle_incoming_fut = handle_incoming_fut.instrument(span.clone());

                                                match handle_incoming_fut.await {
                                                    Ok(response_bytes) => {
                                                        // Send response (reuse request_id)
                                                        if let Some(ref gate) = gate {
                                                            // Use already decoded caller_id
                                                            match caller_id_result {
                                                                Ok(caller) => {
                                                                    // Construct response RpcEnvelope (reuse request_id!)
                                                                    #[cfg_attr(not(feature = "opentelemetry"), allow(unused_mut))]
                                                                    let mut response_envelope = RpcEnvelope {
                                                                        request_id, // Reuse!
                                                                        route_key: envelope.route_key.clone(),
                                                                        payload: Some(response_bytes),
                                                                        error: None,
                                                                        traceparent: envelope.traceparent.clone(),
                                                                        tracestate: envelope.tracestate.clone(),
                                                                        metadata: Vec::new(), // Response doesn't need extra metadata
                                                                        timeout_ms: 30000,
                                                                    };
                                                                    // Inject tracing context
                                                                    #[cfg(feature = "opentelemetry")]
                                                                    inject_span_context_to_rpc(&span, &mut response_envelope);

                                                                    let send_response_fut = gate.send_response(&caller, response_envelope);
                                                                    #[cfg(feature = "opentelemetry")]
                                                                    let send_response_fut = send_response_fut.instrument(span);
                                                                    if let Err(e) = send_response_fut.await {
                                                                        tracing::error!(
                                                                            severity = 7,
                                                                            error_category = "transport_error",
                                                                            request_id = %envelope.request_id,
                                                                            "❌ Failed to send response: {:?}",
                                                                            e
                                                                        );
                                                                    }
                                                                }
                                                                Err(e) => {
                                                                    tracing::error!(
                                                                        severity = 8,
                                                                        error_category = "protobuf_decode",
                                                                        request_id = %envelope.request_id,
                                                                        "❌ Failed to decode caller_id: {:?}",
                                                                        e
                                                                    );
                                                                }
                                                            }
                                                        }

                                                        // ACK message
                                                        if let Err(e) = mailbox.ack(msg_record.id).await {
                                                            tracing::error!(
                                                                severity = 9,
                                                                error_category = "mailbox_error",
                                                                request_id = %envelope.request_id,
                                                                message_id = %msg_record.id,
                                                                "❌ Mailbox ACK failed: {:?}",
                                                                e
                                                            );
                                                        }
                                                    }
                                                    Err(e) => {
                                                        tracing::error!(
                                                            severity = 6,
                                                            error_category = "handler_error",
                                                            request_id = %envelope.request_id,
                                                            route_key = %envelope.route_key,
                                                            "❌ handle_incoming failed: {:?}", e
                                                        );
                                                        // ACK to avoid infinite retries
                                                        // Application errors are caller's responsibility
                                                        let _ = mailbox.ack(msg_record.id).await;
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                // Poison message - cannot decode RpcEnvelope
                                                tracing::error!(
                                                    severity = 9,
                                                    error_category = "protobuf_decode",
                                                    message_id = %msg_record.id,
                                                    "❌ Poison message: Failed to deserialize RpcEnvelope: {:?}",
                                                    e
                                                );

                                                // Write to Dead Letter Queue
                                                use actr_runtime_mailbox::DlqRecord;
                                                use chrono::Utc;
                                                use uuid::Uuid;

                                                let dlq_record = DlqRecord {
                                                    id: Uuid::new_v4(),
                                                    original_message_id: Some(msg_record.id.to_string()),
                                                    from: Some(msg_record.from.clone()),
                                                    to: node.actor_id.as_ref().map(|id| {
                                                        let mut buf = Vec::new();
                                                        id.encode(&mut buf).unwrap();
                                                        buf
                                                    }),
                                                    raw_bytes: msg_record.payload.clone(),
                                                    error_message: format!("Protobuf decode failed: {e}"),
                                                    error_category: "protobuf_decode".to_string(),
                                                    trace_id: format!("mailbox-{}", msg_record.id),
                                                    request_id: None,
                                                    created_at: Utc::now(),
                                                    redrive_attempts: 0,
                                                    last_redrive_at: None,
                                                    context: Some(format!(
                                                        r#"{{"source":"mailbox","priority":"{}"}}"#,
                                                        match msg_record.priority {
                                                            actr_runtime_mailbox::MessagePriority::High => "high",
                                                            actr_runtime_mailbox::MessagePriority::Normal => "normal",
                                                        }
                                                    )),
                                                };

                                                if let Err(dlq_err) = node.dlq.enqueue(dlq_record).await {
                                                    tracing::error!(
                                                        severity = 10,
                                                        "❌ CRITICAL: Failed to write poison message to DLQ: {:?}",
                                                        dlq_err
                                                    );
                                                } else {
                                                    tracing::warn!(
                                                        severity = 9,
                                                        "☠️ Poison message moved to DLQ: message_id={}",
                                                        msg_record.id
                                                    );
                                                }

                                                // ACK the poison message to remove from mailbox
                                                let _ = mailbox.ack(msg_record.id).await;
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::error!(
                                        severity = 9,
                                        error_category = "mailbox_error",
                                        "❌ Mailbox dequeue failed: {:?}", e
                                    );
                                    tokio::time::sleep(Duration::from_secs(1)).await;
                                }
                            }
                        }
                    }
                }
                tracing::info!("✅ Mailbox processing loop terminated gracefully");
            });

            task_handles.push(mailbox_handle);
        }
        tracing::info!("✅ Mailbox processing loop started");
        tracing::info!("✅ ActrNode started successfully");

        // Create ActrRefShared
        let shared = Arc::new(ActrRefShared {
            actor_id,
            context_factory,
            credential_state,
            shutdown_token,
            task_handles: Mutex::new(task_handles),
        });

        // Create ActrRef
        tracing::info!("✅ ActrRef created (Shell → Guest communication handle)");

        Ok(ActrRef { shared })
    }
}
