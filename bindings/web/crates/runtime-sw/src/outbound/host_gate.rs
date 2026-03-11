//! HostGate - in-process outbound transport adapter.
//!
//! Web-specific HostGate for communication between actors inside the Service Worker.

use actr_protocol::{ActorResult, ActrError, ActrId, PayloadType, RpcEnvelope};
use bytes::Bytes;
use futures::channel::oneshot;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

use actr_framework::MediaSample;
/// HostGate - in-process transport adapter.
///
/// # Design notes
///
/// The Web version of HostGate is similar to the core actr version, but differs in that it:
/// - Uses JS/WASM-friendly async primitives (`futures::channel`)
/// - Does not require an mpsc channel because the SW environment is single-threaded
/// - Implements request/response matching through `request_id`
///
/// # Communication modes
///
/// 1. **Request/response (`send_request`)**
///    - Create a oneshot channel
///    - Register `request_id -> sender` in `pending_requests`
///    - Send the request to the target actor
///    - Wait for the response
///
/// 2. **One-way message (`send_message`)**
///    - Send directly without waiting for a response
///
/// 3. **DataStream (Fast Path)**
///    - Bypass serialization and pass bytes directly
pub struct HostGate {
    /// Pending requests: request_id → oneshot sender
    pending_requests: Arc<Mutex<HashMap<String, oneshot::Sender<Bytes>>>>,

    /// Message handling callback installed by System.
    /// Receives `(target_id, envelope)` and routes it to the target actor.
    message_handler: Arc<Mutex<Option<MessageHandler>>>,
}

/// Message handling callback type.
/// No `Send + Sync` is required in the single-threaded WASM/Service Worker environment.
pub type MessageHandler = Box<dyn Fn(ActrId, RpcEnvelope)>;

impl HostGate {
    /// Create a new HostGate.
    pub fn new() -> Self {
        Self {
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
            message_handler: Arc::new(Mutex::new(None)),
        }
    }

    /// Set the message handling callback.
    ///
    /// # Parameters
    /// - `handler`: Callback receiving `(target_id, envelope)`
    ///
    /// # Purpose
    /// Called during System initialization to route messages to the appropriate actor.
    pub fn set_message_handler<F>(&self, handler: F)
    where
        F: Fn(ActrId, RpcEnvelope) + 'static,
    {
        let mut guard = self.message_handler.lock();
        *guard = Some(Box::new(handler));
    }

    /// Send a request and wait for the response.
    ///
    /// # Implementation
    /// 1. Create a oneshot channel
    /// 2. Register the pending request
    /// 3. Invoke `message_handler` to deliver the request
    /// 4. Wait for the response
    pub async fn send_request(&self, target: &ActrId, envelope: RpcEnvelope) -> ActorResult<Bytes> {
        log::debug!(
            "HostGate::send_request to {:?}, request_id={}",
            target,
            envelope.request_id
        );

        // 1. Create a oneshot channel.
        let (tx, rx) = oneshot::channel();

        // 2. Register the pending request.
        {
            let mut pending = self.pending_requests.lock();
            pending.insert(envelope.request_id.clone(), tx);
        }

        // 3. Send the request to the target actor.
        {
            let guard = self.message_handler.lock();
            match guard.as_ref() {
                Some(handler) => {
                    handler(target.clone(), envelope);
                }
                None => {
                    // Clean up the pending request.
                    drop(guard); // Release the lock.
                    self.pending_requests.lock().remove(&envelope.request_id);

                    return Err(ActrError::Unavailable(
                        "HostGate message_handler not set".to_string(),
                    ));
                }
            }
        }

        // 4. Wait for the response.
        let response = rx
            .await
            .map_err(|_| ActrError::Unavailable("Response channel closed".to_string()))?;

        Ok(response)
    }

    /// Send a one-way message without waiting for a response.
    pub async fn send_message(&self, target: &ActrId, envelope: RpcEnvelope) -> ActorResult<()> {
        log::debug!(
            "HostGate::send_message to {:?}, request_id={}",
            target,
            envelope.request_id
        );

        // Fetch and invoke the message_handler.
        let guard = self.message_handler.lock();
        match guard.as_ref() {
            Some(handler) => {
                handler(target.clone(), envelope);
                Ok(())
            }
            None => Err(ActrError::Unavailable(
                "HostGate message_handler not set".to_string(),
            )),
        }
    }

    /// Send a DataStream through the Fast Path.
    ///
    /// # Parameters
    /// - `target`: Target actor ID
    /// - `payload_type`: PayloadType (`StreamReliable` or `StreamLatencyFirst`)
    /// - `data`: Serialized DataStream bytes
    pub async fn send_data_stream(
        &self,
        target: &ActrId,
        _payload_type: PayloadType,
        data: Bytes,
    ) -> ActorResult<()> {
        log::debug!(
            "HostGate::send_data_stream to {:?}, size={} bytes",
            target,
            data.len()
        );

        // Temporarily send through RpcEnvelope. This can be optimized further later.
        let envelope = RpcEnvelope {
            route_key: "__fast_path_data_stream__".to_string(),
            payload: Some(data),
            error: None,
            traceparent: None,
            tracestate: None,
            request_id: format!("ds-{}", js_sys::Math::random()),
            metadata: vec![],
            timeout_ms: 0,
        };

        self.send_message(target, envelope).await
    }

    /// Send a MediaSample through the Fast Path.
    ///
    /// # Parameters
    /// - `target`: Target actor ID
    /// - `track_id`: Track ID
    /// - `sample`: Media sample
    pub async fn send_media_sample(
        &self,
        target: &ActrId,
        track_id: &str,
        _sample: MediaSample,
    ) -> ActorResult<()> {
        log::warn!(
            "HostGate::send_media_sample to {:?}, track={} - not implemented",
            target,
            track_id
        );

        Err(ActrError::NotImplemented(
            "send_media_sample not yet implemented for Web HostGate".to_string(),
        ))
    }

    /// Handle a received response.
    ///
    /// # Purpose
    /// Called by System when a response arrives so it can resolve the matching pending request.
    pub fn handle_response(&self, request_id: &str, response: Bytes) {
        let mut pending = self.pending_requests.lock();
        if let Some(tx) = pending.remove(request_id) {
            let _ = tx.send(response); // Ignore send failure if the receiver was dropped.
        } else {
            log::warn!("Received response for unknown request_id: {}", request_id);
        }
    }
}

impl Default for HostGate {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_gate_creation() {
        let _gate = HostGate::new();
    }

    #[test]
    fn test_handle_response_unknown_request() {
        let gate = HostGate::new();
        gate.handle_response("unknown-id", Bytes::from("test"));
        // This should only log a warning and must not panic.
    }
}
