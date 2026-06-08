use std::time::Duration;

use actr_hyper::test_support::{TestHarness, make_actor_id};
use actr_protocol::{
    ActrId, IceCandidate, SessionDescription, SignalingEnvelope, actr_relay,
    session_description::Type as SdpType, signaling_envelope,
};

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_file(true)
        .with_line_number(true)
        .with_test_writer()
        .try_init()
        .ok();
}

fn collect_session_descriptions(
    messages: &[SignalingEnvelope],
) -> Vec<(ActrId, ActrId, SessionDescription)> {
    messages
        .iter()
        .filter_map(|envelope| {
            let signaling_envelope::Flow::ActrRelay(relay) = envelope.flow.as_ref()? else {
                return None;
            };
            let actr_relay::Payload::SessionDescription(sd) = relay.payload.as_ref()? else {
                return None;
            };
            Some((relay.source.clone(), relay.target.clone(), sd.clone()))
        })
        .collect()
}

fn collect_ice_candidates(messages: &[SignalingEnvelope]) -> Vec<(ActrId, ActrId, IceCandidate)> {
    messages
        .iter()
        .filter_map(|envelope| {
            let signaling_envelope::Flow::ActrRelay(relay) = envelope.flow.as_ref()? else {
                return None;
            };
            let actr_relay::Payload::IceCandidate(candidate) = relay.payload.as_ref()? else {
                return None;
            };
            Some((
                relay.source.clone(),
                relay.target.clone(),
                candidate.clone(),
            ))
        })
        .collect()
}

fn assert_sdp_metadata(sd: &SessionDescription) {
    assert!(
        sd.negotiation_id
            .as_deref()
            .map(|id| !id.is_empty())
            .unwrap_or(false),
        "SessionDescription must include negotiation_id: {:?}",
        sd
    );
    assert!(
        sd.offer_version.unwrap_or(0) > 0,
        "SessionDescription must include a positive offer_version: {:?}",
        sd
    );
    assert!(
        sd.ice_ufrag
            .as_deref()
            .map(|ufrag| !ufrag.is_empty())
            .unwrap_or(false),
        "SessionDescription must include ice_ufrag: {:?}",
        sd
    );
}

async fn wait_for_restart_transaction(
    harness: &TestHarness,
) -> Vec<(ActrId, ActrId, SessionDescription)> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    loop {
        let messages = harness.server.received_messages().await;
        let descriptions = collect_session_descriptions(&messages);
        let has_restart_offer = descriptions
            .iter()
            .any(|(_, _, sd)| sd.r#type() == SdpType::IceRestartOffer);
        let has_restart_answer = descriptions.iter().any(|(_, _, sd)| {
            sd.r#type() == SdpType::Answer
                && descriptions.iter().any(|(_, _, offer)| {
                    offer.r#type() == SdpType::IceRestartOffer
                        && offer.negotiation_id == sd.negotiation_id
                })
        });

        if has_restart_offer && has_restart_answer {
            return descriptions;
        }

        assert!(
            tokio::time::Instant::now() < deadline,
            "Timed out waiting for ICE restart offer/answer transaction; saw {:?}",
            descriptions
                .iter()
                .map(|(_, _, sd)| (sd.r#type(), sd.negotiation_id.clone(), sd.offer_version))
                .collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_metadata_candidate(
    harness: &TestHarness,
    source_id: &ActrId,
    target_id: &ActrId,
) -> IceCandidate {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    loop {
        let messages = harness.server.received_messages().await;
        let candidates = collect_ice_candidates(&messages);
        if let Some((_, _, candidate)) = candidates.iter().find(|(source, target, candidate)| {
            source == source_id
                && target == target_id
                && candidate.negotiation_id.is_some()
                && candidate.offer_version.is_some()
                && candidate
                    .username_fragment
                    .as_deref()
                    .map(|ufrag| !ufrag.is_empty())
                    .unwrap_or(false)
        }) {
            return candidate.clone();
        }

        assert!(
            tokio::time::Instant::now() < deadline,
            "Timed out waiting for ICE candidate with transaction metadata; saw {:?}",
            candidates
                .iter()
                .map(|(source, target, candidate)| {
                    (
                        source.clone(),
                        target.clone(),
                        candidate.negotiation_id.clone(),
                        candidate.offer_version,
                        candidate.username_fragment.clone(),
                    )
                })
                .collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdp_transactions_are_versioned_and_answers_echo_offer_metadata() {
    init_tracing();

    let mut harness = TestHarness::new().await;
    harness.add_peer(100).await;
    harness.add_peer(200).await;
    harness.connect(100, 200).await;

    harness
        .peer(100)
        .restart_ice(200)
        .await
        .expect("ICE restart should start");

    let descriptions = wait_for_restart_transaction(&harness).await;

    let initial_offer = descriptions
        .iter()
        .find(|(_, _, sd)| sd.r#type() == SdpType::Offer)
        .expect("initial offer should be signaled");
    let initial_offer_sd = &initial_offer.2;
    assert_sdp_metadata(initial_offer_sd);

    let initial_answer = descriptions
        .iter()
        .find(|(_, _, sd)| {
            sd.r#type() == SdpType::Answer && sd.negotiation_id == initial_offer_sd.negotiation_id
        })
        .expect("initial answer should echo initial offer metadata");
    assert_sdp_metadata(&initial_answer.2);
    assert_eq!(
        initial_answer.2.offer_version, initial_offer_sd.offer_version,
        "initial answer must echo offer_version"
    );

    let restart_offer = descriptions
        .iter()
        .find(|(_, _, sd)| sd.r#type() == SdpType::IceRestartOffer)
        .expect("ICE restart offer should be signaled");
    let restart_offer_sd = &restart_offer.2;
    assert_sdp_metadata(restart_offer_sd);

    let restart_answer = descriptions
        .iter()
        .find(|(_, _, sd)| {
            sd.r#type() == SdpType::Answer && sd.negotiation_id == restart_offer_sd.negotiation_id
        })
        .expect("ICE restart answer should echo restart offer metadata");
    assert_sdp_metadata(&restart_answer.2);
    assert_eq!(
        restart_answer.2.offer_version, restart_offer_sd.offer_version,
        "restart answer must echo offer_version"
    );
    assert_ne!(
        initial_offer_sd.negotiation_id, restart_offer_sd.negotiation_id,
        "ICE restart must use a fresh negotiation_id"
    );
    assert!(
        restart_offer_sd.offer_version.unwrap() > initial_offer_sd.offer_version.unwrap(),
        "ICE restart offer_version must increase"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ice_candidates_include_metadata_and_stale_candidates_are_filtered() {
    init_tracing();

    let mut harness = TestHarness::new().await;
    harness.add_peer(4100).await;
    harness.add_peer(4200).await;
    harness.connect(4100, 4200).await;

    let offerer_id = make_actor_id(4100);
    let answerer_id = make_actor_id(4200);
    let candidate = wait_for_metadata_candidate(&harness, &offerer_id, &answerer_id).await;

    assert!(
        candidate.sdp_mid.is_some(),
        "ICE candidate should preserve sdp_mid: {:?}",
        candidate
    );
    assert!(
        candidate.sdp_mline_index.is_some(),
        "ICE candidate should preserve sdp_mline_index: {:?}",
        candidate
    );

    let answerer = harness.peer(4200).coordinator.clone();
    assert!(
        answerer
            .candidate_matches_active_peer_for_test(&offerer_id, &candidate)
            .await,
        "fresh candidate from active transaction should pass the receive filter"
    );

    let mut stale_ufrag = candidate.clone();
    stale_ufrag.username_fragment = Some("stale-ufrag".to_string());
    assert!(
        !answerer
            .candidate_matches_active_peer_for_test(&offerer_id, &stale_ufrag)
            .await,
        "candidate with stale username_fragment should be filtered"
    );

    let mut stale_version = candidate.clone();
    stale_version.offer_version = Some(candidate.offer_version.unwrap() + 1);
    assert!(
        !answerer
            .candidate_matches_active_peer_for_test(&offerer_id, &stale_version)
            .await,
        "candidate with stale offer_version should be filtered"
    );

    let mut missing_version = candidate.clone();
    missing_version.offer_version = None;
    assert!(
        !answerer
            .candidate_matches_active_peer_for_test(&offerer_id, &missing_version)
            .await,
        "candidate without offer_version should be filtered"
    );

    let mut stale_negotiation = candidate;
    stale_negotiation.negotiation_id = Some("stale-negotiation".to_string());
    assert!(
        !answerer
            .candidate_matches_active_peer_for_test(&offerer_id, &stale_negotiation)
            .await,
        "candidate with stale negotiation_id should be filtered"
    );
}
