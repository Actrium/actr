//! # actr-web-protoc-codegen
//!
//! Protoc code generator for producing actr-web code from Protobuf definitions.
//!
//! ## Features
//!
//! - Generate Rust WASM actor code from `.proto` files
//! - Generate TypeScript type definitions
//! - Generate TypeScript ActorRef wrappers
//! - Optionally generate React Hooks
//!
//! ## Usage
//!
//! ### Option 1: use it from `build.rs`
//!
//! ```rust,no_run
//! use actr_web_protoc_codegen::{WebCodegen, WebCodegenConfig};
//!
//! let config = WebCodegenConfig {
//!     proto_files: vec!["proto/echo.proto".into()],
//!     rust_output_dir: "src/generated".into(),
//!     ts_output_dir: "../packages/web-sdk/src/generated".into(),
//!     generate_react_hooks: true,
//!     includes: vec!["proto".into()],
//!     custom_templates_dir: None,
//!     format_code: true,
//! };
//!
//! WebCodegen::new(config)
//!     .generate()
//!     .expect("Failed to generate code");
//! ```
//!
//! ### Option 2: use it through `actr-cli`
//!
//! ```bash
//! actr gen --platform web \
//!   --input proto/ \
//!   --output crates/actors/src/generated/ \
//!   --ts-output packages/web-sdk/src/generated/ \
//!   --react-hooks
//! ```

use std::path::PathBuf;

pub mod codegen;
mod config;
mod error;
mod generator;
pub mod request;
mod templates;
mod typescript;

pub use codegen::generate;
pub use config::*;
pub use error::*;
pub use generator::*;
pub use request::{
    ActrTypeInfo, DependencyInfo, FileInfo, MethodInfo, ObservabilityInfo, ServiceInfo,
    WebCodegenRequest, WebCodegenResponse,
};

/// Code generator for the web platform.
pub struct WebCodegen {
    config: WebCodegenConfig,
}

impl WebCodegen {
    /// Create a new code generator instance.
    pub fn new(config: WebCodegenConfig) -> Self {
        Self { config }
    }

    /// Generate all outputs: Rust and TypeScript.
    pub fn generate(&self) -> Result<GeneratedFiles> {
        tracing::info!("Starting actr-web code generation");

        let mut files = GeneratedFiles::default();

        // 1. Parse proto files.
        let services = self.parse_proto_files()?;
        tracing::info!("Parsed {} services", services.len());

        // 2. Generate Rust WASM actor code.
        tracing::info!("Generating Rust WASM code");
        files.rust_files = self.generate_rust_actors(&services)?;

        // 3. Generate TypeScript types.
        tracing::info!("Generating TypeScript types");
        files.ts_types = self.generate_typescript_types(&services)?;

        // 4. Generate TypeScript ActorRef wrappers.
        tracing::info!("Generating ActorRef wrappers");
        files.ts_actor_refs = self.generate_actor_refs(&services)?;

        // 5. Optionally generate React Hooks.
        if self.config.generate_react_hooks {
            tracing::info!("Generating React Hooks");
            files.react_hooks = self.generate_react_hooks(&services)?;
        }

        // 6. Write files.
        files.write_to_disk()?;

        // 7. Format generated code.
        if self.config.format_code {
            files.format_code()?;
        }

        tracing::info!(
            "Code generation finished. Generated {} files",
            files.total_count()
        );

        Ok(files)
    }

    /// Generate Rust output only, intended for `build.rs`.
    pub fn generate_rust_only(&self) -> Result<Vec<GeneratedFile>> {
        let services = self.parse_proto_files()?;
        self.generate_rust_actors(&services)
    }

    /// Generate TypeScript output only.
    pub fn generate_typescript_only(&self) -> Result<Vec<GeneratedFile>> {
        let services = self.parse_proto_files()?;
        let mut files = Vec::new();
        files.extend(self.generate_typescript_types(&services)?);
        files.extend(self.generate_actor_refs(&services)?);
        Ok(files)
    }

    /// Parse proto files.
    fn parse_proto_files(&self) -> Result<Vec<ProtoService>> {
        generator::parse_proto_files(&self.config)
    }

    /// Generate Rust actor code.
    fn generate_rust_actors(&self, services: &[ProtoService]) -> Result<Vec<GeneratedFile>> {
        generator::generate_rust_actors(&self.config, services)
    }

    /// Generate TypeScript types.
    fn generate_typescript_types(&self, services: &[ProtoService]) -> Result<Vec<GeneratedFile>> {
        typescript::generate_types(&self.config, services)
    }

    /// Generate ActorRef wrappers.
    fn generate_actor_refs(&self, services: &[ProtoService]) -> Result<Vec<GeneratedFile>> {
        typescript::generate_actor_refs(&self.config, services)
    }

    /// Generate React Hooks.
    fn generate_react_hooks(&self, services: &[ProtoService]) -> Result<Vec<GeneratedFile>> {
        typescript::generate_react_hooks(&self.config, services)
    }
}

/// All files generated in a run.
#[derive(Default, Debug)]
pub struct GeneratedFiles {
    pub rust_files: Vec<GeneratedFile>,
    pub ts_types: Vec<GeneratedFile>,
    pub ts_actor_refs: Vec<GeneratedFile>,
    pub react_hooks: Vec<GeneratedFile>,
}

impl GeneratedFiles {
    /// Return an iterator over all generated files.
    pub fn all_files(&self) -> impl Iterator<Item = &GeneratedFile> {
        self.rust_files
            .iter()
            .chain(self.ts_types.iter())
            .chain(self.ts_actor_refs.iter())
            .chain(self.react_hooks.iter())
    }

    /// Return the total generated file count.
    pub fn total_count(&self) -> usize {
        self.rust_files.len()
            + self.ts_types.len()
            + self.ts_actor_refs.len()
            + self.react_hooks.len()
    }

    /// Write all generated files to disk.
    pub fn write_to_disk(&self) -> Result<()> {
        for file in self.all_files() {
            file.write_to_disk()?;
        }
        Ok(())
    }

    /// Format all generated code.
    pub fn format_code(&self) -> Result<()> {
        tracing::info!("Formatting generated code");

        // Format Rust files.
        for file in &self.rust_files {
            if file.path.extension().and_then(|s| s.to_str()) == Some("rs") {
                format_rust_file(&file.path)?;
            }
        }

        // Format TypeScript files.
        let ts_files: Vec<_> = self
            .ts_types
            .iter()
            .chain(self.ts_actor_refs.iter())
            .chain(self.react_hooks.iter())
            .collect();

        for file in ts_files {
            if file.path.extension().and_then(|s| s.to_str()) == Some("ts") {
                format_typescript_file(&file.path)?;
            }
        }

        tracing::info!("Generated code formatting completed");
        Ok(())
    }
}

/// A single generated file.
#[derive(Debug, Clone)]
pub struct GeneratedFile {
    pub path: PathBuf,
    pub content: String,
}

impl GeneratedFile {
    /// Create a new generated file.
    pub fn new(path: PathBuf, content: String) -> Self {
        Self { path, content }
    }

    /// Write the file to disk.
    pub fn write_to_disk(&self) -> Result<()> {
        use std::fs;

        // Create the parent directory first.
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write the file.
        fs::write(&self.path, &self.content)?;
        tracing::debug!("Wrote file: {}", self.path.display());

        Ok(())
    }
}

/// Proto service definition.
#[derive(Debug, Clone)]
pub struct ProtoService {
    pub name: String,
    pub package: String,
    pub methods: Vec<ProtoMethod>,
    pub messages: Vec<ProtoMessage>,
}

/// Proto method definition.
#[derive(Debug, Clone)]
pub struct ProtoMethod {
    pub name: String,
    pub input_type: String,
    pub output_type: String,
    pub is_streaming: bool,
}

/// Proto message definition.
#[derive(Debug, Clone)]
pub struct ProtoMessage {
    pub name: String,
    pub fields: Vec<ProtoField>,
}

/// Proto field definition.
#[derive(Debug, Clone)]
pub struct ProtoField {
    pub name: String,
    pub field_type: String,
    pub number: u32,
    pub is_repeated: bool,
    pub is_optional: bool,
}

/// Format a Rust file.
fn format_rust_file(path: &std::path::Path) -> Result<()> {
    use std::process::Command;

    let output = Command::new("rustfmt")
        .arg("--edition")
        .arg("2021")
        .arg(path)
        .output();

    match output {
        Ok(output) if output.status.success() => {
            tracing::debug!("Formatted Rust file: {}", path.display());
            Ok(())
        }
        Ok(output) => {
            tracing::warn!(
                "rustfmt failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            Ok(()) // Formatting failures must not block code generation.
        }
        Err(e) => {
            tracing::warn!("rustfmt not found or failed to execute: {}", e);
            Ok(()) // Formatting failures must not block code generation.
        }
    }
}

/// Format a TypeScript file.
fn format_typescript_file(path: &std::path::Path) -> Result<()> {
    use std::process::Command;

    // Try prettier first.
    let output = Command::new("npx")
        .args(["prettier", "--write", path.to_str().unwrap()])
        .output();

    match output {
        Ok(output) if output.status.success() => {
            tracing::debug!("Formatted TypeScript file: {}", path.display());
            Ok(())
        }
        Ok(output) => {
            tracing::warn!(
                "prettier failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            Ok(())
        }
        Err(_) => {
            // Fall back to dprint when prettier is unavailable.
            let output = Command::new("dprint")
                .args(["fmt", path.to_str().unwrap()])
                .output();

            match output {
                Ok(output) if output.status.success() => {
                    tracing::debug!("Formatted TypeScript file with dprint: {}", path.display());
                    Ok(())
                }
                _ => {
                    tracing::warn!("No TypeScript formatter found (prettier/dprint)");
                    Ok(())
                }
            }
        }
    }
}
