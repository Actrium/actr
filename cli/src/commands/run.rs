//! Run command implementation - Execute .actr packages

use crate::commands::Command;
use crate::commands::runtime_state::{
    RuntimeRecord, RuntimeStateStore, absolutize_from_cwd, log_path_for_pid,
};
use crate::error::{ActrCliError, Result};
use async_trait::async_trait;
use chrono::Utc;
use clap::Args;
use std::path::{Path, PathBuf};
use tracing::info;

#[derive(Args)]
pub struct RunCommand {
    /// Runtime configuration file (defaults to ./actr.toml if not specified)
    #[arg(short = 'c', long = "config", value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Run in detached mode (background)
    #[arg(short = 'd', long = "detach")]
    pub detach: bool,
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

        // 11. Choose run mode based on detach flag
        let config_path = absolutize_from_cwd(&config_path)?;

        if self.detach {
            self.run_detached(actr_ref, &config, &config_path).await?;
        } else {
            self.run_foreground(actr_ref).await?;
        }

        Ok(())
    }

    async fn run_foreground(&self, actr_ref: actr_hyper::ActrRef) -> Result<()> {
        info!("📡 Running in foreground mode (Ctrl+C to stop)");

        // Block and wait for Ctrl+C
        actr_ref
            .wait_for_ctrl_c_and_shutdown()
            .await
            .map_err(|e| ActrCliError::command_error(format!("Shutdown error: {}", e)))?;

        info!("👋 Shutdown complete");
        Ok(())
    }

    async fn run_detached(
        &self,
        actr_ref: actr_hyper::ActrRef,
        config: &actr_config::RuntimeConfig,
        config_path: &Path,
    ) -> Result<()> {
        #[cfg(unix)]
        {
            self.daemonize_unix(actr_ref, config, config_path).await
        }

        #[cfg(not(unix))]
        {
            Err(ActrCliError::command_error(
                "Detached mode is only supported on Unix systems".to_string(),
            ))
        }
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
    async fn daemonize_unix(
        &self,
        actr_ref: actr_hyper::ActrRef,
        config: &actr_config::RuntimeConfig,
        config_path: &Path,
    ) -> Result<()> {
        use nix::unistd::{ForkResult, fork, setsid};
        use std::fs::OpenOptions;
        use std::os::unix::io::AsRawFd;

        info!("🚀 Starting in detached mode...");

        let runtime_store = RuntimeStateStore::new(config.hyper_data_dir.clone());
        runtime_store.ensure_layout().await?;

        // Fork process
        match unsafe { fork() } {
            Ok(ForkResult::Parent { child }) => {
                // Parent process: print info and exit
                let actr_id = actr_protocol::ActrIdExt::to_string_repr(actr_ref.actor_id());
                let child_pid = child.as_raw() as u32;
                let log_file = log_path_for_pid(runtime_store.hyper_dir(), child_pid);
                println!("✅ ActrNode started in background");
                println!("   ActrId: {}", actr_id);
                println!("   PID: {}", child_pid);
                println!("   Logs: {}", log_file.display());
                println!("\nTo view logs: tail -f {}", log_file.display());
                println!("To stop: actr stop --actr-id {}", actr_id);

                // Parent process exits immediately
                std::process::exit(0);
            }
            Ok(ForkResult::Child) => {
                // Child process: continue running

                // 1. Create new session (detach from terminal)
                setsid().map_err(|e| {
                    ActrCliError::command_error(format!("Failed to create new session: {}", e))
                })?;

                let pid = std::process::id();
                let log_file = log_path_for_pid(runtime_store.hyper_dir(), pid);

                // 2. Redirect stdout/stderr to log file
                let log = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log_file)?;

                let log_fd = log.as_raw_fd();
                nix::unistd::dup2(log_fd, std::io::stdout().as_raw_fd())
                    .map_err(|e| ActrCliError::command_error(format!("dup2 failed: {}", e)))?;
                nix::unistd::dup2(log_fd, std::io::stderr().as_raw_fd())
                    .map_err(|e| ActrCliError::command_error(format!("dup2 failed: {}", e)))?;

                let actr_id_str = actr_protocol::ActrIdExt::to_string_repr(actr_ref.actor_id());
                let record = RuntimeRecord::new(
                    actr_id_str.clone(),
                    pid,
                    config_path.to_path_buf(),
                    log_file.clone(),
                    Utc::now(),
                );
                runtime_store.write_record(&record).await?;

                info!("🚀 Running as daemon, PID: {}", pid);
                info!("📝 Log file: {}", log_file.display());

                // 3. Run until signal received
                actr_ref
                    .wait_for_ctrl_c_and_shutdown()
                    .await
                    .map_err(|e| ActrCliError::command_error(format!("Shutdown error: {}", e)))?;

                runtime_store.mark_stopped(pid, Utc::now()).await?;

                info!("👋 Daemon shutdown complete");
                Ok(())
            }
            Err(e) => Err(ActrCliError::command_error(format!("Fork failed: {}", e))),
        }
    }
}
