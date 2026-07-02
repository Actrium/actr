use super::*;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use webrtc::api::APIBuilder;
use webrtc::peer_connection::configuration::RTCConfiguration;
/// Helper: create a WebRtcConnection for testing
async fn create_test_connection() -> WebRtcConnection {
    let api = APIBuilder::new().build();
    let peer_connection = api
        .new_peer_connection(RTCConfiguration::default())
        .await
        .expect("Failed to create RTCPeerConnection");
    let (event_tx, _) = broadcast::channel(16);
    let peer_id = ActrId {
        realm: actr_protocol::Realm { realm_id: 1 },
        serial_number: 42,
        r#type: actr_protocol::ActrType {
            manufacturer: "test".to_string(),
            name: "node".to_string(),
            version: "1.0.0".to_string(),
        },
    };
    WebRtcConnection::new(peer_id, Arc::new(peer_connection), event_tx)
}

/// Test: multiple tasks calling close() concurrently do not deadlock
///
/// close() acquires write locks on multiple RwLocks sequentially (connected, data_channels,
/// media_tracks, track_sequence_numbers, track_ssrcs, lane_cache).
/// If two close() calls acquire them in different order or wait while holding locks, deadlock occurs.
/// This test detects deadlock via timeout.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_concurrent_close_no_deadlock() {
    let conn = create_test_connection().await;
    let num_tasks = 10;
    let mut handles = Vec::with_capacity(num_tasks);

    for i in 0..num_tasks {
        let conn = conn.clone();
        handles.push(tokio::spawn(async move {
            let result = conn.close().await;
            tracing::info!("Task {} close result: {:?}", i, result.is_ok());
            result
        }));
    }

    // Detect deadlock via timeout: no deadlock if all tasks finish within 2 seconds
    let all_tasks = futures_util::future::join_all(handles);
    let result = tokio::time::timeout(Duration::from_secs(2), all_tasks).await;

    match result {
        Ok(results) => {
            // All tasks should succeed (first close actually closes, subsequent ones may encounter already-closed connection)
            let completed = results.iter().filter(|r| r.is_ok()).count();
            assert_eq!(
                completed, num_tasks,
                "all {} tasks should complete, actually completed {}",
                num_tasks, completed
            );
        }
        Err(_) => {
            panic!(
                "deadlock detected: {} concurrent close() calls did not finish within 2 seconds, possible deadlock!",
                num_tasks
            );
        }
    }
}

/// Test: close() with concurrent read operations does not deadlock
///
/// Scenario: some tasks continuously read is_connected() / has_open_data_channel(),
/// while others call close(). RwLock read-write contention should not cause deadlock.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_close_with_concurrent_reads_no_deadlock() {
    let conn: WebRtcConnection = create_test_connection().await;
    let mut handles = Vec::new();

    // Spawn 5 reader tasks that continuously read connection state
    for i in 0..5 {
        let conn = conn.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..20 {
                // Use async read instead of blocking_read (is_connected) to avoid async context issues
                let _ = *conn.connected.read().await;
                let _ = conn.has_open_data_channel().await;
                tokio::task::yield_now().await;
            }
            tracing::info!("Reader task {} done", i);
        }));
    }

    // Spawn 5 close tasks
    for i in 0..5 {
        let conn = conn.clone();
        handles.push(tokio::spawn(async move {
            let result = conn.close().await;
            tracing::info!("Close task {} result: {:?}", i, result.is_ok());
        }));
    }

    let all_tasks = futures_util::future::join_all(handles);
    let result = tokio::time::timeout(Duration::from_secs(2), all_tasks).await;

    match result {
        Ok(results) => {
            let completed = results.iter().filter(|r| r.is_ok()).count();
            assert_eq!(completed, 10, "all 10 tasks should complete");
        }
        Err(_) => {
            panic!(
                "deadlock detected: close() with concurrent reads did not finish within 2 seconds, possible deadlock!"
            );
        }
    }
}

/// Test: close() with concurrent handle_state_change() does not deadlock
///
/// Real-world reproduction: after ICE restart failure, cleanup_cancelled_connection calls
/// peer_connection.close(), which triggers a state_change callback invoking handle_state_change(Closed),
/// and handle_state_change(Closed) internally calls self.close() again.
/// This simulates the actual 3-way concurrent close race.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_close_with_handle_state_change_no_deadlock() {
    let conn = create_test_connection().await;
    let mut handles = Vec::new();

    // Simulate cleanup_cancelled_connection path: call close() directly
    {
        let conn = conn.clone();
        handles.push(tokio::spawn(async move {
            let _ = conn.close().await;
            tracing::info!("Direct close() done");
        }));
    }

    // Simulate state_change callback path: handle_state_change(Closed)
    // handle_state_change internally also calls close() when was_connected && Closed
    {
        let conn = conn.clone();
        handles.push(tokio::spawn(async move {
            conn.handle_state_change(
                webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState::Closed,
            )
            .await;
            tracing::info!("handle_state_change(Closed) done");
        }));
    }

    // Simulate event listener path: call close() after receiving StateChanged(Closed)
    {
        let conn = conn.clone();
        handles.push(tokio::spawn(async move {
            let _ = conn.close().await;
            tracing::info!("Event listener close() done");
        }));
    }

    let all_tasks = futures_util::future::join_all(handles);
    let result = tokio::time::timeout(Duration::from_secs(2), all_tasks).await;

    match result {
        Ok(results) => {
            let completed = results.iter().filter(|r| r.is_ok()).count();
            assert_eq!(completed, 3, "all 3 tasks should complete");
        }
        Err(_) => {
            panic!(
                "deadlock detected: close() with concurrent handle_state_change did not finish within 2 seconds, \
                     possible deadlock! This reproduces the 3-way close race after ICE restart failure."
            );
        }
    }
}

/// Test: stress test with many concurrent close() calls
///
/// Uses more concurrent tasks to increase lock contention probability, making potential deadlocks easier to expose.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_stress_concurrent_close() {
    let conn = create_test_connection().await;
    let num_tasks = 50;
    let mut handles = Vec::with_capacity(num_tasks);

    for i in 0..num_tasks {
        let conn = conn.clone();
        handles.push(tokio::spawn(async move {
            // Mix close and read operations to increase lock contention
            if i % 3 == 0 {
                let _ = *conn.connected.read().await;
            }
            if i % 5 == 0 {
                let _ = conn.has_open_data_channel().await;
            }
            let _ = conn.close().await;
        }));
    }

    let all_tasks = futures_util::future::join_all(handles);
    let result = tokio::time::timeout(Duration::from_secs(3), all_tasks).await;

    match result {
        Ok(results) => {
            let completed = results.iter().filter(|r| r.is_ok()).count();
            assert_eq!(
                completed, num_tasks,
                "all {} stress test tasks should complete",
                num_tasks
            );
            // Verify final state: connection should be closed
            assert!(
                !*conn.connected.read().await,
                "connected should be false after close()"
            );
        }
        Err(_) => {
            panic!(
                "stress test deadlock detected: {} concurrent close() calls did not finish within 3 seconds, possible deadlock!",
                num_tasks
            );
        }
    }
}

/// Regression test: close() with concurrent invalidate_lane() does not block due to lock order inversion
///
/// This test simulates a historically reproduced sequence:
/// - close() cleans up cache;
/// - invalidate_lane() fires concurrently (lane_cache -> data_channels).
/// After fix, both should complete within the timeout window without waiting on each other.
#[tokio::test]
async fn repro_close_blocked_by_lock_order_inversion() {
    use tokio::time::{Duration, sleep};

    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init();

    let conn = create_test_connection().await;
    let payload_type = PayloadType::RpcReliable;

    // First create a DataChannel lane to ensure related caches and callback paths are established.
    let _ = conn
        .get_lane(payload_type)
        .await
        .expect("failed to create lane for repro");

    // Artificially stall close(): hold media_tracks first to ensure a concurrency window
    // between close and invalidate_lane (historically this triggered lock order contention).
    let media_tracks_guard = conn.media_tracks.write().await;

    let conn_for_close = conn.clone();
    let mut close_task = tokio::spawn(async move { conn_for_close.close().await });

    // Give close a brief moment to enter the cleanup path.
    sleep(Duration::from_millis(50)).await;

    // Trigger invalidate_lane concurrently (historically this would contend with close on lock order).
    let conn_for_invalidate = conn.clone();
    let mut invalidate_task = tokio::spawn(async move {
        conn_for_invalidate.invalidate_lane(payload_type).await;
    });

    sleep(Duration::from_millis(50)).await;

    // Release media_tracks to let close finish remaining cleanup.
    drop(media_tracks_guard);

    let result = tokio::time::timeout(Duration::from_millis(3000), async {
        let close_res = (&mut close_task).await;
        let invalidate_res = (&mut invalidate_task).await;
        (close_res, invalidate_res)
    })
    .await;

    match result {
        Ok((close_res, invalidate_res)) => {
            assert!(close_res.is_ok(), "close task panicked unexpectedly");
            assert!(
                invalidate_res.is_ok(),
                "invalidate task panicked unexpectedly"
            );
        }
        Err(_) => {
            close_task.abort();
            invalidate_task.abort();
            let _ = close_task.await;
            let _ = invalidate_task.await;
            panic!("close()/invalidate_lane() should not block after lock-order fix");
        }
    }
}
