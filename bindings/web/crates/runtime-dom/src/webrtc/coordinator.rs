//! WebRTC coordinator for the DOM side.
//!
//! Responsibilities:
//! - Receive P2P creation requests from the Service Worker
//! - Create PeerConnections through the WebRTC APIs
//! - Notify the Service Worker when setup completes

use crate::error_reporter::get_global_error_reporter;
use actr_web_common::{
    ControlMessage, CreateP2PRequest, Dest, ErrorSeverity, P2PReadyEvent, WebError, WebResult,
};
use dashmap::DashMap;
use parking_lot::Mutex;
use std::sync::Arc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{RtcConfiguration, RtcDataChannel, RtcDataChannelInit, RtcPeerConnection};

/// WebRTC coordinator for the DOM side.
///
/// This is a pure helper role: it does not manage live connections and only
/// creates them before notifying the Service Worker.
pub struct WebRtcCoordinator {
    /// Service Worker channel used to receive requests and send ready events.
    sw_channel: Arc<Mutex<Option<super::super::transport::lane::DataLane>>>,

    /// Active PeerConnections keyed by peer ID.
    peer_connections: Arc<DashMap<String, RtcPeerConnection>>,

    /// ICE server configuration.
    ice_servers: Vec<String>,
}

impl WebRtcCoordinator {
    /// Create a new WebRTC coordinator.
    pub fn new(ice_servers: Vec<String>) -> Self {
        Self {
            sw_channel: Arc::new(Mutex::new(None)),
            peer_connections: Arc::new(DashMap::new()),
            ice_servers,
        }
    }

    /// Set the Service Worker channel.
    pub fn set_sw_channel(&self, channel: super::super::transport::lane::DataLane) {
        let mut sw = self.sw_channel.lock();
        *sw = Some(channel);
        log::info!("[WebRtcCoordinator] SW channel set");
    }

    /// Start the event loop that receives Service Worker requests.
    pub fn start(&self) {
        let sw_channel = Arc::clone(&self.sw_channel);
        let peer_connections = Arc::clone(&self.peer_connections);
        let ice_servers = self.ice_servers.clone();

        wasm_bindgen_futures::spawn_local(async move {
            log::info!("[WebRtcCoordinator] Event loop started");

            loop {
                // Acquire the channel.
                let channel = {
                    let sw = sw_channel.lock();
                    sw.as_ref().cloned()
                };

                if let Some(lane) = channel {
                    match lane.recv().await {
                        Some(data) => {
                            // Parse the control message.
                            match ControlMessage::deserialize(&data) {
                                Ok(ControlMessage::CreateP2P(request)) => {
                                    log::info!(
                                        "[WebRtcCoordinator] Received P2P request: {:?}",
                                        request.dest
                                    );

                                    // Handle the request asynchronously.
                                    Self::handle_create_p2p(
                                        request,
                                        peer_connections.clone(),
                                        lane.clone(),
                                        ice_servers.clone(),
                                    );
                                }
                                Ok(ControlMessage::P2PReady(_)) => {
                                    // Ignore SW-originated ready events; they are for the opposite direction.
                                    log::trace!("[WebRtcCoordinator] Ignoring P2PReady from SW");
                                }
                                Ok(ControlMessage::ErrorReport(_)) => {
                                    // Ignore SW-originated error reports; they are for the opposite direction.
                                    log::trace!("[WebRtcCoordinator] Ignoring ErrorReport from SW");
                                }
                                Err(e) => {
                                    log::error!(
                                        "[WebRtcCoordinator] Failed to parse control message: {}",
                                        e
                                    );
                                }
                            }
                        }
                        None => {
                            log::warn!("[WebRtcCoordinator] SW channel closed");
                            break;
                        }
                    }
                } else {
                    log::warn!("[WebRtcCoordinator] SW channel not available");
                    break;
                }
            }

            log::info!("[WebRtcCoordinator] Event loop stopped");
        });
    }

    /// Handle a P2P creation request asynchronously.
    fn handle_create_p2p(
        request: CreateP2PRequest,
        peer_connections: Arc<DashMap<String, RtcPeerConnection>>,
        sw_channel: super::super::transport::lane::DataLane,
        ice_servers: Vec<String>,
    ) {
        wasm_bindgen_futures::spawn_local(async move {
            let peer_id = match &request.dest {
                Dest::Peer(id) => id.clone(),
                _ => {
                    log::error!(
                        "[WebRtcCoordinator] Invalid dest for P2P: {:?}",
                        request.dest
                    );
                    Self::send_failure(
                        request.request_id,
                        request.dest,
                        "Invalid dest type".to_string(),
                        sw_channel,
                    )
                    .await;
                    return;
                }
            };

            log::info!("[WebRtcCoordinator] Creating P2P for: {}", peer_id);

            match Self::create_peer_connection(&peer_id, ice_servers).await {
                Ok(pc) => {
                    // Store the PeerConnection.
                    peer_connections.insert(peer_id.clone(), pc);

                    log::info!("[WebRtcCoordinator] P2P created successfully: {}", peer_id);

                    // Send a success event.
                    Self::send_success(request.request_id, request.dest, sw_channel).await;
                }
                Err(e) => {
                    log::error!(
                        "[WebRtcCoordinator] P2P creation failed: {}: {}",
                        peer_id,
                        e
                    );

                    // Report the error.
                    if let Some(reporter) = get_global_error_reporter() {
                        reporter.report_webrtc_error(
                            &request.dest,
                            format!("Failed to create P2P connection: {}", e),
                            ErrorSeverity::Error,
                        );
                    }

                    // Send a failure event.
                    Self::send_failure(request.request_id, request.dest, e.to_string(), sw_channel)
                        .await;
                }
            }
        });
    }

    /// Create a PeerConnection.
    async fn create_peer_connection(
        peer_id: &str,
        ice_servers: Vec<String>,
    ) -> WebResult<RtcPeerConnection> {
        // 1. Build the configuration.
        let config = RtcConfiguration::new();

        // Configure ICE servers. The current implementation only logs them.
        if !ice_servers.is_empty() {
            // TODO: Apply the actual ICE server configuration.
            log::debug!("[WebRtcCoordinator] ICE servers: {:?}", ice_servers);
        }

        // 2. Create the PeerConnection.
        let pc = RtcPeerConnection::new_with_configuration(&config).map_err(|e| {
            WebError::Transport(format!("Failed to create PeerConnection: {:?}", e))
        })?;

        log::debug!("[WebRtcCoordinator] PeerConnection created: {}", peer_id);

        // 3. Create the DataChannel.
        let dc_config = RtcDataChannelInit::new();
        dc_config.set_ordered(true);

        let dc = pc.create_data_channel_with_data_channel_dict("data", &dc_config);

        log::debug!("[WebRtcCoordinator] DataChannel created: {}", peer_id);

        // 4. Install event handlers.
        Self::setup_datachannel_handlers(&dc, peer_id);

        // 5. Create the offer. Real SDP exchange still needs a signaling server.
        // TODO: Implement actual SDP exchange through signaling.
        log::warn!("[WebRtcCoordinator] SDP exchange not implemented yet (Phase 4)");

        Ok(pc)
    }

    /// Set DataChannel event handlers.
    fn setup_datachannel_handlers(dc: &RtcDataChannel, peer_id: &str) {
        let peer_id = peer_id.to_string();

        // onopen
        let peer_id_clone = peer_id.clone();
        let onopen = Closure::wrap(Box::new(move || {
            log::info!("[WebRtcCoordinator] DataChannel opened: {}", peer_id_clone);
        }) as Box<dyn FnMut()>);
        dc.set_onopen(Some(onopen.as_ref().unchecked_ref()));
        onopen.forget();

        // onerror
        let peer_id_clone = peer_id.clone();
        let onerror = Closure::wrap(Box::new(move |e: web_sys::ErrorEvent| {
            let error_msg = e.message();
            log::error!(
                "[WebRtcCoordinator] DataChannel error: {}: {:?}",
                peer_id_clone,
                error_msg
            );

            // Report the error.
            if let Some(reporter) = get_global_error_reporter() {
                reporter.report_webrtc_error(
                    &Dest::Peer(peer_id_clone.clone()),
                    format!("DataChannel error: {:?}", error_msg),
                    ErrorSeverity::Error,
                );
            }
        }) as Box<dyn FnMut(_)>);
        dc.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        onerror.forget();

        // onclose
        let onclose = Closure::wrap(Box::new(move || {
            log::info!("[WebRtcCoordinator] DataChannel closed: {}", peer_id);
        }) as Box<dyn FnMut()>);
        dc.set_onclose(Some(onclose.as_ref().unchecked_ref()));
        onclose.forget();
    }

    /// Send a success event to the Service Worker.
    async fn send_success(
        request_id: String,
        dest: Dest,
        sw_channel: super::super::transport::lane::DataLane,
    ) {
        let event = P2PReadyEvent::success(request_id, dest);
        let msg = ControlMessage::P2PReady(event);

        match msg.serialize() {
            Ok(data) => {
                if let Err(e) = sw_channel.send(data).await {
                    log::error!("[WebRtcCoordinator] Failed to send success event: {}", e);
                }
            }
            Err(e) => {
                log::error!(
                    "[WebRtcCoordinator] Failed to serialize success event: {}",
                    e
                );
            }
        }
    }

    /// Send a failure event to the Service Worker.
    async fn send_failure(
        request_id: String,
        dest: Dest,
        error: String,
        sw_channel: super::super::transport::lane::DataLane,
    ) {
        let event = P2PReadyEvent::failure(request_id, dest.clone(), error.clone());
        let msg = ControlMessage::P2PReady(event);

        match msg.serialize() {
            Ok(data) => {
                if let Err(e) = sw_channel.send(data).await {
                    log::error!("[WebRtcCoordinator] Failed to send failure event: {}", e);

                    // Report the MessagePort error.
                    if let Some(reporter) = get_global_error_reporter() {
                        reporter.report_messageport_error(
                            format!("Failed to send P2P failure event: {}", e),
                            ErrorSeverity::Warning,
                        );
                    }
                }
            }
            Err(e) => {
                log::error!(
                    "[WebRtcCoordinator] Failed to serialize failure event: {}",
                    e
                );

                // Report the serialization error.
                if let Some(reporter) = get_global_error_reporter() {
                    reporter.report_messageport_error(
                        format!("Failed to serialize P2P failure event: {}", e),
                        ErrorSeverity::Warning,
                    );
                }
            }
        }
    }

    /// Close the connection for the given peer.
    pub fn close_peer(&self, peer_id: &str) -> WebResult<()> {
        if let Some((_, pc)) = self.peer_connections.remove(peer_id) {
            pc.close();
            log::info!("[WebRtcCoordinator] Closed peer connection: {}", peer_id);
        }
        Ok(())
    }

    /// Close all connections.
    pub fn close_all(&self) -> WebResult<()> {
        log::info!(
            "[WebRtcCoordinator] Closing all peer connections (count: {})",
            self.peer_connections.len()
        );

        for entry in self.peer_connections.iter() {
            entry.value().close();
        }

        self.peer_connections.clear();
        Ok(())
    }
}

impl Default for WebRtcCoordinator {
    fn default() -> Self {
        Self::new(vec!["stun:stun.l.google.com:19302".to_string()])
    }
}
