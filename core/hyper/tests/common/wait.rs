//! Shared wait/expect helpers for integration tests.
//!
//! Extracted from `outproc_disconnect_reconnect.rs` so they can be reused
//! across multiple test files (retry behavior, stale state, etc.).

use crate::transport::{ConnectionEvent, ConnectionState};
use actr_framework::Bytes;
use actr_protocol::{ActorResult, ActrId, PayloadType};
use std::time::Duration;

/// Wait until a `DataChannelOpened` event is observed for the given peer and
/// `PayloadType`.
///
/// Returns the `session_id` of the opened channel.
pub async fn wait_for_data_channel_opened(
    event_rx: &mut tokio::sync::broadcast::Receiver<ConnectionEvent>,
    peer_id: &ActrId,
    payload_type: PayloadType,
    timeout: Duration,
) -> u64 {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(
            !remaining.is_zero(),
            "timed out waiting for {:?} DataChannelOpened for peer {:?}",
            payload_type,
            peer_id
        );

        match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Ok(ConnectionEvent::DataChannelOpened {
                peer_id: event_peer,
                session_id,
                payload_type: event_payload_type,
            })) if &event_peer == peer_id && event_payload_type == payload_type => {
                return session_id;
            }
            Ok(Ok(_)) => {}
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                tracing::warn!("Connection event receiver lagged by {} events", n);
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                panic!("connection event channel closed while waiting for DataChannelOpened");
            }
            Err(_) => {
                panic!(
                    "timed out waiting for {:?} DataChannelOpened for peer {:?}",
                    payload_type, peer_id
                );
            }
        }
    }
}

/// Wait until the full close chain (DataChannelClosed → PeerConnection Closed
/// → ConnectionClosed) is observed for the given peer and session.
///
/// Returns the `PayloadType` of the first `DataChannelClosed` event observed.
pub async fn wait_for_data_channel_close_chain(
    event_rx: &mut tokio::sync::broadcast::Receiver<ConnectionEvent>,
    peer_id: &ActrId,
    session_id: u64,
    timeout: Duration,
) -> PayloadType {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut closed_payload_type = None;
    let mut saw_peer_connection_closed = false;
    let mut saw_connection_closed = false;

    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let event = match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Ok(event)) => event,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                tracing::warn!("Connection event receiver lagged by {} events", n);
                continue;
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                panic!("connection event channel closed while waiting for close chain");
            }
            Err(_) => break,
        };

        match event {
            ConnectionEvent::DataChannelClosed {
                peer_id: event_peer,
                session_id: event_session_id,
                payload_type,
            } if &event_peer == peer_id && event_session_id == session_id => {
                tracing::info!(
                    "Observed DataChannelClosed for peer {:?}, session_id={}, payload_type={:?}",
                    peer_id,
                    session_id,
                    payload_type
                );
                closed_payload_type.get_or_insert(payload_type);
            }
            ConnectionEvent::ConnectionClosed {
                peer_id: event_peer,
                session_id: event_session_id,
            } if &event_peer == peer_id && event_session_id == session_id => {
                tracing::info!(
                    "Observed ConnectionClosed for peer {:?}, session_id={}",
                    peer_id,
                    session_id
                );
                saw_connection_closed = true;
            }
            ConnectionEvent::StateChanged {
                peer_id: event_peer,
                session_id: event_session_id,
                state: ConnectionState::Closed,
            } if &event_peer == peer_id && event_session_id == session_id => {
                tracing::info!(
                    "Observed PeerConnection Closed for peer {:?}, session_id={}",
                    peer_id,
                    session_id
                );
                saw_peer_connection_closed = true;
            }
            _ => {}
        }

        if let Some(payload_type) = closed_payload_type
            && saw_peer_connection_closed
            && saw_connection_closed
        {
            return payload_type;
        }
    }

    panic!(
        "timed out waiting for DataChannelClosed -> PeerConnection Closed -> ConnectionClosed chain for peer {:?}, session_id={}, saw_data_channel_closed={}, saw_peer_connection_closed={}, saw_connection_closed={}",
        peer_id,
        session_id,
        closed_payload_type.is_some(),
        saw_peer_connection_closed,
        saw_connection_closed
    );
}

/// Wait until a peer enters one of the given `ConnectionState`s.
///
/// Returns the `(session_id, state)` of the matching event.
pub async fn wait_for_peer_state(
    event_rx: &mut tokio::sync::broadcast::Receiver<ConnectionEvent>,
    peer_id: &ActrId,
    wanted_states: &[ConnectionState],
    timeout: Duration,
) -> (u64, ConnectionState) {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(
            !remaining.is_zero(),
            "timed out waiting for peer {} to enter one of {:?}",
            peer_id,
            wanted_states
        );

        match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Ok(ConnectionEvent::StateChanged {
                peer_id: event_peer,
                session_id,
                state,
            })) if &event_peer == peer_id && wanted_states.contains(&state) => {
                return (session_id, state);
            }
            Ok(Ok(_)) => {}
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                tracing::warn!("Connection event receiver lagged by {} events", n);
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                panic!("connection event channel closed while waiting for peer state");
            }
            Err(_) => {
                panic!(
                    "timed out waiting for peer {} to enter one of {:?}",
                    peer_id, wanted_states
                );
            }
        }
    }
}

/// Assert that a spawned request handle fails with "connection not ready".
pub async fn expect_connection_recovering(
    handle: tokio::task::JoinHandle<ActorResult<Bytes>>,
    label: &str,
) {
    match tokio::time::timeout(Duration::from_secs(3), handle).await {
        Ok(Ok(Err(err))) => {
            let msg = err.to_string();
            assert!(
                msg.contains("connection not ready"),
                "{label} failed, but not with connection not ready: {msg}"
            );
        }
        Ok(Ok(Ok(response))) => {
            panic!(
                "{label} unexpectedly succeeded with {} response bytes",
                response.len()
            );
        }
        Ok(Err(err)) => panic!("{label} task panicked: {err}"),
        Err(_) => panic!("{label} did not finish within the outer timeout"),
    }
}

/// Poll a request until it succeeds or the total timeout expires.
///
/// This helper re-sends on transient errors (connection not ready, timeout,
/// Connection*) and returns the response bytes on success.
pub async fn expect_request_eventually_ok(
    harness: &super::harness::TestHarness,
    from_serial: u64,
    to_serial: u64,
    request_prefix: &str,
    total_timeout: Duration,
    attempt_timeout_ms: u32,
) -> Bytes {
    let deadline = tokio::time::Instant::now() + total_timeout;
    let mut attempt = 0;

    loop {
        attempt += 1;
        let request_id = format!("{request_prefix}_{attempt}");
        let handle =
            harness
                .peer(from_serial)
                .spawn_request(to_serial, &request_id, attempt_timeout_ms);

        let last_error = match tokio::time::timeout(
            Duration::from_millis(attempt_timeout_ms as u64) + Duration::from_secs(1),
            handle,
        )
        .await
        {
            Ok(Ok(Ok(response))) => return response,
            Ok(Ok(Err(err))) => {
                let msg = err.to_string();
                assert!(
                    msg.contains("connection not ready")
                        || msg.contains("Request timeout")
                        || msg.contains("Connection"),
                    "unexpected retry error while waiting for recovery: {msg}"
                );
                msg
            }
            Ok(Err(err)) => panic!("{request_prefix} retry task panicked: {err}"),
            Err(_) => format!("{request_prefix} attempt {attempt} timed out"),
        };

        if tokio::time::Instant::now() >= deadline {
            panic!(
                "{request_prefix} did not succeed within {:?}; last error: {}",
                total_timeout, last_error
            );
        }

        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}
