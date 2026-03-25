use crate::commands::SupportedLanguage;
use crate::commands::codegen::proto_model::{MethodModel, ProtoModel, ServiceModel};
use crate::error::{ActrCliError, Result};
use actr_protocol::{ActrType, ActrTypeExt};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const ACTR_GEN_META_FILE: &str = "actr-gen-meta.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActrGenMetadata {
    pub plugin_version: String,
    pub language: String,
    #[serde(default)]
    pub local_services: Vec<LocalServiceMetadata>,
    #[serde(default)]
    pub remote_services: Vec<RemoteServiceMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalServiceMetadata {
    pub name: String,
    pub package: String,
    pub proto_file: String,
    pub handler_interface: String,
    pub workload_type: String,
    pub dispatcher_type: String,
    pub methods: Vec<MethodMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteServiceMetadata {
    pub name: String,
    pub package: String,
    pub proto_file: String,
    pub actr_type: String,
    pub client_type: String,
    pub methods: Vec<MethodMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodMetadata {
    pub name: String,
    pub snake_name: String,
    pub input_type: String,
    pub output_type: String,
    pub route_key: String,
}

impl ActrGenMetadata {
    pub fn from_proto_model(language: SupportedLanguage, proto_model: &ProtoModel) -> Self {
        Self {
            plugin_version: "actr-cli".to_string(),
            language: language_key(language).to_string(),
            local_services: proto_model
                .local_services
                .iter()
                .map(build_local_service_metadata)
                .collect(),
            remote_services: proto_model
                .remote_services
                .iter()
                .map(build_remote_service_metadata)
                .collect(),
        }
    }
}

pub fn metadata_path(output_dir: &Path) -> PathBuf {
    output_dir.join(ACTR_GEN_META_FILE)
}

pub fn load_metadata(output_dir: &Path) -> Result<Option<ActrGenMetadata>> {
    let path = metadata_path(output_dir);
    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&path).map_err(|e| {
        ActrCliError::config_error(format!("Failed to read {}: {e}", path.display()))
    })?;
    let metadata = serde_json::from_str(&content).map_err(|e| {
        ActrCliError::config_error(format!("Failed to parse {}: {e}", path.display()))
    })?;
    Ok(Some(metadata))
}

pub fn write_metadata(output_dir: &Path, metadata: &ActrGenMetadata) -> Result<PathBuf> {
    std::fs::create_dir_all(output_dir).map_err(|e| {
        ActrCliError::config_error(format!(
            "Failed to create metadata output directory {}: {e}",
            output_dir.display()
        ))
    })?;

    let path = metadata_path(output_dir);
    let content = serde_json::to_string_pretty(metadata)?;
    std::fs::write(&path, content).map_err(|e| {
        ActrCliError::config_error(format!("Failed to write {}: {e}", path.display()))
    })?;

    Ok(path)
}

fn language_key(language: SupportedLanguage) -> &'static str {
    match language {
        SupportedLanguage::Rust => "rust",
        SupportedLanguage::Python => "python",
        SupportedLanguage::Swift => "swift",
        SupportedLanguage::Kotlin => "kotlin",
        SupportedLanguage::TypeScript => "typescript",
        SupportedLanguage::Web => "web",
    }
}

fn build_local_service_metadata(service: &ServiceModel) -> LocalServiceMetadata {
    LocalServiceMetadata {
        name: service.name.clone(),
        package: service.package.clone(),
        proto_file: service.relative_path.to_string_lossy().to_string(),
        handler_interface: format!("{}Handler", service.name),
        workload_type: format!("{}Workload", service.name),
        dispatcher_type: format!("{}Dispatcher", service.name),
        methods: service.methods.iter().map(build_method_metadata).collect(),
    }
}

fn build_remote_service_metadata(service: &ServiceModel) -> RemoteServiceMetadata {
    RemoteServiceMetadata {
        name: service.name.clone(),
        package: service.package.clone(),
        proto_file: service.relative_path.to_string_lossy().to_string(),
        actr_type: service.actr_type.clone().unwrap_or_else(|| {
            ActrType {
                manufacturer: "acme".to_string(),
                name: service.name.clone(),
                version: "1.0.0".to_string(),
            }
            .to_string_repr()
        }),
        client_type: format!("{}Client", service.name),
        methods: service.methods.iter().map(build_method_metadata).collect(),
    }
}

fn build_method_metadata(method: &MethodModel) -> MethodMetadata {
    MethodMetadata {
        name: method.name.clone(),
        snake_name: method.snake_name.clone(),
        input_type: method.input_type.clone(),
        output_type: method.output_type.clone(),
        route_key: method.route_key.clone(),
    }
}
