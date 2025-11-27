//! WebRTC negotiator
//!
//! Responsible for WebRTC Connect's Offer/Answer protocol quotient

use crate::transport::error::NetworkResult;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

/// WebRTC configuration
#[derive(Clone, Debug, Default)]
pub struct WebRtcConfig {
    /// ICE service device list
    pub ice_servers: Vec<IceServer>,
}

/// ICE service device configuration
#[derive(Clone, Debug)]
pub struct IceServer {
    /// service device URL list
    pub urls: Vec<String>,

    /// usage user name （TURN service device need）
    pub username: Option<String>,

    /// credential verify （TURN service device need）
    pub credential: Option<String>,
}

/// WebRTC negotiator
pub struct WebRtcNegotiator {
    /// WebRTC configuration
    config: WebRtcConfig,
}

impl WebRtcNegotiator {
    /// Create newnegotiator
    ///
    /// # Arguments
    /// - `config`: WebRTC configuration
    pub fn new(config: WebRtcConfig) -> Self {
        Self { config }
    }

    /// usingdefaultconfigurationCreatenegotiator
    pub fn with_defaults() -> Self {
        Self {
            config: WebRtcConfig::default(),
        }
    }

    /// Create RTCPeerConnection
    ///
    /// # Returns
    /// newCreate's PeerConnection
    #[tracing::instrument(
        level = "info",
        skip(self),
        fields(ice_servers = self.config.ice_servers.len())
    )]
    pub async fn create_peer_connection(&self) -> NetworkResult<RTCPeerConnection> {
        use webrtc::api::APIBuilder;
        use webrtc::api::media_engine::MediaEngine;
        use webrtc::ice_transport::ice_server::RTCIceServer;
        use webrtc::peer_connection::configuration::RTCConfiguration;
        use webrtc::rtp_transceiver::rtp_codec::{
            RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType,
        };

        // Create MediaEngine and register codecs
        let mut media_engine = MediaEngine::default();

        // Register VP8 video codec
        media_engine.register_codec(
            RTCRtpCodecParameters {
                capability: RTCRtpCodecCapability {
                    mime_type: "video/VP8".to_owned(),
                    clock_rate: 90000,
                    channels: 0,
                    sdp_fmtp_line: "".to_owned(),
                    rtcp_feedback: vec![],
                },
                payload_type: 96,
                ..Default::default()
            },
            RTPCodecType::Video,
        )?;

        // Register H264 video codec
        media_engine.register_codec(
            RTCRtpCodecParameters {
                capability: RTCRtpCodecCapability {
                    mime_type: "video/H264".to_owned(),
                    clock_rate: 90000,
                    channels: 0,
                    sdp_fmtp_line:
                        "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"
                            .to_owned(),
                    rtcp_feedback: vec![],
                },
                payload_type: 102,
                ..Default::default()
            },
            RTPCodecType::Video,
        )?;

        // Register OPUS audio codec
        media_engine.register_codec(
            RTCRtpCodecParameters {
                capability: RTCRtpCodecCapability {
                    mime_type: "audio/opus".to_owned(),
                    clock_rate: 48000,
                    channels: 2,
                    sdp_fmtp_line: "minptime=10;useinbandfec=1".to_owned(),
                    rtcp_feedback: vec![],
                },
                payload_type: 111,
                ..Default::default()
            },
            RTPCodecType::Audio,
        )?;

        // convert exchange ICE service device configuration
        let ice_servers: Vec<RTCIceServer> = self
            .config
            .ice_servers
            .iter()
            .map(|server| RTCIceServer {
                urls: server.urls.clone(),
                username: server.username.clone().unwrap_or_default(),
                credential: server.credential.clone().unwrap_or_default(),
            })
            .collect();

        // Create WebRTC configuration
        let rtc_config = RTCConfiguration {
            ice_servers,
            ..Default::default()
        };

        // Create API with MediaEngine
        let api = APIBuilder::new().with_media_engine(media_engine).build();

        // Create PeerConnection
        let peer_connection = api.new_peer_connection(rtc_config).await?;

        tracing::info!("✅ Create RTCPeerConnection with VP8, H264, OPUS codecs");

        Ok(peer_connection)
    }

    /// Create Offer (Trickle ICE mode)
    ///
    /// # Arguments
    /// - `peer_connection`: PeerConnection
    ///
    /// # Returns
    /// Offer SDP string (ICE candidates sent separately via on_ice_candidate callback)
    #[tracing::instrument(level = "info", skip_all)]
    pub async fn create_offer(&self, peer_connection: &RTCPeerConnection) -> NetworkResult<String> {
        // Note: Negotiated DataChannel should be created BEFORE calling this method
        // to trigger ICE gathering (done in coordinator.rs)

        // Create Offer
        let offer = peer_connection.create_offer(None).await?;
        let offer_sdp = offer.sdp.clone();

        // Set local Description (this triggers ICE gathering)
        peer_connection.set_local_description(offer).await?;

        // DO NOT wait for ICE gathering - this is Trickle ICE
        // ICE candidates will be sent via on_ice_candidate callback

        tracing::info!(
            "✅ Create Offer (SDP length: {}, Trickle ICE mode)",
            offer_sdp.len()
        );

        Ok(offer_sdp)
    }

    /// Handle Answer
    ///
    /// # Arguments
    /// - `peer_connection`: PeerConnection
    /// - `answer_sdp`: Answer SDP string
    #[tracing::instrument(
        level = "info",
        skip_all,
        fields(answer_len = answer_sdp.len())
    )]
    pub async fn handle_answer(
        &self,
        peer_connection: &RTCPeerConnection,
        answer_sdp: String,
    ) -> NetworkResult<()> {
        // Setremote Description
        let answer = RTCSessionDescription::answer(answer_sdp)?;
        peer_connection.set_remote_description(answer).await?;

        tracing::info!("✅ Handle Answer");

        Ok(())
    }

    /// Create Answer (passive side, Trickle ICE mode)
    ///
    /// # Arguments
    /// - `peer_connection`: PeerConnection
    /// - `offer_sdp`: Offer SDP string
    ///
    /// # Returns
    /// Answer SDP string (ICE candidates sent separately)
    #[tracing::instrument(level = "info", skip_all)]
    pub async fn create_answer(
        &self,
        peer_connection: &RTCPeerConnection,
        offer_sdp: String,
    ) -> NetworkResult<String> {
        // Set remote Description（Offer）
        let offer = RTCSessionDescription::offer(offer_sdp)?;
        peer_connection.set_remote_description(offer).await?;

        // Create Answer
        let answer = peer_connection.create_answer(None).await?;
        let answer_sdp = answer.sdp.clone();

        // Set local Description (triggers ICE gathering)
        peer_connection.set_local_description(answer).await?;

        // DO NOT wait for ICE gathering - Trickle ICE mode
        // ICE candidates will be sent via on_ice_candidate callback

        tracing::info!(
            "✅ Create Answer (SDP length: {}, Trickle ICE mode)",
            answer_sdp.len()
        );

        Ok(answer_sdp)
    }

    /// add ICE Candidate
    ///
    /// # Arguments
    /// - `peer_connection`: PeerConnection
    /// - `candidate`: ICE Candidate string
    #[tracing::instrument(
        level = "trace",
        skip_all,
        fields(candidate_len = candidate.len())
    )]
    pub async fn add_ice_candidate(
        &self,
        peer_connection: &RTCPeerConnection,
        candidate: String,
    ) -> NetworkResult<()> {
        use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;

        let ice_candidate = RTCIceCandidateInit {
            candidate,
            ..Default::default()
        };

        peer_connection.add_ice_candidate(ice_candidate).await?;

        tracing::trace!("✅ add ICE Candidate");

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = WebRtcConfig::default();
        // Default config has no ICE servers for local-to-local scenarios
        assert!(config.ice_servers.is_empty());
    }

    #[test]
    fn test_negotiator_creation() {
        let negotiator = WebRtcNegotiator::with_defaults();
        // Default negotiator has no ICE servers for local-to-local scenarios
        assert!(negotiator.config.ice_servers.is_empty());
    }

    #[test]
    fn test_custom_ice_servers() {
        let config = WebRtcConfig {
            ice_servers: vec![IceServer {
                urls: vec!["stun:stun.l.google.com:19302".to_string()],
                username: None,
                credential: None,
            }],
        };
        let negotiator = WebRtcNegotiator::new(config);
        assert!(!negotiator.config.ice_servers.is_empty());
        assert_eq!(negotiator.config.ice_servers.len(), 1);
    }
}
