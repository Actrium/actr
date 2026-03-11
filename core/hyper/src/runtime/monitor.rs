//! Mode 2 child process health monitoring and restart logic
//!
//! Continuously monitors child process status in a background task,
//! deciding whether to re-spawn based on RestartPolicy.
//! Gracefully exits and terminates the child process upon receiving a shutdown signal.

use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::error::HyperResult;

use super::handle::{ChildProcessHandle, ChildProcessState};
use super::spawn::{RestartPolicy, SpawnConfig, spawn};

/// Start a background monitoring task that decides whether to restart based on restart policy
///
/// # Behavior
///
/// - Waits for the process to exit (`handle.wait().await`)
/// - Decides whether to re-spawn based on RestartPolicy
/// - On shutdown signal, kills the current child process first, then returns
/// - No sleep between restarts; directly re-spawns and enters the next monitoring loop
///
/// # Parameters
///
/// - `handle`: initial child process handle
/// - `config`: spawn configuration (reused on restart)
/// - `shutdown`: external cancellation token, triggers graceful exit
pub async fn monitor_process(
    mut handle: ChildProcessHandle,
    config: SpawnConfig,
    shutdown: CancellationToken,
) -> HyperResult<()> {
    let mut retries_used: u32 = 0;

    loop {
        // Wait for either: child process exit OR shutdown signal
        let exit_state = tokio::select! {
            // child process exited
            result = handle.wait() => {
                match result {
                    Ok(state) => state,
                    Err(e) => {
                        error!(
                            pid = handle.pid,
                            actr_type = %handle.actr_type,
                            error = %e,
                            "error waiting for child process, treating as crash"
                        );
                        ChildProcessState::Crashed
                    }
                }
            },
            // shutdown signal
            _ = shutdown.cancelled() => {
                info!(
                    pid = handle.pid,
                    actr_type = %handle.actr_type,
                    "received shutdown signal, terminating child process"
                );
                // gracefully terminate child process
                if let Err(e) = handle.kill().await {
                    error!(
                        pid = handle.pid,
                        error = %e,
                        "failed to terminate child process during shutdown"
                    );
                }
                info!(
                    actr_type = %handle.actr_type,
                    "monitor task gracefully exited"
                );
                return Ok(());
            }
        };

        // Log process exit reason
        match &exit_state {
            ChildProcessState::Exited(0) => {
                info!(
                    actr_type = %handle.actr_type,
                    pid = handle.pid,
                    "child process exited normally (exit code 0)"
                );
            }
            ChildProcessState::Exited(code) => {
                error!(
                    actr_type = %handle.actr_type,
                    pid = handle.pid,
                    exit_code = code,
                    "child process exited with non-zero exit code"
                );
            }
            ChildProcessState::Crashed => {
                error!(
                    actr_type = %handle.actr_type,
                    pid = handle.pid,
                    "child process terminated abnormally (signal or unknown cause)"
                );
            }
            ChildProcessState::Running => {
                // wait() should not return Running, defensive handling
                warn!(
                    actr_type = %handle.actr_type,
                    "wait() returned Running state, skipping this round"
                );
                continue;
            }
        }

        // If shutdown has been triggered, do not restart
        if shutdown.is_cancelled() {
            info!(
                actr_type = %handle.actr_type,
                "shutdown triggered, not restarting child process"
            );
            return Ok(());
        }

        // Decide whether to restart based on RestartPolicy
        let should_restart = match &config.restart_policy {
            RestartPolicy::Never => {
                info!(
                    actr_type = %handle.actr_type,
                    "RestartPolicy::Never, not restarting"
                );
                false
            }
            RestartPolicy::OnFailure { max_retries } => {
                let failed = !matches!(exit_state, ChildProcessState::Exited(0));
                if !failed {
                    info!(
                        actr_type = %handle.actr_type,
                        "child process exited normally, OnFailure policy does not restart"
                    );
                    false
                } else if retries_used >= *max_retries {
                    error!(
                        actr_type = %handle.actr_type,
                        retries_used,
                        max_retries,
                        "max retry count reached, stopping restarts"
                    );
                    false
                } else {
                    true
                }
            }
            RestartPolicy::Always { max_retries } => {
                if retries_used >= *max_retries {
                    error!(
                        actr_type = %handle.actr_type,
                        retries_used,
                        max_retries,
                        "max retry count reached, stopping restarts"
                    );
                    false
                } else {
                    true
                }
            }
        };

        if !should_restart {
            info!(
                actr_type = %handle.actr_type,
                "monitor task finished, no more restarts"
            );
            return Ok(());
        }

        retries_used += 1;
        warn!(
            actr_type = %handle.actr_type,
            retries_used,
            "restarting child process (attempt {retries_used})"
        );

        // Re-spawn without sleep — directly enter the next loop iteration
        handle = match spawn(SpawnConfig {
            binary_path: config.binary_path.clone(),
            args: config.args.clone(),
            credential: config.credential.clone(),
            extra_env: config.extra_env.clone(),
            restart_policy: config.restart_policy.clone(),
            actr_type: config.actr_type.clone(),
        })
        .await
        {
            Ok(h) => {
                warn!(
                    pid = h.pid,
                    actr_type = %config.actr_type,
                    retries_used,
                    "child process restarted"
                );
                h
            }
            Err(e) => {
                error!(
                    actr_type = %config.actr_type,
                    error = %e,
                    "failed to restart child process, monitor task exiting"
                );
                return Err(e);
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::spawn::{RestartPolicy, SpawnConfig, spawn};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    fn basic_config(binary: &str, args: Vec<&str>, policy: RestartPolicy) -> SpawnConfig {
        SpawnConfig {
            binary_path: PathBuf::from(binary),
            args: args.into_iter().map(|s| s.to_string()).collect(),
            credential: vec![],
            extra_env: HashMap::new(),
            restart_policy: policy,
            actr_type: "test:monitor".to_string(),
        }
    }

    /// Test RestartPolicy::Never: process exits and is not restarted, monitor finishes normally
    #[tokio::test]
    async fn never_policy_does_not_restart() {
        let config = basic_config("/usr/bin/true", vec![], RestartPolicy::Never);
        let handle = spawn(SpawnConfig {
            binary_path: config.binary_path.clone(),
            args: config.args.clone(),
            credential: config.credential.clone(),
            extra_env: config.extra_env.clone(),
            restart_policy: config.restart_policy.clone(),
            actr_type: config.actr_type.clone(),
        })
        .await
        .expect("spawn should succeed");

        let shutdown = CancellationToken::new();
        // monitor_process should return immediately after true exits
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            monitor_process(handle, config, shutdown),
        )
        .await
        .expect("monitor should finish within 5 seconds");

        assert!(
            result.is_ok(),
            "monitor should finish normally with Never policy"
        );
    }

    /// Test that shutdown signal can terminate a monitoring long-running child process
    #[tokio::test]
    async fn shutdown_cancels_monitoring() {
        // sleep 100 seconds, will not exit naturally
        let config = basic_config("/bin/sleep", vec!["100"], RestartPolicy::Never);
        let handle = spawn(SpawnConfig {
            binary_path: config.binary_path.clone(),
            args: config.args.clone(),
            credential: config.credential.clone(),
            extra_env: config.extra_env.clone(),
            restart_policy: config.restart_policy.clone(),
            actr_type: config.actr_type.clone(),
        })
        .await
        .expect("spawn sleep should succeed");

        let shutdown = CancellationToken::new();
        let shutdown_clone = shutdown.clone();

        // trigger shutdown after 0.5 seconds
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            shutdown_clone.cancel();
        });

        let result = tokio::time::timeout(
            Duration::from_secs(10),
            monitor_process(handle, config, shutdown),
        )
        .await
        .expect("monitor should finish within 10 seconds after shutdown");

        assert!(
            result.is_ok(),
            "monitor should exit normally after shutdown"
        );
    }

    /// Test OnFailure policy: restart after failure, stop after reaching max retries
    #[tokio::test]
    async fn on_failure_retries_and_stops() {
        // false command exits with code 1 (failure)
        let config = basic_config(
            "/usr/bin/false",
            vec![],
            RestartPolicy::OnFailure { max_retries: 2 },
        );
        let handle = spawn(SpawnConfig {
            binary_path: config.binary_path.clone(),
            args: config.args.clone(),
            credential: config.credential.clone(),
            extra_env: config.extra_env.clone(),
            restart_policy: config.restart_policy.clone(),
            actr_type: config.actr_type.clone(),
        })
        .await
        .expect("spawn should succeed");

        let shutdown = CancellationToken::new();
        // max 2 retries, total spawn = 3 (initial + 2 restarts)
        // each false exits immediately, should complete within 5 seconds
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            monitor_process(handle, config, shutdown),
        )
        .await
        .expect("monitor should finish within 5 seconds");

        assert!(
            result.is_ok(),
            "should exit normally after reaching max retry count"
        );
    }
}
