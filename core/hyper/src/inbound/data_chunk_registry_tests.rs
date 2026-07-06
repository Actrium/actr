use super::*;
use std::sync::Mutex;

fn chunk(stream_id: &str) -> DataChunk {
    DataChunk {
        stream_id: stream_id.to_string(),
        sequence: 1,
        payload: Default::default(),
        metadata: vec![],
        timestamp_ms: None,
    }
}

fn counting_callback() -> (DataChunkCallback, Arc<Mutex<u32>>) {
    let count = Arc::new(Mutex::new(0u32));
    let c = count.clone();
    let cb: DataChunkCallback = Arc::new(move |_chunk, _sender| {
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
    let reg = DataChunkRegistry::default();
    assert_eq!(reg.callbacks.len(), 0);
    let (cb, _) = counting_callback();
    reg.register("s1".into(), cb);
    assert_eq!(reg.callbacks.len(), 1);
}

#[test]
fn unregister_removes_stream() {
    let reg = DataChunkRegistry::new();
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
    let reg = DataChunkRegistry::new();
    let (cb, count) = counting_callback();
    reg.register("s1".into(), cb);

    reg.dispatch(chunk("s1"), ActrId::default()).await;
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
    let reg = DataChunkRegistry::new();
    reg.dispatch(chunk("missing"), ActrId::default()).await;
    assert_eq!(reg.callbacks.len(), 0);
}
