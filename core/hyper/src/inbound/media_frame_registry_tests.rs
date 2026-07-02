use super::*;
use actr_framework::Bytes;
use std::sync::Mutex;

fn sample() -> MediaSample {
    MediaSample {
        data: Bytes::from_static(b"frame"),
        timestamp: 0,
        codec: "opus".into(),
        media_type: actr_framework::MediaType::Audio,
    }
}

fn counting_callback() -> (MediaTrackCallback, Arc<Mutex<u32>>) {
    let count = Arc::new(Mutex::new(0u32));
    let c = count.clone();
    let cb: MediaTrackCallback = Arc::new(move |_s, _id| {
        let c = c.clone();
        Box::pin(async move {
            *c.lock().unwrap() += 1;
            Ok(())
        })
    });
    (cb, count)
}

#[test]
fn register_and_active_tracks() {
    let reg = MediaFrameRegistry::new();
    assert_eq!(reg.active_tracks(), 0);
    let (cb, _) = counting_callback();
    reg.register("t1".into(), cb);
    assert_eq!(reg.active_tracks(), 1);
}

#[test]
fn unregister_removes_track() {
    let reg = MediaFrameRegistry::new();
    let (cb, _) = counting_callback();
    reg.register("t1".into(), cb);
    assert_eq!(reg.active_tracks(), 1);
    reg.unregister("t1");
    assert_eq!(reg.active_tracks(), 0);
    // Unregistering an unknown track is a no-op.
    reg.unregister("never");
    assert_eq!(reg.active_tracks(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dispatch_invokes_registered_callback() {
    let reg = MediaFrameRegistry::new();
    let (cb, count) = counting_callback();
    reg.register("t1".into(), cb);

    reg.dispatch("t1", sample(), ActrId::default()).await;
    // The callback runs in a spawned task; wait for it.
    for _ in 0..50 {
        if *count.lock().unwrap() == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(*count.lock().unwrap(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dispatch_unknown_track_is_noop() {
    let reg = MediaFrameRegistry::new();
    // No callback registered → dispatch must not panic and does nothing.
    reg.dispatch("missing", sample(), ActrId::default()).await;
    assert_eq!(reg.active_tracks(), 0);
}

#[test]
fn default_impl_matches_new() {
    let reg = MediaFrameRegistry::default();
    assert_eq!(reg.active_tracks(), 0);
}
