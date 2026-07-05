use super::*;
use crate::lifecycle::CredentialState;
use actr_protocol::AIdCredential;
use bytes::Bytes;
use std::time::Duration;

fn test_credential_state() -> CredentialState {
    CredentialState::new(
        AIdCredential {
            key_id: 7,
            claims: Bytes::from_static(b"claims"),
            signature: Bytes::from(vec![0u8; 64]),
        },
        None,
        None,
    )
}

/// Drive ICE gathering to completion on a peer connection and return the
/// `(ip, port)` pairs of the gathered host candidates.
async fn gather_host_candidates(pc: &RTCPeerConnection) -> Vec<(String, u16)> {
    let mut gather_done = pc.gathering_complete_promise().await;

    // A data channel is required so the offer has a media section that
    // triggers ICE gathering.
    let _dc = pc
        .create_data_channel("gather-probe", None)
        .await
        .expect("data channel should be created");
    let offer = pc.create_offer(None).await.expect("offer should be built");
    pc.set_local_description(offer)
        .await
        .expect("local description should be set");

    tokio::time::timeout(Duration::from_secs(10), gather_done.recv())
        .await
        .expect("ICE gathering should complete within 10s");

    let sdp = pc
        .local_description()
        .await
        .expect("local description should exist")
        .sdp;

    // Candidate lines: "a=candidate:<foundation> <component> udp <priority> <ip> <port> typ host ..."
    sdp.lines()
        .filter(|line| line.starts_with("a=candidate:") && line.contains("typ host"))
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let ip = fields.get(4)?.to_string();
            let port = fields.get(5)?.parse::<u16>().ok()?;
            Some((ip, port))
        })
        .collect()
}

/// Smoke test for the opt-in shared ICE UDP mux (`ice_udp_mux_port`):
///
/// - default (unset): each peer connection gathers host candidates on its own
///   ephemeral sockets, so two connections never share an `(ip, port)` pair;
/// - set: every peer connection created by the negotiator advertises host
///   candidates on the single configured mux port.
///
/// Skipped (with a message) when the environment has no gatherable IPv4
/// interface, since host candidate gathering excludes loopback.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ice_udp_mux_port_controls_socket_sharing() {
    // --- Default config: per-connection ephemeral UDP (pre-mux behavior) ---
    let negotiator = WebRtcNegotiator::new(WebRtcConfig::default(), test_credential_state());
    let pc_a = negotiator
        .create_peer_connection(false, false)
        .await
        .expect("peer connection should be created");
    let pc_b = negotiator
        .create_peer_connection(false, false)
        .await
        .expect("peer connection should be created");

    let ephemeral_a = gather_host_candidates(&pc_a).await;
    let ephemeral_b = gather_host_candidates(&pc_b).await;
    let _ = pc_a.close().await;
    let _ = pc_b.close().await;

    if ephemeral_a.is_empty() || ephemeral_b.is_empty() {
        eprintln!(
            "skipping ice_udp_mux_port_controls_socket_sharing: \
             no IPv4 host candidates gathered in this environment"
        );
        return;
    }

    // Two ephemeral connections cannot share a bound (ip, port) pair.
    for pair in &ephemeral_a {
        assert!(
            !ephemeral_b.contains(pair),
            "ephemeral connections must not share sockets, but {pair:?} \
             appears in both candidate sets"
        );
    }

    // --- Mux config: one shared socket on the configured port ---
    // Reserve a free UDP port, then hand it to the negotiator config.
    let probe = tokio::net::UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, 0))
        .await
        .expect("probe socket should bind");
    let mux_port = probe
        .local_addr()
        .expect("probe socket should have an address")
        .port();
    drop(probe);

    let mut config = WebRtcConfig::default();
    config.advanced.ice_udp_mux_port = Some(mux_port);
    let negotiator = WebRtcNegotiator::new(config, test_credential_state());

    let pc_a = negotiator
        .create_peer_connection(false, false)
        .await
        .expect("muxed peer connection should be created");
    let pc_b = negotiator
        .create_peer_connection(true, false)
        .await
        .expect("muxed peer connection should be created");

    let muxed_a = gather_host_candidates(&pc_a).await;
    let muxed_b = gather_host_candidates(&pc_b).await;
    let _ = pc_a.close().await;
    let _ = pc_b.close().await;

    assert!(
        !muxed_a.is_empty() && !muxed_b.is_empty(),
        "muxed connections should gather host candidates \
         (ephemeral gathering worked in this environment)"
    );
    for (ip, port) in muxed_a.iter().chain(muxed_b.iter()) {
        assert_eq!(
            *port, mux_port,
            "every muxed host candidate must advertise the shared mux port \
             (candidate {ip}:{port}, expected port {mux_port})"
        );
    }
}
