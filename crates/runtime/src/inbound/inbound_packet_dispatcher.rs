//! InboundPacketDispatcher - Inbound packet dispatcher
//!
//! # Responsibilities
//! - Receive packets from three inbound paths:
//!   - InprocChannel (intra-process)
//!   - WebRtcCoordinator (WebRTC)
//!   - WebSocketConnection (WebSocket)
//! - Decode RpcEnvelope
//! - Route to different handlers based on PayloadType:
//!   - Signal/Reliable → Mailbox
//!   - LatencyFirst → DataStreamRegistry
//!   - MediaTrack → MediaFrameRegistry
//! - For response messages, wake up OutprocOutGate.pending_requests

use super::data_stream_registry::DataStreamRegistry;
use crate::outbound::OutprocOutGate;
use actr_mailbox::{Mailbox, MessagePriority};
use actr_protocol::{ActrId, DataStream, PayloadType, RpcEnvelope};
use std::sync::Arc;

/// Inbound packet metadata
///
/// Packet contains essential information about incoming data packets for dispatch decisions
#[derive(Debug, Clone)]
pub struct InboundPacket {
    /// PayloadType (determines dispatch path)
    pub payload_type: PayloadType,

    /// Message payload (serialized)
    pub data: Vec<u8>,

    /// Sender ActrId (Protobuf bytes)
    pub from: Vec<u8>,
}

/// InboundPacketDispatcher - Inbound packet dispatcher
///
/// # Design Principles
/// - **Single Responsibility**: Only responsible for routing inbound packets
/// - **Zero-copy**: Pass bytes directly, no deserialization until needed
/// - **Thread-safe**: All components wrapped in Arc
/// - **Response matching**: Wake up pending_requests via OutprocOutGate
///
/// # Note on MediaTrack
/// MediaTrack (PayloadType::MediaRtp) is NOT handled here because:
/// - MediaTrack uses WebRTC native RTP channels, not DataChannel
/// - Media frames are delivered directly via RTCTrackRemote callbacks
/// - No protobuf serialization involved
/// - MediaFrameRegistry is registered at WebRTC PeerConnection level
pub struct InboundPacketDispatcher {
    /// Mailbox (state path: Signal/Reliable)
    mailbox: Arc<dyn Mailbox>,

    /// DataStreamRegistry (fast path: LatencyFirst)
    data_stream_registry: Arc<DataStreamRegistry>,

    /// OutprocOutGate (for waking up pending_requests)
    outproc_out_gate: Option<Arc<OutprocOutGate>>,
}

impl InboundPacketDispatcher {
    /// Create new InboundPacketDispatcher
    ///
    /// # Arguments
    /// - `mailbox`: Mailbox instance
    /// - `data_stream_registry`: DataStream registry
    /// - `outproc_out_gate`: OutprocOutGate instance (optional, for RPC response matching)
    pub fn new(
        mailbox: Arc<dyn Mailbox>,
        data_stream_registry: Arc<DataStreamRegistry>,
        outproc_out_gate: Option<Arc<OutprocOutGate>>,
    ) -> Self {
        Self {
            mailbox,
            data_stream_registry,
            outproc_out_gate,
        }
    }

    /// Dispatch inbound packet
    ///
    /// # Core Logic
    /// 1. Check if this is an RPC response (has request_id)
    /// 2. If response, wake up OutprocOutGate.pending_requests
    /// 3. If request, route to appropriate handler by PayloadType
    ///
    /// # Arguments
    /// - `packet`: Inbound packet
    pub async fn dispatch(&self, packet: InboundPacket) {
        tracing::debug!(
            "📥 InboundPacketDispatcher::dispatch: payload_type={:?}, size={}",
            packet.payload_type,
            packet.data.len()
        );

        // Route based on PayloadType
        match packet.payload_type {
            PayloadType::RpcReliable | PayloadType::RpcSignal => {
                // State path: enqueue to Mailbox (RpcEnvelope only)
                self.dispatch_to_mailbox(packet).await;
            }
            PayloadType::StreamReliable | PayloadType::StreamLatencyFirst => {
                // Fast path: DataStream (both reliable and low-latency)
                self.dispatch_to_data_stream(packet).await;
            }
            PayloadType::MediaRtp => {
                // MediaRtp packets should NOT arrive here!
                // MediaTrack uses WebRTC native RTP channels (RTCTrackRemote),
                // not DataChannel, so they bypass InboundPacketDispatcher entirely.
                tracing::error!(
                    "❌ MediaRtp packet received in DataChannel dispatcher! \
                     This should never happen. MediaTrack frames are delivered \
                     via RTCTrackRemote callbacks, not through InboundPacketDispatcher."
                );
            }
        }
    }

    /// Dispatch to Mailbox (state path)
    ///
    /// # Design
    /// - Signal → High Priority
    /// - Reliable → Normal Priority
    /// - Store raw bytes directly, no RpcEnvelope deserialization
    async fn dispatch_to_mailbox(&self, packet: InboundPacket) {
        let priority = match packet.payload_type {
            PayloadType::RpcSignal => MessagePriority::High,
            PayloadType::RpcReliable => MessagePriority::Normal,
            PayloadType::StreamReliable
            | PayloadType::StreamLatencyFirst
            | PayloadType::MediaRtp => {
                tracing::error!(
                    "❌ Invalid PayloadType for Mailbox: {:?}",
                    packet.payload_type
                );
                return;
            }
        };

        match self
            .mailbox
            .enqueue(packet.from, packet.data, priority)
            .await
        {
            Ok(msg_id) => {
                tracing::debug!("✅ Packet enqueued: id={}, priority={:?}", msg_id, priority);
            }
            Err(e) => {
                tracing::error!("❌ Failed to enqueue packet: {:?}", e);
            }
        }
    }

    /// Dispatch to DataStreamRegistry (fast path)
    ///
    /// # Design
    /// - Decode DataStream
    /// - Decode sender ActrId
    /// - Invoke callback concurrently
    async fn dispatch_to_data_stream(&self, packet: InboundPacket) {
        use actr_protocol::prost::Message as ProstMessage;

        // Decode DataStream
        match DataStream::decode(&packet.data[..]) {
            Ok(chunk) => {
                tracing::debug!("📦 Dispatching DataStream: stream_id={}", chunk.stream_id);

                // Decode sender ActrId
                match ActrId::decode(&packet.from[..]) {
                    Ok(sender_id) => {
                        self.data_stream_registry.dispatch(chunk, sender_id).await;
                    }
                    Err(e) => {
                        tracing::error!("❌ Failed to decode sender ActrId: {:?}", e);
                    }
                }
            }
            Err(e) => {
                tracing::error!("❌ Failed to decode DataStream: {:?}", e);
            }
        }
    }

    /// Handle RPC response (wake up pending_requests)
    ///
    /// # Design
    /// - Only handle if outproc_out_gate exists
    /// - Decode RpcEnvelope, extract request_id
    /// - Call OutprocOutGate::handle_response to wake up waiting request
    ///
    /// # Returns
    /// - `true`: Successfully matched and woke up
    /// - `false`: No matching pending request found
    pub async fn handle_response(&self, envelope: RpcEnvelope) -> bool {
        if let Some(outproc_out_gate) = &self.outproc_out_gate {
            // Convert envelope to result (success or error)
            let result = match (envelope.payload, envelope.error) {
                (Some(payload), None) => Ok(payload),
                (None, Some(error)) => Err(actr_protocol::ProtocolError::TransportError(format!(
                    "RPC error {}: {}",
                    error.code, error.message
                ))),
                _ => Err(actr_protocol::ProtocolError::DecodeError(
                    "Invalid RpcEnvelope: payload and error fields inconsistent".to_string(),
                )),
            };

            match outproc_out_gate
                .handle_response(&envelope.request_id, result)
                .await
            {
                Ok(matched) => matched,
                Err(e) => {
                    tracing::error!("❌ Failed to handle response: {:?}", e);
                    false
                }
            }
        } else {
            tracing::warn!("⚠️ No OutprocOutGate available for response handling");
            false
        }
    }
}
