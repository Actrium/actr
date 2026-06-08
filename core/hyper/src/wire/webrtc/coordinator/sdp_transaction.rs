use super::{PeerState, WebRtcCoordinator};
use actr_protocol::session_description::Type as SdpType;
use actr_protocol::{ActorResult, ActrError, ActrId, SessionDescription};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

#[derive(Clone, Debug, Default)]
pub(super) struct SdpTransactionMetadata {
    pub(super) negotiation_id: Option<String>,
    pub(super) offer_version: Option<u64>,
    pub(super) ice_ufrag: Option<String>,
}

impl WebRtcCoordinator {
    pub(super) fn extract_ice_ufrag(sdp: &str) -> Option<String> {
        Self::extract_ice_ufrag_from_parsed_sdp(sdp)
            .or_else(|| Self::extract_ice_ufrag_from_sdp_lines(sdp))
    }

    fn extract_ice_ufrag_from_parsed_sdp(sdp: &str) -> Option<String> {
        let mut description = RTCSessionDescription::default();
        description.sdp = sdp.to_owned();
        let parsed = description.unmarshal().ok()?;

        parsed
            .attribute("ice-ufrag")
            .filter(|ufrag| !ufrag.is_empty())
            .cloned()
            .or_else(|| {
                parsed.media_descriptions.iter().find_map(|media| {
                    media
                        .attribute("ice-ufrag")
                        .and_then(|value| value)
                        .map(str::trim)
                        .filter(|ufrag| !ufrag.is_empty())
                        .map(ToOwned::to_owned)
                })
            })
    }

    fn extract_ice_ufrag_from_sdp_lines(sdp: &str) -> Option<String> {
        sdp.lines().find_map(|line| {
            let line = line.trim();
            let ufrag = line.strip_prefix("a=ice-ufrag:")?.trim();
            (!ufrag.is_empty()).then(|| ufrag.to_owned())
        })
    }

    pub(super) fn metadata_from_session_description(
        sd: &SessionDescription,
    ) -> SdpTransactionMetadata {
        SdpTransactionMetadata {
            negotiation_id: sd.negotiation_id.clone(),
            offer_version: sd.offer_version,
            ice_ufrag: sd
                .ice_ufrag
                .clone()
                .or_else(|| Self::extract_ice_ufrag(&sd.sdp)),
        }
    }

    pub(super) fn build_session_description(
        sdp_type: SdpType,
        sdp: String,
        metadata: &SdpTransactionMetadata,
    ) -> SessionDescription {
        SessionDescription {
            r#type: sdp_type as i32,
            sdp,
            negotiation_id: metadata.negotiation_id.clone(),
            offer_version: metadata.offer_version,
            ice_ufrag: metadata.ice_ufrag.clone(),
        }
    }

    async fn next_local_offer_version(
        local_offer_versions: &Arc<RwLock<HashMap<ActrId, u64>>>,
        peer_id: &ActrId,
    ) -> u64 {
        let mut versions = local_offer_versions.write().await;
        let version = versions.entry(peer_id.clone()).or_insert(0);
        *version = version.saturating_add(1);
        *version
    }

    pub(super) async fn begin_local_offer_transaction_inner(
        peers: &Arc<RwLock<HashMap<ActrId, PeerState>>>,
        local_offer_versions: &Arc<RwLock<HashMap<ActrId, u64>>>,
        peer_id: &ActrId,
        expected_session_id: Option<u64>,
    ) -> ActorResult<SdpTransactionMetadata> {
        let offer_version = Self::next_local_offer_version(local_offer_versions, peer_id).await;
        let negotiation_id = uuid::Uuid::new_v4().to_string();

        let mut peers_guard = peers.write().await;
        let state = peers_guard.get_mut(peer_id).ok_or_else(|| {
            ActrError::Internal(format!(
                "Peer not found while starting SDP transaction: {}",
                peer_id
            ))
        })?;
        if let Some(expected) = expected_session_id {
            if state.session_id != expected {
                return Err(ActrError::Internal(format!(
                    "Stale peer session while starting SDP transaction: peer={}, active_session_id={}, expected_session_id={}",
                    peer_id, state.session_id, expected
                )));
            }
        }

        state.negotiation_id = Some(negotiation_id.clone());
        state.offer_version = Some(offer_version);
        state.local_ice_ufrag = None;

        Ok(SdpTransactionMetadata {
            negotiation_id: Some(negotiation_id),
            offer_version: Some(offer_version),
            ice_ufrag: None,
        })
    }

    pub(super) async fn begin_local_offer_transaction(
        &self,
        peer_id: &ActrId,
        expected_session_id: Option<u64>,
    ) -> ActorResult<SdpTransactionMetadata> {
        Self::begin_local_offer_transaction_inner(
            &self.peers,
            &self.local_offer_versions,
            peer_id,
            expected_session_id,
        )
        .await
    }

    pub(super) async fn complete_local_offer_transaction_for_peer(
        peers: &Arc<RwLock<HashMap<ActrId, PeerState>>>,
        peer_id: &ActrId,
        expected_session_id: Option<u64>,
        metadata: &SdpTransactionMetadata,
        sdp: &str,
    ) -> ActorResult<SdpTransactionMetadata> {
        let mut metadata = metadata.clone();
        metadata.ice_ufrag = Self::extract_ice_ufrag(sdp);

        let mut peers_guard = peers.write().await;
        let state = peers_guard.get_mut(peer_id).ok_or_else(|| {
            ActrError::Internal(format!(
                "Peer not found while completing SDP transaction: {}",
                peer_id
            ))
        })?;
        if let Some(expected) = expected_session_id {
            if state.session_id != expected {
                return Err(ActrError::Internal(format!(
                    "Stale peer session while completing SDP transaction: peer={}, active_session_id={}, expected_session_id={}",
                    peer_id, state.session_id, expected
                )));
            }
        }

        if state.negotiation_id != metadata.negotiation_id {
            return Err(ActrError::Internal(format!(
                "SDP transaction id changed while creating offer: peer={}, expected={:?}, active={:?}",
                peer_id, metadata.negotiation_id, state.negotiation_id
            )));
        }

        state.local_ice_ufrag = metadata.ice_ufrag.clone();
        Ok(metadata)
    }

    pub(super) async fn complete_local_offer_transaction(
        &self,
        peer_id: &ActrId,
        expected_session_id: Option<u64>,
        metadata: &SdpTransactionMetadata,
        sdp: &str,
    ) -> ActorResult<SdpTransactionMetadata> {
        Self::complete_local_offer_transaction_for_peer(
            &self.peers,
            peer_id,
            expected_session_id,
            metadata,
            sdp,
        )
        .await
    }

    pub(super) async fn is_stale_remote_offer(
        &self,
        peer_id: &ActrId,
        sd: &SessionDescription,
    ) -> bool {
        let Some(version) = sd.offer_version else {
            tracing::warn!(
                "⏭️ Ignoring remote offer from {}: missing offer_version",
                peer_id
            );
            return true;
        };

        let versions = self.remote_offer_versions.read().await;
        if let Some(last_seen) = versions.get(peer_id).copied()
            && version < last_seen
        {
            tracing::warn!(
                "⏭️ Ignoring stale remote offer from {}, version={} < last_seen={}",
                peer_id,
                version,
                last_seen
            );
            return true;
        }
        false
    }

    pub(super) async fn record_remote_offer_version(
        &self,
        peer_id: &ActrId,
        sd: &SessionDescription,
    ) {
        if let Some(version) = sd.offer_version {
            let mut versions = self.remote_offer_versions.write().await;
            let entry = versions.entry(peer_id.clone()).or_insert(0);
            *entry = (*entry).max(version);
        }
    }

    pub(super) fn answer_matches_current_offer(
        state: &PeerState,
        answer: &SessionDescription,
    ) -> bool {
        if let (Some(expected), Some(actual)) = (
            state.negotiation_id.as_ref(),
            answer.negotiation_id.as_ref(),
        ) && expected != actual
        {
            tracing::warn!(
                "⏭️ Ignoring stale Answer: negotiation_id mismatch, expected={}, actual={}",
                expected,
                actual
            );
            return false;
        }

        if let Some(expected_version) = state.offer_version {
            match answer.offer_version {
                Some(actual_version) if expected_version == actual_version => {}
                Some(actual_version) => {
                    tracing::warn!(
                        "⏭️ Ignoring stale Answer: offer_version mismatch, expected={}, actual={}",
                        expected_version,
                        actual_version
                    );
                    return false;
                }
                None => {
                    tracing::warn!(
                        "⏭️ Ignoring Answer: missing offer_version, expected={}",
                        expected_version
                    );
                    return false;
                }
            }
        }

        true
    }
}
