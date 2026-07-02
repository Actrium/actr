use super::*;

#[test]
fn rpc_signal_retry_policy() {
    let p = PayloadType::RpcSignal.retry_policy();
    assert_eq!(p.max_attempts, 2, "one retry only");
    assert_eq!(p.initial_delay, Duration::from_millis(500));
    assert_eq!(p.max_delay, Duration::from_millis(500));
}

#[test]
fn rpc_reliable_retry_policy() {
    let p = PayloadType::RpcReliable.retry_policy();
    assert_eq!(p.max_attempts, 5, "four retries");
    assert_eq!(p.initial_delay, Duration::from_secs(1));
    assert_eq!(p.max_delay, Duration::from_secs(5));
}

#[test]
fn stream_and_media_no_retry() {
    for pt in [
        PayloadType::StreamReliable,
        PayloadType::StreamLatencyFirst,
        PayloadType::MediaRtp,
    ] {
        let p = pt.retry_policy();
        assert_eq!(p.max_attempts, 1, "{pt:?} should have no retry");
    }
}

#[test]
fn rpc_reliable_lane_types() {
    let lanes = PayloadType::RpcReliable.data_lane_types();
    assert!(lanes.contains(&DataLaneType::WebRtcDataChannel(DataChannelQoS::Reliable)));
    assert!(lanes.contains(&DataLaneType::WebSocket));
}

#[test]
fn rpc_signal_lane_types() {
    let lanes = PayloadType::RpcSignal.data_lane_types();
    assert!(lanes.contains(&DataLaneType::WebRtcDataChannel(DataChannelQoS::Signal)));
}

#[test]
fn media_rtp_has_no_lane() {
    assert!(PayloadType::MediaRtp.data_lane_types().is_empty());
}
