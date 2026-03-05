//! Fingerprint command implementation
//!
//! Computes and displays semantic fingerprints for proto files

use crate::core::{Command, CommandContext, CommandResult, ComponentType};
use actr_config::ConfigParser;
use actr_version::{Fingerprint, ProtoFile};
use anyhow::{Context, Result};
use async_trait::async_trait;
use clap::Args;
use std::fs;
use std::path::Path;
use tracing::{error, info};

/// Verification status
#[derive(Debug, Clone)]
enum VerificationStatus {
    Passed {
        matched_fingerprint: String,
    },
    Failed {
        mismatches: Vec<(String, String, String)>,
    }, // (file_path, expected_fingerprint, actual_fingerprint)
    NoLockFile,
    NotRequested,
}

/// Fingerprint command - computes semantic fingerprints
#[derive(Args, Debug)]
#[command(
    about = "Compute project and service fingerprints",
    long_about = "Compute and display semantic fingerprints for proto files and services"
)]
pub struct FingerprintCommand {
    /// Configuration file path
    #[arg(short, long, default_value = "Actr.toml")]
    pub config: String,

    /// Output format (text, json, yaml)
    #[arg(long, default_value = "text")]
    pub format: String,

    /// Calculate fingerprint for a specific proto file
    #[arg(long)]
    pub proto: Option<String>,

    /// Calculate service-level fingerprint (default)
    #[arg(long)]
    pub service_level: bool,

    /// Verify fingerprint against lock file
    #[arg(long)]
    pub verify: bool,
}

#[async_trait]
impl Command for FingerprintCommand {
    async fn execute(&self, _context: &CommandContext) -> Result<CommandResult> {
        if let Some(proto_path) = &self.proto {
            // Calculate fingerprint for a specific proto file
            info!(
                "🔍 Computing proto semantic fingerprint for: {}",
                proto_path
            );
            execute_proto_fingerprint(self, proto_path).await?;
        } else {
            // Calculate service-level fingerprint (default)
            info!("🔍 Computing service semantic fingerprint...");
            execute_service_fingerprint(self).await?;
        }

        Ok(CommandResult::Success(
            "Fingerprint calculation completed".to_string(),
        ))
    }

    fn required_components(&self) -> Vec<ComponentType> {
        vec![] // Fingerprint calculation doesn't require external components
    }

    fn name(&self) -> &str {
        "fingerprint"
    }

    fn description(&self) -> &str {
        "Compute semantic fingerprints for proto files and services"
    }
}

/// Execute proto-level fingerprint calculation
async fn execute_proto_fingerprint(args: &FingerprintCommand, proto_path: &str) -> Result<()> {
    let path = Path::new(proto_path);
    if !path.exists() {
        return Err(anyhow::anyhow!("Proto file not found: {}", proto_path));
    }

    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read proto file: {}", proto_path))?;

    let fingerprint = Fingerprint::calculate_proto_semantic_fingerprint(&content)
        .context("Failed to calculate proto fingerprint")?;

    // Output
    match args.format.as_str() {
        "text" => show_proto_text_output(&fingerprint, proto_path),
        "json" => show_proto_json_output(&fingerprint, proto_path)?,
        "yaml" => show_proto_yaml_output(&fingerprint, proto_path)?,
        _ => {
            error!("Unsupported output format: {}", args.format);
            return Err(anyhow::anyhow!("Unsupported format: {}", args.format));
        }
    }

    Ok(())
}

/// Execute service-level fingerprint calculation
async fn execute_service_fingerprint(args: &FingerprintCommand) -> Result<()> {
    // Load configuration
    let config_path = Path::new(&args.config);
    let config = ConfigParser::from_file(config_path)
        .with_context(|| format!("Failed to load config from {}", args.config))?;

    // Convert actr_config::ProtoFile to actr_version::ProtoFile
    let mut proto_files: Vec<ProtoFile> = config
        .exports
        .iter()
        .map(|pf| ProtoFile {
            name: pf.file_name().unwrap_or("unknown.proto").to_string(),
            content: pf.content.clone(),
            path: Some(pf.path.to_string_lossy().to_string()),
        })
        .collect();

    // If verifying, also collect proto files from protos/remote directory
    if args.verify {
        let config_dir = config_path.parent().unwrap_or(Path::new("."));
        let remote_dir = config_dir.join("protos").join("remote");

        if remote_dir.exists() {
            collect_proto_files_from_directory(&remote_dir, &mut proto_files)?;
        }
    }

    if proto_files.is_empty() {
        // No proto files to calculate fingerprint for, but we can still verify lock file
        if args.verify {
            let verification_status = verify_fingerprint_against_lock("", &[], config_path)?;
            match args.format.as_str() {
                "text" => {
                    show_verification_status_only(&verification_status);
                }
                "json" => {
                    let verification_info = match verification_status {
                        VerificationStatus::Passed { .. } => {
                            serde_json::json!({"status": "passed"})
                        }
                        VerificationStatus::Failed { mismatches } => serde_json::json!({
                            "status": "failed",
                            "mismatches": mismatches.iter().map(|(file_path, expected, actual)| {
                                serde_json::json!({
                                    "file_path": file_path,
                                    "expected": expected,
                                    "actual": actual
                                })
                            }).collect::<Vec<_>>()
                        }),
                        VerificationStatus::NoLockFile => {
                            serde_json::json!({"status": "no_lock_file"})
                        }
                        _ => serde_json::json!({"status": "not_requested"}),
                    };
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&verification_info).unwrap()
                    );
                }
                "yaml" => {
                    let verification_info = match verification_status {
                        VerificationStatus::Passed { .. } => {
                            let mut map = serde_yaml::Mapping::new();
                            map.insert(
                                serde_yaml::Value::String("status".to_string()),
                                serde_yaml::Value::String("passed".to_string()),
                            );
                            map
                        }
                        VerificationStatus::Failed { mismatches } => {
                            let mut map = serde_yaml::Mapping::new();
                            map.insert(
                                serde_yaml::Value::String("status".to_string()),
                                serde_yaml::Value::String("failed".to_string()),
                            );
                            map.insert(
                                serde_yaml::Value::String("mismatches".to_string()),
                                serde_yaml::Value::Sequence(
                                    mismatches
                                        .iter()
                                        .map(|(file_path, expected, actual)| {
                                            let mut mismatch_map = serde_yaml::Mapping::new();
                                            mismatch_map.insert(
                                                serde_yaml::Value::String("file_path".to_string()),
                                                serde_yaml::Value::String(file_path.clone()),
                                            );
                                            mismatch_map.insert(
                                                serde_yaml::Value::String("expected".to_string()),
                                                serde_yaml::Value::String(expected.clone()),
                                            );
                                            mismatch_map.insert(
                                                serde_yaml::Value::String("actual".to_string()),
                                                serde_yaml::Value::String(actual.clone()),
                                            );
                                            serde_yaml::Value::Mapping(mismatch_map)
                                        })
                                        .collect(),
                                ),
                            );
                            map
                        }
                        VerificationStatus::NoLockFile => {
                            let mut map = serde_yaml::Mapping::new();
                            map.insert(
                                serde_yaml::Value::String("status".to_string()),
                                serde_yaml::Value::String("no_lock_file".to_string()),
                            );
                            map
                        }
                        _ => {
                            let mut map = serde_yaml::Mapping::new();
                            map.insert(
                                serde_yaml::Value::String("status".to_string()),
                                serde_yaml::Value::String("not_requested".to_string()),
                            );
                            map
                        }
                    };
                    println!(
                        "{}",
                        serde_yaml::to_string(&serde_yaml::Value::Mapping(verification_info))
                            .unwrap()
                    );
                }
                _ => {
                    show_verification_status_only(&verification_status);
                }
            }
        } else {
            match args.format.as_str() {
                "text" => {
                    println!("ℹ️  No proto files found in exports");
                    println!(
                        "   Add proto files to the 'exports' array in {} to calculate fingerprints",
                        args.config
                    );
                }
                "json" => {
                    let output = serde_json::json!({
                        "status": "no_exports",
                        "message": "No proto files found in exports",
                        "config_file": args.config
                    });
                    println!("{}", serde_json::to_string_pretty(&output).unwrap());
                }
                "yaml" => {
                    let output = serde_yaml::Value::Mapping({
                        let mut map = serde_yaml::Mapping::new();
                        map.insert(
                            serde_yaml::Value::String("status".to_string()),
                            serde_yaml::Value::String("no_exports".to_string()),
                        );
                        map.insert(
                            serde_yaml::Value::String("message".to_string()),
                            serde_yaml::Value::String(
                                "No proto files found in exports".to_string(),
                            ),
                        );
                        map.insert(
                            serde_yaml::Value::String("config_file".to_string()),
                            serde_yaml::Value::String(args.config.clone()),
                        );
                        map
                    });
                    println!("{}", serde_yaml::to_string(&output).unwrap());
                }
                _ => {
                    println!("ℹ️  No proto files found in exports");
                }
            }
        }
        return Ok(());
    }

    // Calculate semantic fingerprint
    let fingerprint = Fingerprint::calculate_service_semantic_fingerprint(&proto_files)
        .context("Failed to calculate service fingerprint")?;

    // Verify against lock file if requested
    let verification_status = if args.verify {
        verify_fingerprint_against_lock(&fingerprint, &proto_files, config_path)?
    } else {
        VerificationStatus::NotRequested
    };

    // Output
    match args.format.as_str() {
        "text" => show_text_output(&fingerprint, &proto_files, &verification_status),
        "json" => show_json_output(&fingerprint, &proto_files, &verification_status)?,
        "yaml" => show_yaml_output(&fingerprint, &proto_files, &verification_status)?,
        _ => {
            error!("Unsupported output format: {}", args.format);
            return Err(anyhow::anyhow!("Unsupported format: {}", args.format));
        }
    }

    Ok(())
}

/// Show proto text output format
fn show_proto_text_output(fingerprint: &str, proto_path: &str) {
    println!("📋 Proto Semantic Fingerprint:");
    println!("  File: {}", proto_path);
    println!("  {fingerprint}");
}

/// Show proto JSON output format
fn show_proto_json_output(fingerprint: &str, proto_path: &str) -> Result<()> {
    let output = ProtoJsonOutput {
        proto_file: proto_path.to_string(),
        fingerprint: fingerprint.to_string(),
    };

    let json = serde_json::to_string_pretty(&output).context("Failed to serialize output")?;
    println!("{json}");

    Ok(())
}

/// Show proto YAML output format
fn show_proto_yaml_output(fingerprint: &str, proto_path: &str) -> Result<()> {
    let output = ProtoJsonOutput {
        proto_file: proto_path.to_string(),
        fingerprint: fingerprint.to_string(),
    };

    let yaml = serde_yaml::to_string(&output).context("Failed to serialize output")?;
    println!("{yaml}");

    Ok(())
}

/// Collect proto files from a directory recursively
fn collect_proto_files_from_directory(dir: &Path, proto_files: &mut Vec<ProtoFile>) -> Result<()> {
    fn visit_dir(dir: &Path, proto_files: &mut Vec<ProtoFile>, base_dir: &Path) -> Result<()> {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries {
                let entry = entry?;
                let path = entry.path();

                if path.is_dir() {
                    visit_dir(&path, proto_files, base_dir)?;
                } else if path.extension().and_then(|s| s.to_str()) == Some("proto") {
                    let content = fs::read_to_string(&path).with_context(|| {
                        format!("Failed to read proto file: {}", path.display())
                    })?;

                    // Get relative path from the base directory
                    let relative_path = path.strip_prefix(base_dir).unwrap_or(&path);
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown.proto")
                        .to_string();

                    proto_files.push(ProtoFile {
                        name,
                        content,
                        path: Some(relative_path.to_string_lossy().to_string()),
                    });
                }
            }
        }
        Ok(())
    }

    visit_dir(dir, proto_files, dir)
}

/// Show verification status only (for cases where no proto files are available)
fn show_verification_status_only(verification_status: &VerificationStatus) {
    match verification_status {
        VerificationStatus::Passed { .. } => {
            println!("✅ Fingerprint verification: PASSED");
            println!("  All lock file fingerprints verified against actual files");
        }
        VerificationStatus::Failed { mismatches } => {
            println!("❌ Fingerprint verification: FAILED");
            println!("  File-level mismatches:");
            for (file_path, expected, actual) in mismatches {
                println!("    File: {}", file_path);
                println!("      Expected: {}", expected);
                println!("      Actual:   {}", actual);
            }
        }
        VerificationStatus::NoLockFile => {
            println!("⚠️  Fingerprint verification: No lock file found");
        }
        VerificationStatus::NotRequested => {
            // No verification requested, don't show anything
        }
    }
}

/// Show text output format
fn show_text_output(
    fingerprint: &str,
    proto_files: &[ProtoFile],
    verification_status: &VerificationStatus,
) {
    println!("📋 Service Semantic Fingerprint:");
    println!("  {fingerprint}");
    println!("\n📦 Proto Files ({}):", proto_files.len());
    for pf in proto_files {
        println!("  - {}", pf.name);
    }

    // Show verification status
    match verification_status {
        VerificationStatus::Passed {
            matched_fingerprint: _,
        } => {
            println!("\n✅ Fingerprint verification: PASSED");
            println!("  All lock file fingerprints verified against actual files");
        }
        VerificationStatus::Failed { mismatches } => {
            println!("\n❌ Fingerprint verification: FAILED");
            println!("  File-level mismatches:");
            for (file_path, expected, actual) in mismatches {
                println!("    File: {}", file_path);
                println!("      Expected: {}", expected);
                println!("      Actual:   {}", actual);
            }
        }
        VerificationStatus::NoLockFile => {
            println!("\n⚠️  Fingerprint verification: No lock file found");
        }
        VerificationStatus::NotRequested => {
            // No verification requested, don't show anything
        }
    }
}

/// Show JSON output format
fn show_json_output(
    fingerprint: &str,
    proto_files: &[ProtoFile],
    verification_status: &VerificationStatus,
) -> Result<()> {
    let verification_info = match verification_status {
        VerificationStatus::Passed {
            matched_fingerprint,
        } => serde_json::json!({
            "status": "passed",
            "matched_fingerprint": matched_fingerprint
        }),
        VerificationStatus::Failed { mismatches } => serde_json::json!({
            "status": "failed",
            "mismatches": mismatches.iter().map(|(file_path, expected, actual)| {
                serde_json::json!({
                    "file_path": file_path,
                    "expected": expected,
                    "actual": actual
                })
            }).collect::<Vec<_>>()
        }),
        VerificationStatus::NoLockFile => serde_json::json!({
            "status": "no_lock_file"
        }),
        VerificationStatus::NotRequested => serde_json::json!({
            "status": "not_requested"
        }),
    };

    let output = serde_json::json!({
        "service_fingerprint": fingerprint,
        "proto_files": proto_files.iter().map(|pf| pf.name.clone()).collect::<Vec<_>>(),
        "verification": verification_info
    });

    let json = serde_json::to_string_pretty(&output).context("Failed to serialize output")?;
    println!("{json}");

    Ok(())
}

/// Show YAML output format
fn show_yaml_output(
    fingerprint: &str,
    proto_files: &[ProtoFile],
    verification_status: &VerificationStatus,
) -> Result<()> {
    let verification_info = match verification_status {
        VerificationStatus::Passed {
            matched_fingerprint,
        } => serde_yaml::Value::Mapping({
            let mut map = serde_yaml::Mapping::new();
            map.insert(
                serde_yaml::Value::String("status".to_string()),
                serde_yaml::Value::String("passed".to_string()),
            );
            map.insert(
                serde_yaml::Value::String("matched_fingerprint".to_string()),
                serde_yaml::Value::String(matched_fingerprint.clone()),
            );
            map
        }),
        VerificationStatus::Failed { mismatches } => serde_yaml::Value::Mapping({
            let mut map = serde_yaml::Mapping::new();
            map.insert(
                serde_yaml::Value::String("status".to_string()),
                serde_yaml::Value::String("failed".to_string()),
            );
            map.insert(
                serde_yaml::Value::String("mismatches".to_string()),
                serde_yaml::Value::Sequence(
                    mismatches
                        .iter()
                        .map(|(file_path, expected, actual)| {
                            let mut mismatch_map = serde_yaml::Mapping::new();
                            mismatch_map.insert(
                                serde_yaml::Value::String("file_path".to_string()),
                                serde_yaml::Value::String(file_path.clone()),
                            );
                            mismatch_map.insert(
                                serde_yaml::Value::String("expected".to_string()),
                                serde_yaml::Value::String(expected.clone()),
                            );
                            mismatch_map.insert(
                                serde_yaml::Value::String("actual".to_string()),
                                serde_yaml::Value::String(actual.clone()),
                            );
                            serde_yaml::Value::Mapping(mismatch_map)
                        })
                        .collect(),
                ),
            );
            map
        }),
        VerificationStatus::NoLockFile => serde_yaml::Value::Mapping({
            let mut map = serde_yaml::Mapping::new();
            map.insert(
                serde_yaml::Value::String("status".to_string()),
                serde_yaml::Value::String("no_lock_file".to_string()),
            );
            map
        }),
        VerificationStatus::NotRequested => serde_yaml::Value::Mapping({
            let mut map = serde_yaml::Mapping::new();
            map.insert(
                serde_yaml::Value::String("status".to_string()),
                serde_yaml::Value::String("not_requested".to_string()),
            );
            map
        }),
    };

    let output = serde_yaml::Value::Mapping({
        let mut map = serde_yaml::Mapping::new();
        map.insert(
            serde_yaml::Value::String("service_fingerprint".to_string()),
            serde_yaml::Value::String(fingerprint.to_string()),
        );
        map.insert(
            serde_yaml::Value::String("proto_files".to_string()),
            serde_yaml::Value::Sequence(
                proto_files
                    .iter()
                    .map(|pf| serde_yaml::Value::String(pf.name.clone()))
                    .collect(),
            ),
        );
        map.insert(
            serde_yaml::Value::String("verification".to_string()),
            verification_info,
        );
        map
    });

    let yaml = serde_yaml::to_string(&output).context("Failed to serialize output")?;
    println!("{yaml}");

    Ok(())
}

/// Verify fingerprint against lock file
fn verify_fingerprint_against_lock(
    current_fingerprint: &str,
    proto_files: &[ProtoFile],
    config_path: &Path,
) -> Result<VerificationStatus> {
    let lock_path = config_path.with_file_name("actr.lock.toml");
    if !lock_path.exists() {
        return Ok(VerificationStatus::NoLockFile);
    }

    let lock_content = fs::read_to_string(&lock_path)
        .with_context(|| format!("Failed to read lock file: {}", lock_path.display()))?;

    let lock_file: toml::Value = toml::from_str(&lock_content)
        .with_context(|| format!("Failed to parse lock file: {}", lock_path.display()))?;

    let mut mismatches = Vec::new();
    let mut service_fingerprint_mismatch = None;

    // Check service-level fingerprints first
    if let Some(dependencies) = lock_file.get("dependency").and_then(|d| d.as_array()) {
        for dep in dependencies {
            if let Some(expected_service_fp) = dep.get("fingerprint").and_then(|f| f.as_str())
                && expected_service_fp.starts_with("service_semantic:")
            {
                // Use the current fingerprint passed in
                let expected_fp = expected_service_fp.to_string();
                let actual_fp = current_fingerprint.to_string();

                if expected_fp != actual_fp {
                    service_fingerprint_mismatch = Some((expected_fp, actual_fp));
                }
                break; // Only check the first dependency for now
            }
        }
    }

    // Check each proto file from lock file against actual proto files
    if let Some(dependencies) = lock_file.get("dependency").and_then(|d| d.as_array()) {
        for dep in dependencies {
            if let Some(files) = dep.get("files").and_then(|f| f.as_array()) {
                for file in files {
                    if let (Some(lock_path), Some(expected_fp)) = (
                        file.get("path").and_then(|p| p.as_str()),
                        file.get("fingerprint").and_then(|f| f.as_str()),
                    ) {
                        // Empty fingerprints in lock file are considered mismatches
                        if expected_fp.is_empty() {
                            mismatches.push((
                                lock_path.to_string(),
                                expected_fp.to_string(),
                                "ERROR: Empty fingerprint in lock file".to_string(),
                            ));
                            continue;
                        }

                        // Find the corresponding proto file in our proto_files list
                        let mut found = false;
                        for proto_file in proto_files {
                            if let Some(proto_path) = &proto_file.path
                                && proto_path == lock_path
                            {
                                match Fingerprint::calculate_proto_semantic_fingerprint(
                                    &proto_file.content,
                                ) {
                                    Ok(actual_fp) => {
                                        if actual_fp != expected_fp {
                                            mismatches.push((
                                                lock_path.to_string(),
                                                expected_fp.to_string(),
                                                actual_fp,
                                            ));
                                        }
                                    }
                                    Err(e) => {
                                        // Could not calculate fingerprint for this file
                                        mismatches.push((
                                            lock_path.to_string(),
                                            expected_fp.to_string(),
                                            format!("ERROR: {}", e),
                                        ));
                                    }
                                }
                                found = true;
                                break;
                            }
                        }

                        if !found {
                            // Proto file not found in our list
                            mismatches.push((
                                lock_path.to_string(),
                                expected_fp.to_string(),
                                "ERROR: Proto file not found".to_string(),
                            ));
                        }
                    }
                }
            }
        }
    }

    // If there are service fingerprint mismatches, add them to the mismatches list
    if let Some((expected, actual)) = service_fingerprint_mismatch {
        mismatches.push(("SERVICE_FINGERPRINT".to_string(), expected, actual));
    }

    if mismatches.is_empty() {
        // All proto files match
        Ok(VerificationStatus::Passed {
            matched_fingerprint: "all_files_verified".to_string(),
        })
    } else {
        // Some proto files don't match
        Ok(VerificationStatus::Failed { mismatches })
    }
}

/// JSON output structure for proto files
#[derive(serde::Serialize)]
struct ProtoJsonOutput {
    proto_file: String,
    fingerprint: String,
}
