//! Runtime-side workload hook plumbing.
//!
//! The user-facing [`actr_framework::Workload`] trait is **not** object-safe
//! (it carries an associated `Dispatcher` type and generic `<C: Context>`
//! methods), so `Arc<dyn Workload>` is not representable. The node still
//! needs a way to dispatch observation events (signaling / transport /
//! credential / mailbox) into whatever workload the shell is hosting
//! *without* holding the dispatch Mutex.
//!
//! This module bridges the gap by defining [`WorkloadHookObserver`] — an
//! object-safe counterpart of the framework's observation hooks — that can
//! be stored as `Option<Arc<dyn WorkloadHookObserver>>` on the running
//! node. Event sources (signaling client, WebRTC coordinator, WebSocket
//! gate, mailbox loop, credential renewal) call into the observer through
//! [`spawn_hook`], which wraps the call in `AssertUnwindSafe` + async
//! `catch_unwind` so a panicking observer cannot take the node down with it.
//!
//! The framework's built-in tracing defaults still fire regardless of
//! whether an observer is installed — they are invoked by the event-source
//! wire-up sites directly via the existing `HookCallback` plumbing.

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use actr_framework::{BackpressureEvent, CredentialEvent, ErrorEvent, PeerEvent};
use async_trait::async_trait;
use futures_util::FutureExt as _;

use crate::context::RuntimeContext;

/// Object-safe observer that mirrors the observation hooks defined on
/// [`actr_framework::Workload`] but uses the concrete [`RuntimeContext`]
/// and trait objects throughout so it can live behind an `Arc`.
///
/// Hyper wires this observer up from an external adapter (e.g. the FFI
/// `DynamicWorkload`). Each method has a no-op default so adopters can
/// override only the hooks they care about.
#[async_trait]
#[allow(dead_code)]
pub(crate) trait WorkloadHookObserver: Send + Sync + 'static {
    // Lifecycle (fallible — but in hook path we always swallow Err after
    // logging since the trait-object boundary erases the error semantics
    // the user-facing framework `Workload` trait offers).
    async fn on_start(&self, _ctx: &RuntimeContext) {}
    async fn on_ready(&self, _ctx: &RuntimeContext) {}
    async fn on_stop(&self, _ctx: &RuntimeContext) {}
    async fn on_error(&self, _ctx: &RuntimeContext, _event: &ErrorEvent) {}

    // Signaling
    async fn on_signaling_connecting(&self, _ctx: Option<&RuntimeContext>) {}
    async fn on_signaling_connected(&self, _ctx: Option<&RuntimeContext>) {}
    async fn on_signaling_disconnected(&self, _ctx: &RuntimeContext) {}

    // WebSocket C/S
    async fn on_websocket_connecting(&self, _ctx: &RuntimeContext, _event: &PeerEvent) {}
    async fn on_websocket_connected(&self, _ctx: &RuntimeContext, _event: &PeerEvent) {}
    async fn on_websocket_disconnected(&self, _ctx: &RuntimeContext, _event: &PeerEvent) {}

    // WebRTC P2P
    async fn on_webrtc_connecting(&self, _ctx: &RuntimeContext, _event: &PeerEvent) {}
    async fn on_webrtc_connected(&self, _ctx: &RuntimeContext, _event: &PeerEvent) {}
    async fn on_webrtc_disconnected(&self, _ctx: &RuntimeContext, _event: &PeerEvent) {}

    // Credential
    async fn on_credential_renewed(&self, _ctx: &RuntimeContext, _event: &CredentialEvent) {}
    async fn on_credential_expiring(&self, _ctx: &RuntimeContext, _event: &CredentialEvent) {}

    // Mailbox
    async fn on_mailbox_backpressure(
        &self,
        _ctx: &RuntimeContext,
        _event: &BackpressureEvent,
    ) {
    }
}

/// Shared observer handle held by the running node.
pub(crate) type WorkloadHookObserverRef = Arc<dyn WorkloadHookObserver>;

/// Run a workload-hook invocation in a detached task with panic isolation.
///
/// Any panic raised by the observer is caught and logged at
/// `tracing::error`; the node is never taken down by a misbehaving hook.
/// Returns immediately; the hook body runs on a spawned Tokio task so hot
/// event-source code paths are not blocked by slow observers.
#[allow(dead_code)]
pub(crate) fn spawn_hook<F>(label: &'static str, fut: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        match AssertUnwindSafe(fut).catch_unwind().await {
            Ok(()) => {}
            Err(panic_payload) => {
                let info = extract_panic_info(panic_payload);
                tracing::error!(
                    hook = label,
                    panic = %info,
                    "workload hook panicked; isolated",
                );
            }
        }
    });
}

fn extract_panic_info(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic>".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_hook_survives_panic() {
        spawn_hook("test", async {
            panic!("intentional");
        });
        // Give the spawned task a chance to run.
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        // If we got here without aborting, the panic was isolated.
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_hook_runs_clean_body() {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        spawn_hook("test", async move {
            let _ = tx.send(());
        });
        tokio::time::timeout(std::time::Duration::from_secs(1), rx)
            .await
            .expect("hook did not run")
            .expect("sender dropped");
    }
}
