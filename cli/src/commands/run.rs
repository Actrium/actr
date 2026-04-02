//! Run command implementation - Execute .actr packages

use crate::commands::Command;
use crate::commands::runtime_state::{
    RuntimeRecord, RuntimeStateStore, absolutize_from_cwd, log_path_for_wid, resolve_hyper_dir,
};
use crate::error::{ActrCliError, Result};
use async_trait::async_trait;
use chrono::Utc;
use clap::Args;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use tracing::info;

#[derive(Args)]
pub struct RunCommand {
    /// Runtime configuration file (defaults to ./actr.toml if not specified)
    #[arg(short = 'c', long = "config", value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Run in detached mode (background)
    #[arg(short = 'd', long = "detach")]
    pub detach: bool,

    /// Internal flag used when the detached child re-executes this command.
    #[arg(long = "internal-detached-child", hide = true)]
    pub internal_detached_child: bool,

    /// Internal: WID passed from parent to detached child (or from start/restart for reuse).
    #[arg(long = "internal-wid", hide = true)]
    pub internal_wid: Option<String>,
}

#[async_trait]
impl Command for RunCommand {
    async fn execute(&self) -> Result<()> {
        // The run command only supports packaged workloads via runtime config.
        self.execute_package_mode().await
    }
}

impl RunCommand {
    async fn execute_package_mode(&self) -> Result<()> {
        use actr_hyper::{WorkloadPackage, init_observability};

        info!("🚀 Starting packaged workload");

        // Resolve runtime config path: use the provided path or default to ./actr.toml.
        let config_path = self
            .config
            .clone()
            .unwrap_or_else(|| PathBuf::from("actr.toml"));

        // Check if the runtime config file exists.
        if !config_path.exists() {
            return Err(ActrCliError::command_error(format!(
                "Runtime config file not found: {}\n\n\
                 Create a runtime config file or specify one with -c/--config.",
                config_path.display()
            )));
        }

        let config_path = absolutize_from_cwd(&config_path)?;

        if self.detach && !self.internal_detached_child {
            return self.spawn_detached_child(&config_path).await;
        }

        let detached_runtime = if self.internal_detached_child {
            Some(self.prepare_detached_child(&config_path).await?)
        } else {
            None
        };

        // 1. Resolve package path from the runtime config.
        let package_path = self.resolve_package_path(&config_path).await?;
        info!("📦 Loading package: {}", package_path.display());

        // 2. Load package bytes
        let package_bytes = tokio::fs::read(&package_path).await.map_err(|e| {
            ActrCliError::command_error(format!("Failed to read package file: {}", e))
        })?;
        let package = WorkloadPackage::new(package_bytes.clone());

        // 3. Parse package manifest
        let manifest = actr_pack::read_manifest(&package_bytes).map_err(|e| {
            ActrCliError::command_error(format!("Failed to parse package manifest: {}", e))
        })?;
        let package_info = self.build_package_info(&manifest);

        // 4. Load runtime configuration
        let config = actr_config::ConfigParser::from_runtime_file(
            &config_path,
            package_info.clone(),
            vec![],
        )?;

        info!("📡 Signaling server: {}", config.signaling_url.as_str());
        info!("🔐 Trust mode: {}", config.trust_mode);

        // 6. Initialize observability
        let _obs_guard = init_observability(&config.observability).map_err(|e| {
            ActrCliError::command_error(format!("Failed to initialize observability: {}", e))
        })?;

        // 7. Initialize Hyper
        let hyper = self.init_hyper(&config, &package_path).await?;
        info!("✅ Hyper initialized");

        // 8. Attach package
        let mut node = hyper
            .attach_package(&package, config.clone())
            .await
            .map_err(|e| ActrCliError::command_error(format!("Failed to attach package: {}", e)))?;
        info!("✅ Package attached");

        // 9. Bootstrap credential via AIS
        let register_ok = self
            .bootstrap_credential(&hyper, &node, &config, &package_bytes, &manifest)
            .await?;
        node.inject_credential(register_ok);
        info!("✅ AIS registration successful");

        // 10. Start ActrNode
        let actr_ref = node
            .start()
            .await
            .map_err(|e| ActrCliError::command_error(format!("Failed to start ActrNode: {}", e)))?;
        info!("✅ ActrNode started");

        if let Some(runtime) = detached_runtime.as_ref() {
            self.write_runtime_record(runtime, &actr_ref).await?;
            info!("📝 Detached runtime state recorded");
        }

        self.run_foreground(actr_ref, detached_runtime.as_ref())
            .await?;

        Ok(())
    }

    async fn run_foreground(
        &self,
        actr_ref: actr_hyper::ActrRef,
        detached_runtime: Option<&DetachedRuntimeContext>,
    ) -> Result<()> {
        info!("📡 Running in foreground mode (Ctrl+C to stop)");

        // Block and wait for Ctrl+C
        actr_ref
            .wait_for_ctrl_c_and_shutdown()
            .await
            .map_err(|e| ActrCliError::command_error(format!("Shutdown error: {}", e)))?;

        if let Some(runtime) = detached_runtime {
            runtime
                .runtime_store
                .mark_stopped_by_wid(&runtime.wid, Utc::now())
                .await?;
        }

        info!("👋 Shutdown complete");
        Ok(())
    }

    async fn resolve_package_path(&self, config_path: &Path) -> Result<PathBuf> {
        // Load runtime config to get the packaged workload path.
        let config_content = tokio::fs::read_to_string(config_path).await?;
        let raw_config: actr_config::RuntimeRawConfig = toml::from_str(&config_content)
            .map_err(|e| ActrCliError::command_error(format!("Failed to parse config: {}", e)))?;

        if let Some(package_config) = raw_config.package {
            if let Some(path) = package_config.path {
                let resolved_path = if path.is_absolute() {
                    path
                } else {
                    config_path.parent().unwrap_or(Path::new(".")).join(path)
                };
                return Ok(resolved_path);
            }
        }

        Err(ActrCliError::command_error(format!(
            "Package path not specified in runtime config: {}\n\n\
             Add the packaged workload path to your config:\n\
             [package]\n\
             path = \"dist/service.actr\"",
            config_path.display()
        )))
    }

    fn build_package_info(
        &self,
        manifest: &actr_pack::PackageManifest,
    ) -> actr_config::PackageInfo {
        actr_config::PackageInfo {
            name: manifest.name.clone(),
            actr_type: actr_protocol::ActrType {
                manufacturer: manifest.manufacturer.clone(),
                name: manifest.name.clone(),
                version: manifest.version.clone(),
            },
            description: manifest.metadata.description.clone(),
            authors: vec![],
            license: manifest.metadata.license.clone(),
        }
    }

    async fn init_hyper(
        &self,
        config: &actr_config::RuntimeConfig,
        package_path: &Path,
    ) -> Result<actr_hyper::Hyper> {
        use actr_hyper::{Hyper, HyperConfig, TrustMode};
        use actr_platform_native::NativePlatformProvider;

        let trust_mode = match config.trust_mode.as_str() {
            "development" => {
                // Load public key from package directory
                let public_key = self.load_public_key(package_path).await?;
                TrustMode::Development {
                    self_signed_pubkey: public_key,
                }
            }
            "production" => {
                // Use AIS-based MFR certificate cache.
                // TrustMode::Production expects the base endpoint without /ais suffix.
                let base_endpoint = config.ais_endpoint.trim_end_matches("/ais").to_string();
                TrustMode::Production {
                    ais_endpoint: base_endpoint,
                }
            }
            other => {
                return Err(ActrCliError::command_error(format!(
                    "Invalid trust mode in runtime config: {}.\n\n\
                     Set [deployment] trust_mode to \"development\" or \"production\".",
                    other
                )));
            }
        };

        let hyper_config = HyperConfig::new(&config.hyper_data_dir).with_trust_mode(trust_mode);

        let platform_provider = std::sync::Arc::new(NativePlatformProvider::new());

        Hyper::init_with_platform(hyper_config, platform_provider)
            .await
            .map_err(|e| ActrCliError::command_error(format!("Failed to initialize Hyper: {}", e)))
    }

    async fn load_public_key(&self, package_path: &Path) -> Result<Vec<u8>> {
        let package_dir = package_path.parent().unwrap_or(Path::new("."));
        let key_path = package_dir.join("public-key.json");

        if !key_path.exists() {
            return Err(ActrCliError::command_error(format!(
                "Public key not found for development trust mode.\n\n\
                 Expected location: {}\n\n\
                 Update your runtime config or package directory:\n\
                 1. Keep [deployment] trust_mode = \"development\" and place public-key.json next to the .actr package\n\
                 2. Or switch to production mode in config:\n\
                    [deployment]\n\
                    trust_mode = \"production\"\n\
                    [ais_endpoint]\n\
                    url = \"http://localhost:8081/ais\"",
                key_path.display()
            )));
        }

        let key_content = tokio::fs::read_to_string(&key_path).await?;
        let key_json: serde_json::Value = serde_json::from_str(&key_content)?;

        let key_base64 = key_json["public_key"].as_str().ok_or_else(|| {
            ActrCliError::command_error(
                "Invalid public-key.json format: missing 'public_key' field".to_string(),
            )
        })?;

        use base64::Engine;
        let key_bytes = base64::engine::general_purpose::STANDARD
            .decode(key_base64)
            .map_err(|e| {
                ActrCliError::command_error(format!("Invalid base64 in public key: {}", e))
            })?;

        if key_bytes.len() != 32 {
            return Err(ActrCliError::command_error(format!(
                "Invalid public key size: expected 32 bytes, got {}",
                key_bytes.len()
            )));
        }

        Ok(key_bytes)
    }

    async fn bootstrap_credential(
        &self,
        hyper: &actr_hyper::Hyper,
        node: &actr_hyper::ActrNode,
        config: &actr_config::RuntimeConfig,
        package_bytes: &[u8],
        manifest: &actr_pack::PackageManifest,
    ) -> Result<actr_protocol::register_response::RegisterOk> {
        let ais_endpoint = &config.ais_endpoint;
        let realm = &config.realm;

        // Calculate ServiceSpec from package manifest and proto files
        let service_spec = actr_pack::calculate_service_spec_from_package(package_bytes, manifest)
            .map_err(|e| {
                ActrCliError::command_error(format!(
                    "Failed to calculate ServiceSpec from package: {}",
                    e
                ))
            })?;

        // Log ServiceSpec info if present
        if let Some(ref spec) = service_spec {
            info!(
                "📋 ServiceSpec: name={}, fingerprint={}, {} proto files",
                spec.name,
                spec.fingerprint,
                spec.protobufs.len()
            );
        } else {
            info!("📋 No ServiceSpec (package contains no proto files)");
        }

        let acl = config.acl.clone();

        hyper
            .bootstrap_node_credential(node, ais_endpoint, realm.realm_id, service_spec, acl)
            .await
            .map_err(|e| {
                ActrCliError::command_error(format!(
                    "Failed to register with AIS at {}.\n\n\
             Possible causes:\n\
             - AIS server is not running\n\
             - Incorrect [ais_endpoint] url in the runtime config\n\
             - Network connectivity issues\n\n\
             Error: {}",
                    ais_endpoint, e
                ))
            })
    }

    #[cfg(unix)]
    async fn prepare_detached_child(&self, config_path: &Path) -> Result<DetachedRuntimeContext> {
        use nix::unistd::setsid;
        use std::fs::OpenOptions;
        use std::os::unix::io::AsRawFd;

        let wid = self.internal_wid.clone().ok_or_else(|| {
            ActrCliError::command_error("--internal-wid is required for detached child".to_string())
        })?;

        let hyper_dir = resolve_hyper_dir(Some(config_path), None)?;
        let runtime_store = RuntimeStateStore::new(hyper_dir);
        runtime_store.ensure_layout().await?;
        setsid().map_err(|e| {
            ActrCliError::command_error(format!("Failed to create new session: {}", e))
        })?;

        let pid = std::process::id();
        let log_file = log_path_for_wid(runtime_store.hyper_dir(), &wid);
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file)?;

        let log_fd = log.as_raw_fd();
        nix::unistd::dup2(log_fd, std::io::stdout().as_raw_fd())
            .map_err(|e| ActrCliError::command_error(format!("dup2 failed: {}", e)))?;
        nix::unistd::dup2(log_fd, std::io::stderr().as_raw_fd())
            .map_err(|e| ActrCliError::command_error(format!("dup2 failed: {}", e)))?;

        info!("🚀 Detached child process initialized, PID: {}", pid);
        info!("📝 Log file: {}", log_file.display());

        Ok(DetachedRuntimeContext {
            runtime_store,
            config_path: config_path.to_path_buf(),
            log_file,
            pid,
            wid,
        })
    }

    #[cfg(not(unix))]
    async fn prepare_detached_child(&self, _config_path: &Path) -> Result<DetachedRuntimeContext> {
        Err(ActrCliError::command_error(
            "Detached mode is only supported on Unix systems".to_string(),
        ))
    }

    async fn write_runtime_record(
        &self,
        detached_runtime: &DetachedRuntimeContext,
        actr_ref: &actr_hyper::ActrRef,
    ) -> Result<()> {
        let actr_id_str = actr_protocol::ActrIdExt::to_string_repr(actr_ref.actor_id());

        // Upsert: if a record already exists for this wid (start/restart scenario),
        // update pid/started_at and clear stopped_at while preserving wid and actr_id.
        let existing = detached_runtime
            .runtime_store
            .read_record_by_wid(&detached_runtime.wid)
            .await?;

        let record = if let Some(mut r) = existing {
            r.pid = detached_runtime.pid;
            r.started_at = Utc::now();
            r.stopped_at = None;
            r.config_path = detached_runtime.config_path.clone();
            r.log_path = detached_runtime.log_file.clone();
            r
        } else {
            RuntimeRecord::new(
                detached_runtime.wid.clone(),
                actr_id_str,
                detached_runtime.pid,
                detached_runtime.config_path.clone(),
                detached_runtime.log_file.clone(),
                Utc::now(),
            )
        };

        detached_runtime.runtime_store.write_record(&record).await
    }

    async fn spawn_detached_child(&self, config_path: &Path) -> Result<()> {
        #[cfg(unix)]
        {
            use uuid::Uuid;

            let hyper_dir = resolve_hyper_dir(Some(config_path), None)?;
            let runtime_store = RuntimeStateStore::new(hyper_dir);
            runtime_store.ensure_layout().await?;

            // Generate a new wid in the parent; pass it to the child via --internal-wid.
            // For start/restart, the caller sets self.internal_wid to the existing wid.
            let wid = self
                .internal_wid
                .clone()
                .unwrap_or_else(|| Uuid::new_v4().to_string());

            let current_exe = std::env::current_exe().map_err(|e| {
                ActrCliError::command_error(format!(
                    "Failed to resolve current executable for detached mode: {}",
                    e
                ))
            })?;

            let mut child = StdCommand::new(current_exe);
            child
                .arg("run")
                .arg("--config")
                .arg(config_path)
                .arg("--internal-detached-child")
                .arg("--internal-wid")
                .arg(&wid)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());

            let child = child.spawn().map_err(|e| {
                ActrCliError::command_error(format!(
                    "Failed to launch detached child process: {}",
                    e
                ))
            })?;

            let pid = child.id();
            // The child runs as a daemon; we intentionally do not wait on it.
            std::mem::forget(child);

            println!("Detached runtime started");
            println!("   WID:  {}", &wid[..12]);
            println!("   PID:  {}", pid);
            println!();
            println!("Follow logs: actr logs {} -f", &wid[..12]);
            return Ok(());
        }

        #[cfg(not(unix))]
        {
            let _ = config_path;
            Err(ActrCliError::command_error(
                "Detached mode is only supported on Unix systems".to_string(),
            ))
        }
    }
}

struct DetachedRuntimeContext {
    runtime_store: RuntimeStateStore,
    config_path: PathBuf,
    log_file: PathBuf,
    pid: u32,
    wid: String,
}
