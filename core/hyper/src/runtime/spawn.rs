//! Mode 2 child process spawn implementation
//!
//! Responsible for launching ActrSystem+Workload as a child process,
//! and securely passing AIS credential to the child process via environment variables.

use std::collections::HashMap;
use std::path::PathBuf;

use base64::Engine as _;
use tokio::process::Command;
use tracing::info;

use crate::error::{HyperError, HyperResult};

use super::handle::ChildProcessHandle;

/// Mode 2 child process spawn configuration
pub struct SpawnConfig {
    /// Executable path (already verified via signature)
    pub binary_path: PathBuf,
    /// Command-line arguments passed to the child process
    pub args: Vec<String>,
    /// Credential bytes (base64-encoded and written to the ACTR_CREDENTIAL environment variable)
    pub credential: Vec<u8>,
    /// Extra environment variables (beyond ACTR_CREDENTIAL)
    pub extra_env: HashMap<String, String>,
    /// restart policy
    pub restart_policy: RestartPolicy,
    /// ActrType string (for debugging/logging)
    pub actr_type: String,
}

/// Restart policy
#[derive(Debug, Clone)]
pub enum RestartPolicy {
    /// Never auto-restart
    Never,
    /// Restart when exit code is non-zero, up to N times
    OnFailure { max_retries: u32 },
    /// Always restart (unless explicitly shutdown), up to N times
    Always { max_retries: u32 },
}

/// Spawn a Mode 2 child process
///
/// Steps:
/// 1. Base64-encode the credential, write to the ACTR_CREDENTIAL environment variable
/// 2. Set extra environment variables
/// 3. Spawn the child process
/// 4. Log spawn success (with binary path and pid)
/// 5. Return ChildProcessHandle
pub async fn spawn(config: SpawnConfig) -> HyperResult<ChildProcessHandle> {
    // base64-encode the credential to avoid writing binary data directly to env vars
    let credential_b64 = base64::engine::general_purpose::STANDARD.encode(&config.credential);

    let mut cmd = Command::new(&config.binary_path);

    // pass command-line arguments
    cmd.args(&config.args);

    // pass AIS credential
    cmd.env("ACTR_CREDENTIAL", &credential_b64);

    // pass extra environment variables
    for (key, value) in &config.extra_env {
        cmd.env(key, value);
    }

    // spawn child process, keep stdin/stdout/stderr inherited (parent does no I/O proxying)
    let child = cmd.spawn().map_err(|e| {
        HyperError::Runtime(format!(
            "failed to spawn child process (binary: {}): {e}",
            config.binary_path.display()
        ))
    })?;

    // get PID (always available after successful spawn)
    let pid = child.id().ok_or_else(|| {
        HyperError::Runtime("unable to get child process PID after spawn".to_string())
    })?;

    info!(
        pid,
        actr_type = %config.actr_type,
        binary = %config.binary_path.display(),
        "Mode 2 child process started"
    );

    Ok(ChildProcessHandle::from_child(pid, config.actr_type, child))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Test that spawn() can successfully start a real child process (true command exits immediately with 0)
    #[tokio::test]
    async fn spawn_true_succeeds() {
        let config = SpawnConfig {
            binary_path: PathBuf::from("/usr/bin/true"),
            args: vec![],
            credential: vec![1, 2, 3],
            extra_env: HashMap::new(),
            restart_policy: RestartPolicy::Never,
            actr_type: "test:unit".to_string(),
        };

        let mut handle = spawn(config).await.expect("spawn should succeed");
        assert!(handle.pid > 0, "PID should be greater than 0");

        let state = handle.wait().await.expect("wait should succeed");
        assert_eq!(
            state,
            crate::runtime::handle::ChildProcessState::Exited(0),
            "true command should exit with code 0"
        );
    }

    /// Test that ACTR_CREDENTIAL env var is correctly passed to child process
    ///
    /// Uses `printenv ACTR_CREDENTIAL` to capture output, verifying base64-encoded credential is passed correctly.
    #[tokio::test]
    async fn credential_env_is_passed_to_child() {
        let credential = b"test-credential-bytes".to_vec();
        let expected_b64 = base64::engine::general_purpose::STANDARD.encode(&credential);

        // write ACTR_CREDENTIAL to a temp file
        let tmp = tempfile::NamedTempFile::new().expect("create temp file");
        let tmp_path = tmp.path().to_str().unwrap().to_string();

        // sh -c 'printf "%s" "$ACTR_CREDENTIAL" > /tmp/xxx'
        let script = format!("printf '%s' \"$ACTR_CREDENTIAL\" > {tmp_path}");

        let config = SpawnConfig {
            binary_path: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_string(), script],
            credential,
            extra_env: HashMap::new(),
            restart_policy: RestartPolicy::Never,
            actr_type: "test:credential".to_string(),
        };

        let mut handle = spawn(config).await.expect("spawn should succeed");
        let state = handle.wait().await.expect("wait should succeed");
        assert_eq!(
            state,
            crate::runtime::handle::ChildProcessState::Exited(0),
            "sh script should exit successfully"
        );

        // read output, verify credential was correctly passed
        let actual = std::fs::read_to_string(&tmp_path).expect("read temp file");
        assert_eq!(
            actual.trim(),
            expected_b64.trim(),
            "ACTR_CREDENTIAL env var should be base64-encoded credential"
        );
    }

    /// Test that extra environment variables are correctly passed
    #[tokio::test]
    async fn extra_env_is_passed_to_child() {
        let tmp = tempfile::NamedTempFile::new().expect("create temp file");
        let tmp_path = tmp.path().to_str().unwrap().to_string();

        let script = format!("printf '%s' \"$MY_EXTRA_VAR\" > {tmp_path}");

        let mut extra_env = HashMap::new();
        extra_env.insert("MY_EXTRA_VAR".to_string(), "hello-extra".to_string());

        let config = SpawnConfig {
            binary_path: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_string(), script],
            credential: vec![],
            extra_env,
            restart_policy: RestartPolicy::Never,
            actr_type: "test:extra-env".to_string(),
        };

        let mut handle = spawn(config).await.expect("spawn should succeed");
        handle.wait().await.expect("wait should succeed");

        let actual = std::fs::read_to_string(&tmp_path).expect("read temp file");
        assert_eq!(
            actual, "hello-extra",
            "extra env var should be correctly passed"
        );
    }

    /// Test that kill() can terminate a running child process
    #[tokio::test]
    async fn kill_terminates_running_process() {
        // sleep 100 seconds, will not exit naturally
        let mut cmd = tokio::process::Command::new("/bin/sleep");
        cmd.arg("100");
        let child = cmd.spawn().expect("spawn sleep should succeed");
        let pid = child.id().expect("should have pid");

        let mut handle = ChildProcessHandle::from_child(pid, "test:kill", child);
        assert!(handle.is_running(), "should be Running after spawn");

        // kill should complete within 10 seconds (SIGTERM waits up to 5 seconds, then SIGKILL)
        tokio::time::timeout(Duration::from_secs(10), handle.kill())
            .await
            .expect("kill should not timeout")
            .expect("kill should succeed");

        // state should be updated to non-Running
        assert!(
            !handle.is_running(),
            "process should no longer be running after kill"
        );
    }

    /// Test that try_check_alive() returns false for an exited process
    #[tokio::test]
    async fn try_check_alive_returns_false_after_exit() {
        use crate::runtime::handle::ChildProcessState;

        let mut cmd = tokio::process::Command::new("/usr/bin/true");
        let child = cmd.spawn().expect("spawn should succeed");
        let pid = child.id().expect("should have pid");

        let mut handle = ChildProcessHandle::from_child(pid, "test:alive", child);

        // wait for the process to exit naturally, then try_wait can detect it
        tokio::time::sleep(Duration::from_millis(300)).await;

        let alive = handle.try_check_alive();
        assert!(
            !alive,
            "try_check_alive should return false after process exits"
        );
        assert!(
            matches!(handle.state, ChildProcessState::Exited(0)),
            "exit state should be Exited(0), actual: {:?}",
            handle.state
        );
    }
}
