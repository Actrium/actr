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
use std::pin::Pin;
use std::sync::Arc;

use actr_framework::{BackpressureEvent, CredentialEvent, ErrorEvent, PeerEvent};
use async_trait::async_trait;
use futures_util::FutureExt as _;

use crate::context::RuntimeContext;
use crate::wire::webrtc::{HookCallback, HookEvent};

/// Object-safe observer that mirrors the observation hooks defined on
/// [`actr_framework::Workload`] but uses the concrete [`RuntimeContext`]
/// and trait objects throughout so it can live behind an `Arc`.
///
/// Hyper wires this observer up from an external adapter (e.g. the FFI
/// `DynamicWorkload`). Each method has a no-op default so adopters can
/// override only the hooks they care about.
///
/// This trait is the object-safe surface behind
/// [`crate::LinkedWorkloadHandle`] — the linked-workload handle is a
/// `WorkloadHookObserver` plus (at a future point) a dispatch method. The
/// current `Node<Init>::attach_linked_handle` entry point stores the
/// handle as this hook observer; inbound RPC dispatch into a
/// linked-workload guest is not yet implemented (linked nodes are
/// client-only).
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
    async fn on_mailbox_backpressure(&self, _ctx: &RuntimeContext, _event: &BackpressureEvent) {}
}

/// Shared observer handle held by the running node.
pub(crate) type WorkloadHookObserverRef = Arc<dyn WorkloadHookObserver>;

/// Future type produced by a [`HookContextBuilder`].
pub(crate) type HookContextFut = Pin<Box<dyn Future<Output = Option<RuntimeContext>> + Send>>;

/// Lazy builder that produces a `RuntimeContext` (or `None`, when the node
/// does not yet have an identity) used by hook callbacks to invoke the
/// observer trait methods.
pub(crate) type HookContextBuilder = Arc<dyn Fn() -> HookContextFut + Send + Sync + 'static>;

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

/// Build a [`HookCallback`] that logs framework tracing defaults for every
/// emitted [`HookEvent`] and, when an observer is installed, forwards the
/// event into the observer with panic isolation (via `spawn_hook`).
///
/// The event-source wiring (`WebSocketSignalingClient`,
/// `WebRtcCoordinator`, `WebSocketGate`, mailbox loop, credential flow)
/// installs the returned closure via `set_hook_callback` so that every
/// state change produces a structured tracing record at the appropriate
/// level regardless of whether a user observer is plugged in.
///
/// `ctx_builder` lazily constructs the `RuntimeContext` needed by
/// observer callbacks; for initial-connection signaling events (where the
/// node has not yet acquired an identity) callers should return `None`.
pub(crate) fn build_hook_callback(
    observer: Option<WorkloadHookObserverRef>,
    ctx_builder: HookContextBuilder,
) -> HookCallback {
    Arc::new(move |event: HookEvent| {
        let observer = observer.clone();
        let ctx_builder = ctx_builder.clone();
        Box::pin(async move {
            // Always log the framework tracing default for the event.
            log_hook_event(&event);

            // If an observer is installed, forward with panic isolation.
            let Some(observer) = observer else {
                return;
            };

            let ctx_opt = ctx_builder().await;

            match event {
                HookEvent::SignalingConnectStart { .. } => {
                    let label = "on_signaling_connecting";
                    let observer = observer.clone();
                    spawn_hook(label, async move {
                        observer.on_signaling_connecting(ctx_opt.as_ref()).await;
                    });
                }
                HookEvent::SignalingConnected => {
                    let label = "on_signaling_connected";
                    let observer = observer.clone();
                    spawn_hook(label, async move {
                        observer.on_signaling_connected(ctx_opt.as_ref()).await;
                    });
                }
                HookEvent::SignalingDisconnected => {
                    let label = "on_signaling_disconnected";
                    if let Some(ctx) = ctx_opt {
                        let observer = observer.clone();
                        spawn_hook(label, async move {
                            observer.on_signaling_disconnected(&ctx).await;
                        });
                    }
                }
                HookEvent::WebRtcConnectStart { peer_id } => {
                    if let Some(ctx) = ctx_opt {
                        let event = PeerEvent {
                            peer: peer_id,
                            relayed: None,
                        };
                        spawn_hook("on_webrtc_connecting", async move {
                            observer.on_webrtc_connecting(&ctx, &event).await;
                        });
                    }
                }
                HookEvent::WebRtcConnected { peer_id, relayed } => {
                    if let Some(ctx) = ctx_opt {
                        let event = PeerEvent {
                            peer: peer_id,
                            relayed: Some(relayed),
                        };
                        spawn_hook("on_webrtc_connected", async move {
                            observer.on_webrtc_connected(&ctx, &event).await;
                        });
                    }
                }
                HookEvent::WebRtcDisconnected { peer_id } => {
                    if let Some(ctx) = ctx_opt {
                        let event = PeerEvent {
                            peer: peer_id,
                            relayed: None,
                        };
                        spawn_hook("on_webrtc_disconnected", async move {
                            observer.on_webrtc_disconnected(&ctx, &event).await;
                        });
                    }
                }
                HookEvent::WebSocketConnectStart { peer_id } => {
                    if let Some(ctx) = ctx_opt {
                        let event = PeerEvent {
                            peer: peer_id,
                            relayed: None,
                        };
                        spawn_hook("on_websocket_connecting", async move {
                            observer.on_websocket_connecting(&ctx, &event).await;
                        });
                    }
                }
                HookEvent::WebSocketConnected { peer_id } => {
                    if let Some(ctx) = ctx_opt {
                        let event = PeerEvent {
                            peer: peer_id,
                            relayed: None,
                        };
                        spawn_hook("on_websocket_connected", async move {
                            observer.on_websocket_connected(&ctx, &event).await;
                        });
                    }
                }
                HookEvent::WebSocketDisconnected { peer_id } => {
                    if let Some(ctx) = ctx_opt {
                        let event = PeerEvent {
                            peer: peer_id,
                            relayed: None,
                        };
                        spawn_hook("on_websocket_disconnected", async move {
                            observer.on_websocket_disconnected(&ctx, &event).await;
                        });
                    }
                }
                HookEvent::CredentialRenewed { new_expiry } => {
                    if let Some(ctx) = ctx_opt {
                        let event = CredentialEvent { new_expiry };
                        spawn_hook("on_credential_renewed", async move {
                            observer.on_credential_renewed(&ctx, &event).await;
                        });
                    }
                }
                HookEvent::CredentialExpiring { new_expiry } => {
                    if let Some(ctx) = ctx_opt {
                        let event = CredentialEvent { new_expiry };
                        spawn_hook("on_credential_expiring", async move {
                            observer.on_credential_expiring(&ctx, &event).await;
                        });
                    }
                }
                HookEvent::MailboxBackpressure {
                    queue_len,
                    threshold,
                } => {
                    if let Some(ctx) = ctx_opt {
                        let event = BackpressureEvent {
                            queue_len,
                            threshold,
                        };
                        spawn_hook("on_mailbox_backpressure", async move {
                            observer.on_mailbox_backpressure(&ctx, &event).await;
                        });
                    }
                }
            }
        }) as Pin<Box<dyn Future<Output = ()> + Send>>
    })
}

/// Emit the framework-default tracing record for a hook event.
fn log_hook_event(event: &HookEvent) {
    match event {
        HookEvent::SignalingConnectStart { attempt } => {
            tracing::debug!(attempt = *attempt, "signaling connecting");
        }
        HookEvent::SignalingConnected => tracing::info!("signaling connected"),
        HookEvent::SignalingDisconnected => tracing::warn!("signaling disconnected"),
        HookEvent::WebRtcConnectStart { peer_id } => {
            tracing::debug!(peer = %peer_id, "webrtc connecting");
        }
        HookEvent::WebRtcConnected { peer_id, relayed } => {
            tracing::info!(peer = %peer_id, relayed = *relayed, "webrtc connected");
        }
        HookEvent::WebRtcDisconnected { peer_id } => {
            tracing::warn!(peer = %peer_id, "webrtc disconnected");
        }
        HookEvent::WebSocketConnectStart { peer_id } => {
            tracing::debug!(peer = %peer_id, "websocket connecting");
        }
        HookEvent::WebSocketConnected { peer_id } => {
            tracing::info!(peer = %peer_id, "websocket connected");
        }
        HookEvent::WebSocketDisconnected { peer_id } => {
            tracing::warn!(peer = %peer_id, "websocket disconnected");
        }
        HookEvent::CredentialRenewed { new_expiry } => {
            tracing::info!(new_expiry = ?new_expiry, "credential renewed");
        }
        HookEvent::CredentialExpiring { new_expiry } => {
            tracing::warn!(new_expiry = ?new_expiry, "credential expiring soon");
        }
        HookEvent::MailboxBackpressure {
            queue_len,
            threshold,
        } => {
            tracing::warn!(
                queue_len = *queue_len,
                threshold = *threshold,
                "mailbox backpressure",
            );
        }
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
