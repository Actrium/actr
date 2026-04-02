use crate::commands::Command;
use crate::commands::runtime_state::{RuntimeStateStore, absolutize_from_cwd, resolve_hyper_dir};
use crate::error::{ActrCliError, Result};
use async_trait::async_trait;
use clap::Args;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

#[derive(Args, Debug)]
pub struct LogsCommand {
    /// WID (or unique prefix, min 8 chars) of the runtime
    #[arg(value_name = "WID")]
    pub wid: String,

    /// Runtime configuration file
    #[arg(short = 'c', long = "config", value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Hyper data directory
    #[arg(long = "hyper-dir", value_name = "DIR")]
    pub hyper_dir: Option<PathBuf>,

    /// Follow appended log output
    #[arg(short = 'f', long = "follow")]
    pub follow: bool,
}

#[async_trait]
impl Command for LogsCommand {
    async fn execute(&self) -> Result<()> {
        let hyper_dir = resolve_hyper_dir(self.config.as_deref(), self.hyper_dir.as_deref())?;
        let store = RuntimeStateStore::new(hyper_dir);
        let entry = store.resolve_wid_prefix(&self.wid).await?;

        let log_path = absolutize_log_path(&entry.record.log_path)?;
        if !log_path.exists() {
            return Err(ActrCliError::command_error(format!(
                "Log file not found: {}",
                log_path.display()
            )));
        }

        stream_log_file(&log_path, self.follow).await
    }
}

fn absolutize_log_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        absolutize_from_cwd(path)
    }
}

async fn stream_log_file(path: &Path, follow: bool) -> Result<()> {
    let mut file = tokio::fs::File::open(path).await.map_err(|error| {
        ActrCliError::command_error(format!(
            "Failed to open log file {}: {}",
            path.display(),
            error
        ))
    })?;
    let mut offset = 0u64;
    let mut stdout = std::io::stdout();

    loop {
        let metadata = file.metadata().await.map_err(|error| {
            ActrCliError::command_error(format!(
                "Failed to stat log file {}: {}",
                path.display(),
                error
            ))
        })?;
        if metadata.len() < offset {
            offset = 0;
        }

        file.seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(|error| {
                ActrCliError::command_error(format!(
                    "Failed to seek log file {}: {}",
                    path.display(),
                    error
                ))
            })?;

        let mut buf = [0u8; 8192];
        let read = file.read(&mut buf).await.map_err(|error| {
            ActrCliError::command_error(format!(
                "Failed to read log file {}: {}",
                path.display(),
                error
            ))
        })?;

        if read == 0 {
            if !follow {
                return Ok(());
            }

            tokio::time::sleep(Duration::from_millis(250)).await;
            continue;
        }

        offset += read as u64;
        stdout.write_all(&buf[..read])?;
        stdout.flush()?;
    }
}
