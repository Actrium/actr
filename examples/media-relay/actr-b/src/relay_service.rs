//! Relay Service Implementation - receives and displays media frames

use async_trait::async_trait;
use actr_framework::Context;
use actr_protocol::ActorResult;

use crate::generated::media_relay::*;
use crate::generated::relay_service_actor::RelayServiceHandler;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Relay Service - receives media frames via RPC
pub struct RelayService {
    received_count: AtomicU64,
}

impl RelayService {
    pub fn new() -> Self {
        Self {
            received_count: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl RelayServiceHandler for RelayService {
    async fn relay_frame<C: Context>(
        &self,
        req: RelayFrameRequest,
        _ctx: &C,
    ) -> ActorResult<RelayFrameResponse> {
        let frame = req.frame.ok_or_else(|| {
            actr_protocol::ProtocolError::Actr(actr_protocol::ActrError::DecodeFailure {
                message: "MediaFrame is missing in RelayFrameRequest".to_string(),
            })
        })?;

        let count = self.received_count.fetch_add(1, Ordering::SeqCst) + 1;

        tracing::info!(
            "🎮 Received frame #{} (seq={}, ts={}, codec={}, size={} bytes)",
            count,
            frame.frame_number,
            frame.timestamp,
            frame.codec,
            frame.data.len()
        );

        let received_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| actr_protocol::ProtocolError::Actr(
                actr_protocol::ActrError::DecodeFailure {
                    message: format!("SystemTime error: {}", e),
                }
            ))?
            .as_millis() as u64;

        Ok(RelayFrameResponse {
            success: true,
            received_at,
        })
    }
}
