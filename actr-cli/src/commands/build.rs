//! Build command implementation

use crate::commands::Command;
use crate::error::{ActrCliError, Result};
use crate::utils::{
    check_required_tools, execute_command_streaming, get_target_dir,
    is_actr_project, warn_if_not_actr_project,
};
use actr_config::{ActrConfig, ProtoDependency};
use async_trait::async_trait;
use clap::Args;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use tracing::{debug, info};

#[derive(Args)]
pub struct BuildCommand {
    /// Build in release mode
    #[arg(long)]
    pub release: bool,

    /// Skip proto dependency resolution
    #[arg(long)]
    pub skip_proto_deps: bool,

    /// Clean build (remove target directory first)
    #[arg(long)]
    pub clean: bool,

    /// Additional arguments to pass to cargo
    #[arg(last = true)]
    pub cargo_args: Vec<String>,
}

#[async_trait]
impl Command for BuildCommand {
    async fn execute(&self) -> Result<()> {
        info!("🔨 Building Actor-RTC project");

        // Check that we're in an Actor-RTC project
        warn_if_not_actr_project();

        // Check required tools
        check_required_tools()?;

        let project_root = std::env::current_dir()?;

        // Load configuration
        let config = if is_actr_project() {
            Some(ActrConfig::from_file("actr.toml")?)
        } else {
            None
        };

        // Clean if requested
        if self.clean {
            self.clean_build(&project_root).await?;
        }

        // Resolve proto dependencies if we have a config
        if let Some(ref config) = config {
            if !self.skip_proto_deps {
                self.resolve_proto_dependencies(config, &project_root).await?;
            }
        }

        // Generate code from proto files
        self.generate_proto_code(&project_root).await?;

        // Generate main.rs for auto-runner mode if needed
        if let Some(ref config) = config {
            if config.is_auto_runner_mode() {
                self.generate_main_rs(config, &project_root)?;
            }
        }

        // Build the project
        self.build_project(&project_root).await?;

        info!("✅ Build completed successfully");

        Ok(())
    }
}

impl BuildCommand {
    async fn clean_build(&self, project_root: &Path) -> Result<()> {
        info!("🧹 Cleaning build artifacts");

        let target_dir = get_target_dir(project_root);
        if target_dir.exists() {
            std::fs::remove_dir_all(&target_dir)?;
            info!("Removed target directory: {}", target_dir.display());
        }

        Ok(())
    }

    async fn resolve_proto_dependencies(
        &self,
        config: &ActrConfig,
        project_root: &Path,
    ) -> Result<()> {
        if config.dependencies.protos.dependencies.is_empty() {
            debug!("No proto dependencies to resolve");
            return Ok(());
        }

        info!("📦 Resolving proto dependencies");

        // Create a temporary directory for downloading dependencies
        let temp_dir = TempDir::new()?;
        let mut resolved_protos = HashMap::new();

        for (name, dependency) in &config.dependencies.protos.dependencies {
            info!("Resolving dependency: {}", name);
            let proto_path = self.resolve_single_dependency(name, dependency, temp_dir.path()).await?;
            resolved_protos.insert(name.clone(), proto_path);
        }

        // Copy resolved protos to project's protos directory
        let protos_dir = project_root.join("protos");
        std::fs::create_dir_all(&protos_dir)?;

        for (name, source_path) in resolved_protos {
            let dest_path = protos_dir.join(format!("{}.proto", name));
            std::fs::copy(&source_path, &dest_path)?;
            debug!("Copied {} to {}", source_path.display(), dest_path.display());
        }

        info!("✅ Proto dependencies resolved");
        Ok(())
    }

    async fn resolve_single_dependency(
        &self,
        name: &str,
        dependency: &ProtoDependency,
        temp_dir: &Path,
    ) -> Result<PathBuf> {
        match dependency {
            ProtoDependency::Git { git, path, tag, branch, rev } => {
                self.resolve_git_dependency(name, git, path, tag, branch, rev, temp_dir).await
            }
            ProtoDependency::Http { url } => {
                self.resolve_http_dependency(name, url, temp_dir).await
            }
            ProtoDependency::Local { path } => {
                Ok(PathBuf::from(path))
            }
        }
    }

    async fn resolve_git_dependency(
        &self,
        name: &str,
        git_url: &str,
        proto_path: &str,
        tag: &Option<String>,
        branch: &Option<String>,
        rev: &Option<String>,
        temp_dir: &Path,
    ) -> Result<PathBuf> {
        debug!("Resolving git dependency: {} from {}", name, git_url);

        let repo_dir = temp_dir.join(format!("git_{}", name));
        
        // Clone the repository
        let repo = git2::Repository::clone(git_url, &repo_dir)?;

        // Checkout the specified version
        if let Some(tag) = tag {
            self.checkout_git_ref(&repo, &format!("refs/tags/{}", tag))?;
        } else if let Some(branch) = branch {
            self.checkout_git_ref(&repo, &format!("refs/remotes/origin/{}", branch))?;
        } else if let Some(rev) = rev {
            self.checkout_git_commit(&repo, rev)?;
        }

        let proto_file_path = repo_dir.join(proto_path);
        
        if !proto_file_path.exists() {
            return Err(ActrCliError::ProtoDependency(format!(
                "Proto file '{}' not found in git repository '{}'",
                proto_path, git_url
            )));
        }

        Ok(proto_file_path)
    }

    fn checkout_git_ref(&self, repo: &git2::Repository, ref_name: &str) -> Result<()> {
        let obj = repo.revparse_single(ref_name)?;
        repo.checkout_tree(&obj, None)?;
        Ok(())
    }

    fn checkout_git_commit(&self, repo: &git2::Repository, commit_hash: &str) -> Result<()> {
        let commit = repo.find_commit(git2::Oid::from_str(commit_hash)?)?;
        let tree = commit.tree()?;
        repo.checkout_tree(tree.as_object(), None)?;
        Ok(())
    }

    async fn resolve_http_dependency(
        &self,
        name: &str,
        url: &str,
        temp_dir: &Path,
    ) -> Result<PathBuf> {
        debug!("Resolving HTTP dependency: {} from {}", name, url);

        let client = reqwest::Client::new();
        let response = client.get(url).send().await?;

        if !response.status().is_success() {
            return Err(ActrCliError::ProtoDependency(format!(
                "Failed to download proto from '{}': HTTP {}",
                url, response.status()
            )));
        }

        let content = response.text().await?;
        let file_path = temp_dir.join(format!("{}.proto", name));
        
        std::fs::write(&file_path, content)?;
        Ok(file_path)
    }

    fn generate_main_rs(&self, config: &ActrConfig, project_root: &Path) -> Result<()> {
        if Path::new("src/main.rs").exists() {
            debug!("src/main.rs already exists, skipping generation");
            return Ok(());
        }

        info!("🎯 Generating main.rs for auto-runner mode");

        let main_rs_content = self.create_main_rs_template(config)?;
        let src_dir = project_root.join("src");
        std::fs::create_dir_all(&src_dir)?;
        
        let main_rs_path = src_dir.join("main.rs");
        std::fs::write(main_rs_path, main_rs_content)?;

        info!("Generated src/main.rs for auto-runner mode");
        Ok(())
    }

    fn create_main_rs_template(&self, config: &ActrConfig) -> Result<String> {
        // This is a simple template for auto-runner mode
        // In a real implementation, this would be more sophisticated
        let template = format!(
            r#"//! Auto-generated main.rs for Actor-RTC project: {}
//! 
//! This file is automatically generated by actr-cli for projects using
//! the auto-runner mode. Do not edit this file directly - it will be
//! overwritten on the next build.

use actor_rtc_framework::prelude::*;
use actor_rtc_framework::signaling::WebSocketSignaling;
use std::env;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {{
    // Initialize logging
    tracing_subscriber::fmt::init();

    info!("🚀 Starting {} service");

    // Get configuration from environment
    let actor_id_str = env::var("ACTOR_ID").unwrap_or_else(|_| "1001".to_string());
    let actor_id: u64 = actor_id_str.parse().unwrap_or(1001);
    
    let signaling_url = env::var("SIGNALING_URL")
        .unwrap_or_else(|_| "ws://localhost:8081".to_string());

    // Create actor ID
    let actor_id = ActorId::new(
        actor_id,
        ActorTypeCode::Service,
        "{}".to_string()
    );

    // Create and start the actor system
    let signaling = WebSocketSignaling::new(signaling_url)?;
    let actor_system = ActorSystem::new(actor_id)
        .with_signaling(Box::new(signaling));

    // TODO: Add actor attachment based on generated code
    // This would be filled in based on the proto definitions

    info!("✅ {} service started successfully");
    
    // Keep the service running
    tokio::signal::ctrl_c().await?;
    info!("🛑 Shutting down {} service");

    Ok(())
}}
"#,
            config.package.name,
            config.package.name,
            config.package.name,
            config.package.name,
            config.package.name
        );

        Ok(template)
    }

    async fn generate_proto_code(&self, project_root: &Path) -> Result<()> {
        let proto_dir = project_root.join("proto");
        if !proto_dir.exists() {
            debug!("No proto directory found, skipping proto generation");
            return Ok(());
        }

        info!("📋 Generating code from proto files");

        // Find all .proto files
        let proto_files = std::fs::read_dir(&proto_dir)?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if path.extension()? == "proto" {
                    Some(path)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        if proto_files.is_empty() {
            debug!("No .proto files found in proto directory");
            return Ok(());
        }

        // Check for protoc-gen-actorframework plugin
        let plugin_path = project_root.join("target/debug/protoc-gen-actorframework");
        let has_plugin = plugin_path.exists();

        if !has_plugin {
            info!("🔧 Building protoc-gen-actorframework plugin first...");
            execute_command_streaming("cargo", &["build", "--bin", "protoc-gen-actorframework"], Some(project_root)).await?;
        }

        let shared_protocols_dir = project_root.join("shared-protocols/src");
        std::fs::create_dir_all(&shared_protocols_dir)?;

        // Generate code for each proto file
        for proto_file in proto_files {
            let proto_name = proto_file.file_stem().unwrap().to_string_lossy();
            info!("📋 Processing {}.proto", proto_name);

            let protoc_args = vec![
                "--proto_path=proto".to_string(),
                format!("--plugin=protoc-gen-actorframework={}", plugin_path.display()),
                "--actorframework_out=shared-protocols/src".to_string(),
                proto_file.file_name().unwrap().to_string_lossy().to_string(),
            ];

            // Run protoc with our plugin
            execute_command_streaming("protoc", &protoc_args.iter().map(|s| s.as_str()).collect::<Vec<_>>(), Some(project_root)).await?;
        }

        info!("✅ Proto code generation completed");
        Ok(())
    }

    async fn build_project(&self, project_root: &Path) -> Result<()> {
        info!("🔧 Building Rust project");

        let mut args = vec!["build"];
        
        if self.release {
            args.push("--release");
        }

        // Add any additional cargo arguments
        for arg in &self.cargo_args {
            args.push(arg);
        }

        execute_command_streaming("cargo", &args, Some(project_root)).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_main_rs_template_generation() {
        let config = ActrConfig::default_template("test-service");
        let cmd = BuildCommand {
            release: false,
            skip_proto_deps: false,
            clean: false,
            cargo_args: vec![],
        };

        let template = cmd.create_main_rs_template(&config).unwrap();
        assert!(template.contains("test-service"));
        assert!(template.contains("ActorSystem"));
    }

    #[tokio::test]
    async fn test_resolve_local_dependency() {
        let temp_dir = TempDir::new().unwrap();
        let proto_file = temp_dir.path().join("test.proto");
        std::fs::write(&proto_file, "syntax = \"proto3\";").unwrap();

        let cmd = BuildCommand {
            release: false,
            skip_proto_deps: false,
            clean: false,
            cargo_args: vec![],
        };

        let dependency = ProtoDependency::Local {
            path: proto_file.to_string_lossy().to_string(),
        };

        let result = cmd.resolve_single_dependency("test", &dependency, temp_dir.path()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), proto_file);
    }
}