use super::*;

#[cfg(unix)]
#[test]
fn is_process_alive_returns_false_for_nonexistent_pid() {
    assert!(!is_process_alive(9_999_999));
}

#[cfg(unix)]
#[test]
fn terminate_process_returns_false_for_nonexistent_pid() {
    assert!(!terminate_process(9_999_999).unwrap());
}

#[cfg(unix)]
#[test]
fn kill_process_returns_false_for_nonexistent_pid() {
    assert!(!kill_process(9_999_999).unwrap());
}

#[cfg(unix)]
#[tokio::test]
async fn wait_for_exit_returns_true_for_nonexistent_pid() {
    assert!(wait_for_exit(9_999_999, std::time::Duration::from_secs(1)).await);
}
