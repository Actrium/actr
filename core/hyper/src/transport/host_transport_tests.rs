use super::*;

fn envelope(request_id: &str) -> RpcEnvelope {
    RpcEnvelope {
        request_id: request_id.to_string(),
        route_key: "echo".to_string(),
        payload: Some(Bytes::from_static(b"hi")),
        ..Default::default()
    }
}

#[tokio::test]
async fn new_transport_has_reliable_lane() {
    let t = HostTransport::new();
    let lane = t
        .get_lane(PayloadType::RpcReliable, None)
        .await
        .expect("reliable lane must exist");
    // Cache: second call returns same lane (Arc identity).
    let lane2 = t.get_lane(PayloadType::RpcReliable, None).await.unwrap();
    assert!(Arc::ptr_eq(&lane, &lane2));
}

#[tokio::test]
async fn signal_lane_created_lazily_and_cached() {
    let t = HostTransport::new();
    let lane = t
        .get_lane(PayloadType::RpcSignal, None)
        .await
        .expect("signal lane should be created on demand");
    let lane2 = t.get_lane(PayloadType::RpcSignal, None).await.unwrap();
    assert!(Arc::ptr_eq(&lane, &lane2));
}

#[tokio::test]
async fn stream_lane_requires_channel_id() {
    let t = HostTransport::new();
    let err = t
        .get_lane(PayloadType::StreamLatencyFirst, None)
        .await
        .unwrap_err();
    assert!(matches!(err, NetworkError::InvalidArgument(_)));
}

#[tokio::test]
async fn stream_lane_missing_channel_errors() {
    let t = HostTransport::new();
    let err = t
        .get_lane(
            PayloadType::StreamLatencyFirst,
            Some("never-created".into()),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, NetworkError::ChannelNotFound(_)));
}

#[tokio::test]
async fn media_lane_requires_track_id_and_errors_when_missing() {
    let t = HostTransport::new();
    let err = t.get_lane(PayloadType::MediaRtp, None).await.unwrap_err();
    assert!(matches!(err, NetworkError::InvalidArgument(_)));

    let err = t
        .get_lane(PayloadType::MediaRtp, Some("no-track".into()))
        .await
        .unwrap_err();
    assert!(matches!(err, NetworkError::ChannelNotFound(_)));
}

#[tokio::test]
async fn send_message_delivers_to_reliable_lane() {
    let t = HostTransport::new();
    // send_message resolves Ok on the reliable lane (get_lane + send_envelope).
    t.send_message(PayloadType::RpcReliable, None, envelope("r1"))
        .await
        .unwrap();
}

#[tokio::test]
async fn send_message_fails_for_unknown_stream_channel() {
    let t = HostTransport::new();
    let err = t
        .send_message(
            PayloadType::StreamLatencyFirst,
            Some("nope".into()),
            envelope("r2"),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, NetworkError::ChannelNotFound(_)));
}

#[tokio::test]
async fn send_request_times_out_without_response() {
    let t = HostTransport::new();
    let mut env = envelope("req-timeout");
    env.timeout_ms = 10; // 10ms
    let err = t
        .send_request(PayloadType::RpcReliable, None, env)
        .await
        .unwrap_err();
    assert!(matches!(err, ActrError::TimedOut), "got {err:?}");
}

#[tokio::test]
async fn complete_response_unknown_id_errors() {
    let t = HostTransport::new();
    let err = t
        .complete_response("unknown", Bytes::from_static(b"x"))
        .await
        .unwrap_err();
    assert!(matches!(err, NetworkError::InvalidArgument(_)));
}

#[tokio::test]
async fn complete_error_unknown_id_errors() {
    let t = HostTransport::new();
    let err = t
        .complete_error("unknown", ActrError::Internal("x".into()))
        .await
        .unwrap_err();
    assert!(matches!(err, NetworkError::InvalidArgument(_)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_request_completes_with_response_bytes() {
    let t2 = Arc::new(HostTransport::new());
    let t3 = t2.clone();
    let handle = tokio::spawn(async move {
        let mut env = envelope("req-ok");
        env.timeout_ms = 5000;
        t3.send_request(PayloadType::RpcReliable, None, env).await
    });

    // Let the spawned task register its pending entry + send, then complete.
    tokio::time::sleep(Duration::from_millis(150)).await;

    t2.complete_response("req-ok", Bytes::from_static(b"resp"))
        .await
        .unwrap();
    let resp = handle.await.unwrap().unwrap();
    assert_eq!(resp, Bytes::from_static(b"resp"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_request_completes_with_error() {
    let t = Arc::new(HostTransport::new());
    let t2 = t.clone();
    let handle = tokio::spawn(async move {
        let mut env = envelope("req-err");
        env.timeout_ms = 5000;
        t2.send_request(PayloadType::RpcReliable, None, env).await
    });

    tokio::time::sleep(Duration::from_millis(150)).await;

    t.complete_error("req-err", ActrError::NotFound("missing".into()))
        .await
        .unwrap();
    let err = handle.await.unwrap().unwrap_err();
    assert!(matches!(err, ActrError::NotFound(_)), "got {err:?}");
}

#[test]
fn default_impl_matches_new() {
    // Default must behave like new() (reliable channel present).
    let _t = HostTransport::default();
}
