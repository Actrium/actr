//! ActrRef - Lightweight reference to a running Actor
//!
//! # Key Characteristics
//!
//! - **Cloneable**: Can be shared across tasks
//! - **Lightweight**: Contains only an `Arc` to shared state
//! - **Auto-cleanup**: Last `ActrRef` drop triggers resource cleanup
//!
//! # Usage
//!
//! ```rust,ignore
//! let node = Hyper::init(hyper_config).await?.attach_package(&package, config).await?;
//! let actr = node.start().await?;
//!
//! println!("actor id = {:?}", actr.actor_id());
//!
//! // Wait for process signals and then perform a graceful shutdown.
//! actr.wait_for_ctrl_c_and_shutdown().await?;
//! ```
//!
//! `Hyper::attach_package()` is a one-shot operation. Create a new `Hyper`
//! instance when hosting another package.

use crate::context::RuntimeContext;
use crate::context_factory::ContextFactory;
use crate::lifecycle::CredentialState;
use actr_framework::{Context as _, Dest};
use actr_protocol::{ActorResult, ActrError, ActrId, ActrType, RpcRequest};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Shared state between all `ActrRef` clones
///
/// This is an internal implementation detail. When the last `ActrRef` is dropped,
/// this struct's `Drop` impl will trigger shutdown and cleanup all resources.
pub(crate) struct ActrRefShared {
    /// Actor ID
    pub(crate) actor_id: ActrId,
    /// Context factory used to create application-side runtime contexts.
    pub(crate) context_factory: ContextFactory,
    /// Current credential state for building application-side contexts.
    pub(crate) credential_state: CredentialState,
    /// Shutdown signal
    pub(crate) shutdown_token: CancellationToken,
    /// Background task handles (receive loops, WebRTC coordinator, etc.)
    pub(crate) task_handles: Mutex<Vec<JoinHandle<()>>>,
}

/// ActrRef - Lightweight reference to a running Actor
///
/// This is the primary handle returned by `ActrNode::start()`.
pub struct ActrRef {
    pub(crate) shared: Arc<ActrRefShared>,
}

impl Clone for ActrRef {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl ActrRef {
    /// Get Actor ID
    pub fn actor_id(&self) -> &ActrId {
        &self.shared.actor_id
    }

    /// Call the local workload with a typed RPC request.
    ///
    /// Convenience wrapper around `app_context().call(&Dest::Local, request)`.
    /// Use this from app-side code to invoke the local guest workload.
    pub async fn call<R: RpcRequest>(&self, request: R) -> ActorResult<R::Response> {
        self.app_context().await.call(&Dest::Local, request).await
    }

    /// Call a remote actor directly with a typed RPC request.
    ///
    /// Convenience wrapper around `app_context().call(&Dest::Actor(target), request)`.
    /// Use this when the client has no local guest workload and calls the remote actor directly.
    pub async fn call_remote<R: RpcRequest>(
        &self,
        target: ActrId,
        request: R,
    ) -> ActorResult<R::Response> {
        self.app_context()
            .await
            .call(&Dest::Actor(target), request)
            .await
    }

    /// Discover route candidates for the given actor type.
    ///
    /// Returns up to `count` actor IDs registered under `target_type`.
    /// Convenience wrapper for app-side discovery without holding a `RuntimeContext`.
    pub async fn discover_route_candidates(
        &self,
        target_type: &ActrType,
        count: usize,
    ) -> ActorResult<Vec<ActrId>> {
        let ctx = self.app_context().await;
        let mut results = Vec::with_capacity(count);
        for _ in 0..count {
            match ctx.discover_route_candidate(target_type).await {
                Ok(id) => {
                    if !results.contains(&id) {
                        results.push(id);
                    }
                    break; // signaling returns one candidate per request; stop after first
                }
                Err(e) => return Err(e),
            }
        }
        Ok(results)
    }

    /// Create an application-side runtime context bound to this running actor.
    pub async fn app_context(&self) -> RuntimeContext {
        let credential = self.shared.credential_state.credential().await;
        self.shared
            .context_factory
            .create_bootstrap(&self.shared.actor_id, &credential)
    }

    /// Trigger Actor shutdown
    ///
    /// This signals the Actor to stop, but does not wait for completion.
    /// Use `wait_for_shutdown()` to wait for cleanup to finish.
    pub fn shutdown(&self) {
        tracing::info!(
            "🛑 Shutdown requested for Actor {}",
            actr_protocol::ActrIdExt::to_string_repr(&self.shared.actor_id)
        );
        self.shared.shutdown_token.cancel();
    }

    /// Wait for Actor to fully shutdown
    ///
    /// This waits for the shutdown signal to be triggered.
    /// All background tasks will be aborted when the last `ActrRef` is dropped.
    pub async fn wait_for_shutdown(&self) {
        self.shared.shutdown_token.cancelled().await;
        // Take ownership of the current handles so we can await them as Futures.
        let mut guard = self.shared.task_handles.lock().await;
        let handles = std::mem::take(&mut *guard);
        drop(guard);
        tracing::debug!("Waiting for tasks to complete: {:?}", handles.len());

        // All tasks have been asked to shut down; wait for them with a timeout,
        // and abort any that don't finish in time to avoid leaking background work.
        for handle in handles {
            let sleep = tokio::time::sleep(Duration::from_secs(5));
            tokio::pin!(handle);
            tokio::pin!(sleep);

            tokio::select! {
                res = &mut handle => {
                    match res {
                        Ok(_) => {
                            tracing::debug!("Task completed");
                        }
                        Err(e) => {
                            tracing::error!("Task failed: {:?}", e);
                        }
                    }
                }
                _ = sleep => {
                    tracing::warn!("Task timed out after 5s, aborting");
                    handle.abort();
                }
            }
        }
    }

    /// Check if Actor is shutting down
    pub fn is_shutting_down(&self) -> bool {
        self.shared.shutdown_token.is_cancelled()
    }

    /// This consumes the `ActrRef` and waits for signal (Ctrl+C / SIGTERM),
    /// then triggers shutdown.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let actr = node.start().await?;
    /// actr.wait_for_ctrl_c_and_shutdown().await?;
    /// ```
    pub async fn wait_for_ctrl_c_and_shutdown(self) -> ActorResult<()> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};

            let mut sigint = signal(SignalKind::interrupt()).map_err(|e| {
                ActrError::Unavailable(format!("Signal handler error (SIGINT): {e}"))
            })?;
            let mut sigterm = signal(SignalKind::terminate()).map_err(|e| {
                ActrError::Unavailable(format!("Signal handler error (SIGTERM): {e}"))
            })?;

            tokio::select! {
                _ = sigint.recv() => tracing::info!("📡 Received SIGINT (Ctrl+C) signal"),
                _ = sigterm.recv() => tracing::info!("📡 Received SIGTERM signal"),
            }
        }

        #[cfg(not(unix))]
        {
            tokio::signal::ctrl_c()
                .await
                .map_err(|e| ActrError::Unavailable(format!("Ctrl+C signal error: {e}")))?;
            tracing::info!("📡 Received Ctrl+C signal");
        }

        self.shutdown();
        self.wait_for_shutdown().await;
        Ok(())
    }
}

impl Drop for ActrRefShared {
    fn drop(&mut self) {
        tracing::info!(
            "🧹 ActrRefShared dropping - cleaning up Actor {}",
            actr_protocol::ActrIdExt::to_string_repr(&self.actor_id)
        );

        // Cancel shutdown token
        self.shutdown_token.cancel();
        // Abort all background tasks (best-effort)
        if let Ok(mut handles) = self.task_handles.try_lock() {
            for handle in handles.drain(..) {
                handle.abort();
            }
        } else {
            tracing::warn!(
                "⚠️ Failed to lock task_handles mutex during Drop; some tasks may still be running"
            );
        }

        tracing::debug!(
            "✅ All background tasks aborted for Actor {}",
            actr_protocol::ActrIdExt::to_string_repr(&self.actor_id)
        );
    }
}
