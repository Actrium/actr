//! `actr build` - build source artifacts and package signed `.actr` workloads.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use actr_config::{BuildArtifact, BuildConfig, BuildProfile, ConfigParser, ManifestConfig};
use anyhow::{Context, Result};
use cargo_metadata::MetadataCommand;
use clap::Args;

use crate::commands::codegen::metadata_path;
use crate::commands::package_build::{
    PackageBuildInput, build_package, default_dist_output_path, print_build_summary,
    resolve_key_path,
};
use crate::project_language::DetectedProjectLanguage;

#[derive(Args, Debug)]
#[command(
    about = "Build source artifact and package a signed .actr workload",
    long_about = "Build source artifact and package a signed .actr workload from manifest.toml"
)]
pub struct BuildCommand {
    /// manifest.toml path
    #[arg(
        long,
        short = 'f',
        default_value = "manifest.toml",
        value_name = "FILE"
    )]
    pub file: PathBuf,

    /// Override target triple
    #[arg(long, short = 't', value_name = "TARGET")]
    pub target: Option<String>,

    /// Output .actr file path
    #[arg(long, short = 'o', value_name = "FILE")]
    pub output: Option<PathBuf>,

    /// Signing key file (overrides config mfr.keychain)
    #[arg(long, short = 'k', value_name = "FILE")]
    pub key: Option<PathBuf>,

    /// Skip compilation and only package the declared binary artifact
    #[arg(long)]
    pub no_compile: bool,
}

pub async fn execute(args: BuildCommand) -> Result<()> {
    let manifest_path = resolve_manifest_path(&args.file)?;
    let config = ConfigParser::from_manifest_file(&manifest_path).with_context(|| {
        format!(
            "Failed to load manifest configuration from {}",
            manifest_path.display()
        )
    })?;

    let binary = config.binary.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "manifest.toml is missing [binary].\nDeclare the final packaged artifact path before running `actr build`."
        )
    })?;

    let effective_target = resolve_effective_target(&args, &config)?;
    let output_path = resolve_output_path(&manifest_path, &effective_target, args.output.as_ref())?;

    if !args.no_compile {
        let build = config.build.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "manifest.toml is missing [build].\nAdd [build] or rerun with `--no-compile` to package an existing artifact."
            )
        })?;
        ensure_rust_codegen_ready(build)?;
        compile_project(
            &manifest_path,
            &output_path,
            &binary.path,
            &effective_target,
            build,
        )?;
    }

    if !binary.path.exists() {
        anyhow::bail!(
            "Configured binary artifact not found: {}\nCheck [binary].path or your post_build steps.",
            binary.path.display()
        );
    }

    let cli_config = crate::config::resolver::resolve_effective_cli_config()?;
    let key_path = resolve_key_path(args.key.as_deref(), cli_config.mfr.keychain.as_deref())?;

    let summary = build_package(PackageBuildInput {
        binary_path: binary.path.clone(),
        config_path: manifest_path,
        key_path,
        output_path,
        target: effective_target,
        resources: vec![],
    })?;

    print_build_summary(&summary);
    Ok(())
}

fn ensure_rust_codegen_ready(build: &BuildConfig) -> Result<()> {
    let project_root = build
        .manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();

    if DetectedProjectLanguage::detect(&project_root) != DetectedProjectLanguage::Rust {
        return Ok(());
    }

    let generated_dir = project_root.join("src/generated");
    let generated_meta = metadata_path(&generated_dir);
    if generated_dir.exists() && generated_meta.exists() {
        return Ok(());
    }

    anyhow::bail!(
        "Rust generated sources are missing or stale for {}.\nRun `actr gen -l rust` before `actr build`.",
        project_root.display()
    );
}

fn resolve_manifest_path(path: &Path) -> Result<PathBuf> {
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };

    if !candidate.exists() {
        anyhow::bail!(
            "manifest.toml not found: {}\nBy default `actr build` looks for ./manifest.toml. Use `-f, --file` to specify a different path.",
            candidate.display()
        );
    }

    Ok(candidate)
}

fn resolve_effective_target(args: &BuildCommand, config: &ManifestConfig) -> Result<String> {
    if let Some(target) = &args.target {
        return Ok(target.clone());
    }

    if let Some(target) = config
        .binary
        .as_ref()
        .and_then(|binary| binary.target.clone())
    {
        return Ok(target);
    }

    if let Some(target) = config.build.as_ref().and_then(|build| build.target.clone()) {
        return Ok(target);
    }

    resolve_host_target()
}

fn resolve_output_path(
    manifest_path: &Path,
    effective_target: &str,
    output: Option<&PathBuf>,
) -> Result<PathBuf> {
    let manifest_dir = manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();

    match output {
        Some(path) if path.is_absolute() => Ok(path.clone()),
        Some(path) => Ok(manifest_dir.join(path)),
        None => default_dist_output_path(manifest_path, effective_target),
    }
}

fn compile_project(
    manifest_path: &Path,
    output_path: &Path,
    binary_path: &Path,
    effective_target: &str,
    build: &BuildConfig,
) -> Result<()> {
    if !build.manifest_path.exists() {
        anyhow::bail!(
            "Cargo manifest not found: {}",
            build.manifest_path.display()
        );
    }

    ensure_target_installed(effective_target)?;
    run_cargo_build(build, effective_target)?;
    run_post_build_steps(
        manifest_path,
        output_path,
        binary_path,
        effective_target,
        build,
    )?;

    if !binary_path.exists() {
        anyhow::bail!(
            "Binary artifact was not produced after build/post_build: {}",
            binary_path.display()
        );
    }

    Ok(())
}

fn ensure_target_installed(target: &str) -> Result<()> {
    let host_target = resolve_host_target()?;
    if target == host_target {
        return Ok(());
    }

    let status = Command::new("rustup")
        .arg("target")
        .arg("add")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("Failed to run `rustup target add {target}`"))?;

    if !status.success() {
        anyhow::bail!("`rustup target add {target}` failed with status {status}");
    }

    Ok(())
}

fn run_cargo_build(build: &BuildConfig, effective_target: &str) -> Result<()> {
    let mut command = Command::new("cargo");
    command.arg("build");
    command.arg("--manifest-path").arg(&build.manifest_path);

    match build.artifact {
        BuildArtifact::Lib => {
            command.arg("--lib");
        }
        BuildArtifact::Bin => {
            command
                .arg("--bin")
                .arg(resolve_cargo_bin_name(&build.manifest_path)?);
        }
    }

    if build.profile == BuildProfile::Release {
        command.arg("--release");
    }

    command.arg("--target").arg(effective_target);

    if !build.features.is_empty() {
        command.arg("--features").arg(build.features.join(","));
    }

    if build.no_default_features {
        command.arg("--no-default-features");
    }

    let status = command
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| {
            format!(
                "Failed to run cargo build for manifest {}",
                build.manifest_path.display()
            )
        })?;

    if !status.success() {
        anyhow::bail!("cargo build failed with status {status}");
    }

    Ok(())
}

fn run_post_build_steps(
    manifest_path: &Path,
    output_path: &Path,
    binary_path: &Path,
    effective_target: &str,
    build: &BuildConfig,
) -> Result<()> {
    if build.post_build.is_empty() {
        return Ok(());
    }

    let manifest_dir = manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();

    for command_text in &build.post_build {
        let output = Command::new("sh")
            .arg("-c")
            .arg(command_text)
            .current_dir(&manifest_dir)
            .env("ACTR_BUILD_MANIFEST_PATH", manifest_path)
            .env("ACTR_BUILD_PROJECT_DIR", &manifest_dir)
            .env("ACTR_BUILD_BINARY_PATH", binary_path)
            .env("ACTR_BUILD_TARGET", effective_target)
            .env("ACTR_BUILD_PROFILE", build.profile.as_str())
            .env("ACTR_BUILD_OUTPUT_PATH", output_path)
            .output()
            .with_context(|| format!("Failed to run post_build command: {command_text}"))?;

        if !output.stdout.is_empty() {
            print!("{}", String::from_utf8_lossy(&output.stdout));
        }
        if !output.stderr.is_empty() {
            eprint!("{}", String::from_utf8_lossy(&output.stderr));
        }

        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            anyhow::bail!(
                "post_build command failed: {command_text}\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
                output.status,
                stdout,
                stderr,
            );
        }
    }

    Ok(())
}

fn resolve_cargo_bin_name(manifest_path: &Path) -> Result<String> {
    let metadata = MetadataCommand::new()
        .manifest_path(manifest_path)
        .no_deps()
        .exec()
        .with_context(|| {
            format!(
                "Failed to read Cargo metadata from {}",
                manifest_path.display()
            )
        })?;

    let manifest_path =
        std::fs::canonicalize(manifest_path).unwrap_or_else(|_| manifest_path.to_path_buf());

    let package = metadata
        .packages
        .iter()
        .find(|package| {
            std::fs::canonicalize(package.manifest_path.as_std_path())
                .map(|path| path == manifest_path)
                .unwrap_or(false)
        })
        .or_else(|| metadata.root_package())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Unable to resolve Cargo package for {}",
                manifest_path.display()
            )
        })?;

    Ok(package.name.clone())
}

fn resolve_host_target() -> Result<String> {
    let output = Command::new("rustc")
        .arg("-vV")
        .output()
        .context("Failed to run `rustc -vV` to resolve host target")?;

    if !output.status.success() {
        anyhow::bail!("`rustc -vV` failed with status {}", output.status);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let host = stdout
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .ok_or_else(|| anyhow::anyhow!("Unable to resolve host target from `rustc -vV`"))?;

    Ok(host.trim().to_string())
}
