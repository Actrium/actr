//! Web platform code generator
//!
//! Delegates to the `protoc-gen-actr-web --generate` plugin binary for all
//! code generation.  The CLI builds a [`WebCodegenRequest`], pipes it as JSON
//! to the plugin's stdin, and reads a [`WebCodegenResponse`] from stdout.
//!
//! Generated artifacts (produced by the plugin):
//! - `src/generated/actr-config.ts`  — Configuration from manifest.toml
//! - `src/generated/*.actorref.ts`   — Typed ActorRef wrappers for local services
//! - `src/generated/index.ts`        — Re-exports
//! - `wasm/`                         — Rust WASM crate (Cargo.toml, build.sh, src/lib.rs, handlers)
//! - `public/actor.sw.js`            — Service Worker entry
//! - `build.sh`                      — Root build script

use crate::commands::codegen::traits::{GenContext, LanguageGenerator};
use crate::error::{ActrCliError, Result};
use actr_web_protoc_codegen::{
    ActrTypeInfo, DependencyInfo, FileInfo, MethodInfo, ObservabilityInfo, ServiceInfo,
    WebCodegenRequest, WebCodegenResponse,
};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use tracing::info;

pub struct WebGenerator;

#[async_trait]
impl LanguageGenerator for WebGenerator {
    async fn generate_infrastructure(&self, context: &GenContext) -> Result<Vec<PathBuf>> {
        info!("🌐 Generating Web infrastructure code via plugin...");

        let plugin_path = find_plugin_binary()?;
        let request = build_codegen_request(context)?;

        let json_input = serde_json::to_string(&request).map_err(|e| {
            ActrCliError::config_error(format!("Failed to serialise WebCodegenRequest: {e}"))
        })?;

        let child = StdCommand::new(&plugin_path)
            .arg("--generate")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                ActrCliError::config_error(format!(
                    "Failed to spawn protoc-gen-actr-web ({}): {e}",
                    plugin_path.display()
                ))
            })?;

        use std::io::Write;
        let mut child = child;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(json_input.as_bytes()).map_err(|e| {
                ActrCliError::config_error(format!("Failed to write to plugin stdin: {e}"))
            })?;
        }

        let output = child
            .wait_with_output()
            .map_err(|e| ActrCliError::config_error(format!("Plugin process failed: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ActrCliError::config_error(format!(
                "protoc-gen-actr-web exited with {}: {}",
                output.status, stderr
            )));
        }

        let response: WebCodegenResponse = serde_json::from_slice(&output.stdout).map_err(|e| {
            ActrCliError::config_error(format!(
                "Failed to parse plugin response: {e}\nstdout: {}",
                String::from_utf8_lossy(&output.stdout)
            ))
        })?;

        if !response.success {
            return Err(ActrCliError::config_error(format!(
                "Plugin reported errors: {}",
                response.errors.join("; ")
            )));
        }

        for f in &response.generated_files {
            info!("  📄 {}", f.display());
        }

        Ok(response.generated_files)
    }

    async fn generate_scaffold(&self, _context: &GenContext) -> Result<Vec<PathBuf>> {
        // Web scaffold is generated as part of infrastructure (wasm/ directory)
        Ok(Vec::new())
    }

    async fn format_code(&self, _context: &GenContext, _files: &[PathBuf]) -> Result<()> {
        // No auto-formatting for web projects (TypeScript + Rust mix)
        Ok(())
    }

    async fn validate_code(&self, _context: &GenContext) -> Result<()> {
        // Validation is skipped for web projects
        Ok(())
    }

    fn print_next_steps(&self, _context: &GenContext) {
        info!("");
        info!("🌐 Web code generation complete!");
        info!("");
        info!("Next steps:");
        info!("  cd wasm && bash build.sh && cd ..  # Build WASM");
        info!("  npm run dev                        # Start dev server");
    }
}

// ═════════════════════════════════════════════════════════════════
// Plugin binary location
// ═════════════════════════════════════════════════════════════════

const PLUGIN_BIN_NAME: &str = "protoc-gen-actr-web";

fn find_plugin_binary() -> Result<PathBuf> {
    // 1. Check workspace target/debug (dev mode)
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        let workspace_bin = PathBuf::from(&manifest)
            .parent()
            .unwrap_or(Path::new("."))
            .join("target/debug")
            .join(PLUGIN_BIN_NAME);
        if workspace_bin.exists() {
            info!("Using workspace plugin: {}", workspace_bin.display());
            return Ok(workspace_bin);
        }
    }

    // 2. Check adjacent to current exe
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let adjacent = dir.join(PLUGIN_BIN_NAME);
            if adjacent.exists() {
                return Ok(adjacent);
            }
        }
    }

    // 3. Search PATH
    if let Ok(output) = StdCommand::new("which").arg(PLUGIN_BIN_NAME).output() {
        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path_str.is_empty() {
                return Ok(PathBuf::from(path_str));
            }
        }
    }

    // 4. Try to build from workspace
    info!("protoc-gen-actr-web not found, attempting to build...");
    try_build_plugin()
}

fn try_build_plugin() -> Result<PathBuf> {
    // Try to find the workspace root (where Cargo.toml with workspace is)
    let workspace_root = find_workspace_root()?;

    let status = StdCommand::new("cargo")
        .arg("build")
        .arg("-p")
        .arg("actr-web-protoc-codegen")
        .current_dir(&workspace_root)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| ActrCliError::config_error(format!("Failed to run cargo build: {e}")))?;

    if !status.success() {
        return Err(ActrCliError::config_error(
            "Failed to build protoc-gen-actr-web. Please run: cargo build -p actr-web-protoc-codegen"
                .to_string(),
        ));
    }

    let built = workspace_root.join("target/debug").join(PLUGIN_BIN_NAME);
    if built.exists() {
        Ok(built)
    } else {
        Err(ActrCliError::config_error(format!(
            "Built plugin not found at {}",
            built.display()
        )))
    }
}

fn find_workspace_root() -> Result<PathBuf> {
    let output = StdCommand::new("cargo")
        .args(["locate-project", "--workspace", "--message-format=plain"])
        .output()
        .map_err(|e| ActrCliError::config_error(format!("cargo locate-project failed: {e}")))?;

    if output.status.success() {
        let cargo_toml = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let root = PathBuf::from(&cargo_toml)
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();
        return Ok(root);
    }

    Err(ActrCliError::config_error(
        "Could not locate workspace root".to_string(),
    ))
}

// ═════════════════════════════════════════════════════════════════
// Build WebCodegenRequest from GenContext
// ═════════════════════════════════════════════════════════════════

fn build_codegen_request(context: &GenContext) -> Result<WebCodegenRequest> {
    let config = &context.config;
    let project_root = context
        .output
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(Path::new("."))
        .to_path_buf();

    let raw_toml = std::fs::read_to_string(&context.config_path).unwrap_or_default();

    // Map dependencies
    let dependencies: Vec<DependencyInfo> = config
        .dependencies
        .iter()
        .map(|d| DependencyInfo {
            alias: d.alias.clone(),
            actr_type: d.actr_type.as_ref().map(|at| ActrTypeInfo {
                manufacturer: at.manufacturer.clone(),
                name: at.name.clone(),
            }),
        })
        .collect();

    // ICE servers → stun/turn URLs
    let stun_urls: Vec<String> = config
        .webrtc
        .ice_servers
        .iter()
        .flat_map(|s| s.urls.iter())
        .filter(|u| u.starts_with("stun:"))
        .cloned()
        .collect();
    let turn_urls: Vec<String> = config
        .webrtc
        .ice_servers
        .iter()
        .flat_map(|s| s.urls.iter())
        .filter(|u| u.starts_with("turn:"))
        .cloned()
        .collect();

    // Observability
    let obs = &config.observability;
    let observability = ObservabilityInfo {
        filter_level: obs.filter_level.clone(),
        tracing_enabled: obs.tracing_enabled,
        tracing_endpoint: obs.tracing_endpoint.clone(),
        tracing_service_name: obs.tracing_service_name.clone(),
    };

    // Map proto model services
    let local_services = map_services(&context.proto_model.local_services);
    let remote_services = map_services(&context.proto_model.remote_services);

    // Map proto files
    let files: Vec<FileInfo> = context
        .proto_model
        .files
        .iter()
        .map(|f| FileInfo {
            proto_file: f.proto_file.clone(),
            relative_path: f.relative_path.clone(),
            package: f.package.clone(),
            is_local: f.side == crate::commands::codegen::proto_model::ProtoSide::Local,
            declared_type_names: f.declared_type_names.clone(),
        })
        .collect();

    Ok(WebCodegenRequest {
        config_path: context.config_path.clone(),
        output_dir: context.output.clone(),
        project_root,
        overwrite_user_code: context.overwrite_user_code,
        package_name: config.package.name.clone(),
        manufacturer: config.package.actr_type.manufacturer.clone(),
        actr_name: config.package.actr_type.name.clone(),
        description: config.package.description.clone().unwrap_or_default(),
        authors: config.package.authors.clone(),
        license: config
            .package
            .license
            .clone()
            .unwrap_or_else(|| "Apache-2.0".to_string()),
        tags: config.tags.clone(),
        signaling_url: config.signaling_url.as_ref().map(|u| u.to_string()).unwrap_or_default(),
        realm_id: config.realm.as_ref().map(|r| r.realm_id).unwrap_or(0),
        visible_in_discovery: config.visible_in_discovery,
        dependencies,
        stun_urls,
        turn_urls,
        observability,
        raw_toml,
        local_services,
        remote_services,
        files,
    })
}

fn map_services(
    models: &[crate::commands::codegen::proto_model::ServiceModel],
) -> Vec<ServiceInfo> {
    models
        .iter()
        .map(|s| ServiceInfo {
            name: s.name.clone(),
            package: s.package.clone(),
            relative_path: s.relative_path.clone(),
            methods: s
                .methods
                .iter()
                .map(|m| MethodInfo {
                    name: m.name.clone(),
                    snake_name: m.snake_name.clone(),
                    input_type: m.input_type.clone(),
                    output_type: m.output_type.clone(),
                    route_key: m.route_key.clone(),
                })
                .collect(),
            actr_type: s.actr_type.clone(),
        })
        .collect()
}
