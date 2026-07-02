use super::*;
use actr_protocol::{ActrId, ActrType, PayloadType, Realm};

fn test_peer_id() -> ActrId {
    ActrId {
        realm: Realm { realm_id: 1 },
        serial_number: 1,
        r#type: ActrType {
            manufacturer: "test".to_string(),
            name: "device".to_string(),
            version: "1.0.0".to_string(),
        },
    }
}

#[tokio::test]
async fn test_broadcaster_send_receive() {
    let broadcaster = ConnectionEventBroadcaster::new();
    let mut rx = broadcaster.subscribe();

    let peer_id = test_peer_id();
    broadcaster.send(ConnectionEvent::ConnectionClosed {
        peer_id: peer_id.clone(),
        session_id: 0,
    });

    let event = rx.recv().await.unwrap();
    assert!(matches!(event, ConnectionEvent::ConnectionClosed { .. }));
}

#[tokio::test]
async fn test_multiple_subscribers() {
    let broadcaster = ConnectionEventBroadcaster::new();
    let mut rx1 = broadcaster.subscribe();
    let mut rx2 = broadcaster.subscribe();

    let peer_id = test_peer_id();
    let count = broadcaster.send(ConnectionEvent::StateChanged {
        peer_id: peer_id.clone(),
        session_id: 0,
        state: ConnectionState::Connected,
    });

    assert_eq!(count, 2);

    let event1 = rx1.recv().await.unwrap();
    let event2 = rx2.recv().await.unwrap();

    assert!(matches!(
        event1,
        ConnectionEvent::StateChanged {
            state: ConnectionState::Connected,
            ..
        }
    ));
    assert!(matches!(
        event2,
        ConnectionEvent::StateChanged {
            state: ConnectionState::Connected,
            ..
        }
    ));
}

#[test]
fn test_should_trigger_cleanup() {
    let peer_id = test_peer_id();

    // Should trigger cleanup
    assert!(
        ConnectionEvent::ConnectionClosed {
            peer_id: peer_id.clone(),
            session_id: 0,
        }
        .should_trigger_cleanup()
    );

    assert!(
        ConnectionEvent::StateChanged {
            peer_id: peer_id.clone(),
            session_id: 0,
            state: ConnectionState::Closed,
        }
        .should_trigger_cleanup()
    );

    assert!(
        ConnectionEvent::IceRestartCompleted {
            peer_id: peer_id.clone(),
            session_id: 0,
            success: false,
        }
        .should_trigger_cleanup()
    );

    // Should NOT trigger cleanup
    assert!(
        !ConnectionEvent::StateChanged {
            peer_id: peer_id.clone(),
            session_id: 0,
            state: ConnectionState::Disconnected,
        }
        .should_trigger_cleanup()
    );

    assert!(
        !ConnectionEvent::IceRestartCompleted {
            peer_id: peer_id.clone(),
            session_id: 0,
            success: true,
        }
        .should_trigger_cleanup()
    );
}

#[test]
fn test_is_recoverable_state() {
    let peer_id = test_peer_id();

    // Recoverable states
    assert!(
        ConnectionEvent::StateChanged {
            peer_id: peer_id.clone(),
            session_id: 0,
            state: ConnectionState::Disconnected,
        }
        .is_recoverable_state()
    );

    assert!(
        ConnectionEvent::StateChanged {
            peer_id: peer_id.clone(),
            session_id: 0,
            state: ConnectionState::Failed,
        }
        .is_recoverable_state()
    );

    // Not recoverable
    assert!(
        !ConnectionEvent::StateChanged {
            peer_id: peer_id.clone(),
            session_id: 0,
            state: ConnectionState::Closed,
        }
        .is_recoverable_state()
    );

    assert!(
        !ConnectionEvent::ConnectionClosed {
            peer_id: peer_id.clone(),
            session_id: 0,
        }
        .is_recoverable_state()
    );
}

#[test]
fn connection_state_display_covers_all_variants() {
    assert_eq!(ConnectionState::New.to_string(), "New");
    assert_eq!(ConnectionState::Connecting.to_string(), "Connecting");
    assert_eq!(ConnectionState::Connected.to_string(), "Connected");
    assert_eq!(ConnectionState::Disconnected.to_string(), "Disconnected");
    assert_eq!(ConnectionState::Failed.to_string(), "Failed");
    assert_eq!(ConnectionState::Closed.to_string(), "Closed");
}

#[test]
fn peer_id_accessor_covers_all_variants() {
    let id = test_peer_id();
    let cases = [
        ConnectionEvent::StateChanged {
            peer_id: id.clone(),
            session_id: 0,
            state: ConnectionState::New,
        },
        ConnectionEvent::DataChannelClosed {
            peer_id: id.clone(),
            session_id: 0,
            payload_type: PayloadType::RpcReliable,
        },
        ConnectionEvent::DataChannelOpened {
            peer_id: id.clone(),
            session_id: 0,
            payload_type: PayloadType::RpcReliable,
        },
        ConnectionEvent::ConnectionClosed {
            peer_id: id.clone(),
            session_id: 0,
        },
        ConnectionEvent::IceRestartStarted {
            peer_id: id.clone(),
            session_id: 0,
        },
        ConnectionEvent::IceRestartCompleted {
            peer_id: id.clone(),
            session_id: 0,
            success: true,
        },
        ConnectionEvent::NewOfferReceived {
            peer_id: id.clone(),
            sdp: "sdp".into(),
        },
        ConnectionEvent::NewRoleAssignment {
            peer_id: id.clone(),
            is_offerer: true,
        },
    ];
    for ev in &cases {
        assert_eq!(ev.peer_id(), &id);
    }
}

#[test]
fn session_id_accessor_distinguishes_events() {
    let id = test_peer_id();
    // Events with session_id → Some.
    assert_eq!(
        ConnectionEvent::ConnectionClosed {
            peer_id: id.clone(),
            session_id: 7,
        }
        .session_id(),
        Some(7)
    );
    // Events without session_id → None.
    assert_eq!(
        ConnectionEvent::NewOfferReceived {
            peer_id: id.clone(),
            sdp: "sdp".into(),
        }
        .session_id(),
        None
    );
    assert_eq!(
        ConnectionEvent::NewRoleAssignment {
            peer_id: id,
            is_offerer: false,
        }
        .session_id(),
        None
    );
}

#[test]
fn broadcaster_send_without_subscribers_returns_zero() {
    let bc = ConnectionEventBroadcaster::new();
    let n = bc.send(ConnectionEvent::ConnectionClosed {
        peer_id: test_peer_id(),
        session_id: 0,
    });
    assert_eq!(n, 0);
}

#[tokio::test]
async fn broadcaster_with_capacity_and_clone_and_sender() {
    let bc = ConnectionEventBroadcaster::with_capacity(2);
    let mut rx = bc.subscribe();

    // Clone preserves the shared channel.
    let bc2 = bc.clone();
    bc2.send(ConnectionEvent::IceRestartStarted {
        peer_id: test_peer_id(),
        session_id: 1,
    });
    let ev = rx.recv().await.unwrap();
    assert_eq!(ev.session_id(), Some(1));

    // sender() returns a usable clone of the sender.
    let tx = bc.sender();
    let _ = tx.send(ConnectionEvent::ConnectionClosed {
        peer_id: test_peer_id(),
        session_id: 2,
    });
    let ev = rx.recv().await.unwrap();
    assert_eq!(ev.session_id(), Some(2));

    // Default == new().
    let _default = ConnectionEventBroadcaster::default();
}
