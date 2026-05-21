//! Plugin request/response types for CLI → plugin communication.
//!
//! The CLI serializes a `WebCodegenRequest` as JSON to stdin, and the plugin
//! writes a `WebCodegenResponse` as JSON to stdout.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Complete request from CLI to plugin
#[derive(Debug, Serialize, Deserialize)]
pub struct WebCodegenRequest {
    /// Path to actr.toml (used for reading raw TOML extras)
    pub config_path: PathBuf,
    /// Output directory for generated TS files (e.g. src/generated)
    pub output_dir: PathBuf,
    /// Project root directory (parent of src/)
    pub project_root: PathBuf,
    /// Whether already-existing user code may be overwritten
    pub overwrite_user_code: bool,

    // ── Package info ──
    pub package_name: String,
    pub manufacturer: String,
    pub actr_name: String,
    #[serde(default)]
    pub version: String,
    pub description: String,
    pub authors: Vec<String>,
    pub license: String,
    pub tags: Vec<String>,

    // ── System config ──
    pub signaling_url: String,
    pub realm_id: u32,
    pub visible_in_discovery: bool,

    /// AIS (Actor Identity Service) HTTP endpoint
    #[serde(default)]
    pub ais_endpoint: String,

    /// Whether to force relay (TURN-only) for ICE transport
    #[serde(default)]
    pub force_relay: bool,

    // ── Dependencies ──
    pub dependencies: Vec<DependencyInfo>,

    // ── WebRTC ──
    pub stun_urls: Vec<String>,
    pub turn_urls: Vec<String>,

    // ── Observability ──
    pub observability: ObservabilityInfo,

    // ── Raw TOML (for edition, platform.web, acl, etc.) ──
    pub raw_toml: String,

    // ── Proto model ──
    pub local_services: Vec<ServiceInfo>,
    pub remote_services: Vec<ServiceInfo>,
    pub files: Vec<FileInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DependencyInfo {
    pub alias: String,
    pub actr_type: Option<ActrTypeInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ActrTypeInfo {
    pub manufacturer: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ObservabilityInfo {
    pub filter_level: String,
    pub tracing_enabled: bool,
    pub tracing_endpoint: String,
    pub tracing_service_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub name: String,
    pub package: String,
    pub relative_path: PathBuf,
    pub methods: Vec<MethodInfo>,
    pub actr_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MethodInfo {
    pub name: String,
    pub snake_name: String,
    pub input_type: String,
    pub output_type: String,
    pub route_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileInfo {
    pub proto_file: PathBuf,
    pub relative_path: PathBuf,
    pub package: String,
    pub is_local: bool,
    pub declared_type_names: Vec<String>,
}

/// Response from plugin to CLI
#[derive(Debug, Serialize, Deserialize)]
pub struct WebCodegenResponse {
    pub success: bool,
    pub generated_files: Vec<PathBuf>,
    pub errors: Vec<String>,
}

impl WebCodegenRequest {
    /// Whether this project exports services only and does not call out to
    /// any remote actor. Used by the WASM scaffold generator to decide
    /// whether to emit a service handler (provider) or a forwarding stub.
    ///
    /// Note: this is purely a codegen topology signal derived from the
    /// proto model — it does NOT imply a passive connection role. Any
    /// actor can be called by any other actor; connection initiator /
    /// acceptor roles are negotiated per-peer at runtime.
    pub fn is_service_provider_only(&self) -> bool {
        !self.local_services.is_empty()
            && self.remote_services.is_empty()
            && self.dependencies.is_empty()
    }

    /// Get ACL allow types from raw TOML
    pub fn get_acl_allow_types(&self) -> Vec<String> {
        let raw_table: toml::Table = self.raw_toml.parse().unwrap_or_default();
        let mut types = Vec::new();
        if let Some(acl) = raw_table.get("acl") {
            if let Some(rules) = acl.get("rules").and_then(|r| r.as_array()) {
                for rule in rules {
                    if let Some(rule_types) = rule.get("types").and_then(|t| t.as_array()) {
                        for t in rule_types {
                            if let Some(s) = t.as_str() {
                                types.push(s.to_string());
                            }
                        }
                    }
                }
            }
        }
        types
    }

    /// Get the target actr type for peer discovery (manufacturer:name:version).
    ///
    /// For an actor that imports a remote service, the target is the
    /// declared dependency. For an actor that only exports services, the
    /// target is the first ACL-allowed type — useful for RPCs initiated
    /// from that actor side when it needs to reach a caller by type.
    pub fn target_actr_type(&self) -> String {
        if self.is_service_provider_only() {
            self.get_acl_allow_types()
                .first()
                .cloned()
                .unwrap_or_default()
        } else {
            self.dependencies
                .first()
                .and_then(|d| {
                    d.actr_type
                        .as_ref()
                        .map(|t| format!("{}:{}:{}", t.manufacturer, t.name, t.version))
                })
                .unwrap_or_default()
        }
    }

    /// Get client actr type (this actor's type) — manufacturer:name:version
    pub fn client_actr_type(&self) -> String {
        format!("{}:{}:{}", self.manufacturer, self.actr_name, self.version)
    }

    /// Get crate/WASM module name (snake_case)
    pub fn wasm_module_name(&self) -> String {
        to_snake_case(&self.package_name).replace('-', "_")
    }

    /// Get edition from raw TOML
    pub fn edition(&self) -> i64 {
        let raw_table: toml::Table = self.raw_toml.parse().unwrap_or_default();
        raw_table
            .get("edition")
            .and_then(|v| v.as_integer())
            .unwrap_or(1)
    }

    /// Get exports from raw TOML
    pub fn exports_list(&self) -> Vec<String> {
        let raw_table: toml::Table = self.raw_toml.parse().unwrap_or_default();
        raw_table
            .get("exports")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get platform.web from raw TOML
    pub fn platform_web(&self) -> Option<toml::Value> {
        let raw_table: toml::Table = self.raw_toml.parse().unwrap_or_default();
        raw_table
            .get("platform")
            .and_then(|v| v.get("web"))
            .cloned()
    }

    /// Get raw ACL value from TOML
    pub fn raw_acl(&self) -> Option<toml::Value> {
        let raw_table: toml::Table = self.raw_toml.parse().unwrap_or_default();
        raw_table.get("acl").cloned()
    }
}

fn to_snake_case(name: &str) -> String {
    use heck::ToSnakeCase;
    name.to_snake_case()
}
