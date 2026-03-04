//! Fingerprint command implementation
//!
//! Computes and displays semantic fingerprints for proto files

use crate::error::Result;
use actr::config::ConfigParser;
use actr_version::{Fingerprint, ProtoFile};
use anyhow::Context;
use clap::Args;
use std::path::Path;
use tracing::{error, info};

/// Fingerprint command arguments
#[derive(Debug, Args)]
pub struct FingerprintArgs {
    /// Configuration file path
    #[arg(short, long, default_value = "Actr.toml")]
    pub config: String,

    /// Output format (text or json)
    #[arg(long, default_value = "text")]
    pub format: String,
}

/// Execute fingerprint command
pub async fn execute(args: FingerprintArgs) -> Result<()> {
    info!("🔍 Computing service semantic fingerprint...");

    // Load configuration
    let config_path = Path::new(&args.config);
    let config = ConfigParser::from_file(config_path)
        .with_context(|| format!("Failed to load config from {}", args.config))?;

    // Convert actr::config::ProtoFile to actr::version::ProtoFile
    let proto_files: Vec<ProtoFile> = config
        .exports
        .iter()
        .map(|pf| ProtoFile {
            name: pf.file_name().unwrap_or("unknown.proto").to_string(),
            content: pf.content.clone(),
            path: Some(pf.path.to_string_lossy().to_string()),
        })
        .collect();

    if proto_files.is_empty() {
        info!("ℹ️  No proto files found in exports");
        return Ok(());
    }

    // Calculate semantic fingerprint
    let fingerprint = Fingerprint::calculate_service_semantic_fingerprint(&proto_files)
        .context("Failed to calculate service fingerprint")?;

    // Output
    match args.format.as_str() {
        "text" => show_text_output(&fingerprint, &proto_files),
        "json" => show_json_output(&fingerprint, &proto_files)?,
        _ => {
            error!("Unsupported output format: {}", args.format);
            return Err(anyhow::anyhow!("Unsupported format: {}", args.format).into());
        }
    }

    Ok(())
}

/// Show text output format
fn show_text_output(fingerprint: &str, proto_files: &[ProtoFile]) {
    println!("📋 Service Semantic Fingerprint:");
    println!("  {fingerprint}");
    println!("\n📦 Proto Files ({}):", proto_files.len());
    for pf in proto_files {
        println!("  - {}", pf.name);
    }
}

/// Show JSON output format
fn show_json_output(fingerprint: &str, proto_files: &[ProtoFile]) -> Result<()> {
    let output = JsonOutput {
        service_fingerprint: fingerprint.to_string(),
        proto_files: proto_files.iter().map(|pf| pf.name.clone()).collect(),
    };

    let json = serde_json::to_string_pretty(&output).context("Failed to serialize output")?;
    println!("{json}");

    Ok(())
}

/// JSON output structure
#[derive(serde::Serialize)]
struct JsonOutput {
    service_fingerprint: String,
    proto_files: Vec<String>,
}
