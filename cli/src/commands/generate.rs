//! # Code Generation Command
//!
//! Shared CLI entry point for `actr gen`. Language-specific logic lives in
//! `src/commands/codegen/{rust,swift,typescript,...}.rs`.

use crate::commands::Command;
use crate::commands::SupportedLanguage;
use crate::commands::codegen::{GenContext, ProtoModel, execute_codegen};
use crate::config_compat::load_config_with_legacy_actr_type;
use crate::error::{ActrCliError, Result};
use crate::project_language::DetectedProjectLanguage;
use crate::utils::to_pascal_case;
use async_trait::async_trait;
use clap::Args;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

#[derive(Args, Debug, Clone)]
#[command(
    about = "Generate code from proto files",
    after_help = "Default output paths by language:
  - rust:   src/generated
  - swift:  {PascalName}/Generated (e.g., EchoApp/Generated)
  - kotlin: app/src/main/java/{package}/generated
  - python: generated
  - typescript: src/generated"
)]
pub struct GenCommand {
    /// Input proto file or directory
    #[arg(short, long, default_value = "protos")]
    pub input: PathBuf,

    /// Output directory for generated code (use -o to override language defaults)
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Path to Actr.toml config file
    #[arg(short, long, default_value = "Actr.toml")]
    pub config: PathBuf,

    /// Clean generated outputs before regenerating
    #[arg(long = "clean")]
    pub clean: bool,

    /// Skip user code scaffold generation
    #[arg(long = "no-scaffold")]
    pub no_scaffold: bool,
    /// Whether to overwrite existing user code files
    #[arg(long)]
    pub overwrite_user_code: bool,

    /// Skip formatting
    #[arg(long = "no-format")]
    pub no_format: bool,

    /// Debug mode: keep intermediate generated files
    #[arg(long)]
    pub debug: bool,

    /// Skip code validation after generation
    #[arg(long)]
    pub skip_validation: bool,

    /// Target language for generation
    #[arg(short, long, default_value = "rust")]
    pub language: SupportedLanguage,
}

#[async_trait]
impl Command for GenCommand {
    async fn execute(&self) -> Result<()> {
        self.check_lock_file()?;
        self.validate_project_language_compatibility()?;

        let output = self.determine_output_path()?;

        info!(
            "🚀 Start code generation (language: {:?})...",
            self.language
        );
        let config = load_config_with_legacy_actr_type(&self.config)
            .map_err(|e| ActrCliError::config_error(format!("Failed to parse Actr.toml: {e}")))?;

        let proto_files = self.preprocess()?;
        let proto_model = ProtoModel::parse(&proto_files, &self.input, &config)?;
        let context = GenContext {
            proto_files,
            proto_model,
            input_path: self.input.clone(),
            output,
            config_path: self.config.clone(),
            config: config.clone(),
            no_scaffold: self.no_scaffold,
            overwrite_user_code: self.overwrite_user_code,
            no_format: self.no_format,
            debug: self.debug,
            skip_validation: self.skip_validation,
        };
        execute_codegen(self.language, &context).await?;
        Ok(())
    }
}

impl GenCommand {
    fn validate_project_language_compatibility(&self) -> Result<()> {
        let project_root = self.config.parent().unwrap_or_else(|| Path::new("."));
        let detected = DetectedProjectLanguage::detect(project_root);

        if detected == DetectedProjectLanguage::Unknown {
            eprintln!(
                "Warning: Could not detect project language from '{}'; skipping language compatibility check.",
                project_root.display()
            );
            return Ok(());
        }

        if detected == DetectedProjectLanguage::Ambiguous {
            eprintln!(
                "Warning: Detected multiple project language markers in '{}'; skipping language compatibility check.",
                project_root.display()
            );
            return Ok(());
        }

        let requested = self.requested_project_language();
        if detected == requested {
            return Ok(());
        }

        Err(ActrCliError::config_error(format!(
            "Refusing to generate '{requested}' code in a '{detected}' project.\n\n\
             Run:\n  actr gen -l {detected}"
        )))
    }

    fn requested_project_language(&self) -> DetectedProjectLanguage {
        match self.language {
            SupportedLanguage::Rust => DetectedProjectLanguage::Rust,
            SupportedLanguage::Python => DetectedProjectLanguage::Python,
            SupportedLanguage::Swift => DetectedProjectLanguage::Swift,
            SupportedLanguage::Kotlin => DetectedProjectLanguage::Kotlin,
            SupportedLanguage::TypeScript => DetectedProjectLanguage::TypeScript,
        }
    }

    fn check_lock_file(&self) -> Result<()> {
        let config_dir = self
            .config
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        let lock_file_path = config_dir.join("Actr.lock.toml");

        if !lock_file_path.exists() {
            return Err(ActrCliError::config_error(
                "Actr.lock.toml not found\n\n\
                The lock file is required for code generation. Please run:\n\n\
                \x20\x20\x20\x20actr install\n\n\
                This will generate Actr.lock.toml based on your Actr.toml configuration.",
            ));
        }

        Ok(())
    }

    fn determine_output_path(&self) -> Result<PathBuf> {
        if let Some(ref output) = self.output {
            return Ok(output.clone());
        }

        match self.language {
            SupportedLanguage::Swift => {
                let config = load_config_with_legacy_actr_type(&self.config).map_err(|e| {
                    ActrCliError::config_error(format!("Failed to parse Actr.toml: {e}"))
                })?;
                let project_name = &config.package.name;
                let pascal_name = to_pascal_case(project_name);
                Ok(PathBuf::from(format!("{}/Generated", pascal_name)))
            }
            SupportedLanguage::Kotlin => {
                let config = load_config_with_legacy_actr_type(&self.config).map_err(|e| {
                    ActrCliError::config_error(format!("Failed to parse Actr.toml: {e}"))
                })?;
                let clean_name: String = config
                    .package
                    .name
                    .chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect::<String>()
                    .to_lowercase();
                let package_path = format!("io/actr/{}", clean_name);
                Ok(PathBuf::from(format!(
                    "app/src/main/java/{}/generated",
                    package_path
                )))
            }
            SupportedLanguage::Python => Ok(PathBuf::from("generated")),
            SupportedLanguage::TypeScript => Ok(PathBuf::from("src/generated")),
            SupportedLanguage::Rust => Ok(PathBuf::from("src/generated")),
        }
    }

    fn preprocess(&self) -> Result<Vec<PathBuf>> {
        self.validate_inputs()?;
        self.clean_generated_outputs()?;
        self.prepare_output_dirs()?;

        let proto_files = self.discover_proto_files()?;
        info!("📁 Found {} proto files", proto_files.len());

        Ok(proto_files)
    }

    fn clean_generated_outputs(&self) -> Result<()> {
        use std::fs;

        if !self.clean {
            return Ok(());
        }

        let output = self.determine_output_path()?;
        if !output.exists() {
            return Ok(());
        }

        info!("🧹 Cleaning old generation results: {:?}", output);

        self.make_writable_recursive(&output)?;
        fs::remove_dir_all(&output).map_err(|e| {
            ActrCliError::config_error(format!("Failed to delete generation directory: {e}"))
        })?;

        Ok(())
    }

    #[allow(clippy::only_used_in_recursion)]
    fn make_writable_recursive(&self, path: &Path) -> Result<()> {
        use std::fs;

        if path.is_file() {
            let metadata = fs::metadata(path).map_err(|e| {
                ActrCliError::config_error(format!("Failed to read file metadata: {e}"))
            })?;
            let mut permissions = metadata.permissions();

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = permissions.mode();
                permissions.set_mode(mode | 0o222);
            }

            #[cfg(not(unix))]
            {
                permissions.set_readonly(false);
            }

            fs::set_permissions(path, permissions).map_err(|e| {
                ActrCliError::config_error(format!("Failed to reset file permissions: {e}"))
            })?;
        } else if path.is_dir() {
            for entry in fs::read_dir(path)
                .map_err(|e| ActrCliError::config_error(format!("Failed to read directory: {e}")))?
            {
                let entry = entry.map_err(|e| ActrCliError::config_error(e.to_string()))?;
                self.make_writable_recursive(&entry.path())?;
            }
        }

        Ok(())
    }

    fn validate_inputs(&self) -> Result<()> {
        if !self.input.exists() {
            return Err(ActrCliError::config_error(format!(
                "Input path does not exist: {:?}",
                self.input
            )));
        }

        if self.input.is_file() && self.input.extension().unwrap_or_default() != "proto" {
            warn!("Input file is not a .proto file: {:?}", self.input);
        }

        Ok(())
    }

    fn prepare_output_dirs(&self) -> Result<()> {
        let output = self.determine_output_path()?;
        std::fs::create_dir_all(&output).map_err(|e| {
            ActrCliError::config_error(format!("Failed to create output directory: {e}"))
        })?;

        if !self.no_scaffold {
            let user_code_dir = output.join("../");
            std::fs::create_dir_all(&user_code_dir).map_err(|e| {
                ActrCliError::config_error(format!("Failed to create user code directory: {e}"))
            })?;
        }

        Ok(())
    }

    fn discover_proto_files(&self) -> Result<Vec<PathBuf>> {
        let mut proto_files = Vec::new();

        if self.input.is_file() {
            proto_files.push(self.input.clone());
        } else {
            self.collect_proto_files(&self.input, &mut proto_files)?;
        }

        if proto_files.is_empty() {
            return Err(ActrCliError::config_error("No proto files found"));
        }

        Ok(proto_files)
    }

    #[allow(clippy::only_used_in_recursion)]
    fn collect_proto_files(&self, dir: &PathBuf, proto_files: &mut Vec<PathBuf>) -> Result<()> {
        for entry in std::fs::read_dir(dir)
            .map_err(|e| ActrCliError::config_error(format!("Failed to read directory: {e}")))?
        {
            let entry = entry.map_err(|e| ActrCliError::config_error(e.to_string()))?;
            let path = entry.path();

            if path.is_file() && path.extension().unwrap_or_default() == "proto" {
                proto_files.push(path);
            } else if path.is_dir() {
                self.collect_proto_files(&path, proto_files)?;
            }
        }
        Ok(())
    }
}
