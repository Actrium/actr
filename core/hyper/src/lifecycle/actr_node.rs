//! ActrNode - ActrSystem + Workload (1:1 composition)

use crate::ais_client::AisClient;
use crate::context_factory::ContextFactory;
use crate::lifecycle::dedup::{DedupOutcome, DedupState};
use crate::transport::HostTransport;
#[cfg(feature = "opentelemetry")]
use crate::wire::webrtc::trace::{inject_span_context_to_rpc, set_parent_from_rpc_envelope};
use actr_framework::{Bytes, Workload};
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{
    AIdCredential, ActorResult, ActrError, ActrId, ActrType, PayloadType, RegisterRequest,
    RouteCandidatesRequest, RpcEnvelope, TurnCredential, register_response,
    route_candidates_request,
};
use actr_runtime_mailbox::{DeadLetterQueue, Mailbox};
use futures_util::FutureExt;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
#[cfg(feature = "opentelemetry")]
use tracing::Instrument as _;
// Use types from sub-crates
use crate::wire::webrtc::SignalingClient;
// Use extension traits from actr-protocol
use actr_protocol::{ActrIdExt, ActrTypeExt};
// Use heartbeat functions
use crate::lifecycle::heartbeat::heartbeat_task;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Service Discovery Result
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Result of a service discovery request.
///
/// This struct is returned by `discover_route_candidates` and provides
/// the list of discovered candidates along with their WebSocket addresses.
#[derive(Debug, Clone)]
pub struct DiscoveryResult {
    /// Ordered list of compatible candidates (best match first)
    pub candidates: Vec<ActrId>,
    /// WebSocket direct-connect addresses discovered from signaling server.
    /// Maps each candidate ActrId to its ws:// URL if the server has one.
    /// Clients should use this URL to bypass relay (WebRTC) and connect directly.
    pub ws_addresses: std::collections::HashMap<ActrId, String>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Constants
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

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
    pub(crate) credential_state: Option<CredentialState>,

    /// WebRTC coordinator (created after startup)
    pub(crate) webrtc_coordinator: Option<Arc<crate::wire::webrtc::coordinator::WebRtcCoordinator>>,

    /// WebRTC Gate (created after startup)
    pub(crate) webrtc_gate: Option<Arc<crate::wire::webrtc::gate::WebRtcGate>>,

    /// WebSocket Gate (direct-connect mode inbound, optional)
    pub(crate) websocket_gate: Option<Arc<crate::wire::websocket::WebSocketGate>>,

    /// Shell → Workload Transport Manager
    ///
    /// Workload receives REQUEST from Shell (zero serialization, direct RpcEnvelope passing)
    pub(crate) inproc_mgr: Option<Arc<HostTransport>>,

    /// Workload → Shell Transport Manager
    ///
    /// Workload sends RESPONSE to Shell (separate pending_requests from Shell's)
    pub(crate) workload_to_shell_mgr: Option<Arc<HostTransport>>,

    /// Shutdown token for graceful shutdown
    pub(crate) shutdown_token: CancellationToken,

    /// Actr.lock.toml content (loaded at startup for fingerprint lookups)
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

    /// Pre-injected registration result from the Hyper layer (used in Process / Wasm mode)
    ///
    /// When `execution_mode` is `Process` or `Wasm`, the Hyper layer calls `inject_credential()`
    /// to write an already-issued registration credential into this field. When `start()` detects
    /// this field is not `None`, it skips signaling registration and uses the injected credential.
    pub(crate) injected_registration: Option<actr_protocol::register_response::RegisterOk>,

    /// Shared WebSocket direct-connect address map populated by `discover_route_candidates`.
    ///
    /// This Arc is shared with `DefaultWireBuilder` so that when a connection to a discovered
    /// actor is established, the wire builder can look up the ws:// URL directly instead of
    /// relying on a static url_template.  The map is keyed by `ActrId`.
    pub(crate) discovered_ws_addresses:
        Arc<tokio::sync::RwLock<std::collections::HashMap<ActrId, String>>>,

    /// Dynamic executor adapter (WASM, dynclib, etc.)
    ///
    /// When this field is `Some`, `handle_incoming` dispatches through the executor
    /// instead of native `W::Dispatcher::dispatch`.
    ///
    /// The `Mutex` is the enforcement point for the guest ABI contract:
    /// one executor instance equals one logical guest actor instance, and
    /// dispatch into that instance is serialized.
    pub(crate) executor: Option<Mutex<Box<dyn crate::executor::ExecutorAdapter>>>,
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

/// Extract human-readable info from a panic payload
fn extract_panic_info(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "Unknown panic payload".to_string()
    }
}

/// PendingCall executor - routes guest outbound calls through RuntimeContext
///
/// Called by the executor dispatch path in `handle_incoming` for each asyncify unwind.
async fn executor_call_handler(
    ctx: crate::context::RuntimeContext,
    pending: crate::executor::PendingCall,
) -> crate::executor::IoResult {
    use crate::executor::{IoResult, PendingCall, error_code as code};
    use actr_framework::{Context as _, Dest};
    use actr_protocol::prost::Message as ProstMessage;
    use actr_protocol::{ActrId, ActrType, PayloadType};

    /// Decode guest-encoded dest_bytes back to `Dest`
    ///
    /// Format corresponds to `encode_dest()` in `actr-framework::guest`:
    /// - `0x00` = Shell
    /// - `0x01` = Local
    /// - `0x02` + protobuf ActrId bytes = Actor(id)
    fn decode_dest(bytes: &[u8]) -> Option<Dest> {
        match bytes.first()? {
            0x00 => Some(Dest::Shell),
            0x01 => Some(Dest::Local),
            0x02 => ActrId::decode(&bytes[1..]).ok().map(Dest::Actor),
            tag => {
                tracing::warn!(tag, "WASM dest_bytes: unknown tag");
                None
            }
        }
    }

    /// Map `ActrError` to ABI error code, preserving semantics for guest-side discrimination
    fn actr_error_to_code(err: &ActrError) -> i32 {
        match err {
            ActrError::DecodeFailure(_) | ActrError::InvalidArgument(_) => code::PROTOCOL_ERROR,
            _ => code::GENERIC_ERROR,
        }
    }

    match pending {
        PendingCall::CallRaw {
            route_key,
            target_bytes,
            payload,
        } => {
            let target = match ActrId::decode(target_bytes.as_slice()) {
                Ok(id) => id,
                Err(e) => {
                    tracing::error!("WASM call_raw: target_bytes decode failed: {e}");
                    return IoResult::Error(code::PROTOCOL_ERROR);
                }
            };
            match ctx
                .call_raw(
                    &Dest::Actor(target),
                    route_key,
                    PayloadType::RpcReliable,
                    bytes::Bytes::from(payload),
                    0,
                )
                .await
            {
                Ok(resp) => IoResult::Bytes(resp.to_vec()),
                Err(e) => {
                    tracing::error!("WASM call_raw routing failed: {e:?}");
                    IoResult::Error(actr_error_to_code(&e))
                }
            }
        }

        PendingCall::Call {
            route_key,
            dest_bytes,
            payload,
        } => {
            let dest = match decode_dest(&dest_bytes) {
                Some(d) => d,
                None => {
                    tracing::error!(route_key, "WASM call: dest_bytes decode failed");
                    return IoResult::Error(code::PROTOCOL_ERROR);
                }
            };
            match ctx
                .call_raw(
                    &dest,
                    route_key,
                    PayloadType::RpcReliable,
                    bytes::Bytes::from(payload),
                    0,
                )
                .await
            {
                Ok(resp) => IoResult::Bytes(resp.to_vec()),
                Err(e) => {
                    tracing::error!("WASM call routing failed: {e:?}");
                    IoResult::Error(actr_error_to_code(&e))
                }
            }
        }

        PendingCall::Tell {
            route_key,
            dest_bytes,
            payload,
        } => {
            let dest = match decode_dest(&dest_bytes) {
                Some(d) => d,
                None => {
                    tracing::error!(route_key, "WASM tell: dest_bytes decode failed");
                    return IoResult::Error(code::PROTOCOL_ERROR);
                }
            };
            match ctx
                .tell_raw(
                    &dest,
                    route_key,
                    PayloadType::RpcReliable,
                    bytes::Bytes::from(payload),
                )
                .await
            {
                Ok(()) => IoResult::Done,
                Err(e) => {
                    tracing::error!("WASM tell routing failed: {e:?}");
                    IoResult::Error(actr_error_to_code(&e))
                }
            }
        }

        PendingCall::Discover { type_bytes } => {
            let actr_type = match ActrType::decode(type_bytes.as_slice()) {
                Ok(t) => t,
                Err(e) => {
                    tracing::error!("WASM discover: type_bytes decode failed: {e}");
                    return IoResult::Error(code::PROTOCOL_ERROR);
                }
            };
            match ctx.discover_route_candidate(&actr_type).await {
                Ok(id) => IoResult::Bytes(id.encode_to_vec()),
                Err(e) => {
                    tracing::error!("WASM discover failed: {e:?}");
                    IoResult::Error(actr_error_to_code(&e))
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

/// Check ACL permission for incoming request
///
/// # Arguments
/// - `caller_id`: The ActrId of the caller (None for local calls)
/// - `target_id`: The ActrId of the target (self)
/// - `acl`: ACL rules from configuration
///
/// # Returns
/// - `Ok(true)`: Permission granted
/// - `Ok(false)`: Permission denied
/// - `Err`: ACL check failed (treat as deny)
///
/// # ACL Evaluation Logic
/// 1. If no caller_id (local call), always allow
/// 2. If no ACL configured, allow by default (permissive mode for backward compatibility)
/// 3. If ACL configured but rules are empty, deny all (secure by default)
/// 4. Deny-first: any matching DENY rule overrides all ALLOWs
/// 5. If no DENY matches and at least one ALLOW matches, allow
/// 6. If no rule matches, deny by default
fn check_acl_permission(
    caller_id: Option<&ActrId>,
    target_id: &ActrId,
    acl: Option<&actr_protocol::Acl>,
) -> Result<bool, String> {
    // 1. Local calls (no caller_id) are always allowed
    if caller_id.is_none() {
        tracing::trace!("ACL: Local call, allowing");
        return Ok(true);
    }

    let caller = caller_id.unwrap();

    // 2. No ACL configured - allow by default
    let acl_rules = match acl {
        Some(acl) => acl,
        None => {
            tracing::trace!(
                "ACL: No ACL configured, allowing {} -> {}",
                caller.to_string_repr(),
                target_id.to_string_repr()
            );
            return Ok(true);
        }
    };

    // 3. If ACL is configured but has no rules, deny all (secure by default)
    if acl_rules.rules.is_empty() {
        tracing::warn!(
            "ACL: No rules defined, denying {} -> {} (default deny)",
            caller.to_string_repr(),
            target_id.to_string_repr()
        );
        return Ok(false);
    }

    // 4 & 5. Deny-first evaluation
    let mut any_allow = false;
    for rule in &acl_rules.rules {
        if !matches_rule(caller, rule) {
            continue;
        }
        let is_allow = rule.permission == actr_protocol::acl_rule::Permission::Allow as i32;
        if !is_allow {
            tracing::debug!(
                "ACL: DENY rule matched for {} -> {}",
                caller.to_string_repr(),
                target_id.to_string_repr()
            );
            return Ok(false);
        }
        any_allow = true;
    }

    if any_allow {
        tracing::debug!(
            "ACL: ALLOW rule matched for {} -> {}",
            caller.to_string_repr(),
            target_id.to_string_repr()
        );
        return Ok(true);
    }

    // 6. No rule matched - deny by default
    tracing::warn!(
        "ACL: No matching rule, denying {} -> {} (default deny)",
        caller.to_string_repr(),
        target_id.to_string_repr()
    );
    Ok(false)
}

fn matches_rule(caller: &ActrId, rule: &actr_protocol::AclRule) -> bool {
    use actr_protocol::acl_rule::SourceRealm;

    // Check type match (exact: manufacturer + name + version)
    if caller.r#type.manufacturer != rule.from_type.manufacturer
        || caller.r#type.name != rule.from_type.name
        || caller.r#type.version != rule.from_type.version
    {
        return false;
    }

    // Check realm match
    match &rule.source_realm {
        None | Some(SourceRealm::AnyRealm(_)) => true,
        Some(SourceRealm::RealmId(id)) => caller.realm.realm_id == *id,
    }
}

impl<W: Workload> ActrNode<W> {
    /// Set a dynamic executor adapter (WASM, dynclib, etc.)
    ///
    /// When set, `handle_incoming` dispatches through this executor instead of
    /// the native `W::Dispatcher::dispatch` path.
    /// Call after `build()` and before `start()`.
    pub fn with_executor(mut self, executor: Box<dyn crate::executor::ExecutorAdapter>) -> Self {
        self.executor = Some(Mutex::new(executor));
        self
    }

    /// Configure a WASM actor instance as the executor
    ///
    /// Convenience wrapper around `with_executor` that boxes the `WasmInstance`.
    /// Call after `build()` and before `start()`.
    #[cfg(feature = "wasm-engine")]
    pub fn with_wasm_instance(mut self, instance: crate::wasm::WasmInstance) -> Self {
        self.executor = Some(Mutex::new(Box::new(instance)));
        self
    }

    /// Get Inproc Transport Manager
    ///
    /// # Returns
    /// - `Some(Arc<HostTransport>)`: Initialized manager
    /// - `None`: Not yet started (need to call start() first)
    ///
    /// # Use Cases
    /// - Workload internals need to communicate with Shell
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

    /// Discover remote actors of the specified type via signaling server.
    ///
    /// # Arguments
    /// - `target_type`: The ActrType of the target service to discover
    /// - `candidate_count`: Maximum number of candidates to return
    ///
    /// # Returns
    /// A `DiscoveryResult` containing candidates and their WebSocket addresses
    #[cfg_attr(feature = "opentelemetry", tracing::instrument(skip_all))]
    pub async fn discover_route_candidates(
        &self,
        target_type: &ActrType,
        candidate_count: u32,
    ) -> ActorResult<DiscoveryResult> {
        // Check if node is started (has actor_id and credential)
        let actor_id = self.actor_id.as_ref().ok_or_else(|| {
            ActrError::Internal("Node is not started. Call start() first.".to_string())
        })?;

        // Check if the signaling client is connected
        if !self.signaling_client.is_connected() {
            return Err(ActrError::Unavailable(
                "Signaling client is not connected.".to_string(),
            ));
        }

        let service_name = format!("{}:{}", target_type.manufacturer, target_type.name);

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // Step 1: Get fingerprint from Actr.lock.toml (when available)
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        let client_fingerprint = match self.get_dependency_fingerprint(target_type) {
            Some(fingerprint) => fingerprint,
            None => {
                if self.actr_lock.is_none() {
                    tracing::debug!(
                        "Actr.lock.toml not loaded; sending discovery without fingerprint for '{}'",
                        service_name
                    );
                    String::new()
                } else {
                    tracing::error!(
                        severity = 10,
                        error_category = "dependency_missing",
                        "❌ DEPENDENCY NOT FOUND: Service '{}' is not declared in Actr.lock.toml.\n\
                         Please run 'actr install' to generate the lock file with all dependencies.",
                        service_name
                    );
                    return Err(ActrError::DependencyNotFound {
                        service_name: service_name.clone(),
                        message: format!(
                            "Dependency '{}' not found in Actr.lock.toml. Run 'actr install' to resolve dependencies.",
                            service_name
                        ),
                    });
                }
            }
        };

        if !client_fingerprint.is_empty() {
            tracing::debug!(
                "📋 Found dependency fingerprint for '{}': {}",
                service_name,
                &client_fingerprint[..20.min(client_fingerprint.len())]
            );
        }

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // Step 2: Send discovery request to signaling server
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        let result = self
            .send_discovery_request(actor_id, target_type, candidate_count, client_fingerprint)
            .await?;

        tracing::info!(
            "Discovery result [{}]: {} candidate(s)",
            service_name,
            result.candidates.len(),
        );

        // Populate the shared ws_addresses map so the wire builder can use discovered URLs
        if !result.ws_addresses.is_empty() {
            let mut map = self.discovered_ws_addresses.write().await;
            for (id, url) in &result.ws_addresses {
                tracing::info!(
                    "Caching discovered WS address for {}: {}",
                    id.serial_number,
                    url
                );
                map.insert(id.clone(), url.clone());
            }
        }

        Ok(DiscoveryResult {
            candidates: result.candidates,
            ws_addresses: result.ws_addresses,
        })
    }

    /// Get dependency fingerprint from Actr.lock.toml
    fn get_dependency_fingerprint(&self, target_type: &ActrType) -> Option<String> {
        let actr_lock = self.actr_lock.as_ref()?;

        let key = target_type.to_string_repr();

        // Try by full key
        if let Some(dep) = actr_lock.get_dependency(&key) {
            return Some(dep.fingerprint.clone());
        }

        // Allow lookups without version to match lock entries that include one.
        for dep in &actr_lock.dependencies {
            if Self::matches_dependency_actr_type(&dep.actr_type, target_type) {
                return Some(dep.fingerprint.clone());
            }
        }

        None
    }

    fn matches_dependency_actr_type(raw: &str, target_type: &ActrType) -> bool {
        let Ok(dep_type) = ActrType::from_string_repr(raw) else {
            return raw == format!("{}:{}", target_type.manufacturer, target_type.name);
        };

        dep_type.manufacturer == target_type.manufacturer
            && dep_type.name == target_type.name
            && (target_type.version.is_empty() || dep_type.version == target_type.version)
    }

    /// Internal: Send discovery request to signaling server
    async fn send_discovery_request(
        &self,
        actor_id: &ActrId,
        target_type: &ActrType,
        candidate_count: u32,
        client_fingerprint: String,
    ) -> ActorResult<DiscoveryResult> {
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
            client_fingerprint,
        };

        let credential_state = self.credential_state.clone().ok_or_else(|| {
            ActrError::Internal("Node is not started. Call start() first.".to_string())
        })?;

        let route_response = client
            .send_route_candidates_request(
                actor_id.clone(),
                credential_state.credential().await,
                route_request,
            )
            .await
            .map_err(|e| ActrError::Unavailable(format!("Route candidates request failed: {e}")))?;

        match route_response.result {
            Some(actr_protocol::route_candidates_response::Result::Success(success)) => {
                let ws_addresses: std::collections::HashMap<ActrId, String> = success
                    .ws_address_map
                    .into_iter()
                    .filter_map(|entry| entry.ws_address.map(|url| (entry.candidate_id, url)))
                    .collect();

                Ok(DiscoveryResult {
                    candidates: success.candidates,
                    ws_addresses,
                })
            }
            Some(actr_protocol::route_candidates_response::Result::Error(err)) => {
                Err(ActrError::Unavailable(format!(
                    "Route candidates error {}: {}",
                    err.code, err.message
                )))
            }
            None => Err(ActrError::Unavailable(
                "Invalid route candidates response: missing result".to_string(),
            )),
        }
    }

    /// Build the hook callback and inject it into the signaling client.
    ///
    /// `deps` starts as `None`. The caller sets it to `Some(...)` after registration.
    /// Before that, hooks that accept `Option<&C>` will receive `None`.
    ///
    /// **Must be called before `connect()`** so the first connection events are
    /// captured naturally by `invoke_hook` inside `connect_with_retries`.
    fn build_hook_callback(
        &self,
        deps: Arc<std::sync::OnceLock<(ContextFactory, ActrId, CredentialState)>>,
    ) -> crate::wire::webrtc::signaling::HookCallback {
        use crate::wire::webrtc::signaling::{HookCallback, HookEvent};

        let workload = self.workload.clone();

        let hook_cb: HookCallback = Arc::new(move |event| {
            let workload = workload.clone();
            let deps = deps.clone();

            Box::pin(async move {
                let ctx = if let Some((factory, aid, cred)) = deps.get() {
                    let credential = cred.credential().await;
                    Some(factory.create(aid, None, "hook", &credential))
                } else {
                    None
                };

                let result = match event {
                    HookEvent::SignalingConnectStart { .. } => {
                        tracing::debug!("🪝 on_signaling_connect_start");
                        workload.on_signaling_connect_start(ctx.as_ref()).await
                    }
                    HookEvent::SignalingConnected => {
                        tracing::debug!("🪝 on_signaling_connected");
                        workload.on_signaling_connected(ctx.as_ref()).await
                    }
                    HookEvent::SignalingDisconnected => {
                        tracing::debug!("🪝 on_signaling_disconnected");
                        let ctx = ctx.as_ref().expect("ctx must be available for disconnect");
                        workload.on_signaling_disconnected(ctx).await
                    }
                    HookEvent::WebRtcConnectStart { ref peer_id } => {
                        tracing::debug!("🪝 on_webrtc_connect_start({})", peer_id.serial_number);
                        let ctx = ctx
                            .as_ref()
                            .expect("ctx must be available for webrtc hooks");
                        workload.on_webrtc_connect_start(ctx, peer_id).await
                    }
                    HookEvent::WebRtcConnected {
                        ref peer_id,
                        relayed,
                    } => {
                        tracing::debug!(
                            "🪝 on_webrtc_connected({}, relayed={})",
                            peer_id.serial_number,
                            relayed
                        );
                        let ctx = ctx
                            .as_ref()
                            .expect("ctx must be available for webrtc hooks");
                        workload.on_webrtc_connected(ctx, peer_id, relayed).await
                    }
                    HookEvent::WebRtcDisconnected { ref peer_id } => {
                        tracing::debug!("🪝 on_webrtc_disconnected({})", peer_id.serial_number);
                        let ctx = ctx
                            .as_ref()
                            .expect("ctx must be available for webrtc hooks");
                        workload.on_webrtc_disconnected(ctx, peer_id).await
                    }
                };

                if let Err(e) = result {
                    tracing::warn!("⚠️ Hook callback error: {}", e);
                }
            })
        });

        self.signaling_client.set_hook_callback(hook_cb.clone());
        tracing::info!("✅ Hook callback injected into signaling client");

        hook_cb
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
                                event_processor.process_network_type_changed(*is_wifi, *is_cellular).await
                            }
                            NetworkEvent::CleanupConnections => {
                                event_processor.cleanup_connections().await
                            }
                        };

                        let duration_ms = start.elapsed().as_millis() as u64;

                        // Construct processing result
                        let event_result = match result {
                            Ok(_) => NetworkEventResult::success(latest_event.clone(), duration_ms),
                            Err(e) => NetworkEventResult::failure(latest_event.clone(), e, duration_ms),
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
    ///
    /// Native dispatch: static MessageRouter + panic catching
    async fn native_dispatch(
        &self,
        envelope: &RpcEnvelope,
        ctx: &crate::context::RuntimeContext,
    ) -> ActorResult<Bytes> {
        use actr_framework::MessageDispatcher;

        let r = std::panic::AssertUnwindSafe(W::Dispatcher::dispatch(
            &self.workload,
            envelope.clone(),
            ctx,
        ))
        .catch_unwind()
        .await;
        match r {
            Ok(h) => h,
            Err(p) => {
                let pi = extract_panic_info(p);
                tracing::error!(
                    severity = 8,
                    error_category = "handler_panic",
                    route_key = envelope.route_key,
                    request_id = %envelope.request_id,
                    "❌ Handler panicked: {}", pi
                );
                let _ = self
                    .workload
                    .on_error(ctx, format!("Handler panicked: {pi}"))
                    .await;
                Err(ActrError::DecodeFailure(format!("Handler panicked: {pi}")))
            }
        }
    }

    /// - Single-hop calls: effectively identical
    /// - Multi-hop calls: trace_id spans all hops, request_id per hop
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(skip_all, name = "ActrNode.handle_incoming")
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
                caller.to_string_repr(),
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
                caller = %caller_id.map(|c| c.to_string_repr()).unwrap_or_else(|| "<none>".to_string()),
                "🚫 ACL: Permission denied"
            );

            return Err(ActrError::PermissionDenied(format!(
                "ACL denied: {} is not allowed to call {}",
                caller_id
                    .map(|c| c.to_string_repr())
                    .unwrap_or_else(|| "<unknown>".to_string()),
                actor_id.to_string_repr()
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

        // 2. Dispatch (executor adapter path or native static dispatch)

        let result: ActorResult<Bytes> = if let Some(executor) = &self.executor {
            let envelope_bytes = envelope.encode_to_vec();
            let dispatch_ctx = crate::executor::DispatchContext {
                self_id: actor_id.clone(),
                caller_id: caller_id.cloned(),
                request_id: envelope.request_id.clone(),
            };
            let ctx_for_executor = ctx.clone();
            let call_executor: crate::executor::CallExecutorFn = Box::new(move |pending| {
                let ctx = ctx_for_executor.clone();
                Box::pin(async move { executor_call_handler(ctx, pending).await })
            });
            let mut guard = executor.lock().await;
            guard
                .dispatch(&envelope_bytes, dispatch_ctx, &call_executor)
                .await
                .map(Bytes::from)
                .map_err(|e| {
                    tracing::error!(
                        severity = 7,
                        error_category = "executor_dispatch_error",
                        route_key = envelope.route_key,
                        request_id = %envelope.request_id,
                        "executor dispatch failed: {:?}", e
                    );
                    ActrError::Internal(format!("executor dispatch failed: {e:?}"))
                })
        } else {
            self.native_dispatch(&envelope, &ctx).await
        };

        // 3. Log result
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

        // Store completed result in dedup cache before returning
        self.dedup_state
            .lock()
            .await
            .complete(&envelope.request_id, result.clone());

        result
    }

    /// Inject a pre-issued registration credential (Wasm mode)
    ///
    /// Called by the Hyper layer before `start()`, writing the already-issued `RegisterOk`
    /// into ActrNode so that `start()` skips the signaling registration step.
    ///
    /// Applicable scenario:
    /// - **Wasm mode**: Hyper host injects the credential into the WASM instance via host functions
    pub fn inject_credential(&mut self, register_ok: actr_protocol::register_response::RegisterOk) {
        tracing::debug!(
            execution_mode = %self.config.execution_mode,
            "Injected pre-registration credential; start() will skip signaling registration"
        );
        self.injected_registration = Some(register_ok);
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
        println!("Actr Rust version: {}", env!("CARGO_PKG_VERSION"));

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // 0.5. Build hook callback and inject into signaling (before connect!)
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        let hook_ctx_deps = Arc::new(std::sync::OnceLock::new());
        let hook_cb = self.build_hook_callback(hook_ctx_deps.clone());

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // 1. Build RegisterRequest
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

        // Get ActrType from configuration
        let actr_type = self.config.actr_type().clone();
        tracing::info!("📋 Actor type: {}", actr_type.to_string_repr());

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
        //
        // - Process / Wasm mode: Hyper completed AIS registration in advance and injected RegisterOk
        // - Native mode (default): register via AIS HTTP to obtain ActrId and AID credential
        let register_ok = if let Some(injected) = self.injected_registration.take() {
            tracing::info!(
                execution_mode = %self.config.execution_mode,
                "Using Hyper pre-injected registration credential; skipping AIS registration"
            );
            injected
        } else {
            // Native mode: register via AIS HTTP
            let ais_endpoint = self.config.ais_endpoint.as_ref().ok_or_else(|| {
                ActrError::Unavailable(
                    "ais_endpoint is required for native mode registration".to_string(),
                )
            })?;

            tracing::info!(
                ais_endpoint = %ais_endpoint,
                "Registering actor with AIS via HTTP"
            );

            let ais = AisClient::new(ais_endpoint);
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
                register_ok.credential_expires_at.clone(),
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

            tracing::info!(
                "🆔 Assigned ActrId: {}",
                actr_protocol::ActrIdExt::to_string_repr(&actor_id)
            );
            tracing::info!("🔐 Received credential (key_id: {})", credential.key_id);
            tracing::info!(
                "💓 Signaling heartbeat interval: {} seconds",
                register_ok.signaling_heartbeat_interval_secs
            );

            // TurnCredential is a required field; should always be present under normal conditions
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

            // Populate hook callback ctx dependencies now that we have actor_id + credential.
            // After this, all hook invocations will receive Some(ctx) instead of None.
            let _ = hook_ctx_deps.set((
                self.context_factory
                    .clone()
                    .expect("ContextFactory must exist"),
                actor_id.clone(),
                credential_state.clone(),
            ));

            // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
            // 1.2. Set actr_lock in ContextFactory for fingerprint lookups
            // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
            if let Some(actr_lock) = self.actr_lock.clone() {
                self.context_factory
                    .as_mut()
                    .expect("ContextFactory must exist")
                    .set_actr_lock(actr_lock);
                tracing::info!("✅ Actr.lock.toml set in ContextFactory for fingerprint lookups");
            }

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

            tracing::info!("✅ Inproc infrastructure already ready (created in ActrSystem::new())");

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
            // WebSocket channel always enabled: target ws:// address is fully determined by discovery
            // Direct-connect mode: encode local node ActrId as hex, sent as X-Actr-Source-ID in WS handshake
            let local_id_hex = hex::encode(actor_id.encode_to_vec());
            let wire_builder_config = DefaultWireBuilderConfig {
                local_id_hex,
                enable_webrtc: true,
                enable_websocket: true,
                // Share the discovered_ws_addresses map so that post-discovery connections
                // can use the signaling-provided ws:// URL for this actor node.
                discovered_ws_addresses: self.discovered_ws_addresses.clone(),
                // Pass credential_state so outbound WS handshake carries X-Actr-Credential header,
                // enabling peer WebSocketGate to perform Ed25519 signature verification
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
            let outproc_gate = Arc::new(PeerGate::new(
                transport_manager,
                Some(coordinator.clone()), // Enable MediaTrack support
            ));
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

            // Save references (WebRtcGate kept for backward compatibility if needed)
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

                let ais_endpoint_for_heartbeat =
                    self.config.ais_endpoint.clone().unwrap_or_default();
                let heartbeat_handle = tokio::spawn(heartbeat_task(
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
            {
                let shutdown = self.shutdown_token.clone();
                let client = self.signaling_client.clone();
                let actor_id_for_unreg = actor_id.clone();
                let credential_state_for_unreg = credential_state.clone();
                let webrtc_coordinator = self.webrtc_coordinator.clone();

                let unregister_handle = tokio::spawn(async move {
                    // Wait for shutdown signal
                    shutdown.cancelled().await;
                    tracing::info!(
                        "📡 Shutdown signal received2, sending UnregisterRequest for Actor {:?}",
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
                        std::time::Duration::from_secs(5),
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
            .ok_or_else(|| {
                ActrError::Internal(
                    "Actor ID not set - registration must complete before starting node"
                        .to_string(),
                )
            })?
            .clone();
        let credential_state = self.credential_state.clone().ok_or_else(|| {
            ActrError::Internal(
                "Credential not set - node must be started before handling messages".to_string(),
            )
        })?;

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
                "bootstrap", // request_id
                &credential_state.credential().await,
            );
        node_ref.workload.on_start(&ctx).await?;
        tracing::info!("✅ Lifecycle hook on_start completed");

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // 4.5. Inject hook callback into WebRTC coordinator
        //      Reuse the same callback built in step 0.5.
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        if let Some(coord) = &node_ref.webrtc_coordinator {
            coord.set_hook_callback(hook_cb);
            tracing::info!("✅ Hook callback injected into WebRTC coordinator");
        }

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
                        ActrError::Unavailable(format!("Failed to get Workload receive lane: {e}"))
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
                                        // Extract and set tracing context from envelope
                                        #[cfg(feature = "opentelemetry")]
                                        let span = {
                                                let span = tracing::info_span!("actrNode.lane.receive_rpc", request_id = %request_id);
                                                set_parent_from_rpc_envelope(&span, &envelope);
                                                span
                                            };

                                        tracing::debug!("📨 Workload received REQUEST from Shell: request_id={}", request_id);

                                        // Shell calls have no caller_id (local process communication)
                                        let handle_incoming_fut = node.handle_incoming(envelope.clone(), None);
                                        #[cfg(feature = "opentelemetry")]
                                        let handle_incoming_fut = handle_incoming_fut.instrument(span.clone());
                                        match handle_incoming_fut.await {
                                            Ok(response_bytes) => {
                                                // Send RESPONSE back via workload_to_shell
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

                                                // Send via Workload → Shell channel
                                                let send_response_fut = response_tx.send_message(PayloadType::RpcReliable, None, response_envelope);
                                                #[cfg(feature = "opentelemetry")]
                                                let send_response_fut = send_response_fut.instrument(span.clone());
                                                if let Err(e) = send_response_fut.await {
                                                    tracing::error!(
                                                        severity = 7,
                                                        error_category = "transport_error",
                                                        request_id = %request_id,
                                                        "❌ Failed to send RESPONSE to Shell: {:?}", e
                                                    );
                                                }
                                            }
                                            Err(e) => {
                                                tracing::error!(
                                                    severity = 6,
                                                    error_category = "handler_error",
                                                    request_id = %request_id,
                                                    route_key = %envelope.route_key,
                                                    "❌ Workload message handling failed: {:?}", e
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
                                                if let Err(e) = send_error_response_fut.await {
                                                    tracing::error!(
                                                        severity = 7,
                                                        error_category = "transport_error",
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
                        ActrError::Unavailable(format!("Failed to get Shell receive lane: {e}"))
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
                                                        request_id = %envelope.request_id,
                                                        "⚠️  No pending request found for response: {:?}", e
                                                    );
                                                }
                                            }
                                            (None, Some(error)) => {
                                                // Error response - convert to ActrError and complete with error
                                                let actr_err = ActrError::Unavailable(
                                                    format!("RPC error {}: {}", error.code, error.message)
                                                );
                                                if let Err(e) = request_mgr.complete_error(&envelope.request_id, actr_err).await {
                                                    tracing::warn!(
                                                        severity = 4,
                                                        error_category = "orphan_response",
                                                        request_id = %envelope.request_id,
                                                        "⚠️  No pending request found for error response: {:?}", e
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
                                                #[cfg(feature = "opentelemetry")]
                                                let span = {
                                                        let span = tracing::info_span!("actrNode.mailbox.receive_rpc", request_id = %request_id);
                                                        set_parent_from_rpc_envelope(&span, &envelope);
                                                        span
                                                    };
                                                tracing::debug!("📦 Processing message: request_id={}", request_id);

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
                                                                        request_id,  // Reuse!
                                                                        route_key: envelope.route_key.clone(),
                                                                        payload: Some(response_bytes),
                                                                        error: None,
                                                                        traceparent: envelope.traceparent.clone(),
                                                                        tracestate: envelope.tracestate.clone(),
                                                                        metadata: Vec::new(),  // Response doesn't need extra metadata
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
                                                                            "❌ Failed to send response: {:?}", e
                                                                        );
                                                                    }
                                                                }
                                                                Err(e) => {
                                                                    tracing::error!(
                                                                        severity = 8,
                                                                        error_category = "protobuf_decode",
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
                                                    trace_id: format!("mailbox-{}", msg_record.id),  // Fallback trace_id
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
        // 5.5. Call lifecycle hook on_ready
        //      All components are up: signaling, WebRTC, heartbeat, mailbox loop.
        //      This is the true "Actor is ready to serve" moment.
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        tracing::info!("🪝 Calling lifecycle hook: on_ready");
        node_ref.workload.on_ready(&ctx).await?;
        tracing::info!("✅ Lifecycle hook on_ready completed");

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // 6. Create ActrRef for Shell to interact with Workload
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        use crate::actr_ref::{ActrRef, ActrRefShared};
        use crate::outbound::HostGate;

        // Create HostGate from shell_to_workload transport manager
        let shell_to_workload = node_ref
            .inproc_mgr
            .clone()
            .expect("inproc_mgr must be initialized");
        let inproc_gate = Arc::new(HostGate::new(shell_to_workload));

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use actr_framework::{Context, MessageDispatcher};
    use actr_protocol::{
        AIdCredential, ActorResult, ActrId, ActrType, Pong, Realm, RegisterRequest,
        RegisterResponse, RouteCandidatesRequest, RouteCandidatesResponse, RpcEnvelope,
        ServiceAvailabilityState, SignalingEnvelope, UnregisterResponse,
    };
    use actr_runtime_mailbox::{DeadLetterQueue, Mailbox, SqliteDeadLetterQueue, SqliteMailbox};
    use async_trait::async_trait;
    use prost_types::Timestamp;
    use tokio::sync::broadcast;
    use tokio_util::sync::CancellationToken;

    use crate::context_factory::ContextFactory;
    use crate::executor::{CallExecutorFn, DispatchContext, DispatchResult, ExecutorAdapter};
    use crate::inbound::{DataStreamRegistry, MediaFrameRegistry};
    use crate::outbound::{Gate, HostGate};
    use crate::transport::{HostTransport, NetworkError, NetworkResult};
    use crate::wire::webrtc::{SignalingClient, SignalingEvent, SignalingStats};

    struct NoopSignalingClient {
        event_tx: broadcast::Sender<SignalingEvent>,
    }

    impl NoopSignalingClient {
        fn new() -> Self {
            let (event_tx, _) = broadcast::channel(8);
            Self { event_tx }
        }
    }

    #[async_trait]
    impl SignalingClient for NoopSignalingClient {
        async fn connect(&self) -> NetworkResult<()> {
            Ok(())
        }

        async fn disconnect(&self) -> NetworkResult<()> {
            Ok(())
        }

        async fn send_register_request(
            &self,
            _request: RegisterRequest,
        ) -> NetworkResult<RegisterResponse> {
            Err(NetworkError::SignalingError(
                "unused in unit test".to_string(),
            ))
        }

        async fn send_unregister_request(
            &self,
            _actor_id: ActrId,
            _credential: AIdCredential,
            _reason: Option<String>,
        ) -> NetworkResult<UnregisterResponse> {
            Err(NetworkError::SignalingError(
                "unused in unit test".to_string(),
            ))
        }

        async fn send_heartbeat(
            &self,
            _actor_id: ActrId,
            _credential: AIdCredential,
            _availability: ServiceAvailabilityState,
            _power_reserve: f32,
            _mailbox_backlog: f32,
        ) -> NetworkResult<Pong> {
            Err(NetworkError::SignalingError(
                "unused in unit test".to_string(),
            ))
        }

        async fn send_route_candidates_request(
            &self,
            _actor_id: ActrId,
            _credential: AIdCredential,
            _request: RouteCandidatesRequest,
        ) -> NetworkResult<RouteCandidatesResponse> {
            Err(NetworkError::SignalingError(
                "unused in unit test".to_string(),
            ))
        }

        async fn get_signing_key(
            &self,
            _actor_id: ActrId,
            _credential: AIdCredential,
            _key_id: u32,
        ) -> NetworkResult<(u32, Vec<u8>)> {
            Err(NetworkError::SignalingError(
                "unused in unit test".to_string(),
            ))
        }

        async fn send_credential_update_request(
            &self,
            _actor_id: ActrId,
            _credential: AIdCredential,
        ) -> NetworkResult<RegisterResponse> {
            Err(NetworkError::SignalingError(
                "unused in unit test".to_string(),
            ))
        }

        async fn send_envelope(&self, _envelope: SignalingEnvelope) -> NetworkResult<()> {
            Err(NetworkError::SignalingError(
                "unused in unit test".to_string(),
            ))
        }

        async fn receive_envelope(&self) -> NetworkResult<Option<SignalingEnvelope>> {
            Err(NetworkError::SignalingError(
                "unused in unit test".to_string(),
            ))
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn get_stats(&self) -> SignalingStats {
            SignalingStats::default()
        }

        fn subscribe_events(&self) -> broadcast::Receiver<SignalingEvent> {
            self.event_tx.subscribe()
        }

        async fn set_actor_id(&self, _actor_id: ActrId) {}

        async fn set_credential_state(&self, _credential_state: CredentialState) {}

        async fn clear_identity(&self) {}
    }

    struct TestWorkload;

    #[async_trait]
    impl Workload for TestWorkload {
        type Dispatcher = TestDispatcher;
    }

    struct TestDispatcher;

    #[async_trait]
    impl MessageDispatcher for TestDispatcher {
        type Workload = TestWorkload;

        async fn dispatch<C: Context>(
            _workload: &Self::Workload,
            _envelope: RpcEnvelope,
            _ctx: &C,
        ) -> ActorResult<Bytes> {
            Err(ActrError::Internal(
                "native dispatch path should not be used in executor test".to_string(),
            ))
        }
    }

    #[derive(Clone, Default)]
    struct ExecutorProbe {
        active: Arc<AtomicUsize>,
        max_active: Arc<AtomicUsize>,
    }

    struct SerialProbeExecutor {
        probe: ExecutorProbe,
        delay: Duration,
    }

    impl SerialProbeExecutor {
        fn new(probe: ExecutorProbe) -> Self {
            Self {
                probe,
                delay: Duration::from_millis(25),
            }
        }
    }

    impl ExecutorAdapter for SerialProbeExecutor {
        fn dispatch<'a>(
            &'a mut self,
            _request_bytes: &[u8],
            _ctx: DispatchContext,
            _call_executor: &'a CallExecutorFn,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = DispatchResult> + Send + 'a>>
        {
            let probe = self.probe.clone();
            let delay = self.delay;
            Box::pin(async move {
                let active_now = probe.active.fetch_add(1, Ordering::SeqCst) + 1;
                probe.max_active.fetch_max(active_now, Ordering::SeqCst);

                tokio::time::sleep(delay).await;

                probe.active.fetch_sub(1, Ordering::SeqCst);
                Ok(b"ok".to_vec())
            })
        }
    }

    fn create_test_actor_id(serial_number: u64) -> ActrId {
        ActrId {
            serial_number,
            r#type: ActrType {
                manufacturer: "acme".to_string(),
                name: "test-actor".to_string(),
                version: "1.0.0".to_string(),
            },
            realm: Realm { realm_id: 7 },
        }
    }

    fn create_test_config() -> actr_config::Config {
        actr_config::Config {
            package: actr_config::PackageInfo {
                name: "test-actor".to_string(),
                actr_type: ActrType {
                    manufacturer: "acme".to_string(),
                    name: "test-actor".to_string(),
                    version: "1.0.0".to_string(),
                },
                description: None,
                authors: vec![],
                license: None,
            },
            exports: vec![],
            dependencies: vec![],
            signaling_url: url::Url::parse("ws://localhost:8081").unwrap(),
            ais_endpoint: url::Url::parse("http://localhost:8081/ais").unwrap(),
            realm: Realm { realm_id: 7 },
            realm_secret: None,
            visible_in_discovery: true,
            acl: None,
            mailbox_path: None,
            tags: vec![],
            scripts: HashMap::new(),
            webrtc: actr_config::WebRtcConfig::default(),
            websocket_listen_port: None,
            websocket_advertised_host: None,
            observability: actr_config::ObservabilityConfig {
                filter_level: "info".to_string(),
                tracing_enabled: false,
                tracing_endpoint: String::new(),
                tracing_service_name: "test-actor".to_string(),
            },
            config_dir: std::env::temp_dir(),
            execution_mode: actr_config::ActrMode::Native,
            ais_endpoint: None,
        }
    }

    async fn build_test_node_with_executor(
        executor: Box<dyn ExecutorAdapter>,
    ) -> ActrNode<TestWorkload> {
        let mailbox: Arc<dyn Mailbox> = Arc::new(SqliteMailbox::new(":memory:").await.unwrap());
        let dlq: Arc<dyn DeadLetterQueue> = Arc::new(
            SqliteDeadLetterQueue::new_standalone(":memory:")
                .await
                .unwrap(),
        );

        let shell_to_workload = Arc::new(HostTransport::new());
        let workload_to_shell = Arc::new(HostTransport::new());
        let inproc_gate = Gate::Host(Arc::new(HostGate::new(shell_to_workload.clone())));
        let signaling_client: Arc<dyn SignalingClient> = Arc::new(NoopSignalingClient::new());
        let context_factory = ContextFactory::new(
            inproc_gate,
            shell_to_workload.clone(),
            workload_to_shell.clone(),
            Arc::new(DataStreamRegistry::new()),
            Arc::new(MediaFrameRegistry::new()),
            signaling_client.clone(),
        );

        ActrNode {
            config: create_test_config(),
            workload: Arc::new(TestWorkload),
            mailbox,
            dlq,
            context_factory: Some(context_factory),
            signaling_client,
            actor_id: Some(create_test_actor_id(42)),
            credential_state: Some(CredentialState::new(create_test_credential(1), None, None)),
            webrtc_coordinator: None,
            webrtc_gate: None,
            websocket_gate: None,
            inproc_mgr: Some(shell_to_workload),
            workload_to_shell_mgr: Some(workload_to_shell),
            shutdown_token: CancellationToken::new(),
            actr_lock: None,
            network_event_rx: None,
            network_event_result_tx: None,
            network_event_debounce_config: None,
            dedup_state: Arc::new(Mutex::new(DedupState::new())),
            injected_registration: None,
            discovered_ws_addresses: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            executor: Some(Mutex::new(executor)),
        }
    }

    fn create_test_credential(key_id: u32) -> AIdCredential {
        AIdCredential {
            key_id,
            claims: vec![1, 2, 3, 4].into(),
            signature: vec![0u8; 64].into(),
        }
    }

    fn create_test_timestamp(seconds: i64) -> Timestamp {
        Timestamp { seconds, nanos: 0 }
    }

    #[tokio::test]
    async fn test_credential_state_initialization() {
        let credential = create_test_credential(1);
        let expires_at = Some(create_test_timestamp(1000));

        let state = CredentialState::new(credential.clone(), expires_at, None);

        let retrieved_credential = state.credential().await;
        assert_eq!(retrieved_credential.key_id, 1);
        assert_eq!(retrieved_credential.claims.as_ref(), &[1, 2, 3, 4]);

        let retrieved_expires_at = state.expires_at().await;
        assert_eq!(retrieved_expires_at, expires_at);
    }

    #[tokio::test]
    async fn test_credential_state_without_expiration() {
        let credential = create_test_credential(2);
        let state = CredentialState::new(credential.clone(), None, None);

        let retrieved_credential = state.credential().await;
        assert_eq!(retrieved_credential.key_id, 2);

        let retrieved_expires_at = state.expires_at().await;
        assert!(retrieved_expires_at.is_none());
    }

    #[tokio::test]
    async fn test_credential_state_update() {
        let credential1 = create_test_credential(1);
        let expires_at1 = Some(create_test_timestamp(1000));
        let state = CredentialState::new(credential1, expires_at1, None);

        // Verify initial state
        let initial_credential = state.credential().await;
        assert_eq!(initial_credential.key_id, 1);

        // Update credential
        let credential2 = create_test_credential(2);
        let expires_at2 = Some(create_test_timestamp(2000));
        state.update(credential2.clone(), expires_at2, None).await;

        // Verify updated state
        let updated_credential = state.credential().await;
        assert_eq!(updated_credential.key_id, 2);
        assert_eq!(updated_credential.claims, credential2.claims);

        let updated_expires_at = state.expires_at().await;
        assert_eq!(updated_expires_at, Some(create_test_timestamp(2000)));
    }

    #[tokio::test]
    async fn test_credential_state_concurrent_access() {
        let credential = create_test_credential(1);
        let expires_at = Some(create_test_timestamp(1000));
        let state = CredentialState::new(credential, expires_at, None);

        // Spawn multiple tasks that concurrently access the credential state
        let mut handles = vec![];
        for i in 0..10 {
            let state_clone = state.clone();
            let handle = tokio::spawn(async move {
                let cred = state_clone.credential().await;
                assert_eq!(cred.key_id, 1);
                i
            });
            handles.push(handle);
        }

        // Wait for all tasks to complete
        for handle in handles {
            let result = handle.await.unwrap();
            assert!(result < 10);
        }
    }

    #[tokio::test]
    async fn test_credential_state_update_concurrent() {
        let credential1 = create_test_credential(1);
        let state = CredentialState::new(credential1, None, None);

        // Spawn multiple update tasks
        let mut handles = vec![];
        for i in 2..12 {
            let state_clone = state.clone();
            let credential = create_test_credential(i);
            let handle = tokio::spawn(async move {
                state_clone.update(credential, None, None).await;
            });
            handles.push(handle);
        }

        // Wait for all updates to complete
        for handle in handles {
            handle.await.unwrap();
        }

        // Verify final state (should be the last update)
        let final_credential = state.credential().await;
        // The exact value depends on which update finished last, but it should be valid
        assert!(final_credential.key_id >= 2 && final_credential.key_id <= 11);
    }

    #[tokio::test]
    async fn test_dynamic_executor_dispatch_is_serialized() {
        let probe = ExecutorProbe::default();
        let node = Arc::new(
            build_test_node_with_executor(Box::new(SerialProbeExecutor::new(probe.clone()))).await,
        );

        let mut handles = Vec::new();
        for i in 0..8 {
            let node = node.clone();
            handles.push(tokio::spawn(async move {
                let envelope = RpcEnvelope {
                    route_key: "test.serialized.executor".to_string(),
                    payload: Some(Bytes::new()),
                    error: None,
                    traceparent: None,
                    tracestate: None,
                    request_id: format!("req-{i}"),
                    metadata: vec![],
                    timeout_ms: 0,
                };

                let response = node.handle_incoming(envelope, None).await.unwrap();
                assert_eq!(response.as_ref(), b"ok");
            }));
        }

        for handle in handles {
            handle.await.unwrap();
        }

        assert_eq!(
            probe.max_active.load(Ordering::SeqCst),
            1,
            "dynamic executor dispatch must be serialized per actor instance",
        );
        assert_eq!(probe.active.load(Ordering::SeqCst), 0);
    }
}
