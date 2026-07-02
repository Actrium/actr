use super::*;

fn test_actor_id() -> ActrId {
    ActrId {
        realm: actr_protocol::Realm { realm_id: 1 },
        serial_number: 42,
        r#type: actr_protocol::ActrType {
            manufacturer: "test".into(),
            name: "actor".into(),
            version: "1.0.0".into(),
        },
    }
}

#[tokio::test]
async fn soft_renew_preserves_identity() {
    let snap = SessionSnapshot::empty_with_id(test_actor_id(), 1);
    let state = SessionState::new(snap);

    assert_eq!(state.actor_id().await.serial_number, 42);
    assert_eq!(state.generation().await, 1);
    assert_eq!(state.phase().await, SessionPhase::Active);

    let new_cred = AIdCredential {
        key_id: 99,
        ..Default::default()
    };
    let new_turn = TurnCredential {
        username: "renewed".into(),
        ..Default::default()
    };
    state
        .update_credentials(
            new_cred.clone(),
            Timestamp {
                seconds: 200,
                nanos: 0,
            },
            new_turn.clone(),
            Bytes::from_static(b"new-renewal-token-32-bytes!!!\0"),
            Timestamp {
                seconds: 300,
                nanos: 0,
            },
        )
        .await;

    // Identity unchanged.
    assert_eq!(state.actor_id().await.serial_number, 42);
    assert_eq!(state.generation().await, 1);
    // Credentials updated.
    assert_eq!(state.credential().await.key_id, 99);
    assert_eq!(state.turn_credential().await.username, "renewed");
}

#[tokio::test]
async fn hard_rebind_bumps_generation() {
    let old = SessionSnapshot::empty_with_id(test_actor_id(), 1);
    let state = SessionState::new(old);

    let new_id = ActrId {
        serial_number: 99,
        ..test_actor_id()
    };
    let new_snap = SessionSnapshot::empty_with_id(new_id, 2);
    let _old_snap = state.commit_hard_rebind(new_snap).await;

    assert_eq!(state.actor_id().await.serial_number, 99);
    assert_eq!(state.generation().await, 2);
    assert_eq!(state.phase().await, SessionPhase::Active);
    assert!(!state.is_current_generation(1).await);
    assert!(state.is_current_generation(2).await);
}

#[tokio::test]
async fn phase_transitions_cover_all_states() {
    let state = SessionState::new(SessionSnapshot::empty_with_id(test_actor_id(), 1));
    assert_eq!(state.phase().await, SessionPhase::Active);

    state.enter_rebinding().await;
    assert_eq!(state.phase().await, SessionPhase::Rebinding);

    state.set_active().await;
    assert_eq!(state.phase().await, SessionPhase::Active);

    state.set_realm_unavailable().await;
    assert_eq!(state.phase().await, SessionPhase::RealmUnavailable);
}

#[tokio::test]
async fn sync_readers_return_some_when_unlocked() {
    let state = SessionState::new(SessionSnapshot::empty_with_id(test_actor_id(), 7));
    // No contending lock held → try_read succeeds.
    assert_eq!(state.actor_id_sync().map(|i| i.serial_number), Some(42));
    assert_eq!(state.generation_sync(), Some(7));
}

#[tokio::test]
async fn snapshot_clone_and_credential_accessors_reflect_state() {
    let state = SessionState::new(SessionSnapshot::empty_with_id(test_actor_id(), 3));

    // Full snapshot clone carries the generation.
    assert_eq!(state.snapshot().await.generation, 3);

    // Default-constructed accessors on an empty snapshot.
    assert_eq!(state.credential_expires_at().await, Timestamp::default());
    assert!(state.renewal_token().await.is_empty());
    assert_eq!(state.renewal_token_expires_at().await, Timestamp::default());
    assert_eq!(state.credential().await, AIdCredential::default());
    assert_eq!(state.turn_credential().await, TurnCredential::default());
}

#[tokio::test]
async fn commit_hard_rebind_returns_previous_snapshot() {
    let state = SessionState::new(SessionSnapshot::empty_with_id(test_actor_id(), 1));
    let prev = state
        .commit_hard_rebind(SessionSnapshot::empty_with_id(test_actor_id(), 5))
        .await;
    // Returned snapshot is the pre-rebind one (generation 1).
    assert_eq!(prev.generation, 1);
    // New current generation reflects the committed snapshot.
    assert_eq!(state.generation().await, 5);
    assert!(state.is_current_generation(5).await);
}
