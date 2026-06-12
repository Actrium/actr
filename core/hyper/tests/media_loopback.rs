//! Integration test: MediaFrameRegistry RTP loopback
//!
//! Tests three levels of the media pipeline:
//!
//! 1. **Registry unit** — `register` / `dispatch` / `unregister` without any
//!    network, confirming the fast-path callback routing logic.
//! 2. **Concurrent dispatch** — multiple callbacks registered at the same time,
//!    all fired from a single `dispatch()` call; validates the DashMap
//!    concurrent-read path.
//! 3. **WebRTC media track e2e** (audio loopback) — two in-process peers share
//!    a `MediaFrameRegistry`, peer A adds a track and streams synthetic Opus
//!    frames through the real WebRTC `RTCTrackRemote` path; peer B receives
//!    them via the registry callback.
//!
//! Tests 1 and 2 are pure in-process and run in milliseconds.
//! Test 3 requires real ICE/DTLS negotiation (uses the VNet virtual network)
//! and is marked `#[ignore]` by default — run with `--include-ignored`.

use actr_framework::{MediaSample, MediaType};
use actr_hyper::inbound::{MediaFrameRegistry, MediaTrackCallback};
use actr_hyper::test_support::{
    TestSignalingServer,
    utils::{create_credential_state_for_test, dummy_credential, make_actor_id},
};
use actr_hyper::wire::webrtc::{WebRtcConfig, WebRtcCoordinator, WebSocketSignalingClient};
use actr_protocol::ActrId;
use futures_util::future::BoxFuture;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::{Notify, oneshot};

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_file(true)
        .with_line_number(true)
        .with_test_writer()
        .try_init()
        .ok();
}

/// Build a minimal audio MediaSample (mimics an Opus-encoded packet).
///
/// The payload starts with the Opus TOC byte (0xFC = stereo 20 ms CELT) followed
/// by `size - 1` bytes of 0xFF so the receiver can verify byte integrity.
fn make_audio_sample(timestamp: u32, size: usize) -> MediaSample {
    let mut data = vec![0xFC_u8]; // Opus TOC byte
    data.resize(size, 0xFF);
    MediaSample {
        data: bytes::Bytes::from(data),
        timestamp,
        codec: "OPUS".to_string(),
        media_type: MediaType::Audio,
    }
}

/// Build a minimal video MediaSample (H264 NALU IDR slice header).
fn make_video_sample(timestamp: u32, size: usize) -> MediaSample {
    let mut data = vec![0x00, 0x00, 0x00, 0x01, 0x65]; // H264 Annex-B IDR header
    data.resize(size, 0xAB);
    MediaSample {
        data: bytes::Bytes::from(data),
        timestamp,
        codec: "H264".to_string(),
        media_type: MediaType::Video,
    }
}

/// Create a coordinator with a caller-owned `Arc<MediaFrameRegistry>` so the
/// test can register callbacks independently of the peer setup helpers.
async fn create_coordinator_with_registry(
    id: ActrId,
    server_url: &str,
    registry: Arc<MediaFrameRegistry>,
) -> Arc<WebRtcCoordinator> {
    let credential_state = create_credential_state_for_test(dummy_credential());

    let signaling_client = WebSocketSignalingClient::connect_to_with_identity(
        server_url,
        id.clone(),
        credential_state.clone(),
    )
    .await
    .expect("failed to connect to test signaling server");

    let config = WebRtcConfig::default();

    let coordinator = Arc::new(WebRtcCoordinator::new(
        id,
        credential_state,
        signaling_client as Arc<dyn actr_hyper::wire::webrtc::SignalingClient>,
        config,
        registry,
    ));

    let c = coordinator.clone();
    tokio::spawn(async move {
        let _ = c.start().await;
    });

    coordinator
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Test 1: registry unit — single callback, audio frame
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Verify that a registered audio callback is invoked exactly once per
/// `dispatch()` call and receives the correct payload bytes.
#[tokio::test]
async fn test_registry_audio_callback_invoked() {
    init_tracing();
    tracing::info!("═══ test_registry_audio_callback_invoked ═══");

    let registry = Arc::new(MediaFrameRegistry::new());
    let sender_id = make_actor_id(1);
    let sample = make_audio_sample(1000, 160);

    // Capture: count + first received payload
    let call_count = Arc::new(AtomicUsize::new(0));
    let (result_tx, result_rx) = oneshot::channel::<(bytes::Bytes, u32, MediaType)>();
    let result_tx = Arc::new(tokio::sync::Mutex::new(Some(result_tx)));

    let count = call_count.clone();
    let cb: MediaTrackCallback = Arc::new(
        move |s: MediaSample,
              _sender: ActrId|
              -> BoxFuture<'static, actr_protocol::ActorResult<()>> {
            let count = count.clone();
            let tx = result_tx.clone();
            Box::pin(async move {
                count.fetch_add(1, Ordering::SeqCst);
                if let Some(tx) = tx.lock().await.take() {
                    let _ = tx.send((s.data, s.timestamp, s.media_type));
                }
                Ok(())
            })
        },
    );

    registry.register("audio-track-1".to_string(), cb);
    assert_eq!(
        registry.active_tracks(),
        1,
        "one track should be registered"
    );

    registry
        .dispatch("audio-track-1", sample.clone(), sender_id)
        .await;

    // Wait for the spawned callback task to complete
    let (received_data, received_ts, received_type) =
        tokio::time::timeout(Duration::from_secs(2), result_rx)
            .await
            .expect("callback was not called within 2 s")
            .expect("oneshot sender dropped");

    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "callback must be called exactly once"
    );
    assert_eq!(received_ts, 1000, "timestamp preserved");
    assert_eq!(received_type, MediaType::Audio, "media type preserved");
    assert_eq!(
        received_data, sample.data,
        "payload bytes preserved end-to-end"
    );

    tracing::info!("✅ test_registry_audio_callback_invoked passed");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Test 2: registry unit — video frame callback
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Same as Test 1 but with a video (H264) frame to confirm MediaType routing.
#[tokio::test]
async fn test_registry_video_callback_invoked() {
    init_tracing();
    tracing::info!("═══ test_registry_video_callback_invoked ═══");

    let registry = Arc::new(MediaFrameRegistry::new());
    let sender_id = make_actor_id(2);
    let sample = make_video_sample(9000, 512);

    let (tx, rx) = oneshot::channel::<(bytes::Bytes, MediaType, String)>();
    let tx = Arc::new(tokio::sync::Mutex::new(Some(tx)));

    let cb: MediaTrackCallback = Arc::new(
        move |s: MediaSample, _: ActrId| -> BoxFuture<'static, actr_protocol::ActorResult<()>> {
            let tx = tx.clone();
            Box::pin(async move {
                if let Some(tx) = tx.lock().await.take() {
                    let _ = tx.send((s.data, s.media_type, s.codec));
                }
                Ok(())
            })
        },
    );

    registry.register("video-track-1".to_string(), cb);
    registry
        .dispatch("video-track-1", sample.clone(), sender_id)
        .await;

    let (received_data, received_type, received_codec) =
        tokio::time::timeout(Duration::from_secs(2), rx)
            .await
            .expect("callback not called within 2 s")
            .expect("oneshot dropped");

    assert_eq!(received_type, MediaType::Video);
    assert_eq!(received_codec, "H264");
    assert_eq!(received_data, sample.data);

    tracing::info!("✅ test_registry_video_callback_invoked passed");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Test 3: unregister — dispatch after removal must be silent
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Verify that `unregister` removes the callback and subsequent dispatches to
/// the same track_id are silently dropped (no panic, no stale call).
#[tokio::test]
async fn test_registry_unregister_silences_dispatch() {
    init_tracing();
    tracing::info!("═══ test_registry_unregister_silences_dispatch ═══");

    let registry = Arc::new(MediaFrameRegistry::new());
    let sender_id = make_actor_id(3);
    let sample = make_audio_sample(2000, 80);

    let call_count = Arc::new(AtomicUsize::new(0));
    let count = call_count.clone();

    let cb: MediaTrackCallback = Arc::new(
        move |_: MediaSample, _: ActrId| -> BoxFuture<'static, actr_protocol::ActorResult<()>> {
            let count = count.clone();
            Box::pin(async move {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        },
    );

    // Register → dispatch (should fire)
    registry.register("ephemeral-track".to_string(), cb);
    registry
        .dispatch("ephemeral-track", sample.clone(), sender_id.clone())
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await; // let spawned task run
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "first dispatch must fire"
    );

    // Unregister → dispatch (must NOT fire)
    registry.unregister("ephemeral-track");
    assert_eq!(registry.active_tracks(), 0, "track should be gone");
    registry
        .dispatch("ephemeral-track", sample, sender_id)
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "no additional calls after unregister"
    );

    tracing::info!("✅ test_registry_unregister_silences_dispatch passed");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Test 4: concurrent multi-track dispatch
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Register N tracks concurrently and confirm each callback is invoked exactly
/// once when dispatched, exercising the DashMap concurrent-read path.
#[tokio::test(flavor = "multi_thread")]
async fn test_registry_concurrent_multi_track_dispatch() {
    init_tracing();
    tracing::info!("═══ test_registry_concurrent_multi_track_dispatch ═══");

    const N: usize = 8;

    let registry = Arc::new(MediaFrameRegistry::new());
    let sender_id = make_actor_id(10);
    let notify = Arc::new(Notify::new());
    let call_count = Arc::new(AtomicUsize::new(0));

    // Register N tracks
    for i in 0..N {
        let notify = notify.clone();
        let count = call_count.clone();
        let cb: MediaTrackCallback = Arc::new(
            move |_: MediaSample,
                  _: ActrId|
                  -> BoxFuture<'static, actr_protocol::ActorResult<()>> {
                let notify = notify.clone();
                let count = count.clone();
                Box::pin(async move {
                    let prev = count.fetch_add(1, Ordering::SeqCst);
                    if prev + 1 == N {
                        notify.notify_one();
                    }
                    Ok(())
                })
            },
        );
        registry.register(format!("track-{}", i), cb);
    }

    assert_eq!(registry.active_tracks(), N);

    // Dispatch to all tracks concurrently
    let mut handles = Vec::with_capacity(N);
    for i in 0..N {
        let reg = registry.clone();
        let sid = sender_id.clone();
        let sample = make_audio_sample(i as u32 * 100, 80);
        handles.push(tokio::spawn(async move {
            reg.dispatch(&format!("track-{}", i), sample, sid).await;
        }));
    }
    for h in handles {
        h.await.expect("dispatch task panicked");
    }

    // Wait until all N callbacks have been called
    tokio::time::timeout(Duration::from_secs(5), notify.notified())
        .await
        .expect("not all callbacks fired within 5 s");

    assert_eq!(
        call_count.load(Ordering::SeqCst),
        N,
        "each track callback must fire exactly once"
    );

    tracing::info!(
        "✅ test_registry_concurrent_multi_track_dispatch passed (N={})",
        N
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Test 5: sequential multi-frame dispatch on a single track
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Send M audio frames to the same track and confirm all M callbacks are fired.
///
/// This simulates a real streaming session where the RTP reader loop calls
/// `dispatch()` repeatedly for the same track_id.
#[tokio::test]
async fn test_registry_sequential_frames_same_track() {
    init_tracing();
    tracing::info!("═══ test_registry_sequential_frames_same_track ═══");

    const M: usize = 20;

    let registry = Arc::new(MediaFrameRegistry::new());
    let sender_id = make_actor_id(5);

    let notify = Arc::new(Notify::new());
    let call_count = Arc::new(AtomicUsize::new(0));

    {
        let notify = notify.clone();
        let count = call_count.clone();
        let cb: MediaTrackCallback = Arc::new(
            move |_: MediaSample,
                  _: ActrId|
                  -> BoxFuture<'static, actr_protocol::ActorResult<()>> {
                let notify = notify.clone();
                let count = count.clone();
                Box::pin(async move {
                    let prev = count.fetch_add(1, Ordering::SeqCst);
                    if prev + 1 == M {
                        notify.notify_one();
                    }
                    Ok(())
                })
            },
        );
        registry.register("stream-track".to_string(), cb);
    }

    for i in 0..M {
        let sample = make_audio_sample(i as u32 * 960, 160); // 20 ms at 48 kHz
        registry
            .dispatch("stream-track", sample, sender_id.clone())
            .await;
    }

    tokio::time::timeout(Duration::from_secs(5), notify.notified())
        .await
        .expect("not all frame callbacks fired within 5 s");

    assert_eq!(
        call_count.load(Ordering::SeqCst),
        M,
        "all {} frames must trigger callback",
        M
    );

    tracing::info!(
        "✅ test_registry_sequential_frames_same_track passed (M={})",
        M
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Test 6: sender_id forwarding
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Confirm that the `sender_id` passed to `dispatch()` arrives unchanged at the
/// callback — important for access control and source attribution in real apps.
#[tokio::test]
async fn test_registry_sender_id_forwarded() {
    init_tracing();
    tracing::info!("═══ test_registry_sender_id_forwarded ═══");

    let registry = Arc::new(MediaFrameRegistry::new());
    let expected_id = make_actor_id(42);
    let (tx, rx) = oneshot::channel::<ActrId>();
    let tx = Arc::new(tokio::sync::Mutex::new(Some(tx)));

    let cb: MediaTrackCallback = Arc::new(
        move |_: MediaSample,
              sender: ActrId|
              -> BoxFuture<'static, actr_protocol::ActorResult<()>> {
            let tx = tx.clone();
            Box::pin(async move {
                if let Some(tx) = tx.lock().await.take() {
                    let _ = tx.send(sender);
                }
                Ok(())
            })
        },
    );

    registry.register("id-check-track".to_string(), cb);
    registry
        .dispatch(
            "id-check-track",
            make_audio_sample(0, 40),
            expected_id.clone(),
        )
        .await;

    let received_id = tokio::time::timeout(Duration::from_secs(2), rx)
        .await
        .expect("callback not called within 2 s")
        .expect("sender dropped");

    assert_eq!(
        received_id.serial_number, expected_id.serial_number,
        "serial_number must match"
    );
    assert_eq!(received_id.realm, expected_id.realm, "realm must match");

    tracing::info!("✅ test_registry_sender_id_forwarded passed");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Test 7: WebRTC media track e2e — audio RTP loopback (ignored by default)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Full path: coordinator A adds OPUS track → negotiates SDP with coordinator B
// over in-process WebSocket signaling → A calls send_media_sample() which
// writes RTP via RTCTrackRemote → B's on_track callback fires → B's
// MediaFrameRegistry::dispatch() is called → user callback receives frame.
//
// This test is marked #[ignore] because it requires real ICE/DTLS negotiation
// (~3–10 s) and an in-process mock-actrix signaling server. Run it with:
//
//   cargo test -p actr-hyper --features test-utils -- --include-ignored \
//       test_media_rtp_audio_loopback
//
// Note: `add_dynamic_track` / `send_media_sample` are the production APIs that
// the linked workload binary (not DynclibContext) calls. This test verifies the
// complete on-track → dispatch path without a linked binary.

/// WebRTC audio RTP loopback: peer A streams synthetic Opus frames, peer B
/// receives them via `MediaFrameRegistry`.
///
/// Assertion: at least 3 frames arrive at the registry callback within 15 s.
#[tokio::test]
#[ignore = "requires real WebRTC ICE/DTLS (~5–10 s) — run with --include-ignored"]
async fn test_media_rtp_audio_loopback() {
    init_tracing();
    tracing::info!("═══ test_media_rtp_audio_loopback ═══");

    // ── Start in-process signaling server ───────────────────────────────────
    let server = TestSignalingServer::start()
        .await
        .expect("failed to start signaling server");
    let server_url = server.url();

    // ── Build both coordinators, B owns a testable registry ─────────────────
    let id_a = make_actor_id(701);
    let id_b = make_actor_id(702);

    let registry_b = Arc::new(MediaFrameRegistry::new());

    let coord_a = create_coordinator_with_registry(
        id_a.clone(),
        &server_url,
        Arc::new(MediaFrameRegistry::new()), // A sends, doesn't receive
    )
    .await;

    let coord_b =
        create_coordinator_with_registry(id_b.clone(), &server_url, registry_b.clone()).await;

    // ── Register callback on B's registry ────────────────────────────────────
    const FRAMES_EXPECTED: usize = 3;
    let notify = Arc::new(Notify::new());
    let frame_count = Arc::new(AtomicUsize::new(0));

    {
        let notify = notify.clone();
        let count = frame_count.clone();
        let cb: MediaTrackCallback = Arc::new(
            move |s: MediaSample,
                  _: ActrId|
                  -> BoxFuture<'static, actr_protocol::ActorResult<()>> {
                let notify = notify.clone();
                let count = count.clone();
                Box::pin(async move {
                    let n = count.fetch_add(1, Ordering::SeqCst) + 1;
                    tracing::info!(
                        "📞 B received frame #{}: codec={}, bytes={}",
                        n,
                        s.codec,
                        s.data.len()
                    );
                    if n >= FRAMES_EXPECTED {
                        notify.notify_one();
                    }
                    Ok(())
                })
            },
        );

        // Register with a well-known track_id that A will use
        registry_b.register("opus-audio-loopback".to_string(), cb);
    }

    // ── A adds OPUS track toward B and triggers SDP renegotiation ────────────
    tracing::info!("A: adding dynamic OPUS track toward B ...");
    tokio::time::timeout(
        Duration::from_secs(15),
        coord_a.add_dynamic_track(&id_b, "opus-audio-loopback".to_string(), "OPUS", "audio"),
    )
    .await
    .expect("add_dynamic_track timed out")
    .expect("add_dynamic_track failed");

    tracing::info!("A: track added, starting to stream frames ...");

    // ── A sends synthetic Opus frames to B ───────────────────────────────────
    for i in 0..10u32 {
        let sample = make_audio_sample(i * 960, 160); // 20 ms Opus @ 48 kHz
        if let Err(e) = coord_a
            .send_media_sample(&id_b, "opus-audio-loopback", sample)
            .await
        {
            tracing::warn!("send_media_sample failed (frame {}): {}", i, e);
        }
        // Small gap between frames so the receiver loop can process them
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    // ── Wait for B's registry to receive at least FRAMES_EXPECTED frames ─────
    tokio::time::timeout(Duration::from_secs(15), notify.notified())
        .await
        .expect("B did not receive expected frames within 15 s");

    let total = frame_count.load(Ordering::SeqCst);
    assert!(
        total >= FRAMES_EXPECTED,
        "expected at least {} frames at B's registry, got {}",
        FRAMES_EXPECTED,
        total,
    );

    tracing::info!(
        "✅ test_media_rtp_audio_loopback passed: B received {} frames",
        total
    );

    // Cleanup
    let _ = coord_a
        .remove_dynamic_track(&id_b, "opus-audio-loopback")
        .await;
    let _ = coord_b; // keep alive until here
}
