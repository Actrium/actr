use crate::commands::process::is_process_alive;
use crate::error::{ActrCliError, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const RUNTIME_SCHEMA_VERSION: u32 = 2;
const WID_SHORT_CHARS: usize = 12;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeRecord {
    pub schema_version: u32,
    pub wid: String,
    pub actr_id: String,
    pub pid: u32,
    pub config_path: PathBuf,
    pub log_path: PathBuf,
    pub started_at: DateTime<Utc>,
    pub stopped_at: Option<DateTime<Utc>>,
}

impl RuntimeRecord {
    pub fn new(
        wid: String,
        actr_id: String,
        pid: u32,
        config_path: PathBuf,
        log_path: PathBuf,
        started_at: DateTime<Utc>,
    ) -> Self {
        Self {
            schema_version: RUNTIME_SCHEMA_VERSION,
            wid,
            actr_id,
            pid,
            config_path,
            log_path,
            started_at,
            stopped_at: None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawRuntimeRecord {
    schema_version: u32,
    wid: Option<String>,
    actr_id: String,
    pid: u32,
    config_path: PathBuf,
    log_path: PathBuf,
    started_at: DateTime<Utc>,
    stopped_at: Option<DateTime<Utc>>,
}

impl RawRuntimeRecord {
    fn into_runtime_record(self, path: &Path, run_dir: &Path) -> Result<RuntimeRecord> {
        if self.schema_version != RUNTIME_SCHEMA_VERSION {
            return Err(ActrCliError::command_error(format!(
                "Incompatible runtime record schema v{} in {}.\n\
                 Delete all files in {} and re-run `actr run -d`.",
                self.schema_version,
                path.display(),
                run_dir.display()
            )));
        }

        let wid = self.wid.ok_or_else(|| {
            ActrCliError::command_error(format!(
                "Failed to parse runtime record {}: missing field `wid`",
                path.display()
            ))
        })?;

        Ok(RuntimeRecord {
            schema_version: self.schema_version,
            wid,
            actr_id: self.actr_id,
            pid: self.pid,
            config_path: self.config_path,
            log_path: self.log_path,
            started_at: self.started_at,
            stopped_at: self.stopped_at,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStatus {
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
pub struct RuntimeRecordEntry {
    pub record: RuntimeRecord,
    pub status: RuntimeStatus,
}

impl RuntimeRecordEntry {
    pub(crate) fn started_at_display(&self) -> String {
        self.record
            .started_at
            .to_rfc3339_opts(SecondsFormat::Secs, true)
    }

    pub(crate) fn wid_short(&self) -> &str {
        shorten_wid(&self.record.wid)
    }
}

pub struct RuntimeStateStore {
    hyper_dir: PathBuf,
}

impl RuntimeStateStore {
    pub fn new(hyper_dir: PathBuf) -> Self {
        Self { hyper_dir }
    }

    pub(crate) fn hyper_dir(&self) -> &Path {
        &self.hyper_dir
    }

    pub fn run_dir(&self) -> PathBuf {
        self.hyper_dir.join("run")
    }

    pub async fn ensure_layout(&self) -> Result<()> {
        tokio::fs::create_dir_all(self.run_dir()).await?;
        tokio::fs::create_dir_all(self.hyper_dir.join("logs")).await?;
        Ok(())
    }

    pub(crate) fn record_path_for_wid(&self, wid: &str) -> PathBuf {
        self.run_dir().join(format!("{wid}.json"))
    }

    pub async fn write_record(&self, record: &RuntimeRecord) -> Result<()> {
        self.ensure_layout().await?;
        let content = serde_json::to_vec_pretty(record)?;
        tokio::fs::write(self.record_path_for_wid(&record.wid), content).await?;
        Ok(())
    }

    pub async fn read_record_by_wid(&self, wid: &str) -> Result<Option<RuntimeRecord>> {
        self.read_record_from_path(&self.record_path_for_wid(wid))
            .await
    }

    pub(crate) async fn delete_record_by_wid(&self, wid: &str) -> Result<()> {
        let path = self.record_path_for_wid(wid);
        if path.exists() {
            tokio::fs::remove_file(&path).await?;
        }
        Ok(())
    }

    pub async fn mark_stopped_by_wid(&self, wid: &str, stopped_at: DateTime<Utc>) -> Result<()> {
        let path = self.record_path_for_wid(wid);
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

    pub async fn list_records(&self) -> Result<Vec<RuntimeRecordEntry>> {
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

    pub async fn resolve_wid_prefix(&self, prefix: &str) -> Result<RuntimeRecordEntry> {
        if prefix.len() < 8 {
            return Err(ActrCliError::command_error(
                "WID prefix must be at least 8 characters".to_string(),
            ));
        }
        let all = self.list_records().await?;
        let matches: Vec<_> = all
            .into_iter()
            .filter(|e| e.record.wid.starts_with(prefix))
            .collect();
        match matches.len() {
            0 => Err(ActrCliError::command_error(format!(
                "No runtime record found for WID prefix '{prefix}'"
            ))),
            1 => Ok(matches.into_iter().next().unwrap()),
            _ => {
                let candidates = matches
                    .iter()
                    .map(|e| format!("  {} ({})", shorten_wid(&e.record.wid), e.record.actr_id))
                    .collect::<Vec<_>>()
                    .join("\n");
                Err(ActrCliError::command_error(format!(
                    "Ambiguous WID prefix '{prefix}', matches:\n{candidates}"
                )))
            }
        }
    }

    async fn read_record_from_path(&self, path: &Path) -> Result<Option<RuntimeRecord>> {
        if !path.exists() {
            return Ok(None);
        }

        let content = tokio::fs::read(path).await?;
        let raw: RawRuntimeRecord = serde_json::from_slice(&content).map_err(|error| {
            ActrCliError::command_error(format!(
                "Failed to parse runtime record {}: {}",
                path.display(),
                error
            ))
        })?;
        Ok(Some(raw.into_runtime_record(path, &self.run_dir())?))
    }
}

pub(crate) fn resolve_hyper_dir(
    _config_path: Option<&Path>,
    hyper_dir: Option<&Path>,
) -> Result<PathBuf> {
    let cwd = std::env::current_dir()?;

    if let Some(hyper_dir) = hyper_dir {
        return Ok(absolutize_path(&cwd, hyper_dir));
    }

    actr_config::user_config::resolve_hyper_data_dir()
        .map_err(|e| ActrCliError::command_error(format!("Failed to load CLI config: {e}")))
}

pub(crate) fn absolutize_from_cwd(path: &Path) -> Result<PathBuf> {
    Ok(absolutize_path(&std::env::current_dir()?, path))
}

pub(crate) fn log_path_for_wid(hyper_dir: &Path, wid: &str) -> PathBuf {
    hyper_dir.join("logs").join(format!("actr-{wid}.log"))
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

fn shorten_wid(wid: &str) -> &str {
    let end = wid
        .char_indices()
        .nth(WID_SHORT_CHARS)
        .map(|(index, _)| index)
        .unwrap_or(wid.len());
    &wid[..end]
}
