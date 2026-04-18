//! Business message dispatch layer
//!
//! `ActrDispatch` is the core struct of the new actr-runtime, responsible for:
//! 1. ACL permission checking
//! 2. Route key -> Workload handler static dispatch
//! 3. Handler panic capture and reporting
//! 4. Lifecycle hook delegation (on_start / on_stop)
//!
//! This module contains **no** IO, network, or transport logic,
//! and can be compiled and run on both native and wasm32 targets.

use std::sync::Arc;

use actr_framework::{Context, ErrorCategory, ErrorEvent, MessageDispatcher, Workload};
use actr_protocol::{Acl, ActorResult, ActrError, ActrId, ActrIdExt as _, RpcEnvelope};
use bytes::Bytes;
use futures_util::FutureExt as _;

use crate::acl::check_acl_permission;

/// Pure business dispatcher
///
/// Holds an `Arc<W>` workload instance and optional ACL rules,
/// exposing `dispatch()` and lifecycle methods.
pub struct ActrDispatch<W: Workload> {
    workload: Arc<W>,
    acl: Option<Acl>,
}

impl<W: Workload> ActrDispatch<W> {
    /// Create a dispatcher
    ///
    /// # Arguments
    /// - `workload`: Business Workload instance (wrapped in `Arc`)
    /// - `acl`: Optional ACL rule set; `None` means all calls are allowed by default
    pub fn new(workload: Arc<W>, acl: Option<Acl>) -> Self {
        Self { workload, acl }
    }

    /// Get Workload reference
    pub fn workload(&self) -> &W {
        &self.workload
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // Lifecycle
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    /// Forward on_start lifecycle hook
    pub async fn on_start<C: Context>(&self, ctx: &C) -> ActorResult<()> {
        self.workload.on_start(ctx).await
    }

    /// Forward on_stop lifecycle hook
    pub async fn on_stop<C: Context>(&self, ctx: &C) -> ActorResult<()> {
        self.workload.on_stop(ctx).await
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // Message dispatch
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    /// Dispatch inbound message: ACL check -> routing -> handler execution
    ///
    /// # Arguments
    /// - `self_id`: Current Actor's ID
    /// - `caller_id`: Caller ID (`None` for local calls)
    /// - `envelope`: RPC envelope (contains route_key and payload)
    /// - `ctx`: Execution context (generic, provided by upper layer)
    ///
    /// # Returns
    /// Serialized response bytes, or `ActrError`
    pub async fn dispatch<C: Context>(
        &self,
        self_id: &ActrId,
        caller_id: Option<&ActrId>,
        envelope: RpcEnvelope,
        ctx: &C,
    ) -> ActorResult<Bytes> {
        // -- ACL check --
        let allowed = check_acl_permission(caller_id, self_id, self.acl.as_ref())
            .map_err(|e| ActrError::Internal(format!("ACL check failed: {e}")))?;

        if !allowed {
            tracing::warn!(
                severity = 5,
                error_category = "acl_denied",
                request_id = %envelope.request_id,
                route_key = %envelope.route_key,
                "ACL: permission denied",
            );
            return Err(ActrError::PermissionDenied(format!(
                "ACL denied: {} -> {}",
                caller_id
                    .map(|c| c.to_string_repr())
                    .unwrap_or_else(|| "<unknown>".into()),
                self_id.to_string_repr(),
            )));
        }

        // -- Static dispatch + panic capture --
        self.do_dispatch(envelope, ctx).await
    }

    /// Internal dispatch: call `MessageDispatcher::dispatch`, capture handler panics
    async fn do_dispatch<C: Context>(&self, envelope: RpcEnvelope, ctx: &C) -> ActorResult<Bytes> {
        let route_key = envelope.route_key.clone();
        let request_id = envelope.request_id.clone();

        let result =
            std::panic::AssertUnwindSafe(W::Dispatcher::dispatch(&self.workload, envelope, ctx))
                .catch_unwind()
                .await;

        match result {
            Ok(r) => r,
            Err(panic_payload) => {
                let info = extract_panic_info(panic_payload);
                tracing::error!(
                    severity = 8,
                    error_category = "handler_panic",
                    route_key = %route_key,
                    request_id = %request_id,
                    "handler panicked: {}", info,
                );
                // Notify workload's on_error hook
                let event = ErrorEvent::now(
                    ActrError::Internal(format!("handler panicked: {info}")),
                    ErrorCategory::HandlerPanic,
                    format!("route_key={route_key} request_id={request_id}"),
                );
                let _ = self.workload.on_error(ctx, &event).await;
                Err(ActrError::DecodeFailure(format!(
                    "handler panicked: {info}"
                )))
            }
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Utility functions
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Extract a readable string from a panic payload
fn extract_panic_info(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic>".to_string()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// trait impls
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

impl<W: Workload> Clone for ActrDispatch<W> {
    fn clone(&self) -> Self {
        Self {
            workload: Arc::clone(&self.workload),
            acl: self.acl.clone(),
        }
    }
}

impl<W: Workload> std::fmt::Debug for ActrDispatch<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActrDispatch")
            .field("has_acl", &self.acl.is_some())
            .finish()
    }
}
