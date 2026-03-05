//! Default ProtoProcessor implementation

use anyhow::Result;
use async_trait::async_trait;
use std::path::Path;

use super::{GenerationResult, ProtoFile, ProtoProcessor, ServiceDefinition, ValidationReport};

/// Default proto processor
pub struct DefaultProtoProcessor;

impl DefaultProtoProcessor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DefaultProtoProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ProtoProcessor for DefaultProtoProcessor {
    async fn discover_proto_files(&self, path: &Path) -> Result<Vec<ProtoFile>> {
        let mut files = Vec::new();
        if path.is_dir() {
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().map(|e| e == "proto").unwrap_or(false) {
                    let content = std::fs::read_to_string(&path)?;
                    files.push(ProtoFile {
                        name: path.file_name().unwrap().to_string_lossy().to_string(),
                        path,
                        content,
                        services: Vec::new(),
                    });
                }
            }
        }
        Ok(files)
    }

    async fn parse_proto_services(&self, _files: &[ProtoFile]) -> Result<Vec<ServiceDefinition>> {
        // Simple stub - in a real implementation, parse the proto files
        Ok(Vec::new())
    }

    async fn generate_code(&self, _input: &Path, output: &Path) -> Result<GenerationResult> {
        // Stub implementation
        Ok(GenerationResult {
            generated_files: vec![output.to_path_buf()],
            warnings: Vec::new(),
            errors: Vec::new(),
        })
    }

    async fn validate_proto_syntax(&self, _files: &[ProtoFile]) -> Result<ValidationReport> {
        // Return a valid report with no issues
        Ok(ValidationReport {
            is_valid: true,
            config_validation: super::ConfigValidation {
                is_valid: true,
                errors: Vec::new(),
                warnings: Vec::new(),
            },
            dependency_validation: Vec::new(),
            network_validation: Vec::new(),
            fingerprint_validation: Vec::new(),
            conflicts: Vec::new(),
        })
    }
}
