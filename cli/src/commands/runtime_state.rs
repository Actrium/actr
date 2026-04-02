use crate::error::{ActrCliError, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const RUNTIME_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RuntimeRecord {
    pub schema_version: u32,
    pub actr_id: String,
    pub pid: u32,
    pub config_path: PathBuf,
    pub log_path: PathBuf,
    pub started_at: DateTime<Utc>,
    pub stopped_at: Option<DateTime<Utc>>,
}

impl RuntimeRecord {
    pub(crate) fn new(
        actr_id: String,
        pid: u32,
        config_path: PathBuf,
        log_path: PathBuf,
        started_at: DateTime<Utc>,
    ) -> Self {
        Self {
            schema_version: RUNTIME_SCHEMA_VERSION,
            actr_id,
            pid,
            config_path,
            log_path,
            started_at,
            stopped_at: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeStatus {
    Running,
    Exited,
    Stale,
}

impl RuntimeStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Exited => "exited",
            Self::Stale => "stale",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeRecordEntry {
    pub record: RuntimeRecord,
    pub status: RuntimeStatus,
}

impl RuntimeRecordEntry {
    pub(crate) fn started_at_display(&self) -> String {
        self.record
            .started_at
            .to_rfc3339_opts(SecondsFormat::Secs, true)
    }
}

pub(crate) struct RuntimeStateStore {
    hyper_dir: PathBuf,
}

impl RuntimeStateStore {
    pub(crate) fn new(hyper_dir: PathBuf) -> Self {
        Self { hyper_dir }
    }

    pub(crate) fn hyper_dir(&self) -> &Path {
        &self.hyper_dir
    }

    pub(crate) fn run_dir(&self) -> PathBuf {
        self.hyper_dir.join("run")
    }

    pub(crate) async fn ensure_layout(&self) -> Result<()> {
        tokio::fs::create_dir_all(self.run_dir()).await?;
        tokio::fs::create_dir_all(self.hyper_dir.join("logs")).await?;
        Ok(())
    }

    pub(crate) async fn write_record(&self, record: &RuntimeRecord) -> Result<()> {
        self.ensure_layout().await?;
        let content = serde_json::to_vec_pretty(record)?;
        tokio::fs::write(self.record_path_for_pid(record.pid), content).await?;
        Ok(())
    }

    pub(crate) async fn mark_stopped(&self, pid: u32, stopped_at: DateTime<Utc>) -> Result<()> {
        let path = self.record_path_for_pid(pid);
        let Some(mut record) = self.read_record_from_path(&path).await? else {
            return Ok(());
        };
        if record.stopped_at.is_none() {
            record.stopped_at = Some(stopped_at);
            let content = serde_json::to_vec_pretty(&record)?;
            tokio::fs::write(path, content).await?;
        }
        Ok(())
    }

    pub(crate) async fn list_records(&self) -> Result<Vec<RuntimeRecordEntry>> {
        let run_dir = self.run_dir();
        let mut entries = Vec::new();

        if !run_dir.exists() {
            return Ok(entries);
        }

        let mut dir = tokio::fs::read_dir(&run_dir).await?;
        while let Some(item) = dir.next_entry().await? {
            let path = item.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }

            let Some(record) = self.read_record_from_path(&path).await? else {
                continue;
            };
            entries.push(RuntimeRecordEntry {
                status: runtime_status(&record),
                record,
            });
        }

        entries.sort_by(|left, right| right.record.started_at.cmp(&left.record.started_at));
        Ok(entries)
    }

    pub(crate) async fn records_for_actr_id(
        &self,
        actr_id: &str,
    ) -> Result<Vec<RuntimeRecordEntry>> {
        let mut entries = self.list_records().await?;
        entries.retain(|entry| entry.record.actr_id == actr_id);
        Ok(entries)
    }

    pub(crate) fn record_path_for_pid(&self, pid: u32) -> PathBuf {
        self.run_dir().join(format!("{pid}.json"))
    }

    async fn read_record_from_path(&self, path: &Path) -> Result<Option<RuntimeRecord>> {
        if !path.exists() {
            return Ok(None);
        }

        let content = tokio::fs::read(path).await?;
        let record: RuntimeRecord = serde_json::from_slice(&content).map_err(|error| {
            ActrCliError::command_error(format!(
                "Failed to parse runtime record {}: {}",
                path.display(),
                error
            ))
        })?;

        if record.schema_version != RUNTIME_SCHEMA_VERSION {
            return Err(ActrCliError::command_error(format!(
                "Unsupported runtime record schema_version {} in {}",
                record.schema_version,
                path.display()
            )));
        }

        Ok(Some(record))
    }
}

pub(crate) fn resolve_hyper_dir(
    config_path: Option<&Path>,
    hyper_dir: Option<&Path>,
) -> Result<PathBuf> {
    let cwd = std::env::current_dir()?;

    if let Some(hyper_dir) = hyper_dir {
        return Ok(absolutize_path(&cwd, hyper_dir));
    }

    if let Some(config_path) = config_path {
        return hyper_dir_from_config_path(config_path);
    }

    let default_config = cwd.join("actr.toml");
    if default_config.exists() {
        return hyper_dir_from_config_path(&default_config);
    }

    Ok(cwd.join(".hyper"))
}

pub(crate) fn absolutize_from_cwd(path: &Path) -> Result<PathBuf> {
    Ok(absolutize_path(&std::env::current_dir()?, path))
}

pub(crate) fn log_path_for_pid(hyper_dir: &Path, pid: u32) -> PathBuf {
    hyper_dir.join("logs").join(format!("actr-{pid}.log"))
}

pub(crate) fn select_latest_record(
    entries: &[RuntimeRecordEntry],
    prefer_running: bool,
) -> Option<&RuntimeRecordEntry> {
    if prefer_running {
        entries
            .iter()
            .find(|entry| entry.status == RuntimeStatus::Running)
            .or_else(|| entries.first())
    } else {
        entries.first()
    }
}

fn hyper_dir_from_config_path(config_path: &Path) -> Result<PathBuf> {
    let config_path = absolutize_from_cwd(config_path)?;
    if !config_path.exists() {
        return Err(ActrCliError::command_error(format!(
            "Runtime config file not found: {}",
            config_path.display()
        )));
    }

    let raw = actr_config::RuntimeRawConfig::from_file(&config_path).map_err(|error| {
        ActrCliError::command_error(format!(
            "Failed to parse runtime config {}: {}",
            config_path.display(),
            error
        ))
    })?;
    let base_dir = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    Ok(match raw.storage.hyper_data_dir {
        Some(path) => absolutize_path(&base_dir, &path),
        None => base_dir.join(".hyper"),
    })
}

fn absolutize_path(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

fn runtime_status(record: &RuntimeRecord) -> RuntimeStatus {
    if is_process_alive(record.pid) {
        RuntimeStatus::Running
    } else if record.stopped_at.is_some() {
        RuntimeStatus::Exited
    } else {
        RuntimeStatus::Stale
    }
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
