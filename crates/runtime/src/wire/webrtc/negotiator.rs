//! WebRTC negotiator
//!
//! Responsible for WebRTC Connect's Offer/Answer protocol quotient

use crate::lifecycle::CredentialState;
use crate::transport::error::NetworkResult;
use actr_protocol::turn::Claims;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

// 从 actr-config 重新导出类型
pub use actr_config::{IceServer, IceTransportPolicy, WebRtcConfig};

/// WebRTC negotiator
#[derive(Clone)]
pub struct WebRtcNegotiator {
    /// Base WebRTC configuration (URLs + policy)
    config: WebRtcConfig,
    /// Realm id for TURN credential claims
    realm_id: u32,
    /// Latest credential state (token and PSK refreshes update this)
    credential_state: CredentialState,
}

impl WebRtcNegotiator {
    /// Create newnegotiator
    ///
    /// # Arguments
    /// - `config`: WebRTC configuration
    pub fn new(config: WebRtcConfig, realm_id: u32, credential_state: CredentialState) -> Self {
        Self {
            config,
            realm_id,
            credential_state,
        }
    }

    /// Create RTCPeerConnection
    ///
    /// # Arguments
    /// - `is_answerer`: true if this node is the answerer (passive side), false if offerer (default)
    ///
    /// # Returns
    /// newCreate's PeerConnection
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(level = "info", skip(self), fields(ice_servers = self.config.ice_servers.len(), is_answerer))
    )]
    pub async fn create_peer_connection(
        &self,
        is_answerer: bool,
    ) -> NetworkResult<RTCPeerConnection> {
        use webrtc::api::APIBuilder;
        use webrtc::api::media_engine::MediaEngine;
        use webrtc::ice_transport::ice_server::RTCIceServer;
        use webrtc::peer_connection::configuration::RTCConfiguration;
        use webrtc::peer_connection::policy::ice_transport_policy::RTCIceTransportPolicy;
        use webrtc::rtp_transceiver::rtp_codec::{
            RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType,
        };

        let credential = self.credential_state.credential().await;

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
        let psk = self
            .credential_state
            .psk()
            .await
            .expect("PSK must be available for TURN authentication");
        let ice_servers: Vec<RTCIceServer> = self
            .config
            .ice_servers
            .iter()
            .map(|server| {
                let is_turn = server
                    .urls
                    .iter()
                    .any(|url| url.starts_with("turn:") || url.starts_with("turns:"));

                if is_turn {
                    let claims = Claims {
                        realm_id: self.realm_id,
                        key_id: credential.token_key_id,
                        token: credential.encrypted_token.clone(),
                    };

                    RTCIceServer {
                        urls: server.urls.clone(),
                        username: claims.encode(),
                        credential: hex::encode(&psk),
                    }
                } else {
                    RTCIceServer {
                        urls: server.urls.clone(),
                        username: server.username.clone().unwrap_or_default(),
                        credential: server.credential.clone().unwrap_or_default(),
                    }
                }
            })
            .collect();

        if ice_servers.is_empty() {
            tracing::info!("🌐 No ICE servers configured; proceeding without STUN/TURN servers");
        }
        tracing::info!("🌐 ICE servers configured: {:?}", ice_servers);
        // Convert ICE transport policy
        let ice_transport_policy = match self.config.ice_transport_policy {
            IceTransportPolicy::All => RTCIceTransportPolicy::All,
            IceTransportPolicy::Relay => RTCIceTransportPolicy::Relay,
        };

        // Create WebRTC configuration
        let rtc_config = RTCConfiguration {
            ice_servers,
            ice_transport_policy,
            ..Default::default()
        };

        // Create SettingEngine with role-based configuration
        let mut setting_engine = webrtc::api::setting_engine::SettingEngine::default();

        // Apply ICE candidate acceptance wait times (for both Offerer and Answerer)
        self.apply_ice_wait_times(&mut setting_engine);

        // Apply advanced parameters (UDP ports, NAT 1:1) only for Answerer
        if is_answerer {
            tracing::info!("🎭 Applying advanced WebRTC parameters (Answerer mode)");
            self.apply_answerer_config(&mut setting_engine)?;
        } else {
            tracing::info!("🎭 Using default WebRTC configuration (Offerer mode)");
        }

        // Create API with MediaEngine and SettingEngine
        let api = APIBuilder::new()
            .with_media_engine(media_engine)
            .with_setting_engine(setting_engine)
            .build();

        // Create PeerConnection
        let peer_connection = api.new_peer_connection(rtc_config).await?;

        peer_connection.on_ice_connection_state_change(Box::new(move |state| {
            tracing::info!("🔄 ICE Connection State Changed: {:?}", state);
            Box::pin(async move {})
        }));

        peer_connection.on_ice_gathering_state_change(Box::new(move |state| {
            tracing::info!("🔄 ICE Gathering State Changed: {:?}", state);
            Box::pin(async move {})
        }));

        tracing::info!("✅ Create RTCPeerConnection with VP8, H264, OPUS codecs");

        Ok(peer_connection)
    }

    /// Apply ICE candidate acceptance wait times (for both Offerer and Answerer)
    fn apply_ice_wait_times(
        &self,
        setting_engine: &mut webrtc::api::setting_engine::SettingEngine,
    ) {
        use std::time::Duration;

        let advanced = &self.config.advanced;

        setting_engine.set_host_acceptance_min_wait(Some(Duration::from_millis(
            advanced.ice_host_acceptance_min_wait,
        )));
        setting_engine.set_srflx_acceptance_min_wait(Some(Duration::from_millis(
            advanced.ice_srflx_acceptance_min_wait,
        )));
        setting_engine.set_prflx_acceptance_min_wait(Some(Duration::from_millis(
            advanced.ice_prflx_acceptance_min_wait,
        )));
        setting_engine.set_relay_acceptance_min_wait(Some(Duration::from_millis(
            advanced.ice_relay_acceptance_min_wait,
        )));

        tracing::info!(
            "🔧 ICE wait times: host={}ms, srflx={}ms, prflx={}ms, relay={}ms",
            advanced.ice_host_acceptance_min_wait,
            advanced.ice_srflx_acceptance_min_wait,
            advanced.ice_prflx_acceptance_min_wait,
            advanced.ice_relay_acceptance_min_wait
        );
    }

    /// Apply Answerer-specific configuration (UDP ports, NAT 1:1 mapping)
    fn apply_answerer_config(
        &self,
        setting_engine: &mut webrtc::api::setting_engine::SettingEngine,
    ) -> NetworkResult<()> {
        use webrtc::ice::udp_network::{EphemeralUDP, UDPNetwork};
        use webrtc::ice_transport::ice_candidate_type::RTCIceCandidateType;

        let advanced = &self.config.advanced;

        // Apply UDP port strategy
        if let Some((min, max)) = advanced.udp_ports {
            let ephemeral = EphemeralUDP::new(min, max).map_err(|e| {
                crate::transport::error::NetworkError::Other(anyhow::anyhow!(
                    "Failed to create EphemeralUDP: {}",
                    e
                ))
            })?;
            setting_engine.set_udp_network(UDPNetwork::Ephemeral(ephemeral));
            tracing::info!("🔧 UDP port range: {}-{}", min, max);

            // Apply NAT 1:1 mapping (only when port range is configured)
            if !advanced.public_ips.is_empty() {
                setting_engine
                    .set_nat_1to1_ips(advanced.public_ips.clone(), RTCIceCandidateType::Srflx);
                tracing::info!("🔧 NAT 1:1 IPs: {:?}", advanced.public_ips);
            }
        } else {
            tracing::info!("🔧 Using default random UDP ports");
        }

        Ok(())
    }

    /// Create Offer (Trickle ICE mode)
    ///
    /// # Arguments
    /// - `peer_connection`: PeerConnection
    ///
    /// # Returns
    /// Offer SDP string (ICE candidates sent separately via on_ice_candidate callback)
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(level = "info", skip_all)
    )]
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

    /// Create ICE restart Offer (offerer side)
    pub async fn create_ice_restart_offer(
        &self,
        peer_connection: &RTCPeerConnection,
    ) -> NetworkResult<String> {
        use webrtc::peer_connection::offer_answer_options::RTCOfferOptions;

        let offer = peer_connection
            .create_offer(Some(RTCOfferOptions {
                ice_restart: true,
                ..Default::default()
            }))
            .await?;
        let offer_sdp = offer.sdp.clone();

        peer_connection.set_local_description(offer).await?;

        tracing::info!(
            "✅ Create ICE Restart Offer (SDP length: {}, Trickle ICE mode)",
            offer_sdp.len()
        );

        Ok(offer_sdp)
    }

    /// Handle Answer
    ///
    /// # Arguments
    /// - `peer_connection`: PeerConnection
    /// - `answer_sdp`: Answer SDP string
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(level = "info", skip_all, fields(answer_len = answer_sdp.len()))
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
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(level = "info", skip_all)
    )]
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
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(level = "trace", skip_all, fields(candidate_len = candidate.len()))
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
