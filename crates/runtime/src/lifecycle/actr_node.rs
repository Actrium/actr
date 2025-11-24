//! ActrNode - ActrSystem + Workload (1:1 composition)

use actr_framework::{Bytes, Workload};
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{
    ActorResult, ActrId, ActrType, PayloadType, RouteCandidatesRequest, RpcEnvelope,
    route_candidates_request,
};
use futures_util::FutureExt;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::context_factory::ContextFactory;
use crate::transport::InprocTransportManager;

// Use types from sub-crates
use crate::wire::webrtc::{SignalingClient, WebRtcConfig};
use actr_mailbox::{DeadLetterQueue, Mailbox};
use actr_protocol::{AIdCredential, RegisterRequest, register_response};

// Use extension traits from actr-protocol
use actr_protocol::{ActrIdExt, ActrTypeExt};

/// ActrNode - ActrSystem + Workload (1:1 composition)
///
/// # Generic Parameters
/// - `W`: Workload type
///
/// # MessageDispatcher Association
/// - Statically associated via W::Dispatcher
/// - Does not store Dispatcher instance (not even ZST needed)
/// - Dispatch calls entirely through type system
pub struct ActrNode<W: Workload> {
    /// Runtime configuration
    pub(crate) config: actr_config::Config,

    /// Workload instance (the only business logic)
    pub(crate) workload: Arc<W>,

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
    pub(crate) credential: Option<AIdCredential>,

    /// WebRTC coordinator (created after startup)
    pub(crate) webrtc_coordinator: Option<Arc<crate::wire::webrtc::coordinator::WebRtcCoordinator>>,

    /// WebRTC Gate (created after startup)
    pub(crate) webrtc_gate: Option<Arc<crate::wire::webrtc::gate::WebRtcGate>>,

    /// Shell → Workload Transport Manager
    ///
    /// Workload receives REQUEST from Shell (zero serialization, direct RpcEnvelope passing)
    pub(crate) inproc_mgr: Option<Arc<InprocTransportManager>>,

    /// Workload → Shell Transport Manager
    ///
    /// Workload sends RESPONSE to Shell (separate pending_requests from Shell's)
    pub(crate) workload_to_shell_mgr: Option<Arc<InprocTransportManager>>,

    /// Shutdown token for graceful shutdown
    pub(crate) shutdown_token: CancellationToken,
}

/// Map ProtocolError to error code for ErrorResponse
fn protocol_error_to_code(err: &actr_protocol::ProtocolError) -> u32 {
    use actr_protocol::ProtocolError;
    match err {
        ProtocolError::Actr(_) => 400, // Bad Request - identity/decode error
        ProtocolError::Uri(_) => 400,  // Bad Request - URI parsing error
        ProtocolError::Name(_) => 400, // Bad Request - invalid name
        ProtocolError::SerializationError(_) => 500, // Internal Server Error
        ProtocolError::DeserializationError(_) => 400, // Bad Request - invalid payload
        ProtocolError::DecodeError(_) => 400, // Bad Request - decode failure
        ProtocolError::EncodeError(_) => 500, // Internal Server Error
        ProtocolError::UnknownRoute(_) => 404, // Not Found - route not found
        ProtocolError::TransportError(_) => 503, // Service Unavailable
        ProtocolError::Timeout => 504, // Gateway Timeout
        ProtocolError::TargetNotFound(_) => 404, // Not Found
        ProtocolError::TargetUnavailable(_) => 503, // Service Unavailable
        ProtocolError::InvalidStateTransition(_) => 500, // Internal Server Error
    }
}

impl<W: Workload> ActrNode<W> {
    /// Get Inproc Transport Manager
    ///
    /// # Returns
    /// - `Some(Arc<InprocTransportManager>)`: Initialized manager
    /// - `None`: Not yet started (need to call start() first)
    ///
    /// # Use Cases
    /// - Workload internals need to communicate with Shell
    /// - Create custom LatencyFirst/MediaTrack channels
    pub fn inproc_mgr(&self) -> Option<Arc<InprocTransportManager>> {
        self.inproc_mgr.clone()
    }

    /// Get ActorId (if registration has completed)
    pub fn actor_id(&self) -> Option<&ActrId> {
        self.actor_id.as_ref()
    }

    /// Get credential (if registration has completed)
    pub fn credential(&self) -> Option<&AIdCredential> {
        self.credential.as_ref()
    }

    /// Get signaling client (for manual control such as UnregisterRequest)
    pub fn signaling_client(&self) -> Arc<dyn SignalingClient> {
        self.signaling_client.clone()
    }

    /// Get shutdown token for this node
    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown_token.clone()
    }

    /// Discover remote actors of the specified type via signaling server.
    ///
    /// This requests best route candidates via `RouteCandidatesRequest` using the node's existing registration.
    ///
    /// The method returns the ordered list of candidate `ActrId`s reported by the signaling server.
    ///
    /// # Errors
    /// - Returns `InvalidStateTransition` if the node is not started (no actor_id/credential).
    ///   The node must be started via `start()` before calling this method.
    /// - Returns `TransportError` if the signaling client is not connected.
    pub async fn discover_route_candidates(
        &self,
        target_type: &ActrType,
        candidate_count: u32,
    ) -> ActorResult<Vec<ActrId>> {
        // Check if node is started (has actor_id and credential)
        let actor_id = self.actor_id.as_ref().ok_or_else(|| {
            actr_protocol::ProtocolError::InvalidStateTransition(
                "Node is not started. Call start() first.".to_string(),
            )
        })?;

        let credential = self.credential.as_ref().ok_or_else(|| {
            actr_protocol::ProtocolError::InvalidStateTransition(
                "Node is not started. Call start() first.".to_string(),
            )
        })?;

        // Check if the signaling client is connected
        if !self.signaling_client.is_connected() {
            return Err(actr_protocol::ProtocolError::TransportError(
                "Signaling client is not connected.".to_string(),
            ));
        }

        let client = self.signaling_client.as_ref();

        let criteria = route_candidates_request::NodeSelectionCriteria {
            candidate_count,
            ranking_factors: Vec::new(),
            minimal_dependency_requirement: None,
            minimal_health_requirement: None,
        };

        let route_request = RouteCandidatesRequest {
            target_type: target_type.clone(),
            criteria: Some(criteria),
            client_location: None,
        };

        let route_response = client
            .send_route_candidates_request(actor_id.clone(), credential.clone(), route_request)
            .await
            .map_err(|e| {
                actr_protocol::ProtocolError::TransportError(format!(
                    "Route candidates request failed: {e}"
                ))
            })?;

        match route_response.result {
            Some(actr_protocol::route_candidates_response::Result::Success(success)) => {
                Ok(success.candidates)
            }
            Some(actr_protocol::route_candidates_response::Result::Error(err)) => {
                Err(actr_protocol::ProtocolError::TransportError(format!(
                    "Route candidates error {}: {}",
                    err.code, err.message
                )))
            }
            None => Err(actr_protocol::ProtocolError::TransportError(
                "Invalid route candidates response: missing result".to_string(),
            )),
        }
    }

    /// Handle incoming message envelope
    ///
    /// # Performance Analysis
    /// 1. create_context: ~10ns
    /// 2. W::Dispatcher::dispatch: ~5-10ns (static match, can be inlined)
    /// 3. User business logic: variable
    ///
    /// Framework overhead: ~15-20ns (compared to 50-100ns in traditional approaches)
    ///
    /// # Zero-cost Abstraction
    /// - Compiler can inline entire call chain
    /// - Match branches can be directly expanded
    /// - Final generated code approaches hand-written match expression
    ///
    /// # Parameters
    /// - `envelope`: The RPC envelope containing the message
    /// - `caller_id`: The ActrId of the caller (from transport layer, None for local Shell calls)
    ///
    /// # caller_id Design
    ///
    /// **Why not in RpcEnvelope?**
    /// - Transport layer (WebRTC/Mailbox) already knows the sender
    /// - All connections are direct P2P (no intermediaries)
    /// - Storing in envelope would be redundant duplication
    ///
    /// **How it works:**
    /// - WebRTC/Mailbox stores sender in `MessageRecord.from` (Protobuf bytes)
    /// - Only decoded when creating Context (once per message)
    /// - Shell calls pass `None` (local process, no remote caller)
    /// - Remote calls decode from `MessageRecord.from`
    ///
    /// **trace_id vs request_id:**
    /// - `trace_id`: Distributed tracing across entire call chain (A → B → C)
    /// - `request_id`: Unique identifier for each request-response pair
    /// - Both kept for flexibility in complex scenarios
    /// - Single-hop calls: effectively identical
    /// - Multi-hop calls: trace_id spans all hops, request_id per hop
    pub async fn handle_incoming(
        &self,
        envelope: RpcEnvelope,
        caller_id: Option<&ActrId>,
    ) -> ActorResult<Bytes> {
        use actr_framework::MessageDispatcher;

        // Log received message
        if let Some(caller) = caller_id {
            tracing::debug!(
                "📨 Handling incoming message: route_key={}, caller={}, trace_id={}, request_id={}",
                envelope.route_key,
                caller.to_string_repr(),
                envelope.trace_id,
                envelope.request_id
            );
        } else {
            tracing::debug!(
                "📨 Handling incoming message: route_key={}, trace_id={}, request_id={}",
                envelope.route_key,
                envelope.trace_id,
                envelope.request_id
            );
        }

        // 1. Create Context with caller_id from transport layer
        let actor_id = self.actor_id.as_ref().ok_or_else(|| {
            actr_protocol::ProtocolError::InvalidStateTransition(
                "Actor ID not set - node must be started before handling messages".to_string(),
            )
        })?;
        let credential = self.credential.as_ref().ok_or_else(|| {
            actr_protocol::ProtocolError::InvalidStateTransition(
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
                &envelope.trace_id,
                &envelope.request_id,
                credential,
            );

        // 2. Static MessageRouter dispatch (zero-cost abstraction)
        // Compiler will inline entire call chain, generating code close to hand-written match
        //
        // Wrap dispatch in panic catching to prevent handler panics from crashing the runtime
        let result = std::panic::AssertUnwindSafe(W::Dispatcher::dispatch(
            &self.workload,
            envelope.clone(),
            &ctx,
        ))
        .catch_unwind()
        .await;

        let result = match result {
            Ok(handler_result) => handler_result,
            Err(panic_payload) => {
                // Handler panicked - extract panic info
                let panic_info = if let Some(s) = panic_payload.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "Unknown panic payload".to_string()
                };

                tracing::error!(
                    severity = 8,
                    error_category = "handler_panic",
                    route_key = envelope.route_key,
                    trace_id = envelope.trace_id,
                    "❌ Handler panicked: {}",
                    panic_info
                );

                // Return DecodeFailure error with panic info
                // (using DecodeFailure as a proxy for "cannot process message")
                Err(actr_protocol::ProtocolError::Actr(
                    actr_protocol::ActrError::DecodeFailure {
                        message: format!("Handler panicked: {panic_info}"),
                    },
                ))
            }
        };

        // 3. Log result
        match &result {
            Ok(_) => tracing::debug!(
                trace_id = %envelope.trace_id,
                request_id = %envelope.request_id,
                route_key = %envelope.route_key,
                "✅ Message handled successfully"
            ),
            Err(e) => tracing::error!(
                severity = 6,
                error_category = "handler_error",
                trace_id = %envelope.trace_id,
                request_id = %envelope.request_id,
                route_key = %envelope.route_key,
                "❌ Message handling failed: {:?}", e
            ),
        }

        result
    }

    /// Start the system
    ///
    /// # Startup Sequence
    /// 1. Connect to signaling server and register Actor
    /// 2. Initialize transport layer (WebRTC)
    /// 3. Call lifecycle hook on_start (if Lifecycle trait is implemented)
    /// 4. Start Mailbox processing loop (State Path serial processing)
    /// 5. Start Transport (begin receiving messages)
    /// 6. Create ActrRef for Shell to interact with Workload
    ///
    /// # Returns
    /// - `ActrRef<W>`: Lightweight reference for Shell to call Workload methods
    pub async fn start(mut self) -> ActorResult<crate::actr_ref::ActrRef<W>> {
        tracing::info!("🚀 Starting ActrNode");

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // 1. Connect to signaling server and register
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        tracing::info!("📡 Connecting to signaling server");
        self.signaling_client.connect().await.map_err(|e| {
            actr_protocol::ProtocolError::TransportError(format!("Signaling connect failed: {e}"))
        })?;
        tracing::info!("✅ Connected to signaling server");

        // Get ActrType
        let actr_type = self.workload.actor_type();
        tracing::info!("📋 Actor type: {}", actr_type.to_string_repr());

        // Calculate ServiceSpec from config exports
        let service_spec = self.config.calculate_service_spec();
        if let Some(ref spec) = service_spec {
            tracing::info!("📦 Service fingerprint: {}", spec.fingerprint);
            tracing::info!("📦 Service tags: {:?}", spec.tags);
        } else {
            tracing::info!("📦 No proto exports, ServiceSpec is None");
        }

        // Construct protobuf RegisterRequest
        let register_request = RegisterRequest {
            actr_type: actr_type.clone(),
            realm: self.config.realm,
            service_spec,
            acl: self.config.acl.clone(),
        };

        tracing::info!("📤 Registering actor with signaling server (protobuf)");

        // Use send_register_request to send and wait for response
        let register_response = self
            .signaling_client
            .send_register_request(register_request)
            .await
            .map_err(|e| {
                actr_protocol::ProtocolError::TransportError(format!(
                    "Actor registration failed: {e}"
                ))
            })?;

        // Handle RegisterResponse oneof result
        //
        // Collect background task handles (including unregister task) so they can be managed
        // by ActrRefShared later.
        let mut task_handles = Vec::new();

        match register_response.result {
            Some(register_response::Result::Success(register_ok)) => {
                let actor_id = register_ok.actr_id;
                let credential = register_ok.credential;

                tracing::info!("✅ Registration successful");
                tracing::info!(
                    "🆔 Assigned ActrId: {}",
                    actr_protocol::ActrIdExt::to_string_repr(&actor_id)
                );
                tracing::info!(
                    "🔐 Received credential (token_key_id: {})",
                    credential.token_key_id
                );
                tracing::info!(
                    "💓 Signaling heartbeat interval: {} seconds",
                    register_ok.signaling_heartbeat_interval_secs
                );

                // Log additional information (if available)
                if register_ok.psk.is_some() {
                    tracing::debug!("🔑 Received PSK (bootstrap keying material)");
                }
                if let Some(expires_at) = &register_ok.credential_expires_at {
                    tracing::debug!("⏰ Credential expires at: {}s", expires_at.seconds);
                }

                // Store ActrId and Credential
                self.actor_id = Some(actor_id.clone());
                self.credential = Some(credential.clone());

                // Persist identity into ContextFactory for later Context creation
                // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
                // 1.3. Store references to both inproc managers (already created in ActrSystem::new())
                // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
                let shell_to_workload = self
                    .context_factory
                    .as_ref()
                    .expect("ContextFactory must exist")
                    .shell_to_workload();
                let workload_to_shell = self
                    .context_factory
                    .as_ref()
                    .expect("ContextFactory must exist")
                    .workload_to_shell();
                self.inproc_mgr = Some(shell_to_workload); // Workload receives from this
                self.workload_to_shell_mgr = Some(workload_to_shell); // Workload sends to this

                tracing::info!(
                    "✅ Inproc infrastructure already ready (created in ActrSystem::new())"
                );

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
                let coordinator =
                    Arc::new(crate::wire::webrtc::coordinator::WebRtcCoordinator::new(
                        actor_id.clone(),
                        credential.clone(),
                        self.signaling_client.clone(),
                        WebRtcConfig::default(),
                        media_frame_registry,
                    ));

                // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
                // 1.6. Create OutprocTransportManager + OutprocOutGate (新架构)
                // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
                tracing::info!("🏗️  Creating OutprocTransportManager with WebRTC support");

                // Create DefaultWireBuilder with WebRTC coordinator
                use crate::transport::{DefaultWireBuilder, DefaultWireBuilderConfig};
                let wire_builder_config = DefaultWireBuilderConfig {
                    websocket_url_template: None, // WebSocket disabled for now
                    enable_webrtc: true,
                    enable_websocket: false,
                };
                let wire_builder = Arc::new(DefaultWireBuilder::new(
                    Some(coordinator.clone()),
                    wire_builder_config,
                ));

                // Create OutprocTransportManager
                use crate::transport::OutprocTransportManager;
                let transport_manager =
                    Arc::new(OutprocTransportManager::new(actor_id.clone(), wire_builder));

                // Create OutprocOutGate with WebRTC coordinator for MediaTrack support
                use crate::outbound::{OutGate, OutprocOutGate};
                let outproc_gate = Arc::new(OutprocOutGate::new(
                    transport_manager,
                    Some(coordinator.clone()), // Enable MediaTrack support
                ));
                let outproc_gate_enum = OutGate::OutprocOut(outproc_gate.clone());

                tracing::info!("✅ OutprocTransportManager + OutprocOutGate initialized");

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
                    data_stream_registry,
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

                tracing::info!(
                    "✅ ContextFactory fully initialized (inproc + outproc gates ready)"
                );

                // Save references (WebRtcGate kept for backward compatibility if needed)
                self.webrtc_coordinator = Some(coordinator.clone());
                self.webrtc_gate = Some(gate.clone());

                tracing::info!("✅ WebRTC infrastructure initialized");

                // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
                // 1.8. Spawn dedicated Unregister task (best-effort, with timeout)
                // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
                //
                // This task:
                // - Waits for shutdown_token to be cancelled (e.g., wait_for_ctrl_c_and_shutdown)
                // - Then sends UnregisterRequest via signaling client with a timeout
                //
                // NOTE: we push its JoinHandle into task_handles so it can be aborted
                // by ActrRefShared::Drop if needed.
                {
                    let shutdown = self.shutdown_token.clone();
                    let client = self.signaling_client.clone();
                    let actor_id_for_unreg = actor_id.clone();
                    let credential_for_unreg = credential.clone();
                    let webrtc_coordinator = self.webrtc_coordinator.clone();

                    let unregister_handle = tokio::spawn(async move {
                        // Wait for shutdown signal
                        shutdown.cancelled().await;
                        tracing::info!(
                            "📡 Shutdown signal received2, sending UnregisterRequest for Actor {:?}",
                            actor_id_for_unreg
                        );

                        // 1. 先关闭所有 WebRTC peer 连接（如果存在）
                        if let Some(coord) = webrtc_coordinator {
                            if let Err(e) = coord.close_all_peers().await {
                                tracing::warn!(
                                    "⚠️ Failed to close all WebRTC peers before UnregisterRequest: {}",
                                    e
                                );
                            } else {
                                tracing::info!(
                                    "✅ All WebRTC peers closed before UnregisterRequest"
                                );
                            }
                        } else {
                            tracing::debug!(
                                "WebRTC coordinator not found before UnregisterRequest (no WebRTC?)"
                            );
                        }

                        // 2. 再发送 UnregisterRequest，设置一个超时（例如 5 秒）
                        let result = tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            client.send_unregister_request(
                                actor_id_for_unreg.clone(),
                                credential_for_unreg.clone(),
                                Some("Graceful shutdown".to_string()),
                            ),
                        )
                        .await;
                        tracing::info!("UnregisterRequest result: {:?}", result);
                        match result {
                            Ok(Ok(_)) => {
                                tracing::info!(
                                    "✅ UnregisterRequest sent to signaling server for Actor {:?}",
                                    actor_id_for_unreg
                                );
                            }
                            Ok(Err(e)) => {
                                tracing::warn!(
                                    "⚠️ Failed to send UnregisterRequest for Actor {:?}: {}",
                                    actor_id_for_unreg,
                                    e
                                );
                            }
                            Err(_) => {
                                tracing::warn!(
                                    "⚠️ UnregisterRequest timeout (5s) for Actor {:?}",
                                    actor_id_for_unreg
                                );
                            }
                        }
                    });

                    task_handles.push(unregister_handle);
                }
            }
            Some(register_response::Result::Error(error)) => {
                tracing::error!(
                    severity = 10,
                    error_category = "registration_error",
                    error_code = error.code,
                    "❌ Registration failed: code={}, message={}",
                    error.code,
                    error.message
                );
                return Err(actr_protocol::ProtocolError::TransportError(format!(
                    "Registration rejected: {} (code: {})",
                    error.message, error.code
                )));
            }
            None => {
                tracing::error!(
                    severity = 10,
                    error_category = "registration_error",
                    "❌ Registration response missing result"
                );
                return Err(actr_protocol::ProtocolError::TransportError(
                    "Invalid registration response: missing result".to_string(),
                ));
            }
        }

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
            .ok_or_else(|| {
                actr_protocol::ProtocolError::InvalidStateTransition(
                    "Actor ID not set - registration must complete before starting node"
                        .to_string(),
                )
            })?
            .clone();
        let credential = self
            .credential
            .as_ref()
            .ok_or_else(|| {
                actr_protocol::ProtocolError::InvalidStateTransition(
                    "Credential not set - node must be started before handling messages"
                        .to_string(),
                )
            })?
            .clone();

        let actor_id_for_shell = actor_id.clone();
        let shutdown_token = self.shutdown_token.clone();
        let node_ref = Arc::new(self);

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // 3.5. Start WebRTC background loops (BEFORE on_start)
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // CRITICAL: Start signaling loop before on_start() to avoid deadlock
        // where on_start() tries to send messages but signaling loop isn't running
        tracing::info!("🚀 Starting WebRTC background loops");

        // Start WebRtcCoordinator signaling loop
        if let Some(coordinator) = &node_ref.webrtc_coordinator {
            coordinator.clone().start().await.map_err(|e| {
                actr_protocol::ProtocolError::TransportError(format!(
                    "WebRtcCoordinator start failed: {e}"
                ))
            })?;
            tracing::info!("✅ WebRtcCoordinator signaling loop started");
        }

        // Start WebRtcGate message receive loop (route to Mailbox)
        if let Some(gate) = &node_ref.webrtc_gate {
            gate.start_receive_loop(node_ref.mailbox.clone())
                .await
                .map_err(|e| {
                    actr_protocol::ProtocolError::TransportError(format!(
                        "WebRtcGate receive loop start failed: {e}"
                    ))
                })?;
            tracing::info!("✅ WebRtcGate → Mailbox routing started");
        }

        tracing::info!("✅ WebRTC background loops started");

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // 4. Call lifecycle hook on_start (AFTER WebRTC loops are running)
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        tracing::info!("🪝 Calling lifecycle hook: on_start");

        let ctx = node_ref
            .context_factory
            .as_ref()
            .expect("ContextFactory must be initialized before on_start")
            .create(
                &actor_id,
                None,        // caller_id
                "bootstrap", // trace_id
                "bootstrap", // request_id
                &credential,
            );
        node_ref.workload.on_start(&ctx).await?;
        tracing::info!("✅ Lifecycle hook on_start completed");

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // 4.6. Start Inproc receive loop (Shell → Workload)
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        tracing::info!("🔄 Starting Inproc receive loop (Shell → Workload)");
        // Start Workload receive loop (Shell → Workload REQUEST)
        if let Some(shell_to_workload) = &node_ref.inproc_mgr {
            if let Some(workload_to_shell) = &node_ref.workload_to_shell_mgr {
                let node = node_ref.clone();
                let request_rx_lane = shell_to_workload
                    .get_lane(actr_protocol::PayloadType::RpcReliable, None)
                    .await
                    .map_err(|e| {
                        actr_protocol::ProtocolError::TransportError(format!(
                            "Failed to get Workload receive lane: {e}"
                        ))
                    })?;
                let response_tx = workload_to_shell.clone();
                let shutdown = shutdown_token.clone();

                let inproc_handle = tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = shutdown.cancelled() => {
                                tracing::info!("📭 Workload receive loop (Shell → Workload) received shutdown signal");
                                break;
                            }

                            envelope_result = request_rx_lane.recv_envelope() => {
                                match envelope_result {
                                    Ok(envelope) => {
                                        let request_id = envelope.request_id.clone();
                                        tracing::debug!("📨 Workload received REQUEST from Shell: request_id={}", request_id);

                                        // Shell calls have no caller_id (local process communication)
                                        match node.handle_incoming(envelope.clone(), None).await {
                                            Ok(response_bytes) => {
                                                // Send RESPONSE back via workload_to_shell
                                                // Keep same route_key (no prefix needed - separate channels!)
                                                let response_envelope = RpcEnvelope {
                                                    route_key: envelope.route_key.clone(),
                                                    payload: Some(response_bytes),
                                                    error: None,
                                                    trace_id: envelope.trace_id.clone(),
                                                    request_id: request_id.clone(),
                                                    metadata: Vec::new(),
                                                    timeout_ms: 30000,
                                                };

                                                // Send via Workload → Shell channel
                                                if let Err(e) = response_tx.send_message(PayloadType::RpcReliable, None, response_envelope).await {
                                                    tracing::error!(
                                                        severity = 7,
                                                        error_category = "transport_error",
                                                        trace_id = %envelope.trace_id,
                                                        request_id = %request_id,
                                                        "❌ Failed to send RESPONSE to Shell: {:?}", e
                                                    );
                                                }
                                            }
                                            Err(e) => {
                                                tracing::error!(
                                                    severity = 6,
                                                    error_category = "handler_error",
                                                    trace_id = %envelope.trace_id,
                                                    request_id = %request_id,
                                                    route_key = %envelope.route_key,
                                                    "❌ Workload message handling failed: {:?}", e
                                                );

                                                // Send error response (system-level error on envelope)
                                                let error_response = actr_protocol::ErrorResponse {
                                                    code: protocol_error_to_code(&e),
                                                    message: e.to_string(),
                                                };

                                                let error_envelope = RpcEnvelope {
                                                    route_key: envelope.route_key.clone(),
                                                    payload: None,
                                                    error: Some(error_response),
                                                    trace_id: envelope.trace_id.clone(),
                                                    request_id: request_id.clone(),
                                                    metadata: Vec::new(),
                                                    timeout_ms: 30000,
                                                };

                                                if let Err(e) = response_tx.send_message(PayloadType::RpcReliable, None, error_envelope).await {
                                                    tracing::error!(
                                                        severity = 7,
                                                        error_category = "transport_error",
                                                        trace_id = %envelope.trace_id,
                                                        request_id = %request_id,
                                                        "❌ Failed to send ERROR response to Shell: {:?}", e
                                                    );
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::error!(
                                            severity = 8,
                                            error_category = "transport_error",
                                            "❌ Failed to receive from Shell → Workload lane: {:?}", e
                                        );
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    tracing::info!(
                        "✅ Workload receive loop (Shell → Workload) terminated gracefully"
                    );
                });

                task_handles.push(inproc_handle);
            }
        }
        tracing::info!("✅ Workload receive loop (Shell → Workload REQUEST) started");

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // 4.7. Start Shell receive loop (Workload → Shell RESPONSE)
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        tracing::info!("🔄 Starting Shell receive loop (Workload → Shell RESPONSE)");
        if let Some(workload_to_shell) = &node_ref.workload_to_shell_mgr {
            if let Some(shell_to_workload) = &node_ref.inproc_mgr {
                let response_rx_lane = workload_to_shell
                    .get_lane(actr_protocol::PayloadType::RpcReliable, None)
                    .await
                    .map_err(|e| {
                        actr_protocol::ProtocolError::TransportError(format!(
                            "Failed to get Shell receive lane: {e}"
                        ))
                    })?;
                let request_mgr = shell_to_workload.clone();
                let shutdown = shutdown_token.clone();

                let shell_receive_handle = tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = shutdown.cancelled() => {
                                tracing::info!("📭 Shell receive loop (Workload → Shell) received shutdown signal");
                                break;
                            }

                            envelope_result = response_rx_lane.recv_envelope() => {
                                match envelope_result {
                                    Ok(envelope) => {
                                        tracing::debug!("📨 Shell received RESPONSE from Workload: request_id={}", envelope.request_id);

                                        // Check if response is success or error
                                        match (envelope.payload, envelope.error) {
                                            (Some(payload), None) => {
                                                // Success response
                                                if let Err(e) = request_mgr.complete_response(&envelope.request_id, payload).await {
                                                    tracing::warn!(
                                                        severity = 4,
                                                        error_category = "orphan_response",
                                                        trace_id = %envelope.trace_id,
                                                        request_id = %envelope.request_id,
                                                        "⚠️  No pending request found for response: {:?}", e
                                                    );
                                                }
                                            }
                                            (None, Some(error)) => {
                                                // Error response - convert to ProtocolError and complete with error
                                                let protocol_err = actr_protocol::ProtocolError::TransportError(
                                                    format!("RPC error {}: {}", error.code, error.message)
                                                );
                                                if let Err(e) = request_mgr.complete_error(&envelope.request_id, protocol_err).await {
                                                    tracing::warn!(
                                                        severity = 4,
                                                        error_category = "orphan_response",
                                                        trace_id = %envelope.trace_id,
                                                        request_id = %envelope.request_id,
                                                        "⚠️  No pending request found for error response: {:?}", e
                                                    );
                                                }
                                            }
                                            _ => {
                                                tracing::error!(
                                                    severity = 7,
                                                    error_category = "protocol_error",
                                                    trace_id = %envelope.trace_id,
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
                                            "❌ Failed to receive from Workload → Shell lane: {:?}", e
                                        );
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    tracing::info!(
                        "✅ Shell receive loop (Workload → Shell) terminated gracefully"
                    );
                });

                task_handles.push(shell_receive_handle);
            }
        }
        tracing::info!("✅ Shell receive loop (Workload → Shell RESPONSE) started");

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
                                        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                                        continue;
                                    }

                                    tracing::debug!("📬 Mailbox dequeue: {} messages", messages.len());

                                    // Process messages one by one
                                    for msg_record in messages {
                                        // Deserialize RpcEnvelope (Protobuf)
                                        match RpcEnvelope::decode(&msg_record.payload[..]) {
                                            Ok(envelope) => {
                                                let request_id = envelope.request_id.clone();
                                                tracing::debug!("📦 Processing message: request_id={}", request_id);

                                                // Decode caller_id from MessageRecord.from (transport layer)
                                                let caller_id_result = ActrId::decode(&msg_record.from[..]);
                                                let caller_id_ref = caller_id_result.as_ref().ok();

                                                if caller_id_ref.is_none() {
                                                    tracing::warn!(
                                                        trace_id = %envelope.trace_id,
                                                        request_id = %request_id,
                                                        "⚠️  Failed to decode caller_id from MessageRecord.from"
                                                    );
                                                }

                                                // Call handle_incoming with caller_id from transport layer
                                                match node.handle_incoming(envelope.clone(), caller_id_ref).await {
                                                    Ok(response_bytes) => {
                                                        // Send response (reuse request_id)
                                                        if let Some(ref gate) = gate {
                                                            // Use already decoded caller_id
                                                            match caller_id_result {
                                                                Ok(caller) => {
                                                                    // Construct response RpcEnvelope (reuse request_id!)
                                                                    let response_envelope = RpcEnvelope {
                                                                        request_id,  // Reuse!
                                                                        route_key: envelope.route_key.clone(),
                                                                        payload: Some(response_bytes),
                                                                        error: None,
                                                                        trace_id: envelope.trace_id.clone(),
                                                                        metadata: Vec::new(),  // Response doesn't need extra metadata
                                                                        timeout_ms: 30000,
                                                                    };

                                                                    if let Err(e) = gate.send_response(&caller, response_envelope).await {
                                                                        tracing::error!(
                                                                            severity = 7,
                                                                            error_category = "transport_error",
                                                                            trace_id = %envelope.trace_id,
                                                                            request_id = %envelope.request_id,
                                                                            "❌ Failed to send response: {:?}", e
                                                                        );
                                                                    }
                                                                }
                                                                Err(e) => {
                                                                    tracing::error!(
                                                                        severity = 8,
                                                                        error_category = "protobuf_decode",
                                                                        trace_id = %envelope.trace_id,
                                                                        request_id = %envelope.request_id,
                                                                        "❌ Failed to decode caller_id: {:?}", e
                                                                    );
                                                                }
                                                            }
                                                        }

                                                        // ACK message
                                                        if let Err(e) = mailbox.ack(msg_record.id).await {
                                                            tracing::error!(
                                                                severity = 9,
                                                                error_category = "mailbox_error",
                                                                trace_id = %envelope.trace_id,
                                                                request_id = %envelope.request_id,
                                                                message_id = %msg_record.id,
                                                                "❌ Mailbox ACK failed: {:?}", e
                                                            );
                                                        }
                                                    }
                                                    Err(e) => {
                                                        tracing::error!(
                                                            severity = 6,
                                                            error_category = "handler_error",
                                                            trace_id = %envelope.trace_id,
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
                                                    "❌ Poison message: Failed to deserialize RpcEnvelope: {:?}", e
                                                );

                                                // Write to Dead Letter Queue
                                                use actr_mailbox::DlqRecord;
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
                                                    trace_id: format!("mailbox-{}", msg_record.id),  // Fallback trace_id
                                                    request_id: None,
                                                    created_at: Utc::now(),
                                                    redrive_attempts: 0,
                                                    last_redrive_at: None,
                                                    context: Some(format!(
                                                        r#"{{"source":"mailbox","priority":"{}"}}"#,
                                                        match msg_record.priority {
                                                            actr_mailbox::MessagePriority::High => "high",
                                                            actr_mailbox::MessagePriority::Normal => "normal",
                                                        }
                                                    )),
                                                };

                                                if let Err(dlq_err) = node.dlq.enqueue(dlq_record).await {
                                                    tracing::error!(
                                                        severity = 10,
                                                        "❌ CRITICAL: Failed to write poison message to DLQ: {:?}", dlq_err
                                                    );
                                                } else {
                                                    tracing::warn!(
                                                        severity = 9,
                                                        "☠️ Poison message moved to DLQ: message_id={}", msg_record.id
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
                                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
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

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // 6. Create ActrRef for Shell to interact with Workload
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        use crate::actr_ref::{ActrRef, ActrRefShared};
        use crate::outbound::InprocOutGate;

        // Create InprocOutGate from shell_to_workload transport manager
        let shell_to_workload = node_ref
            .inproc_mgr
            .clone()
            .expect("inproc_mgr must be initialized");
        let inproc_gate = Arc::new(InprocOutGate::new(shell_to_workload));

        // Create ActrRefShared
        let actr_ref_shared = Arc::new(ActrRefShared {
            actor_id: actor_id_for_shell.clone(),
            inproc_gate,
            shutdown_token: shutdown_token.clone(),
            task_handles: tokio::sync::Mutex::new(task_handles),
        });

        // Create ActrRef
        let actr_ref = ActrRef::new(actr_ref_shared, node_ref);

        tracing::info!("✅ ActrRef created (Shell → Workload communication handle)");

        Ok(actr_ref)
    }
}
