//! Service Worker keepalive mechanism.
//!
//! Periodically sends keepalive messages to the Service Worker so the browser
//! does not reclaim it.

use crate::transport::DataLane;
use bytes::Bytes;
use parking_lot::Mutex;
use std::sync::Arc;

/// Service Worker keepalive mechanism.
///
/// As long as the DOM side is active, the Service Worker should remain active.
pub struct ServiceWorkerKeepalive {
    lane: Arc<DataLane>,
    interval_secs: u64,
    running: Arc<Mutex<bool>>,
}

impl ServiceWorkerKeepalive {
    /// Create a new keepalive instance.
    ///
    /// # Parameters
    /// - `lane`: PostMessage lane targeting the Service Worker
    /// - `interval_secs`: Keepalive interval in seconds, defaults to 20
    pub fn new(lane: Arc<DataLane>, interval_secs: Option<u64>) -> Self {
        Self {
            lane,
            interval_secs: interval_secs.unwrap_or(20),
            running: Arc::new(Mutex::new(false)),
        }
    }

    /// Start the keepalive loop.
    ///
    /// Sends one keepalive message to the Service Worker every `interval_secs` seconds.
    pub fn start(&self) {
        let mut running = self.running.lock();
        if *running {
            log::warn!("ServiceWorkerKeepalive is already running");
            return;
        }
        *running = true;
        drop(running);

        let lane = self.lane.clone();
        let interval_ms = self.interval_secs * 1000;
        let running = self.running.clone();

        wasm_bindgen_futures::spawn_local(async move {
            log::info!(
                "ServiceWorkerKeepalive started with {} second interval",
                interval_ms / 1000
            );

            loop {
                // Check whether the loop should stop.
                if !*running.lock() {
                    log::info!("ServiceWorkerKeepalive stopped");
                    break;
                }

                // Wait for the configured interval.
                wasm_bindgen_futures::JsFuture::from(js_sys::Promise::new(&mut |resolve, _| {
                    let window = web_sys::window().unwrap();
                    window
                        .set_timeout_with_callback_and_timeout_and_arguments_0(
                            &resolve,
                            interval_ms as i32,
                        )
                        .unwrap();
                }))
                .await
                .unwrap();

                // Send a keepalive message.
                let keepalive_msg = Bytes::from_static(b"KEEPALIVE");
                match lane.send(keepalive_msg).await {
                    Ok(_) => {
                        log::trace!("ServiceWorkerKeepalive: sent keepalive message");
                    }
                    Err(e) => {
                        log::error!("ServiceWorkerKeepalive: failed to send keepalive message: {:?}", e);
                    }
                }
            }
        });
    }

    /// Stop the keepalive loop.
    pub fn stop(&self) {
        let mut running = self.running.lock();
        *running = false;
        log::info!("ServiceWorkerKeepalive: stop requested");
    }

    /// Check whether the keepalive loop is running.
    pub fn is_running(&self) -> bool {
        *self.running.lock()
    }
}
