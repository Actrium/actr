use async_trait::async_trait;
use tokio::sync::Mutex;
use tracing::{error, warn};

use crate::error::{HyperError, HyperResult};

/// ActrSystem handle trait (Mode 1 / Mode 3)
///
/// Hyper manages in-process ActrSystem lifecycle through this interface.
/// Mode 1 (Native): ActrSystem+Workload compiled into the same binary, directly implements this trait.
/// Mode 3 (WASM): ActrSystem native shell wraps a WASM instance and implements this trait.
#[async_trait]
pub trait ActrSystemHandle: Send + Sync {
    /// Start the ActrSystem
    async fn start(&self) -> HyperResult<()>;

    /// Gracefully shut down the ActrSystem, wait for in-flight messages to complete
    async fn shutdown(&self) -> HyperResult<()>;

    /// Whether healthy (used for Hyper-side monitoring)
    fn is_healthy(&self) -> bool;

    /// ActrSystem unique identifier (for debugging)
    fn id(&self) -> &str;
}

/// Mode 2 (Process) child process handle
///
/// Hyper manages child process lifecycle through this handle: spawn, health check, restart policy.
/// The ActrSystem inside the child process connects directly to signaling; message traffic does not go through Hyper.
pub struct ChildProcessHandle {
    /// Child process PID
    pub pid: u32,
    /// ActrType of the child process (for debugging/logging)
    pub actr_type: String,
    /// Child process state
    pub state: ChildProcessState,
    /// Tokio child process handle (owns it for wait/kill)
    pub(crate) child: Option<Mutex<tokio::process::Child>>,
}

impl std::fmt::Debug for ChildProcessHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChildProcessHandle")
            .field("pid", &self.pid)
            .field("actr_type", &self.actr_type)
            .field("state", &self.state)
            .field("child", &self.child.as_ref().map(|_| "<Child>"))
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildProcessState {
    /// Running
    Running,
    /// Exited with exit code
    Exited(i32),
    /// Abnormally terminated (signal, etc., no exit code available)
    Crashed,
}

impl ChildProcessHandle {
    /// For testing only or scenarios that don't need a real child process handle
    pub fn new(pid: u32, actr_type: impl Into<String>) -> Self {
        Self {
            pid,
            actr_type: actr_type.into(),
            state: ChildProcessState::Running,
            child: None,
        }
    }

    /// Construct from a real tokio Child
    pub fn from_child(
        pid: u32,
        actr_type: impl Into<String>,
        child: tokio::process::Child,
    ) -> Self {
        Self {
            pid,
            actr_type: actr_type.into(),
            state: ChildProcessState::Running,
            child: Some(Mutex::new(child)),
        }
    }

    pub fn is_running(&self) -> bool {
        self.state == ChildProcessState::Running
    }

    /// Wait for the child process to exit, return the final state
    ///
    /// If no child handle is available (already consumed), returns the current state directly.
    pub async fn wait(&mut self) -> HyperResult<ChildProcessState> {
        let Some(child_mutex) = &self.child else {
            return Ok(self.state.clone());
        };

        let mut child = child_mutex.lock().await;
        match child.wait().await {
            Ok(status) => {
                let state = if let Some(code) = status.code() {
                    ChildProcessState::Exited(code)
                } else {
                    // terminated by signal, no exit code
                    ChildProcessState::Crashed
                };
                self.state = state.clone();
                Ok(state)
            }
            Err(e) => {
                error!(
                    pid = self.pid,
                    actr_type = %self.actr_type,
                    error = %e,
                    "error waiting for child process to exit"
                );
                self.state = ChildProcessState::Crashed;
                Err(HyperError::Runtime(format!("wait() failed: {e}")))
            }
        }
    }

    /// Terminate child process: send SIGTERM first, wait up to 5 seconds, then SIGKILL on timeout
    pub async fn kill(&mut self) -> HyperResult<()> {
        let Some(child_mutex) = &self.child else {
            // no handle, consider already terminated
            return Ok(());
        };

        let mut child = child_mutex.lock().await;

        // Send SIGTERM (Unix) / TerminateProcess (Windows)
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            // send SIGTERM to child process
            if let Err(e) = nix_kill(self.pid, libc_sigterm()) {
                warn!(
                    pid = self.pid,
                    actr_type = %self.actr_type,
                    error = %e,
                    "SIGTERM failed, falling back to SIGKILL"
                );
                let _ = child.kill().await;
                self.state = ChildProcessState::Crashed;
                return Ok(());
            }

            warn!(
                pid = self.pid,
                actr_type = %self.actr_type,
                "SIGTERM sent, waiting for graceful exit (up to 5 seconds)"
            );

            // wait up to 5 seconds
            match tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await {
                Ok(Ok(status)) => {
                    let code = status.code().or_else(|| status.signal().map(|s| -s));
                    self.state = match code {
                        Some(0) => ChildProcessState::Exited(0),
                        Some(c) => ChildProcessState::Exited(c),
                        None => ChildProcessState::Crashed,
                    };
                    warn!(
                        pid = self.pid,
                        actr_type = %self.actr_type,
                        state = ?self.state,
                        "child process exited after SIGTERM"
                    );
                    return Ok(());
                }
                Ok(Err(e)) => {
                    error!(
                        pid = self.pid,
                        error = %e,
                        "error waiting for child exit, escalating to SIGKILL"
                    );
                }
                Err(_timeout) => {
                    warn!(
                        pid = self.pid,
                        actr_type = %self.actr_type,
                        "no exit within 5 seconds after SIGTERM, escalating to SIGKILL"
                    );
                }
            }

            // SIGTERM timed out, send SIGKILL
            if let Err(e) = child.kill().await {
                error!(
                    pid = self.pid,
                    error = %e,
                    "SIGKILL failed"
                );
            }
            let _ = child.wait().await;
            self.state = ChildProcessState::Crashed;
        }

        #[cfg(not(unix))]
        {
            warn!(
                pid = self.pid,
                actr_type = %self.actr_type,
                "non-Unix platform, directly killing child process"
            );
            if let Err(e) = child.kill().await {
                error!(pid = self.pid, error = %e, "failed to kill child process");
                return Err(HyperError::Runtime(format!("kill failed: {e}")));
            }
            let _ = child.wait().await;
            self.state = ChildProcessState::Crashed;
        }

        Ok(())
    }

    /// Non-blocking check whether the process is still running
    ///
    /// Uses `try_wait()` to poll without blocking the current thread.
    pub fn try_check_alive(&mut self) -> bool {
        let Some(child_mutex) = &self.child else {
            return self.state == ChildProcessState::Running;
        };

        // try_lock failure means another task is waiting, conservatively return true
        let Ok(mut child) = child_mutex.try_lock() else {
            return true;
        };

        match child.try_wait() {
            Ok(None) => true, // process still running
            Ok(Some(status)) => {
                self.state = if let Some(code) = status.code() {
                    ChildProcessState::Exited(code)
                } else {
                    ChildProcessState::Crashed
                };
                false
            }
            Err(e) => {
                error!(
                    pid = self.pid,
                    error = %e,
                    "try_wait() error, conservatively assuming process has exited"
                );
                self.state = ChildProcessState::Crashed;
                false
            }
        }
    }
}

/// Unix platform: send SIGTERM via libc
#[cfg(unix)]
fn nix_kill(pid: u32, sig: i32) -> Result<(), String> {
    // SAFETY: kill(2) is a standard POSIX system call
    let ret = unsafe { libc::kill(pid as i32, sig) };
    if ret == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

#[cfg(unix)]
fn libc_sigterm() -> i32 {
    libc::SIGTERM
}

/// Mode 3 (WASM) instance handle
///
/// Created and held by the ActrSystem native shell.
/// On hot update, the old instance is unloaded and a new instance handle is created.
#[derive(Debug)]
pub struct WasmInstanceHandle {
    /// WASM instance unique ID (generated on each load)
    pub instance_id: String,
    /// Corresponding ActrType
    pub actr_type: String,
}

impl WasmInstanceHandle {
    pub fn new(instance_id: impl Into<String>, actr_type: impl Into<String>) -> Self {
        Self {
            instance_id: instance_id.into(),
            actr_type: actr_type.into(),
        }
    }
}
