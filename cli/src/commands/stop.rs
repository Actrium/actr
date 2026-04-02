use crate::commands::Command;
use crate::commands::runtime_state::{RuntimeStateStore, RuntimeStatus, resolve_hyper_dir};
use crate::error::{ActrCliError, Result};
use async_trait::async_trait;
use chrono::Utc;
use clap::Args;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Args, Debug)]
pub struct StopCommand {
    /// WID (or unique prefix, min 8 chars) of the runtime to stop
    #[arg(value_name = "WID")]
    pub wid: String,

    /// Runtime configuration file
    #[arg(short = 'c', long = "config", value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Hyper data directory
    #[arg(long = "hyper-dir", value_name = "DIR")]
    pub hyper_dir: Option<PathBuf>,

    /// Graceful shutdown timeout in seconds
    #[arg(long = "timeout", default_value_t = 5)]
    pub timeout: u64,

    /// Send SIGKILL after graceful shutdown timeout
    #[arg(long = "force")]
    pub force: bool,
}

#[async_trait]
impl Command for StopCommand {
    async fn execute(&self) -> Result<()> {
        let hyper_dir = resolve_hyper_dir(self.config.as_deref(), self.hyper_dir.as_deref())?;
        let store = RuntimeStateStore::new(hyper_dir);
        let entry = store.resolve_wid_prefix(&self.wid).await?;

        if entry.status != RuntimeStatus::Running {
            store
                .mark_stopped_by_wid(&entry.record.wid, Utc::now())
                .await?;
            println!("Runtime already stopped: {}", entry.wid_short());
            return Ok(());
        }

        if !terminate_process(entry.record.pid)? {
            store
                .mark_stopped_by_wid(&entry.record.wid, Utc::now())
                .await?;
            println!("Runtime already stopped: {}", entry.wid_short());
            return Ok(());
        }
        if wait_for_exit(entry.record.pid, Duration::from_secs(self.timeout)).await {
            store
                .mark_stopped_by_wid(&entry.record.wid, Utc::now())
                .await?;
            println!("Stopped runtime: {}", entry.wid_short());
            return Ok(());
        }

        if !self.force {
            return Err(ActrCliError::command_error(format!(
                "Timed out after {}s while stopping {}. Retry with --force.",
                self.timeout,
                entry.wid_short()
            )));
        }

        if !kill_process(entry.record.pid)? {
            store
                .mark_stopped_by_wid(&entry.record.wid, Utc::now())
                .await?;
            println!("Runtime already stopped: {}", entry.wid_short());
            return Ok(());
        }
        if wait_for_exit(entry.record.pid, Duration::from_secs(1)).await {
            store
                .mark_stopped_by_wid(&entry.record.wid, Utc::now())
                .await?;
            println!("Force stopped runtime: {}", entry.wid_short());
            return Ok(());
        }

        Err(ActrCliError::command_error(format!(
            "Process {} did not exit after SIGKILL",
            entry.record.pid
        )))
    }
}

#[cfg(unix)]
fn terminate_process(pid: u32) -> Result<bool> {
    use nix::errno::Errno;
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;

    let pid = i32::try_from(pid)
        .map_err(|_| ActrCliError::command_error(format!("Invalid PID {}", pid)))?;
    match kill(Pid::from_raw(pid), Signal::SIGTERM) {
        Ok(()) => Ok(true),
        Err(Errno::ESRCH) => Ok(false),
        Err(error) => Err(ActrCliError::command_error(format!(
            "Failed to send SIGTERM to {}: {}",
            pid, error
        ))),
    }
}

#[cfg(unix)]
fn kill_process(pid: u32) -> Result<bool> {
    use nix::errno::Errno;
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;

    let pid = i32::try_from(pid)
        .map_err(|_| ActrCliError::command_error(format!("Invalid PID {}", pid)))?;
    match kill(Pid::from_raw(pid), Signal::SIGKILL) {
        Ok(()) => Ok(true),
        Err(Errno::ESRCH) => Ok(false),
        Err(error) => Err(ActrCliError::command_error(format!(
            "Failed to send SIGKILL to {}: {}",
            pid, error
        ))),
    }
}

#[cfg(not(unix))]
fn terminate_process(_pid: u32) -> Result<bool> {
    Err(ActrCliError::command_error(
        "stop is only supported on Unix systems".to_string(),
    ))
}

#[cfg(not(unix))]
fn kill_process(_pid: u32) -> Result<bool> {
    Err(ActrCliError::command_error(
        "stop is only supported on Unix systems".to_string(),
    ))
}

async fn wait_for_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !is_process_alive(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    !is_process_alive(pid)
}

#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };

    match kill(Pid::from_raw(pid), None) {
        Ok(()) => true,
        Err(Errno::EPERM) => true,
        Err(Errno::ESRCH) => false,
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn is_process_alive(_pid: u32) -> bool {
    false
}
