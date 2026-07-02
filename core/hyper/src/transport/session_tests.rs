use super::*;

#[test]
fn test_session_id_monotonic() {
    let s1 = ConnectionSession::new();
    let s2 = ConnectionSession::new();
    assert!(s2.session_id > s1.session_id);
}

#[test]
fn test_try_close_idempotent() {
    let session = ConnectionSession::new();
    assert!(session.try_close());
    assert!(!session.try_close());
    assert!(session.is_closed());
}

#[test]
fn test_cancel_token() {
    let session = ConnectionSession::new();
    assert!(!session.is_cancelled());
    session.cancel();
    assert!(session.is_cancelled());
}

#[test]
fn test_clone_shares_state() {
    let s1 = ConnectionSession::new();
    let s2 = s1.clone();
    s1.cancel();
    assert!(s2.is_cancelled());
    assert!(s1.try_close());
    assert!(!s2.try_close());
}
