//! Runtime-side workload hook plumbing.
//!
//! The user-facing [`actr_framework::Workload`] trait is **not** object-safe
//! (it carries an associated `Dispatcher` type and generic `<C: Context>`
//! methods), so `Arc<dyn Workload>` is not representable. The node still
//! needs a way to dispatch observation events (signaling / transport /
//! credential / mailbox) into whatever workload the shell is hosting
//! through a single object-safe callback surface.
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

use actr_framework::{BackpressureEvent, CredentialEvent, ErrorCategory, ErrorEvent, PeerEvent};
use actr_protocol::{ActorResult, ActrError};
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
/// This trait is the object-safe hook surface behind the internal handle
/// used by `Node::link(...)`. Hook delivery flows through this trait;
/// inbound RPC dispatch is handled separately by the sibling
/// `LinkedWorkloadHandle` path in `workload.rs`.
#[async_trait]
#[allow(dead_code)]
pub(crate) trait WorkloadHookObserver: Send + Sync + 'static {
    // Lifecycle (fallible). Startup code awaits these hooks directly when
    // their result participates in node lifecycle semantics.
    async fn on_start(&self, _ctx: &RuntimeContext) -> ActorResult<()> {
        Ok(())
    }
    async fn on_ready(&self, _ctx: &RuntimeContext) -> ActorResult<()> {
        Ok(())
    }
    async fn on_stop(&self, _ctx: &RuntimeContext) -> ActorResult<()> {
        Ok(())
    }
    async fn on_error(&self, _ctx: &RuntimeContext, _event: &ErrorEvent) -> ActorResult<()> {
        Ok(())
    }

    // Signaling
    async fn on_signaling_connecting(&self, _ctx: Option<&RuntimeContext>) {}
    async fn on_signaling_connected(&self, _ctx: Option<&RuntimeContext>) {}
    async fn on_signaling_disconnected(&self, _ctx: &RuntimeContext) {}

    // WebSocket
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

/// Await a lifecycle hook with panic isolation and preserve fallible results.
///
/// Unlike [`spawn_hook`], this helper runs inline so startup/shutdown code can
/// decide whether a lifecycle hook failure should abort or only be logged.
#[allow(dead_code)]
pub(crate) async fn call_lifecycle_hook<F>(label: &'static str, fut: F) -> ActorResult<()>
where
    F: Future<Output = ActorResult<()>>,
{
    match AssertUnwindSafe(fut).catch_unwind().await {
        Ok(result) => result,
        Err(panic_payload) => {
            let info = extract_panic_info(panic_payload);
            Err(ActrError::Internal(format!("{label} panicked: {info}")))
        }
    }
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

            let ctx_opt = ctx_builder().await;

            match event {
                HookEvent::SignalingConnectStart { .. } => {
                    let label = "on_signaling_connecting";
                    if let Some(observer) = observer.clone() {
                        spawn_hook(label, async move {
                            observer.on_signaling_connecting(ctx_opt.as_ref()).await;
                        });
                    }
                }
                HookEvent::SignalingConnected => {
                    let label = "on_signaling_connected";
                    if let Some(observer) = observer.clone() {
                        spawn_hook(label, async move {
                            observer.on_signaling_connected(ctx_opt.as_ref()).await;
                        });
                    }
                }
                HookEvent::SignalingDisconnected => {
                    let label = "on_signaling_disconnected";
                    if let Some(ctx) = ctx_opt {
                        if let Some(observer) = observer.clone() {
                            spawn_hook(label, async move {
                                observer.on_signaling_disconnected(&ctx).await;
                            });
                        }
                    }
                }
                HookEvent::WebRtcConnectStart { peer_id } => {
                    if let Some(ctx) = ctx_opt {
                        let event = PeerEvent {
                            peer: peer_id,
                            relayed: None,
                        };
                        if let Some(observer) = observer.clone() {
                            spawn_hook("on_webrtc_connecting", async move {
                                observer.on_webrtc_connecting(&ctx, &event).await;
                            });
                        }
                    }
                }
                HookEvent::WebRtcConnected { peer_id, relayed } => {
                    if let Some(ctx) = ctx_opt {
                        let event = PeerEvent {
                            peer: peer_id,
                            relayed: Some(relayed),
                        };
                        if let Some(observer) = observer.clone() {
                            spawn_hook("on_webrtc_connected", async move {
                                observer.on_webrtc_connected(&ctx, &event).await;
                            });
                        }
                    }
                }
                HookEvent::WebRtcDisconnected { peer_id } => {
                    if let Some(ctx) = ctx_opt {
                        let event = PeerEvent {
                            peer: peer_id,
                            relayed: None,
                        };
                        if let Some(observer) = observer.clone() {
                            spawn_hook("on_webrtc_disconnected", async move {
                                observer.on_webrtc_disconnected(&ctx, &event).await;
                            });
                        }
                    }
                }
                HookEvent::DataStreamDeliveryUncertain {
                    stream_id,
                    session_id,
                    reason,
                } => {
                    if let Some(ctx) = ctx_opt {
                        let event = ErrorEvent::now(
                            ActrError::Unavailable(
                                "data stream delivery uncertain after WebRTC disconnect"
                                    .to_string(),
                            ),
                            ErrorCategory::DataStreamDeliveryUncertain,
                            format!(
                                "stream_id={stream_id}; session_id={session_id}; reason={reason}"
                            ),
                        );
                        if let Some(observer) = observer.clone() {
                            spawn_hook("on_error", async move {
                                if let Err(e) = observer.on_error(&ctx, &event).await {
                                    tracing::warn!(error = %e, "workload on_error returned Err");
                                }
                            });
                        }
                    }
                }
                HookEvent::WebSocketConnectStart { peer_id } => {
                    if let Some(ctx) = ctx_opt {
                        let event = PeerEvent {
                            peer: peer_id,
                            relayed: None,
                        };
                        if let Some(observer) = observer.clone() {
                            spawn_hook("on_websocket_connecting", async move {
                                observer.on_websocket_connecting(&ctx, &event).await;
                            });
                        }
                    }
                }
                HookEvent::WebSocketConnected { peer_id } => {
                    if let Some(ctx) = ctx_opt {
                        let event = PeerEvent {
                            peer: peer_id,
                            relayed: None,
                        };
                        if let Some(observer) = observer.clone() {
                            spawn_hook("on_websocket_connected", async move {
                                observer.on_websocket_connected(&ctx, &event).await;
                            });
                        }
                    }
                }
                HookEvent::WebSocketDisconnected { peer_id } => {
                    if let Some(ctx) = ctx_opt {
                        let event = PeerEvent {
                            peer: peer_id,
                            relayed: None,
                        };
                        if let Some(observer) = observer.clone() {
                            spawn_hook("on_websocket_disconnected", async move {
                                observer.on_websocket_disconnected(&ctx, &event).await;
                            });
                        }
                    }
                }
                HookEvent::CredentialRenewed { new_expiry } => {
                    if let Some(ctx) = ctx_opt {
                        let event = CredentialEvent { new_expiry };
                        if let Some(observer) = observer.clone() {
                            spawn_hook("on_credential_renewed", async move {
                                observer.on_credential_renewed(&ctx, &event).await;
                            });
                        }
                    }
                }
                HookEvent::CredentialExpiring { new_expiry } => {
                    if let Some(ctx) = ctx_opt {
                        let event = CredentialEvent { new_expiry };
                        if let Some(observer) = observer.clone() {
                            spawn_hook("on_credential_expiring", async move {
                                observer.on_credential_expiring(&ctx, &event).await;
                            });
                        }
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
                        if let Some(observer) = observer.clone() {
                            spawn_hook("on_mailbox_backpressure", async move {
                                observer.on_mailbox_backpressure(&ctx, &event).await;
                            });
                        }
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
        HookEvent::DataStreamDeliveryUncertain {
            stream_id,
            session_id,
            reason,
        } => {
            tracing::warn!(
                stream_id = %stream_id,
                session_id = *session_id,
                reason = %reason,
                "data stream delivery uncertain",
            );
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
    use crate::context::RuntimeContext;
    use crate::inbound::{DataStreamRegistry, MediaFrameRegistry};
    use crate::outbound::{Gate, HostGate};
    use crate::transport::HostTransport;
    use crate::wire::webrtc::{
        ReconnectConfig, SignalingClient, SignalingConfig, WebSocketSignalingClient,
    };
    use actr_framework::Context as _;
    use actr_protocol::{AIdCredential, ActrId, ActrType, Realm};
    use tokio::sync::mpsc;

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

    #[tokio::test(flavor = "current_thread")]
    async fn call_lifecycle_hook_propagates_error() {
        let err = call_lifecycle_hook("on_start", async {
            Err(ActrError::Internal("startup failed".to_string()))
        })
        .await
        .expect_err("lifecycle error must propagate");

        match err {
            ActrError::Internal(msg) => {
                assert!(msg.contains("startup failed"), "unexpected message: {msg}");
            }
            other => panic!("expected Internal, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn call_lifecycle_hook_converts_panic_to_error() {
        let err = call_lifecycle_hook("on_start", async {
            panic!("startup panic");
        })
        .await
        .expect_err("panic must become lifecycle error");

        match err {
            ActrError::Internal(msg) => {
                assert!(
                    msg.contains("on_start panicked: startup panic"),
                    "unexpected message: {msg}"
                );
            }
            other => panic!("expected Internal, got {other:?}"),
        }
    }

    fn test_actr_id(serial_number: u64) -> ActrId {
        ActrId {
            realm: Realm { realm_id: 1 },
            serial_number,
            r#type: ActrType {
                manufacturer: "acme".to_string(),
                name: "node".to_string(),
                version: "1.0.0".to_string(),
            },
        }
    }

    fn test_credential() -> AIdCredential {
        AIdCredential {
            key_id: 1,
            claims: bytes::Bytes::from_static(b"claims"),
            signature: bytes::Bytes::from(vec![0; 64]),
        }
    }

    fn test_runtime_context() -> RuntimeContext {
        let host_transport = Arc::new(HostTransport::new());
        let inproc_gate = Gate::Host(Arc::new(HostGate::new(host_transport)));
        let signaling_client: Arc<dyn SignalingClient> =
            Arc::new(WebSocketSignalingClient::new(SignalingConfig {
                server_url: url::Url::parse("ws://127.0.0.1:9").expect("valid test URL"),
                connection_timeout: 1,
                heartbeat_interval: 30,
                reconnect_config: ReconnectConfig::default(),
                auth_config: None,
                webrtc_role: None,
            }));

        RuntimeContext::new(
            test_actr_id(1),
            None,
            "hook-test".to_string(),
            inproc_gate,
            None,
            Arc::new(DataStreamRegistry::new()),
            Arc::new(MediaFrameRegistry::new()),
            signaling_client,
            test_credential(),
            None,
            Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        )
    }

    struct ErrorRecorder {
        tx: mpsc::UnboundedSender<ErrorEvent>,
    }

    #[async_trait::async_trait]
    impl WorkloadHookObserver for ErrorRecorder {
        async fn on_error(&self, _ctx: &RuntimeContext, event: &ErrorEvent) -> ActorResult<()> {
            let _ = self.tx.send(event.clone());
            Ok(())
        }
    }

    struct RecordingObserver {
        tx: mpsc::UnboundedSender<String>,
    }

    fn relayed_label(relayed: Option<bool>) -> &'static str {
        match relayed {
            Some(true) => "true",
            Some(false) => "false",
            None => "none",
        }
    }

    #[async_trait::async_trait]
    impl WorkloadHookObserver for RecordingObserver {
        async fn on_signaling_connecting(&self, ctx: Option<&RuntimeContext>) {
            let label = if ctx.is_some() { "some" } else { "none" };
            let _ = self.tx.send(format!("on_signaling_connecting:ctx={label}"));
        }

        async fn on_signaling_connected(&self, ctx: Option<&RuntimeContext>) {
            let label = if ctx.is_some() { "some" } else { "none" };
            let _ = self.tx.send(format!("on_signaling_connected:ctx={label}"));
        }

        async fn on_signaling_disconnected(&self, ctx: &RuntimeContext) {
            let _ = self.tx.send(format!(
                "on_signaling_disconnected:self={}",
                ctx.self_id().serial_number
            ));
        }

        async fn on_websocket_connecting(&self, _ctx: &RuntimeContext, event: &PeerEvent) {
            let _ = self.tx.send(format!(
                "on_websocket_connecting:peer={}:relayed={}",
                event.peer.serial_number,
                relayed_label(event.relayed)
            ));
        }

        async fn on_websocket_connected(&self, _ctx: &RuntimeContext, event: &PeerEvent) {
            let _ = self.tx.send(format!(
                "on_websocket_connected:peer={}:relayed={}",
                event.peer.serial_number,
                relayed_label(event.relayed)
            ));
        }

        async fn on_websocket_disconnected(&self, _ctx: &RuntimeContext, event: &PeerEvent) {
            let _ = self.tx.send(format!(
                "on_websocket_disconnected:peer={}:relayed={}",
                event.peer.serial_number,
                relayed_label(event.relayed)
            ));
        }

        async fn on_webrtc_connecting(&self, _ctx: &RuntimeContext, event: &PeerEvent) {
            let _ = self.tx.send(format!(
                "on_webrtc_connecting:peer={}:relayed={}",
                event.peer.serial_number,
                relayed_label(event.relayed)
            ));
        }

        async fn on_webrtc_connected(&self, _ctx: &RuntimeContext, event: &PeerEvent) {
            let _ = self.tx.send(format!(
                "on_webrtc_connected:peer={}:relayed={}",
                event.peer.serial_number,
                relayed_label(event.relayed)
            ));
        }

        async fn on_webrtc_disconnected(&self, _ctx: &RuntimeContext, event: &PeerEvent) {
            let _ = self.tx.send(format!(
                "on_webrtc_disconnected:peer={}:relayed={}",
                event.peer.serial_number,
                relayed_label(event.relayed)
            ));
        }

        async fn on_credential_renewed(&self, _ctx: &RuntimeContext, event: &CredentialEvent) {
            let secs = event
                .new_expiry
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let _ = self.tx.send(format!("on_credential_renewed:expiry={secs}"));
        }

        async fn on_credential_expiring(&self, _ctx: &RuntimeContext, event: &CredentialEvent) {
            let secs = event
                .new_expiry
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let _ = self
                .tx
                .send(format!("on_credential_expiring:expiry={secs}"));
        }

        async fn on_mailbox_backpressure(&self, _ctx: &RuntimeContext, event: &BackpressureEvent) {
            let _ = self.tx.send(format!(
                "on_mailbox_backpressure:queue_len={}:threshold={}",
                event.queue_len, event.threshold
            ));
        }
    }

    async fn expect_recorded(rx: &mut mpsc::UnboundedReceiver<String>, expected: &'static str) {
        let observed = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("observer hook was not called")
            .expect("observer channel dropped");
        assert_eq!(observed, expected);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hook_callback_routes_observation_hooks_to_observer_with_payload() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let observer: WorkloadHookObserverRef = Arc::new(RecordingObserver { tx });
        let ctx = test_runtime_context();
        let ctx_builder: HookContextBuilder = Arc::new(move || {
            let ctx = ctx.clone();
            Box::pin(async move { Some(ctx) })
        });
        let cb = build_hook_callback(Some(observer), ctx_builder);
        let expiry = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_725_000_000);

        let cases = vec![
            (
                HookEvent::SignalingConnectStart { attempt: 3 },
                "on_signaling_connecting:ctx=some",
            ),
            (
                HookEvent::SignalingConnected,
                "on_signaling_connected:ctx=some",
            ),
            (
                HookEvent::SignalingDisconnected,
                "on_signaling_disconnected:self=1",
            ),
            (
                HookEvent::WebRtcConnectStart {
                    peer_id: test_actr_id(2),
                },
                "on_webrtc_connecting:peer=2:relayed=none",
            ),
            (
                HookEvent::WebRtcConnected {
                    peer_id: test_actr_id(3),
                    relayed: false,
                },
                "on_webrtc_connected:peer=3:relayed=false",
            ),
            (
                HookEvent::WebRtcDisconnected {
                    peer_id: test_actr_id(4),
                },
                "on_webrtc_disconnected:peer=4:relayed=none",
            ),
            (
                HookEvent::WebSocketConnectStart {
                    peer_id: test_actr_id(5),
                },
                "on_websocket_connecting:peer=5:relayed=none",
            ),
            (
                HookEvent::WebSocketConnected {
                    peer_id: test_actr_id(6),
                },
                "on_websocket_connected:peer=6:relayed=none",
            ),
            (
                HookEvent::WebSocketDisconnected {
                    peer_id: test_actr_id(7),
                },
                "on_websocket_disconnected:peer=7:relayed=none",
            ),
            (
                HookEvent::CredentialRenewed { new_expiry: expiry },
                "on_credential_renewed:expiry=1725000000",
            ),
            (
                HookEvent::CredentialExpiring { new_expiry: expiry },
                "on_credential_expiring:expiry=1725000000",
            ),
            (
                HookEvent::MailboxBackpressure {
                    queue_len: 9,
                    threshold: 4,
                },
                "on_mailbox_backpressure:queue_len=9:threshold=4",
            ),
        ];

        for (event, expected) in cases {
            cb(event).await;
            expect_recorded(&mut rx, expected).await;
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hook_callback_passes_none_for_early_signaling_context() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let observer: WorkloadHookObserverRef = Arc::new(RecordingObserver { tx });
        let ctx_builder: HookContextBuilder = Arc::new(|| Box::pin(async { None }));
        let cb = build_hook_callback(Some(observer), ctx_builder);

        cb(HookEvent::SignalingConnectStart { attempt: 1 }).await;
        expect_recorded(&mut rx, "on_signaling_connecting:ctx=none").await;

        cb(HookEvent::SignalingConnected).await;
        expect_recorded(&mut rx, "on_signaling_connected:ctx=none").await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hook_callback_invokes_linked_observer_once_per_event() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let observer: WorkloadHookObserverRef = Arc::new(RecordingObserver { tx });
        let ctx = test_runtime_context();
        let ctx_builder: HookContextBuilder = Arc::new(move || {
            let ctx = ctx.clone();
            Box::pin(async move { Some(ctx) })
        });
        let cb = build_hook_callback(Some(observer), ctx_builder);

        cb(HookEvent::WebSocketConnected {
            peer_id: test_actr_id(42),
        })
        .await;

        expect_recorded(&mut rx, "on_websocket_connected:peer=42:relayed=none").await;
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        assert!(
            rx.try_recv().is_err(),
            "observer should receive exactly one hook event"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn data_stream_uncertain_hook_routes_to_on_error() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let observer: WorkloadHookObserverRef = Arc::new(ErrorRecorder { tx });
        let ctx = test_runtime_context();
        let ctx_builder: HookContextBuilder = Arc::new(move || {
            let ctx = ctx.clone();
            Box::pin(async move { Some(ctx) })
        });
        let cb = build_hook_callback(Some(observer), ctx_builder);

        cb(HookEvent::DataStreamDeliveryUncertain {
            stream_id: "mobile-upload".to_string(),
            session_id: 99,
            reason: "data channel closed".to_string(),
        })
        .await;

        let event = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("on_error was not called")
            .expect("error recorder dropped");

        assert_eq!(event.category, ErrorCategory::DataStreamDeliveryUncertain);
        assert!(matches!(event.source, ActrError::Unavailable(_)));
        assert!(event.context.contains("stream_id=mobile-upload"));
        assert!(event.context.contains("session_id=99"));
        assert!(event.context.contains("reason=data channel closed"));
    }
}
