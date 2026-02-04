//! ActrNode - ActrSystem + Workload (1:1 composition)

use crate::context_factory::ContextFactory;
use crate::lifecycle::compat_lock::{CompatLockManager, CompatibilityCheck};
use crate::transport::InprocTransportManager;
#[cfg(feature = "opentelemetry")]
use crate::wire::webrtc::trace::{inject_span_context_to_rpc, set_parent_from_rpc_envelope};
use actr_framework::{Bytes, Workload};
use actr_mailbox::{DeadLetterQueue, Mailbox};
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{
    AIdCredential, ActorResult, ActrId, ActrType, CandidateCompatibilityInfo, PayloadType,
    RegisterRequest, RouteCandidatesRequest, RpcEnvelope, register_response,
    route_candidates_request,
};
use futures_util::FutureExt;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
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

/// Result of a service discovery request with compatibility information.
///
/// This struct is returned by `discover_route_candidates` and provides
/// detailed information about the compatibility status when fingerprint-based
/// discovery is used.
#[derive(Debug, Clone)]
pub struct DiscoveryResult {
    /// Ordered list of compatible candidates (best match first)
    pub candidates: Vec<ActrId>,
    /// True if at least one candidate has an exact fingerprint match
    pub has_exact_match: bool,
    /// True if system is in sub-healthy state (compatible but not exact match)
    pub is_sub_healthy: bool,
    /// Detailed compatibility info for each candidate (when fingerprint was provided)
    pub compatibility_info: Vec<CandidateCompatibilityInfo>,
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

    /// Pre-shared key for TURN authentication (obtained from registration)
    pub(crate) psk: Option<Bytes>,

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
    /// This is updated together with credential when credential is refreshed
    psk: Option<Bytes>,
}

impl CredentialState {
    /// Create a new CredentialState with PSK
    pub(crate) fn new(
        credential: AIdCredential,
        expires_at: Option<prost_types::Timestamp>,
        psk: Option<Bytes>,
    ) -> Self {
        Self {
            inner: Arc::new(RwLock::new(CredentialStateInner {
                credential,
                expires_at,
                psk,
            })),
        }
    }

    pub async fn credential(&self) -> AIdCredential {
        self.inner.read().await.credential.clone()
    }

    pub async fn expires_at(&self) -> Option<prost_types::Timestamp> {
        self.inner.read().await.expires_at
    }

    /// Get the PSK for TURN authentication
    pub async fn psk(&self) -> Option<Bytes> {
        self.inner.read().await.psk.clone()
    }

    /// Update credential along with PSK
    /// This should be called when credential is refreshed and a new PSK is provided
    pub(crate) async fn update(
        &self,
        credential: AIdCredential,
        expires_at: Option<prost_types::Timestamp>,
        psk: Option<Bytes>,
    ) {
        let mut guard = self.inner.write().await;
        guard.credential = credential;
        guard.expires_at = expires_at;
        if psk.is_some() {
            guard.psk = psk;
        }
    }
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
/// 4. Iterate through ACL rules in order (first match wins)
///    - Check if caller matches any principal in the rule
///    - If matched, return the rule's permission (ALLOW/DENY)
/// 5. If no rule matches, deny by default (secure by default)
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
            "ACL: ACL configured but no rules defined, denying {} -> {} (default deny)",
            caller.to_string_repr(),
            target_id.to_string_repr()
        );
        return Ok(false);
    }

    // 4. Iterate through ACL rules (first match wins)
    for (rule_idx, rule) in acl_rules.rules.iter().enumerate() {
        // Check if caller matches any principal in this rule
        let mut matched = false;

        // If no principals specified, skip this rule (empty allow list = no match)
        if rule.principals.is_empty() {
            tracing::trace!(
                "ACL: Rule {} has empty principals list, skipping (no match)",
                rule_idx
            );
            continue;
        }

        // Check each principal
        for principal in &rule.principals {
            if matches_principal(caller, principal) {
                matched = true;
                tracing::trace!(
                    "ACL: Rule {} matched principal: caller={}, principal_realm={:?}, principal_type={:?}",
                    rule_idx,
                    caller.to_string_repr(),
                    principal.realm.as_ref().map(|r| r.realm_id),
                    principal.actr_type.as_ref().map(|t| &t.name)
                );
                break;
            }
        }

        // If matched, return the permission
        if matched {
            let permission = rule.permission;
            let is_allow = permission == actr_protocol::acl_rule::Permission::Allow as i32;

            tracing::debug!(
                "ACL: Rule {} matched, permission={} for {} -> {}",
                rule_idx,
                if is_allow { "ALLOW" } else { "DENY" },
                caller.to_string_repr(),
                target_id.to_string_repr()
            );

            return Ok(is_allow);
        }
    }

    // 5. No rule matched - deny by default (secure by default)
    tracing::warn!(
        "ACL: No matching rule found, denying {} -> {} (default deny)",
        caller.to_string_repr(),
        target_id.to_string_repr()
    );
    Ok(false)
}

/// Check if a caller matches a principal
///
/// A principal matches if:
/// - If principal.realm is specified, it must match caller.realm
/// - If principal.actr_type is specified, it must match caller.type
/// - If both are None, principal matches all (should not happen in practice)
fn matches_principal(caller: &ActrId, principal: &actr_protocol::acl_rule::Principal) -> bool {
    // Check realm match (if specified)
    if let Some(ref principal_realm) = principal.realm
        && caller.realm.realm_id != principal_realm.realm_id
    {
        return false;
    }

    // Check type match (if specified)
    if let Some(ref principal_type) = principal.actr_type
        && (caller.r#type.manufacturer != principal_type.manufacturer
            || caller.r#type.name != principal_type.name)
    {
        return false;
    }

    // If we reach here, all specified fields matched
    true
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
    /// This method implements the full runtime compatibility negotiation workflow
    /// as specified in the documentation:
    ///
    /// # Compatibility Negotiation Flow
    ///
    /// 1. **Step 0: Fast Path (compat.lock.toml)**
    ///    - Check if `compat.lock.toml` has a cached negotiation for this service
    ///    - If found and not expired, use the cached `resolved_fingerprint` directly
    ///
    /// 2. **Step 1: Ideal Path (Exact Match)**
    ///    - Read the expected fingerprint from `Actr.lock.toml`
    ///    - Request exact match from signaling server
    ///    - If found → connection success, system is HEALTHY
    ///
    /// 3. **Step 2: Trigger Negotiation (Match Failure)**
    ///    - If no exact match, enter compatibility negotiation mode
    ///
    /// 4. **Step 3: Compatibility Check (Server-side)**
    ///    - Server performs backward compatibility analysis using proto-sign
    ///
    /// 5. **Step 4: Decision**
    ///    - **Success**: Found compatible version → SUB-HEALTHY state
    ///      - Update `compat.lock.toml` with negotiation result
    ///      - Log warning: "SYSTEM SUB-HEALTHY"
    ///    - **Failure**: No compatible version → FAILED state
    ///      - Log error: "SYSTEM FAILED"
    ///
    /// # Arguments
    /// - `target_type`: The ActrType of the target service to discover
    /// - `candidate_count`: Maximum number of candidates to return
    ///
    /// # Returns
    /// A `DiscoveryResult` containing candidates and compatibility information
    #[cfg_attr(feature = "opentelemetry", tracing::instrument(skip_all))]
    pub async fn discover_route_candidates(
        &self,
        target_type: &ActrType,
        candidate_count: u32,
    ) -> ActorResult<DiscoveryResult> {
        // Check if node is started (has actor_id and credential)
        let actor_id = self.actor_id.as_ref().ok_or_else(|| {
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

        let service_name = format!("{}/{}", target_type.manufacturer, target_type.name);

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // Step 0: Fast Path - Check compat.lock.toml for cached negotiation
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        let mut compat_lock_manager = CompatLockManager::new(self.config.config_dir.clone());
        if let Ok(Some(compat_lock)) = compat_lock_manager.load().await {
            if let Some(cached_entry) = compat_lock.find_valid_entry(&service_name) {
                tracing::info!(
                    "⚡ Fast path: Using cached negotiation for '{}' (resolved: {})",
                    service_name,
                    &cached_entry.resolved_fingerprint
                        [..20.min(cached_entry.resolved_fingerprint.len())]
                );

                // Use the cached resolved_fingerprint to find candidates
                let result = self
                    .send_discovery_request(
                        actor_id,
                        target_type,
                        candidate_count,
                        cached_entry.resolved_fingerprint.clone(),
                    )
                    .await?;

                if !result.candidates.is_empty() {
                    tracing::info!(
                        "📊 服务发现结果 [{}]: {} 个候选 (快速路径, sub_healthy=true)",
                        service_name,
                        result.candidates.len()
                    );
                    return Ok(DiscoveryResult {
                        candidates: result.candidates,
                        has_exact_match: false, // Cached negotiation means not exact
                        is_sub_healthy: true,   // Using compat.lock means sub-healthy
                        compatibility_info: result.compatibility_info,
                    });
                }
                // If fast path fails, fall through to normal discovery
                tracing::warn!(
                    "⚠️ Fast path failed for '{}', falling back to normal discovery",
                    service_name
                );
            }
        }

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // Step 1: Get fingerprint from Actr.lock.toml (REQUIRED)
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        let client_fingerprint = self.get_dependency_fingerprint(target_type).ok_or_else(|| {
            tracing::error!(
                severity = 10,
                error_category = "dependency_missing",
                "❌ DEPENDENCY NOT FOUND: Service '{}' is not declared in Actr.lock.toml.\n\
                 Please run 'actr install' to generate the lock file with all dependencies.",
                service_name
            );
            actr_protocol::ProtocolError::Actr(actr_protocol::ActrError::DependencyNotFound {
                service_name: service_name.clone(),
                message: format!(
                    "Dependency '{}' not found in Actr.lock.toml. Run 'actr install' to resolve dependencies.",
                    service_name
                ),
            })
        })?;

        tracing::debug!(
            "📋 Found dependency fingerprint for '{}': {}",
            service_name,
            &client_fingerprint[..20.min(client_fingerprint.len())]
        );

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // Step 2: Send discovery request to signaling server
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        let result = self
            .send_discovery_request(
                actor_id,
                target_type,
                candidate_count,
                client_fingerprint.clone(),
            )
            .await?;

        let has_exact_match = result.has_exact_match;
        let is_sub_healthy = result.is_sub_healthy;

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // Step 3 & 4: Handle negotiation result
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        self.handle_negotiation_result(
            target_type,
            &client_fingerprint,
            &result.compatibility_info,
            has_exact_match,
            is_sub_healthy,
        )
        .await;

        // Log result
        tracing::info!(
            "📊 服务发现结果 [{}]: {} 个候选, exact_match={}, sub_healthy={}",
            service_name,
            result.candidates.len(),
            has_exact_match,
            is_sub_healthy
        );

        Ok(DiscoveryResult {
            candidates: result.candidates,
            has_exact_match,
            is_sub_healthy,
            compatibility_info: result.compatibility_info,
        })
    }

    /// Get dependency fingerprint from Actr.lock.toml
    fn get_dependency_fingerprint(&self, target_type: &ActrType) -> Option<String> {
        let actr_lock = self.actr_lock.as_ref()?;

        // Try different name formats to find the dependency
        let service_name = format!("{}/{}", target_type.manufacturer, target_type.name);
        let actr_type_name = format!("{}+{}", target_type.manufacturer, target_type.name);

        // First try by service name
        if let Some(dep) = actr_lock.get_dependency(&service_name) {
            return Some(dep.fingerprint.clone());
        }

        // Try by actr_type format
        if let Some(dep) = actr_lock.get_dependency(&actr_type_name) {
            return Some(dep.fingerprint.clone());
        }

        // Try by just the name part
        if let Some(dep) = actr_lock.get_dependency(&target_type.name) {
            return Some(dep.fingerprint.clone());
        }

        // Search through all dependencies by actr_type field
        for dep in &actr_lock.dependencies {
            if dep.actr_type == actr_type_name || dep.actr_type == target_type.name {
                return Some(dep.fingerprint.clone());
            }
        }

        None
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
            actr_protocol::ProtocolError::InvalidStateTransition(
                "Node is not started. Call start() first.".to_string(),
            )
        })?;

        let route_response = client
            .send_route_candidates_request(
                actor_id.clone(),
                credential_state.credential().await,
                route_request,
            )
            .await
            .map_err(|e| {
                actr_protocol::ProtocolError::TransportError(format!(
                    "Route candidates request failed: {e}"
                ))
            })?;

        match route_response.result {
            Some(actr_protocol::route_candidates_response::Result::Success(success)) => {
                Ok(DiscoveryResult {
                    candidates: success.candidates,
                    has_exact_match: success.has_exact_match.unwrap_or(false),
                    is_sub_healthy: success.is_sub_healthy.unwrap_or(false),
                    compatibility_info: success.compatibility_info,
                })
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

    /// Internal: Handle negotiation result - log warnings and update compat.lock.toml
    async fn handle_negotiation_result(
        &self,
        target_type: &ActrType,
        client_fingerprint: &str,
        compatibility_info: &[CandidateCompatibilityInfo],
        has_exact_match: bool,
        is_sub_healthy: bool,
    ) {
        let service_name = format!("{}/{}", target_type.manufacturer, target_type.name);

        // Log detailed compatibility info
        tracing::info!(
            "📊 服务发现结果 [{}]: {} 个候选, exact_match={}, sub_healthy={}",
            service_name,
            compatibility_info.len(),
            has_exact_match,
            is_sub_healthy
        );

        for info in compatibility_info {
            let status = if info.is_exact_match.unwrap_or(false) {
                "✅ 精确匹配"
            } else if let Some(ref result) = info.analysis_result {
                match result.level() {
                    actr_protocol::CompatibilityLevel::FullyCompatible => "✅ 完全兼容",
                    actr_protocol::CompatibilityLevel::BackwardCompatible => "⚠️ 向后兼容",
                    actr_protocol::CompatibilityLevel::BreakingChanges => "❌ 破坏性变更",
                }
            } else {
                "❓ 未知"
            };

            tracing::debug!(
                "   - 候选 {}: {} (指纹: {})",
                info.candidate_id.serial_number,
                status,
                &info.candidate_fingerprint[..20.min(info.candidate_fingerprint.len())]
            );
        }

        // Handle sub-healthy state
        if is_sub_healthy && !has_exact_match {
            // Find the first compatible (non-exact) match for logging
            if let Some(resolved) = compatibility_info.first() {
                tracing::warn!(
                    "🟡 SYSTEM SUB-HEALTHY: Service '{}' using compatible fingerprint ({}) \
                     instead of exact match ({}). Run 'actr install --force-update' to restore health.",
                    service_name,
                    &resolved.candidate_fingerprint[..20.min(resolved.candidate_fingerprint.len())],
                    &client_fingerprint[..20.min(client_fingerprint.len())]
                );

                // Update compat.lock.toml
                let mut manager = CompatLockManager::new(self.config.config_dir.clone());
                if let Err(e) = manager
                    .record_negotiation(
                        &service_name,
                        client_fingerprint,
                        &resolved.candidate_fingerprint,
                        false, // not exact match
                        CompatibilityCheck::BackwardCompatible,
                    )
                    .await
                {
                    tracing::warn!("Failed to update compat.lock.toml: {}", e);
                }
            }
        } else if has_exact_match {
            // Exact match found - try to clean up compat.lock.toml entry if exists
            let mut manager = CompatLockManager::new(self.config.config_dir.clone());
            if let Ok(Some(_)) = manager.load().await {
                if let Some(resolved) = compatibility_info.first() {
                    if let Err(e) = manager
                        .record_negotiation(
                            &service_name,
                            client_fingerprint,
                            &resolved.candidate_fingerprint,
                            true, // exact match
                            CompatibilityCheck::ExactMatch,
                        )
                        .await
                    {
                        tracing::debug!("Could not update compat.lock.toml: {}", e);
                    }
                }
            }
        }
    }
    /// 网络事件处理循环（后台任务）
    ///
    /// # 职责
    /// - 从 Channel 接收网络事件
    /// - 委托给 NetworkEventProcessor 处理
    /// - 记录处理时间并发送结果
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
                // 接收网络事件
                Some(event) = event_rx.recv() => {
                    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
                    // 队列清理：按类型去重，保留每种类型的最新事件
                    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

                    // 收集队列中的所有事件
                    let mut all_events = vec![event];
                    while let Ok(next_event) = event_rx.try_recv() {
                        all_events.push(next_event);
                    }

                    let total_events = all_events.len();

                    // 按类型去重
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

                    // 处理去重后的事件
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

                        // 构造处理结果
                        let event_result = match result {
                            Ok(_) => NetworkEventResult::success(latest_event.clone(), duration_ms),
                            Err(e) => NetworkEventResult::failure(latest_event.clone(), e, duration_ms),
                        };

                        // 发送结果（忽略发送失败，避免阻塞）
                        if let Err(e) = result_tx.send(event_result).await {
                            tracing::warn!("Failed to send event result: {}", e);
                        }
                    }
                }

                // 监听关闭信号
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
        use actr_framework::MessageDispatcher;

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
            actr_protocol::ProtocolError::InvalidStateTransition(
                "Actor ID not set - node must be started before handling messages".to_string(),
            )
        })?;

        // 0.1. ACL Permission Check (before processing message)
        let acl_allowed = check_acl_permission(caller_id, actor_id, self.config.acl.as_ref())
            .map_err(|err_msg| {
                actr_protocol::ProtocolError::TransportError(format!(
                    "ACL check failed: {}",
                    err_msg
                ))
            })?;

        if !acl_allowed {
            tracing::warn!(
                severity = 5,
                error_category = "acl_denied",
                request_id = %envelope.request_id,
                route_key = %envelope.route_key,
                caller = %caller_id.map(|c| c.to_string_repr()).unwrap_or_else(|| "<none>".to_string()),
                "🚫 ACL: Permission denied"
            );

            return Err(actr_protocol::ProtocolError::Actr(
                actr_protocol::ActrError::PermissionDenied {
                    message: format!(
                        "ACL denied: {} is not allowed to call {}",
                        caller_id
                            .map(|c| c.to_string_repr())
                            .unwrap_or_else(|| "<unknown>".to_string()),
                        actor_id.to_string_repr()
                    ),
                },
            ));
        }

        // 1. Create Context with caller_id from transport layer
        let credential_state = self.credential_state.clone().ok_or_else(|| {
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
                &envelope.request_id,
                &credential_state.credential().await,
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
                    request_id = %envelope.request_id,
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
                let credential_state = CredentialState::new(
                    credential,
                    register_ok.credential_expires_at,
                    register_ok.psk.clone(),
                );
                self.credential_state = Some(credential_state.clone());

                // Pass identity to signaling client so reconnect URLs carry auth info.
                self.signaling_client.set_actor_id(actor_id.clone()).await;
                self.signaling_client
                    .set_credential_state(credential_state.clone())
                    .await;
                // Store PSK and public_key for TURN authentication
                self.psk = register_ok.psk.clone();

                // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
                // 1.2. Set actr_lock in ContextFactory for fingerprint lookups
                // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
                if let Some(actr_lock) = self.actr_lock.clone() {
                    self.context_factory
                        .as_mut()
                        .expect("ContextFactory must exist")
                        .set_actr_lock(actr_lock);
                    tracing::info!(
                        "✅ Actr.lock.toml set in ContextFactory for fingerprint lookups"
                    );
                }

                // Set config_dir in ContextFactory for compat.lock.toml Fast Path
                self.context_factory
                    .as_mut()
                    .expect("ContextFactory must exist")
                    .set_config_dir(self.config.config_dir.clone());
                tracing::info!(
                    "✅ config_dir set in ContextFactory for compat.lock.toml Fast Path"
                );

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
                        credential_state.clone(),
                        self.signaling_client.clone(),
                        self.config.webrtc.clone(),
                        self.config.realm.realm_id.clone(),
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

                    // Use interval from registration response, default to 30s
                    let heartbeat_interval_secs = register_ok.signaling_heartbeat_interval_secs;
                    let heartbeat_interval = if heartbeat_interval_secs > 0 {
                        Duration::from_secs(heartbeat_interval_secs as u64)
                    } else {
                        Duration::from_secs(30)
                    };

                    let heartbeat_handle = tokio::spawn(heartbeat_task(
                        shutdown,
                        client,
                        actor_id_for_heartbeat,
                        credential_state_for_heartbeat,
                        mailbox_for_heartbeat,
                        heartbeat_interval,
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

                    // 创建 DefaultNetworkEventProcessor
                    // 如果有防抖配置，使用 new_with_debounce
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
                        Self::network_event_loop(event_rx, result_tx, event_processor, shutdown)
                            .await;
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
        let credential_state = self.credential_state.clone().ok_or_else(|| {
            actr_protocol::ProtocolError::InvalidStateTransition(
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
                "bootstrap", // request_id
                &credential_state.credential().await,
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

#[cfg(test)]
mod tests {
    use super::*;
    use actr_protocol::AIdCredential;
    use prost_types::Timestamp;

    fn create_test_credential(token_key_id: u32) -> AIdCredential {
        AIdCredential {
            encrypted_token: vec![1, 2, 3, 4].into(),
            token_key_id,
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
        assert_eq!(retrieved_credential.token_key_id, 1);
        assert_eq!(retrieved_credential.encrypted_token.as_ref(), &[1, 2, 3, 4]);

        let retrieved_expires_at = state.expires_at().await;
        assert_eq!(retrieved_expires_at, expires_at);
    }

    #[tokio::test]
    async fn test_credential_state_without_expiration() {
        let credential = create_test_credential(2);
        let state = CredentialState::new(credential.clone(), None, None);

        let retrieved_credential = state.credential().await;
        assert_eq!(retrieved_credential.token_key_id, 2);

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
        assert_eq!(initial_credential.token_key_id, 1);

        // Update credential
        let credential2 = create_test_credential(2);
        let expires_at2 = Some(create_test_timestamp(2000));
        state.update(credential2.clone(), expires_at2, None).await;

        // Verify updated state
        let updated_credential = state.credential().await;
        assert_eq!(updated_credential.token_key_id, 2);
        assert_eq!(
            updated_credential.encrypted_token,
            credential2.encrypted_token
        );

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
                assert_eq!(cred.token_key_id, 1);
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
        assert!(final_credential.token_key_id >= 2 && final_credential.token_key_id <= 11);
    }
}
