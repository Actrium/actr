use super::*;
use std::sync::Mutex;
use std::time::Duration;

fn chunk_with_sequence(stream_id: &str, sequence: u64) -> DataStream {
    DataStream {
        stream_id: stream_id.to_string(),
        sequence,
        payload: Default::default(),
        metadata: vec![],
        timestamp_ms: None,
    }
}

fn chunk(stream_id: &str) -> DataStream {
    chunk_with_sequence(stream_id, 1)
}

fn counting_callback() -> (DataStreamCallback, Arc<Mutex<u32>>) {
    let count = Arc::new(Mutex::new(0u32));
    let c = count.clone();
    let cb: DataStreamCallback = Arc::new(move |_chunk, _sender| {
        let c = c.clone();
        Box::pin(async move {
            *c.lock().unwrap() += 1;
            Ok(())
        })
    });
    (cb, count)
}

#[test]
fn register_and_default() {
    let reg = DataStreamRegistry::default();
    assert_eq!(reg.callbacks.len(), 0);
    let (cb, _) = counting_callback();
    reg.register("s1".into(), cb);
    assert_eq!(reg.callbacks.len(), 1);
}

#[test]
fn unregister_removes_stream() {
    let reg = DataStreamRegistry::new();
    let (cb, _) = counting_callback();
    reg.register("s1".into(), cb);
    reg.unregister("s1");
    assert_eq!(reg.callbacks.len(), 0);
    // Unknown id is a no-op.
    reg.unregister("never");
    assert_eq!(reg.callbacks.len(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dispatch_invokes_registered_callback() {
    let reg = DataStreamRegistry::new();
    let (cb, count) = counting_callback();
    reg.register("s1".into(), cb);

    reg.dispatch(chunk("s1"), ActrId::default(), PayloadType::StreamReliable)
        .await;
    for _ in 0..50 {
        if *count.lock().unwrap() == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(*count.lock().unwrap(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dispatch_unknown_stream_is_noop() {
    let reg = DataStreamRegistry::new();
    reg.dispatch(
        chunk("missing"),
        ActrId::default(),
        PayloadType::StreamReliable,
    )
    .await;
    assert_eq!(reg.callbacks.len(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dispatch_preserves_order_for_same_stream() {
    let reg = DataStreamRegistry::new();
    let completed = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let completed_for_callback = completed.clone();
    let cb: DataStreamCallback = Arc::new(move |chunk, _sender| {
        let completed = completed_for_callback.clone();
        Box::pin(async move {
            if chunk.sequence == 1 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            completed.lock().await.push(chunk.sequence);
            Ok(())
        })
    });
    reg.register("ordered".into(), cb);

    reg.dispatch(
        chunk_with_sequence("ordered", 1),
        ActrId::default(),
        PayloadType::StreamReliable,
    )
    .await;
    reg.dispatch(
        chunk_with_sequence("ordered", 2),
        ActrId::default(),
        PayloadType::StreamReliable,
    )
    .await;

    for _ in 0..50 {
        if completed.lock().await.len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    assert_eq!(*completed.lock().await, vec![1, 2]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dispatch_keeps_different_streams_concurrent() {
    let reg = DataStreamRegistry::new();
    let (slow_started_tx, slow_started_rx) = tokio::sync::oneshot::channel();
    let (release_slow_tx, release_slow_rx) = tokio::sync::oneshot::channel();
    let (slow_done_tx, mut slow_done_rx) = tokio::sync::mpsc::unbounded_channel();
    let (fast_done_tx, mut fast_done_rx) = tokio::sync::mpsc::unbounded_channel();

    let slow_started_tx = Arc::new(Mutex::new(Some(slow_started_tx)));
    let release_slow_rx = Arc::new(Mutex::new(Some(release_slow_rx)));
    let slow_cb: DataStreamCallback = Arc::new(move |_chunk, _sender| {
        let slow_started_tx = slow_started_tx.clone();
        let release_slow_rx = release_slow_rx.clone();
        let slow_done_tx = slow_done_tx.clone();
        Box::pin(async move {
            if let Some(tx) = slow_started_tx.lock().unwrap().take() {
                let _ = tx.send(());
            }
            let rx = release_slow_rx.lock().unwrap().take();
            if let Some(rx) = rx {
                let _ = rx.await;
            }
            slow_done_tx.send(()).unwrap();
            Ok(())
        })
    });
    let fast_cb: DataStreamCallback = Arc::new(move |_chunk, _sender| {
        let fast_done_tx = fast_done_tx.clone();
        Box::pin(async move {
            fast_done_tx.send(()).unwrap();
            Ok(())
        })
    });

    reg.register("slow".into(), slow_cb);
    reg.register("fast".into(), fast_cb);

    reg.dispatch(
        chunk("slow"),
        ActrId::default(),
        PayloadType::StreamReliable,
    )
    .await;
    tokio::time::timeout(Duration::from_secs(1), slow_started_rx)
        .await
        .expect("slow stream callback should start")
        .expect("slow callback start sender should not be dropped");

    reg.dispatch(
        chunk("fast"),
        ActrId::default(),
        PayloadType::StreamReliable,
    )
    .await;
    tokio::time::timeout(Duration::from_millis(100), fast_done_rx.recv())
        .await
        .expect("fast stream should complete while slow stream is blocked")
        .expect("fast stream completion should be sent");

    release_slow_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), slow_done_rx.recv())
        .await
        .expect("slow stream should complete after release")
        .expect("slow stream completion should be sent");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dispatch_latency_first_does_not_serialize_same_stream() {
    let reg = DataStreamRegistry::new();
    let (slow_started_tx, slow_started_rx) = tokio::sync::oneshot::channel();
    let (release_slow_tx, release_slow_rx) = tokio::sync::oneshot::channel();
    let (slow_done_tx, mut slow_done_rx) = tokio::sync::mpsc::unbounded_channel();
    let (fast_done_tx, mut fast_done_rx) = tokio::sync::mpsc::unbounded_channel();

    let slow_started_tx = Arc::new(Mutex::new(Some(slow_started_tx)));
    let release_slow_rx = Arc::new(Mutex::new(Some(release_slow_rx)));
    let cb: DataStreamCallback = Arc::new(move |chunk, _sender| {
        let slow_started_tx = slow_started_tx.clone();
        let release_slow_rx = release_slow_rx.clone();
        let slow_done_tx = slow_done_tx.clone();
        let fast_done_tx = fast_done_tx.clone();
        Box::pin(async move {
            match chunk.sequence {
                1 => {
                    if let Some(tx) = slow_started_tx.lock().unwrap().take() {
                        let _ = tx.send(());
                    }
                    let rx = release_slow_rx.lock().unwrap().take();
                    if let Some(rx) = rx {
                        let _ = rx.await;
                    }
                    slow_done_tx.send(()).unwrap();
                }
                2 => {
                    fast_done_tx.send(()).unwrap();
                }
                _ => {}
            }
            Ok(())
        })
    });

    reg.register("latency-first".into(), cb);

    reg.dispatch(
        chunk_with_sequence("latency-first", 1),
        ActrId::default(),
        PayloadType::StreamLatencyFirst,
    )
    .await;
    tokio::time::timeout(Duration::from_secs(1), slow_started_rx)
        .await
        .expect("slow latency-first callback should start")
        .expect("slow callback start sender should not be dropped");

    reg.dispatch(
        chunk_with_sequence("latency-first", 2),
        ActrId::default(),
        PayloadType::StreamLatencyFirst,
    )
    .await;
    tokio::time::timeout(Duration::from_millis(100), fast_done_rx.recv())
        .await
        .expect("second latency-first chunk should not wait for first callback")
        .expect("fast latency-first completion should be sent");

    release_slow_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), slow_done_rx.recv())
        .await
        .expect("slow latency-first chunk should complete after release")
        .expect("slow latency-first completion should be sent");
}
