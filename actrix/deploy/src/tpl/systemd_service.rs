//! Systemd service template processing

use anyhow::Result;
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::InstallConfig;

// Keep template and rendering logic colocated for this minimal deploy helper.
const SYSTEMD_SERVICE_TEMPLATE: &str = r#"# actrix systemd service file template
# This file is a template, actual deployment will generate real service file based on configured paths

[Unit]
Description=Actrix Auxiliary Servers
Documentation=https://github.com/Actrium/actrix
After=network.target

[Service]
Type=simple
User={{SERVICE_USER}}
Group={{SERVICE_GROUP}}
WorkingDirectory={{INSTALL_DIR}}
ExecStart={{INSTALL_DIR}}/bin/actrix --config {{CONFIG_PATH}}
ExecReload=/bin/kill -HUP $MAINPID
Restart=always
RestartSec=5
StandardOutput=journal
StandardError=journal
SyslogIdentifier=actrix
{{CAPABILITY_BLOCK}}

# Security settings
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths={{READ_WRITE_PATHS}}

# Resource limits
LimitNOFILE=65536
LimitNPROC=4096

[Install]
WantedBy=multi-user.target
"#;

#[derive(Debug, Default, Deserialize)]
struct RuntimeConfig {
    pid: Option<String>,
    sqlite_path: Option<String>,
    #[serde(default)]
    bind: RuntimeBindConfig,
}

#[derive(Debug, Default, Deserialize)]
struct RuntimeBindConfig {
    #[serde(default)]
    http: Option<RuntimeListenerConfig>,
    #[serde(default)]
    https: Option<RuntimeListenerConfig>,
    #[serde(default)]
    ice: Option<RuntimeListenerConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct RuntimeListenerConfig {
    port: Option<u16>,
}

/// Systemd service template processor
pub struct SystemdServiceTemplate {
    install_config: InstallConfig,
    config_path: std::path::PathBuf,
    service_name: String,
}

impl SystemdServiceTemplate {
    pub fn new(
        install_config: InstallConfig,
        config_path: std::path::PathBuf,
        service_name: String,
    ) -> Self {
        Self {
            install_config,
            config_path,
            service_name,
        }
    }

    /// Generate systemd service file.
    ///
    /// Refuses to overwrite an existing unit unless `force_overwrite` is set,
    /// protecting `User=`/`ProtectSystem=`/`ReadWritePaths=` hardening from
    /// being clobbered by a re-deploy.
    pub fn generate_service_file(
        &self,
        service_user: &str,
        service_group: &str,
        force_overwrite: bool,
    ) -> Result<()> {
        let service_name = &self.service_name;
        let service_file = format!("/etc/systemd/system/{}.service", service_name);

        println!("📄 Creating systemd service: {}", service_name);

        // Overwrite guard: preserve existing hardening unless explicitly forced.
        if Path::new(&service_file).exists() {
            if !force_overwrite {
                anyhow::bail!(
                    "systemd unit already exists: {service_file}\n\
                     Refusing to overwrite (this would discard User=/ProtectSystem=/ReadWritePaths= hardening).\n\
                     Re-run with --force-overwrite-unit to replace it."
                );
            }
            println!("⚠️  Overwriting existing unit: {service_file}");
            println!(
                "⚠️  Existing hardening (User=/ProtectSystem=/ReadWritePaths=) will be replaced."
            );
        }

        // Create service content
        let service_content = self.create_service_content(service_user, service_group)?;

        // Write service file using sudo
        self.write_service_file(&service_content, &service_file)?;

        // Reload systemd daemon
        self.reload_systemd()?;

        // Enable service
        self.enable_service(service_name)?;

        // Start service
        self.start_service(service_name)?;

        // Show service status
        self.show_service_status(service_name)?;

        println!(
            "✅ Systemd service '{}' deployed successfully",
            service_name
        );
        println!("   • Service file: {}", service_file);
        println!("   • Status: systemctl status {}", service_name);
        println!("   • Logs: journalctl -u {} -f", service_name);

        Ok(())
    }

    fn create_service_content(&self, service_user: &str, service_group: &str) -> Result<String> {
        let install_dir_str = self
            .install_config
            .install_dir
            .to_string_lossy()
            .to_string();
        let config_path_str = self.config_path.to_string_lossy().to_string();
        let read_write_paths = self.collect_read_write_paths().join(" ");
        let capability_block = if self.requires_low_port_capability() {
            "# Allow binding privileged ports (<1024) while running as non-root\nAmbientCapabilities=CAP_NET_BIND_SERVICE\nCapabilityBoundingSet=CAP_NET_BIND_SERVICE"
        } else {
            ""
        };

        println!("ℹ️  ReadWritePaths: {}", read_write_paths);

        let mut placeholders = HashMap::new();
        placeholders.insert("SERVICE_USER".to_string(), service_user.to_string());
        placeholders.insert("SERVICE_GROUP".to_string(), service_group.to_string());
        placeholders.insert("INSTALL_DIR".to_string(), install_dir_str);
        placeholders.insert("CONFIG_PATH".to_string(), config_path_str);
        placeholders.insert("READ_WRITE_PATHS".to_string(), read_write_paths);
        placeholders.insert("CAPABILITY_BLOCK".to_string(), capability_block.to_string());

        let mut result = SYSTEMD_SERVICE_TEMPLATE.to_string();
        for (key, value) in placeholders {
            let placeholder = format!("{{{{{}}}}}", key);
            result = result.replace(&placeholder, &value);
        }

        Ok(result)
    }

    fn write_service_file(&self, content: &str, service_file: &str) -> Result<()> {
        let mut output = Command::new("sudo")
            .arg("tee")
            .arg(service_file)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        if let Some(ref mut stdin) = output.stdin {
            use std::io::Write;
            stdin.write_all(content.as_bytes())?;
        }

        let result = output.wait_with_output()?;
        if !result.status.success() {
            let error = String::from_utf8_lossy(&result.stderr);
            anyhow::bail!("Failed to write service file: {}", error);
        }

        println!("✅ Service file created: {}", service_file);
        Ok(())
    }

    fn reload_systemd(&self) -> Result<()> {
        println!("🔄 Reloading systemd daemon...");
        let output = Command::new("sudo")
            .args(["systemctl", "daemon-reload"])
            .output()?;

        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to reload systemd: {}", error);
        }

        println!("✅ Systemd daemon reloaded");
        Ok(())
    }

    fn enable_service(&self, service_name: &str) -> Result<()> {
        println!("⚡ Enabling service for auto-start...");
        let output = Command::new("sudo")
            .args(["systemctl", "enable", service_name])
            .output()?;

        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to enable service: {}", error);
        }

        println!("✅ Service enabled for auto-start");
        Ok(())
    }

    fn start_service(&self, service_name: &str) -> Result<()> {
        println!("🚀 Starting service...");
        let output = Command::new("sudo")
            .args(["systemctl", "start", service_name])
            .output()?;

        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to start service: {}", error);
        }

        // Check if service is actually running
        let status_output = Command::new("sudo")
            .args(["systemctl", "is-active", service_name])
            .output()?;

        let status_str = String::from_utf8_lossy(&status_output.stdout);
        let status = status_str.trim();
        if status == "active" {
            println!("✅ Service started successfully");
        } else if !status.is_empty() {
            println!("⚠️  Service status: {}", status);
        } else {
            let error = String::from_utf8_lossy(&status_output.stderr);
            println!("⚠️  Unable to read service status after start: {}", error);
        }

        Ok(())
    }

    fn show_service_status(&self, service_name: &str) -> Result<()> {
        println!();
        println!("📊 Service Status");
        println!("════════════════");

        let output = Command::new("sudo")
            .args([
                "systemctl",
                "status",
                service_name,
                "--no-pager",
                "--lines=10",
            ])
            .output()?;

        if output.status.success() {
            let status_output = String::from_utf8_lossy(&output.stdout);
            println!("{}", status_output);
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            println!("⚠️  Failed to get service status: {}", error);
        }

        Ok(())
    }

    fn resolve_runtime_path(&self, raw_path: &str) -> PathBuf {
        let path = PathBuf::from(raw_path);
        if path.is_absolute() {
            path
        } else {
            self.install_config.install_dir.join(path)
        }
    }

    fn collect_read_write_paths(&self) -> Vec<String> {
        let mut paths = BTreeSet::new();
        paths.insert(self.install_config.logs_dir().to_string_lossy().to_string());
        paths.insert(self.install_config.db_dir().to_string_lossy().to_string());
        paths.insert(
            self.install_config
                .shared_dir()
                .to_string_lossy()
                .to_string(),
        );

        match std::fs::read_to_string(&self.config_path) {
            Ok(config_text) => match toml::from_str::<RuntimeConfig>(&config_text) {
                Ok(runtime_cfg) => {
                    if let Some(sqlite_path) = runtime_cfg.sqlite_path
                        && !sqlite_path.trim().is_empty()
                    {
                        let resolved = self.resolve_runtime_path(sqlite_path.trim());
                        paths.insert(resolved.to_string_lossy().to_string());
                    }

                    if let Some(pid_path) = runtime_cfg.pid
                        && !pid_path.trim().is_empty()
                    {
                        let resolved = self.resolve_runtime_path(pid_path.trim());
                        if let Some(parent) = resolved.parent() {
                            paths.insert(parent.to_string_lossy().to_string());
                        }
                    }
                }
                Err(err) => {
                    println!(
                        "⚠️  Failed to parse config TOML for runtime paths (using defaults): {}",
                        err
                    );
                }
            },
            Err(err) => {
                println!(
                    "⚠️  Failed to read config file for runtime paths (using defaults): {}",
                    err
                );
            }
        }

        paths.into_iter().collect()
    }

    fn requires_low_port_capability(&self) -> bool {
        const DEFAULT_HTTP_PORT: u16 = 8080;
        const DEFAULT_HTTPS_PORT: u16 = 8443;
        const DEFAULT_ICE_PORT: u16 = 3478;

        let config_text = match std::fs::read_to_string(&self.config_path) {
            Ok(text) => text,
            Err(_) => return false,
        };
        let runtime_cfg = match toml::from_str::<RuntimeConfig>(&config_text) {
            Ok(cfg) => cfg,
            Err(_) => return false,
        };

        [
            runtime_cfg
                .bind
                .http
                .map(|cfg| cfg.port.unwrap_or(DEFAULT_HTTP_PORT)),
            runtime_cfg
                .bind
                .https
                .map(|cfg| cfg.port.unwrap_or(DEFAULT_HTTPS_PORT)),
            runtime_cfg
                .bind
                .ice
                .map(|cfg| cfg.port.unwrap_or(DEFAULT_ICE_PORT)),
        ]
        .into_iter()
        .flatten()
        .any(|port| port < 1024)
    }
}
