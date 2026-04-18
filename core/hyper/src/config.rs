#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

#[cfg(not(target_arch = "wasm32"))]
use crate::error::{HyperError, HyperResult};
#[cfg(not(target_arch = "wasm32"))]
use crate::verify::TrustProvider;

/// Default storage path template: `{data_dir}/{actr_type}`.
#[cfg(not(target_arch = "wasm32"))]
const DEFAULT_STORAGE_TEMPLATE: &str = "{data_dir}/{actr_type}";

/// Hyper initialization configuration.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct HyperConfig {
    /// Root data directory, corresponds to the namespace template variable `{data_dir}`
    pub data_dir: PathBuf,

    /// Storage namespace path template, defaults to `{data_dir}/{actr_type}`
    ///
    /// Available variables:
    /// - `{data_dir}`      — root data directory
    /// - `{instance_id}`   — locally unique ID generated and persisted at Hyper startup
    /// - `{hostname}`      — OS hostname
    /// - `{manufacturer}`  — Actor manufacturer name
    /// - `{actr_name}`     — Actor name
    /// - `{version}`       — Actor version
    /// - `{actr_type}`     — full three-part type (`{manufacturer}/{actr_name}/{version}`)
    /// - `{realm_id}`      — Actor's realm (available at runtime)
    /// - `{env.VAR}`       — any environment variable
    pub storage_path_template: String,

    /// Pluggable package-signature verifier. Replaces the old `TrustMode` enum.
    ///
    /// Construct via [`crate::verify::StaticTrust`], [`crate::verify::RegistryTrust`],
    /// or [`crate::verify::ChainTrust`] (or bring your own).
    pub trust_provider: Arc<dyn TrustProvider>,
}

#[cfg(not(target_arch = "wasm32"))]
impl std::fmt::Debug for HyperConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HyperConfig")
            .field("data_dir", &self.data_dir)
            .field("storage_path_template", &self.storage_path_template)
            .field("trust_provider", &self.trust_provider)
            .finish()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl HyperConfig {
    /// Build a new HyperConfig with the given `data_dir` and package trust provider.
    ///
    /// There is no default provider — you must explicitly decide how packages
    /// are authenticated (see [`crate::verify::StaticTrust`] /
    /// [`crate::verify::RegistryTrust`] / [`crate::verify::ChainTrust`]).
    pub fn new(data_dir: impl AsRef<Path>, trust_provider: Arc<dyn TrustProvider>) -> Self {
        Self {
            data_dir: data_dir.as_ref().to_path_buf(),
            storage_path_template: DEFAULT_STORAGE_TEMPLATE.to_string(),
            trust_provider,
        }
    }

    pub fn with_storage_template(mut self, template: impl Into<String>) -> Self {
        self.storage_path_template = template.into();
        self
    }

    pub fn with_trust_provider(mut self, trust_provider: Arc<dyn TrustProvider>) -> Self {
        self.trust_provider = trust_provider;
        self
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Namespace template resolver
///
/// Holds runtime-known variables and resolves path templates on demand.
/// Templates are resolved once during Hyper initialization and remain fixed afterwards.
pub(crate) struct NamespaceResolver {
    vars: HashMap<String, String>,
}

#[cfg(not(target_arch = "wasm32"))]
impl NamespaceResolver {
    pub fn new(config: &HyperConfig, instance_id: &str) -> HyperResult<Self> {
        let mut vars = HashMap::new();

        vars.insert(
            "data_dir".to_string(),
            config
                .data_dir
                .to_str()
                .ok_or_else(|| {
                    HyperError::Config("data_dir path contains non-UTF-8 characters".to_string())
                })?
                .to_string(),
        );
        vars.insert("instance_id".to_string(), instance_id.to_string());

        if let Ok(hostname) = std::env::var("HOSTNAME").or_else(|_| {
            // fallback: read system hostname
            std::fs::read_to_string("/etc/hostname")
                .map(|s| s.trim().to_string())
                .map_err(|_| std::env::VarError::NotPresent)
        }) {
            vars.insert("hostname".to_string(), hostname);
        }

        Ok(Self { vars })
    }

    /// Inject Actor type variables (extracted from the verified manifest)
    pub fn with_actor_type(mut self, manufacturer: &str, actr_name: &str, version: &str) -> Self {
        self.vars
            .insert("manufacturer".to_string(), manufacturer.to_string());
        self.vars
            .insert("actr_name".to_string(), actr_name.to_string());
        self.vars.insert("version".to_string(), version.to_string());
        self.vars.insert(
            "actr_type".to_string(),
            format!("{manufacturer}/{actr_name}/{version}"),
        );
        self
    }

    /// Inject runtime realm_id
    #[allow(dead_code)]
    pub fn with_realm(mut self, realm_id: u64) -> Self {
        self.vars
            .insert("realm_id".to_string(), realm_id.to_string());
        self
    }

    /// Resolve a template string, returning the final path
    pub fn resolve(&self, template: &str) -> HyperResult<PathBuf> {
        let mut result = template.to_string();

        // Handle {env.VAR} variables
        let env_prefix = "{env.";
        let mut pos = 0;
        while let Some(start) = result[pos..].find(env_prefix) {
            let abs_start = pos + start;
            if let Some(end) = result[abs_start..].find('}') {
                let var_name = &result[abs_start + env_prefix.len()..abs_start + end];
                let value = std::env::var(var_name)
                    .map_err(|_| HyperError::TemplateVariable(format!("env.{var_name}")))?;
                let placeholder = format!("{{env.{var_name}}}");
                result = result.replacen(&placeholder, &value, 1);
                // do not advance position, re-scan the replaced string
            } else {
                pos = abs_start + 1;
            }
        }

        // Handle regular variables
        for (key, value) in &self.vars {
            result = result.replace(&format!("{{{key}}}"), value);
        }

        // Check for unresolved variables
        if let Some(start) = result.find('{') {
            if let Some(end) = result[start..].find('}') {
                let var = &result[start + 1..start + end];
                return Err(HyperError::TemplateVariable(var.to_string()));
            }
        }

        Ok(PathBuf::from(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::StaticTrust;

    fn stub_config(data_dir: &str) -> HyperConfig {
        HyperConfig::new(data_dir, Arc::new(StaticTrust::new([0u8; 32]).unwrap()))
    }

    #[test]
    fn resolve_basic_template() {
        let config = stub_config("/var/lib/actr");
        let resolver = NamespaceResolver::new(&config, "abc123")
            .unwrap()
            .with_actor_type("acme", "Sensor", "1.0.0");

        let path = resolver.resolve("{data_dir}/{actr_type}").unwrap();
        assert_eq!(path, PathBuf::from("/var/lib/actr/acme/Sensor/1.0.0"));
    }

    #[test]
    fn resolve_missing_var_returns_error() {
        let config = stub_config("/tmp");
        let resolver = NamespaceResolver::new(&config, "id1").unwrap();
        let result = resolver.resolve("{data_dir}/{realm_id}");
        assert!(matches!(result, Err(HyperError::TemplateVariable(_))));
    }

    #[test]
    fn resolve_with_realm() {
        let config = stub_config("/tmp");
        let resolver = NamespaceResolver::new(&config, "id1")
            .unwrap()
            .with_actor_type("acme", "Worker", "2.0")
            .with_realm(42);
        let path = resolver
            .resolve("{data_dir}/{actr_type}/{realm_id}")
            .unwrap();
        assert_eq!(path, PathBuf::from("/tmp/acme/Worker/2.0/42"));
    }
}
