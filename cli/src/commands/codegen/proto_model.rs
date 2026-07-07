use crate::error::{ActrCliError, Result};
use crate::utils::to_snake_case;
use actr_config::ManifestConfig;
use actr_protocol::ActrType;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtoSide {
    Local,
    Remote,
}

#[derive(Debug, Clone)]
pub struct ProtoModel {
    pub files: Vec<ProtoFileModel>,
    pub local_services: Vec<ServiceModel>,
    pub remote_services: Vec<ServiceModel>,
}

#[derive(Debug, Clone)]
pub struct ProtoFileModel {
    pub proto_file: PathBuf,
    pub relative_path: PathBuf,
    pub package: String,
    pub side: ProtoSide,
    pub declared_type_names: Vec<String>,
    pub services: Vec<ServiceModel>,
}

#[derive(Debug, Clone)]
pub struct ServiceModel {
    pub name: String,
    pub package: String,
    pub proto_file: PathBuf,
    pub relative_path: PathBuf,
    pub side: ProtoSide,
    pub methods: Vec<MethodModel>,
    pub actr_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MethodModel {
    pub name: String,
    pub snake_name: String,
    pub input_type: String,
    pub output_type: String,
    pub route_key: String,
}

/// Declaring owner of a proto message/enum type.
///
/// `proto_file` is the path relative to the proto root (matching
/// `ProtoFileModel::relative_path`), so it stays stable across the metadata,
/// scaffold catalog, and language generators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeOwner {
    pub proto_package: String,
    pub proto_file: String,
    pub type_name: String,
}

/// Index mapping declared message/enum/service type names to the proto file
/// that declares them, built from a fully parsed [`ProtoModel`].
///
/// Resolution rules (see `resolve`):
/// - fully-qualified `package.Type` → exact match on the declaring file;
/// - unqualified `Type` → prefer the current proto file, then a unique
///   declaring file;
/// - an unqualified name declared in several non-current files is ambiguous
///   and surfaces as an error so the codegen does not silently pick the wrong
///   owner;
/// - types that cannot be resolved at all (well-known types, external imports
///   not part of the local proto set, nested messages) fall back to the
///   current service's file so existing behaviour is preserved.
#[derive(Debug, Clone, Default)]
pub struct TypeOwnerIndex {
    qualified: HashMap<String, TypeOwner>,
    bare: HashMap<String, Vec<TypeOwner>>,
}

impl TypeOwnerIndex {
    pub fn from_files(files: &[ProtoFileModel]) -> Self {
        let mut index = Self::default();
        for file in files {
            for declared in &file.declared_type_names {
                let owner = TypeOwner {
                    proto_package: file.package.clone(),
                    proto_file: file.relative_path.to_string_lossy().to_string(),
                    type_name: declared.clone(),
                };
                if !file.package.is_empty() {
                    index
                        .qualified
                        .insert(format!("{}.{}", file.package, declared), owner.clone());
                }
                index.bare.entry(declared.clone()).or_default().push(owner);
            }
        }
        index
    }

    /// Resolve `referenced` (a proto type string as written in an RPC
    /// signature, e.g. `ask.ContinuePromptResultStreamsRequest` or
    /// `EchoRequest`) against the index.
    ///
    /// Returns `Ok(Some(owner))` when the declaring file is known, `Ok(None)`
    /// when the type cannot be resolved (caller falls back to the current
    /// service), or `Err(candidates)` when an unqualified type is ambiguous.
    pub fn resolve(
        &self,
        referenced: &str,
        current: &ProtoFileModel,
    ) -> std::result::Result<Option<TypeOwner>, Vec<TypeOwner>> {
        let normalized = normalize_proto_type(referenced);
        if normalized.is_empty() {
            return Ok(None);
        }

        if let Some(owner) = self.qualified.get(&normalized) {
            return Ok(Some(owner.clone()));
        }

        if normalized.contains('.') {
            return Ok(None);
        }

        let type_name = normalized
            .rsplit('.')
            .next()
            .unwrap_or(&normalized)
            .to_string();

        if current.declared_type_names.contains(&type_name) {
            return Ok(Some(TypeOwner {
                proto_package: current.package.clone(),
                proto_file: current.relative_path.to_string_lossy().to_string(),
                type_name,
            }));
        }

        match self.bare.get(&type_name) {
            None => Ok(None),
            Some(candidates) if candidates.len() == 1 => Ok(Some(candidates[0].clone())),
            Some(candidates) => Err(candidates.clone()),
        }
    }
}

impl ProtoModel {
    pub fn parse(
        proto_files: &[PathBuf],
        input_path: &Path,
        config: &ManifestConfig,
    ) -> Result<Self> {
        let proto_root = if input_path.is_file() {
            input_path.parent().unwrap_or_else(|| Path::new("."))
        } else {
            input_path
        };

        let dependency_actr_types: HashMap<String, String> = config
            .dependencies
            .iter()
            .filter_map(|dependency| {
                dependency
                    .actr_type
                    .as_ref()
                    .map(|actr_type| (dependency.alias.clone(), actr_type.to_string_repr()))
            })
            .collect();

        let default_manufacturer = config.package.actr_type.manufacturer.clone();

        let mut files = Vec::new();
        let mut local_services = Vec::new();
        let mut remote_services = Vec::new();

        for proto_file in proto_files {
            let relative_path = proto_file
                .strip_prefix(proto_root)
                .unwrap_or(proto_file)
                .to_path_buf();
            // Reject paths that escape the proto root: a `..` component would
            // let a crafted proto file path (e.g. from a remote dependency)
            // inject traversal sequences into generated import/module paths.
            if relative_path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                return Err(ActrCliError::config_error(format!(
                    "proto file path escapes the proto root: {}",
                    proto_file.display()
                )));
            }
            let side = classify_proto_side(&relative_path);
            let parsed = parse_proto_file(proto_file)?;

            let remote_actr_type = if side == ProtoSide::Remote {
                infer_remote_actr_type(
                    &relative_path,
                    &dependency_actr_types,
                    &default_manufacturer,
                    parsed.services.first().map(|service| service.name.as_str()),
                )
            } else {
                None
            };

            let services: Vec<ServiceModel> = parsed
                .services
                .into_iter()
                .map(|service| {
                    let service_model = ServiceModel {
                        name: service.name,
                        package: parsed.package.clone(),
                        proto_file: proto_file.clone(),
                        relative_path: relative_path.clone(),
                        side,
                        methods: service.methods,
                        actr_type: remote_actr_type.clone(),
                    };

                    if side == ProtoSide::Local {
                        local_services.push(service_model.clone());
                    } else {
                        remote_services.push(service_model.clone());
                    }

                    service_model
                })
                .collect();

            files.push(ProtoFileModel {
                proto_file: proto_file.clone(),
                relative_path,
                package: parsed.package,
                side,
                declared_type_names: parsed.declared_type_names,
                services,
            });
        }

        Ok(Self {
            files,
            local_services,
            remote_services,
        })
    }
}

#[derive(Debug)]
struct ParsedProtoFile {
    package: String,
    declared_type_names: Vec<String>,
    services: Vec<ParsedService>,
}

#[derive(Debug)]
struct ParsedService {
    name: String,
    methods: Vec<MethodModel>,
}

fn classify_proto_side(relative_path: &Path) -> ProtoSide {
    let first_component = relative_path
        .components()
        .next()
        .and_then(|component| component.as_os_str().to_str());

    if first_component == Some("remote") {
        ProtoSide::Remote
    } else {
        ProtoSide::Local
    }
}

fn infer_remote_actr_type(
    relative_path: &Path,
    dependency_actr_types: &HashMap<String, String>,
    default_manufacturer: &str,
    service_name: Option<&str>,
) -> Option<String> {
    let alias = relative_path
        .components()
        .nth(1)
        .and_then(|component| component.as_os_str().to_str());

    if let Some(alias) = alias
        && let Some(actr_type) = dependency_actr_types.get(alias)
    {
        return Some(actr_type.clone());
    }

    service_name.map(|service_name| {
        ActrType {
            manufacturer: default_manufacturer.to_string(),
            name: service_name.to_string(),
            version: "1.0.0".to_string(),
        }
        .to_string_repr()
    })
}

fn parse_proto_file(proto_file: &Path) -> Result<ParsedProtoFile> {
    let content = std::fs::read_to_string(proto_file).map_err(|e| {
        ActrCliError::config_error(format!(
            "Failed to read proto file {}: {e}",
            proto_file.display()
        ))
    })?;

    let mut package = String::new();
    let mut declared_type_names = Vec::new();
    let mut current_service: Option<ParsedService> = None;
    let mut services = Vec::new();

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }

        if let Some(rest) = line.strip_prefix("package ") {
            package = rest
                .trim_end_matches(';')
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_string();
            continue;
        }

        if let Some(rest) = line.strip_prefix("service ") {
            if let Some(service) = current_service.take() {
                services.push(service);
            }

            let name = rest
                .split(|character: char| character.is_whitespace() || character == '{')
                .find(|segment| !segment.is_empty())
                .unwrap_or_default()
                .to_string();

            if !name.is_empty() {
                declared_type_names.push(name.clone());
                current_service = Some(ParsedService {
                    name,
                    methods: Vec::new(),
                });
            }
            continue;
        }

        if let Some(name) = extract_declared_type_name(line, "message ") {
            declared_type_names.push(name);
            continue;
        }

        if let Some(name) = extract_declared_type_name(line, "enum ") {
            declared_type_names.push(name);
            continue;
        }

        if let Some(rest) = line.strip_prefix("rpc ")
            && let Some(service) = current_service.as_mut()
        {
            if let Some(method) = parse_rpc_method(rest, &package, &service.name) {
                service.methods.push(method);
            }
            continue;
        }

        if line.starts_with('}')
            && let Some(service) = current_service.take()
        {
            services.push(service);
        }
    }

    if let Some(service) = current_service.take() {
        services.push(service);
    }

    Ok(ParsedProtoFile {
        package,
        declared_type_names,
        services,
    })
}

fn parse_rpc_method(rest: &str, package: &str, service_name: &str) -> Option<MethodModel> {
    let input_start = rest.find('(')?;
    let method_name = rest[..input_start]
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();
    if method_name.is_empty() {
        return None;
    }

    let after_input_start = &rest[input_start + 1..];
    let input_end = after_input_start.find(')')?;
    let input_type = normalize_proto_type(&after_input_start[..input_end]);

    let returns_pos = after_input_start.find("returns")?;
    let after_returns = &after_input_start[returns_pos + "returns".len()..];
    let output_start = after_returns.find('(')?;
    let output_end = after_returns[output_start + 1..].find(')')?;
    let output_type =
        normalize_proto_type(&after_returns[output_start + 1..output_start + 1 + output_end]);

    let route_key = if package.is_empty() {
        format!("{service_name}.{method_name}")
    } else {
        format!("{package}.{service_name}.{method_name}")
    };

    Some(MethodModel {
        snake_name: to_snake_case(&method_name),
        name: method_name,
        input_type,
        output_type,
        route_key,
    })
}

fn normalize_proto_type(raw_type: &str) -> String {
    raw_type.trim().trim_start_matches('.').to_string()
}

fn extract_declared_type_name(line: &str, prefix: &str) -> Option<String> {
    let rest = line.strip_prefix(prefix)?;
    let name = rest
        .split(|character: char| character.is_whitespace() || character == '{')
        .find(|segment| !segment.is_empty())?;
    Some(name.to_string())
}

#[cfg(test)]
#[path = "proto_model_tests.rs"]
mod tests;
