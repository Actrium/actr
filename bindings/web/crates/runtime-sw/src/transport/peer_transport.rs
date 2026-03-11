//! Cross-destination transport manager.
//!
//! Manages transports for multiple destinations and exposes a unified send API.

use super::dest_transport::DestTransport;
use super::wire_builder::WireBuilder;
use actr_web_common::{Dest, PayloadType, WebResult};
use dashmap::DashMap;
use std::sync::Arc;

/// Per-destination transport state.
///
/// Uses an Either-like model for connection lifecycle management:
/// - `Connecting`: connection creation is in progress and others must wait
/// - `Connected`: a ready `DestTransport` is available
enum DestState {
    /// Connection creation is in progress.
    Connecting(Arc<futures::channel::oneshot::Receiver<Arc<DestTransport>>>),

    /// A ready transport is available.
    Connected(Arc<DestTransport>),
}

/// Cross-destination transport manager.
///
/// Responsibilities:
/// - Manage one `DestTransport` per destination
/// - Lazily create transports on demand
/// - Expose a unified send API
/// - Support pluggable connection factories
/// - Prevent duplicate creation with `DashMap` and `oneshot`
pub struct PeerTransport {
    /// Local ID.
    local_id: String,

    /// Mapping from `Dest` to `DestState`.
    transports: Arc<DashMap<Dest, DestState>>,

    /// Wire builder.
    wire_builder: Arc<dyn WireBuilder>,
}

impl PeerTransport {
    /// Create a new `PeerTransport`.
    ///
    /// # Parameters
    /// - `local_id`: local actor ID or identifier
    /// - `wire_builder`: asynchronously creates wire handles for a destination
    pub fn new(local_id: String, wire_builder: Arc<dyn WireBuilder>) -> Self {
        Self {
            local_id,
            transports: Arc::new(DashMap::new()),
            wire_builder,
        }
    }

    /// Get or create the `DestTransport` for a destination.
    ///
    /// # Parameters
    /// - `dest`: target destination
    ///
    /// # Returns
    /// - Shared `Arc<DestTransport>` for that destination
    ///
    /// # State machine
    /// Uses `DashMap` plus `oneshot` to avoid duplicate connection attempts:
    /// 1. If `Connected`, return the transport
    /// 2. If `Connecting`, wait until completion
    /// 3. If missing, insert `Connecting`, create it, then promote to `Connected`
    pub async fn get_or_create_transport(&self, dest: &Dest) -> WebResult<Arc<DestTransport>> {
        // 1. Fast path: check whether it already exists.
        if let Some(entry) = self.transports.get(dest) {
            match entry.value() {
                DestState::Connected(transport) => {
                    log::debug!(
                        "[PeerTransport] Reusing existing DestTransport: {:?}",
                        dest
                    );
                    return Ok(Arc::clone(transport));
                }
                DestState::Connecting(_rx) => {
                    // Wait for the ongoing creation to finish.
                    log::debug!(
                        "[PeerTransport] Waiting for ongoing connection: {:?}",
                        dest
                    );
                    drop(entry); // Release the lock.

                    // Note: a oneshot receiver can only be consumed once, so this
                    // simplified implementation rechecks the map instead.
                    loop {
                        if let Some(entry) = self.transports.get(dest) {
                            if let DestState::Connected(transport) = entry.value() {
                                return Ok(Arc::clone(transport));
                            }
                        }

                        // Retry after a short delay using `gloo_timers`, which works in SW.
                        gloo_timers::future::TimeoutFuture::new(10).await;
                    }
                }
            }
        }

        // 2. Slow path: create a new connection.
        log::info!(
            "[PeerTransport] Creating new connection for: {:?}",
            dest
        );

        // Create the oneshot channel.
        let (tx, rx) = futures::channel::oneshot::channel();

        // Try to insert the `Connecting` state.
        let inserted = self
            .transports
            .insert(dest.clone(), DestState::Connecting(Arc::new(rx)))
            .is_none();

        if !inserted {
            // Another caller inserted it first, so wait for that path.
            return Box::pin(self.get_or_create_transport(dest)).await;
        }

        // This caller is responsible for creating the connection.
        let result = async {
            let connections = self.wire_builder.create_connections(dest).await?;

            // Zero initial connections are allowed. `WireBuilder` may only trigger
            // asynchronous connection creation, such as asking the DOM to create P2P.
            // The real `WireHandle` can be injected later, and `DestTransport`
            // waits on `ReadyWatcher` in its event-driven send loop.
            log::info!(
                "[PeerTransport] Creating DestTransport: {:?} ({} initial connections)",
                dest,
                connections.len()
            );

            let transport = DestTransport::new(dest.clone(), connections).await?;
            Ok(Arc::new(transport))
        }
        .await;

        // Update the state.
        match result {
            Ok(transport) => {
                log::info!(
                    "[PeerTransport] Connection established: {:?}",
                    dest
                );
                self.transports
                    .insert(dest.clone(), DestState::Connected(Arc::clone(&transport)));

                // Notify waiters.
                tx.send(Arc::clone(&transport)).ok();

                Ok(transport)
            }
            Err(e) => {
                log::error!(
                    "[PeerTransport] Connection failed: {:?}: {}",
                    dest,
                    e
                );
                self.transports.remove(dest);

                // Notify waiters of failure by closing the channel.
                drop(tx);

                Err(e)
            }
        }
    }

    /// Send a message to a destination.
    ///
    /// # Parameters
    /// - `dest`: target destination
    /// - `payload_type`: payload type
    /// - `data`: payload bytes
    pub async fn send(&self, dest: &Dest, payload_type: PayloadType, data: &[u8]) -> WebResult<()> {
        log::debug!(
            "[PeerTransport] Sending to {:?}: type={:?}, size={}",
            dest,
            payload_type,
            data.len()
        );

        // Get or create the transport.
        let transport = self.get_or_create_transport(dest).await?;

        // Send through the destination transport.
        transport.send(payload_type, data).await
    }

    /// Close the `DestTransport` for a destination.
    ///
    /// # Parameters
    /// - `dest`: target destination
    pub async fn close_transport(&self, dest: &Dest) -> WebResult<()> {
        if let Some((_, state)) = self.transports.remove(dest) {
            match state {
                DestState::Connected(transport) => {
                    log::info!(
                        "[PeerTransport] Closing DestTransport: {:?}",
                        dest
                    );
                    transport.close().await?;
                }
                DestState::Connecting(_) => {
                    log::debug!(
                        "[PeerTransport] Removed Connecting state for: {:?}",
                        dest
                    );
                }
            }
        }

        Ok(())
    }

    /// Close all destination transports.
    pub async fn close_all(&self) -> WebResult<()> {
        log::info!(
            "[PeerTransport] Closing all DestTransports (count: {})",
            self.transports.len()
        );

        let dests: Vec<Dest> = self
            .transports
            .iter()
            .map(|entry| entry.key().clone())
            .collect();

        for dest in dests {
            if let Err(e) = self.close_transport(&dest).await {
                log::warn!(
                    "[PeerTransport] Failed to close DestTransport {:?}: {}",
                    dest,
                    e
                );
            }
        }

        Ok(())
    }

    /// Get the number of managed destinations.
    pub fn dest_count(&self) -> usize {
        self.transports.len()
    }

    /// Get the local ID.
    #[inline]
    pub fn local_id(&self) -> &str {
        &self.local_id
    }

    /// List all tracked destinations.
    pub fn list_dests(&self) -> Vec<Dest> {
        self.transports
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Check whether a connection exists for the specified destination.
    pub fn has_dest(&self, dest: &Dest) -> bool {
        self.transports.contains_key(dest)
    }

    /// Inject a new connection into the WirePool for a destination.
    ///
    /// Used after the DOM establishes P2P and passes in a dedicated `MessagePort`:
    /// 1. The `datachannel_open` event fires
    /// 2. The SW receives the transferred `MessagePort`
    /// 3. It builds `WebRtcConnection -> WireHandle::WebRTC`
    /// 4. This method injects it into the destination's `WirePool`
    /// 5. `WirePool` notifies `ReadyWatcher`, waking the `DestTransport` send loop
    ///
    /// If the destination's `DestTransport` does not exist yet, an empty one is created automatically.
    pub async fn inject_connection(
        &self,
        dest: &Dest,
        wire_handle: super::wire_handle::WireHandle,
    ) -> WebResult<()> {
        let transport = self.get_or_create_transport(dest).await?;
        transport.wire_pool().add_connection(wire_handle);
        log::info!(
            "[PeerTransport] Injected connection into {:?}",
            dest
        );
        Ok(())
    }
}

impl Drop for PeerTransport {
    fn drop(&mut self) {
        log::debug!("[PeerTransport] Dropped");
        // Note: asynchronous cleanup still requires the caller to invoke `close_all()`.
    }
}
