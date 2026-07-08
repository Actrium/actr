#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

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

    /// How far in advance of expiry the framework should fire the
    /// `on_credential_expiring` hook.
    ///
    /// Default: 5 minutes.
    pub credential_expiry_warning: Duration,

    /// Queue-length trip point for the `on_mailbox_backpressure` hook.
    ///
    /// When `Some(threshold)`, the hook fires once per incident as soon as
    /// the mailbox queued-message count crosses `threshold`, and re-arms
    /// once the queue falls back below. When `None`, a built-in default of
    /// [`DEFAULT_MAILBOX_BACKPRESSURE_THRESHOLD`] messages is used. The
    /// mailbox trait currently exposes `status()` which reports
    /// queued_messages, so the polling-based implementation in
    /// `lifecycle::node` works against any mailbox backend that supports
    /// the base trait.
    pub mailbox_backpressure_threshold: Option<usize>,

    /// Conflict-key dispatch concurrency (B2). `None` resolves to
    /// [`DispatchConcurrency::default`], whose gate is now **on** (strategy A).
    /// Default-on is free for a keyless actor: with no declared conflict key the
    /// node keeps it bit-for-bit serial and never spawns a scheduler, so the
    /// resident concurrency machinery only engages once a method declares a
    /// conflict key on a workload that can multiplex (native `Linked` /
    /// `Wasm(V2)`). Pass `Some(DispatchConcurrency { enabled: false, .. })` (or
    /// set `ACTR_DISPATCH_SERIAL=1`) to force the fully-serial B1 runner. See
    /// [`DispatchConcurrency`].
    pub dispatch_concurrency: Option<DispatchConcurrency>,
}

/// Conflict-key dispatch concurrency knobs (design doc §4.2, §7).
///
/// The gate defaults **on** (strategy A). Two orthogonal safety nets keep that
/// free and safe:
///
/// * **keyless zero-overhead** — an actor that declares no conflict key is kept
///   fully serial by the node, which does not even spawn a dispatch scheduler
///   for it (it stays on the M4 per-dispatch `run_loop`). Default-on therefore
///   costs a keyless actor nothing.
/// * **undeclared = global barrier** — when a scheduler *is* running (some
///   method declared a key), every *undeclared* method still projects to the
///   global [`crate::dispatch::ConflictKey::Serial`] barrier, so an unkeyed
///   route can never interleave. These two nets are independent of the gate's
///   default value.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchConcurrency {
    /// Master switch. Default `true` (strategy A default-on). A keyless actor is
    /// unaffected (kept serial with no scheduler); set `false` (or export
    /// `ACTR_DISPATCH_SERIAL=1`) to force the fully-serial B1 runner everywhere.
    pub enabled: bool,
    /// `C` — maximum number of dispatches in flight at once. Default `8`.
    pub budget: usize,
    /// `M` — total in-queue + in-flight bound; a full queue applies
    /// back-pressure up to the node entry loop. Default `256`.
    pub queue_cap: usize,
    /// Per-dispatch deadline. `None` (default) = no deadline, exactly the M4
    /// behaviour. When set, the v2 interleaved wasm runner arms this deadline
    /// on every in-flight dispatch; on expiry the dispatch's in-flight
    /// `call_concurrent` future is dropped (a CLEAN cancel that does not
    /// poison the store — validated by the M0 spike's T8/R1), the caller's
    /// reply resolves `TimedOut`, and the freed conflict key lets the next
    /// same-key dispatch start without a serial violation. It is deliberately
    /// enforced *inside* the resident region (a sibling of the guest-awaiting
    /// future in the `select!`), not from an external timer, so the reply is
    /// sent only *after* the timed-out dispatch has truly left the region —
    /// which is what keeps same-key FIFO airtight across a timeout.
    ///
    /// Only in effect on the interleaved runner — a keyed actor with the gate
    /// on. A keyless (serial) actor runs each dispatch to completion on the
    /// pre-B2 loop and does not consult this field.
    pub dispatch_timeout: Option<std::time::Duration>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for DispatchConcurrency {
    fn default() -> Self {
        DispatchConcurrency {
            // Default-on (strategy A): the gate defaults *on*, but an actor that
            // declares no conflict key stays bit-for-bit serial (the node never
            // spawns a scheduler for it — see `lifecycle::node`). So turning the
            // gate on by default costs a keyless actor nothing; concurrency only
            // materializes once a method declares a conflict key.
            enabled: true,
            budget: 8,
            queue_cap: 256,
            dispatch_timeout: None,
        }
    }
}

/// Env escape hatch: `ACTR_DISPATCH_SERIAL=1` (or `true`) forces dispatch fully
/// serial regardless of config. Pure function of the raw env string so it can
/// be unit-tested without touching process env.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn dispatch_serial_env_override(raw: Option<&str>) -> bool {
    matches!(
        raw.map(|s| s.trim()),
        Some("1") | Some("true") | Some("TRUE") | Some("yes")
    )
}

/// Default mailbox backpressure threshold (queued-message count).
///
/// Chosen conservatively — most Actor-RTC workloads are below this in
/// steady state, so a warning at this level means queue growth needs
/// attention. Tune per-actor via
/// [`HyperConfig::mailbox_backpressure_threshold`].
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const DEFAULT_MAILBOX_BACKPRESSURE_THRESHOLD: usize = 1024;

/// Default credential-expiry warning lead time.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const DEFAULT_CREDENTIAL_EXPIRY_WARNING: Duration = Duration::from_secs(5 * 60);

#[cfg(not(target_arch = "wasm32"))]
impl std::fmt::Debug for HyperConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HyperConfig")
            .field("data_dir", &self.data_dir)
            .field("storage_path_template", &self.storage_path_template)
            .field("trust_provider", &self.trust_provider)
            .field("credential_expiry_warning", &self.credential_expiry_warning)
            .field(
                "mailbox_backpressure_threshold",
                &self.mailbox_backpressure_threshold,
            )
            .field("dispatch_concurrency", &self.dispatch_concurrency)
            .finish()
    }
}

/// Raw TOML shape for the optional `[hyper]` section of `actr.toml`.
///
/// All fields optional; each controls one knob of [`HyperConfig`]. The
/// trust anchor lives inside `[hyper.trust]` (singular) and uses the
/// same tag-dispatched schema as the top-level `[[trust]]` array so users
/// can pick whichever style they prefer.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(crate) struct HyperSection {
    /// Root data directory (`{data_dir}` template variable).
    #[serde(default)]
    pub data_dir: Option<std::path::PathBuf>,

    /// Override the default `{data_dir}/{actr_type}` storage template.
    #[serde(default)]
    pub storage_path_template: Option<String>,

    /// Single trust anchor; for chain composition use the top-level
    /// `[[trust]]` array instead.
    #[serde(default)]
    pub trust: Option<HyperTrustAnchor>,
}

/// Trust anchor config for `[hyper.trust]`. Superset of
/// `actr_config::TrustAnchor` with an extra `dev_only` kind that lets
/// tests and examples opt in to [`crate::verify::StaticTrust::dev_only`]
/// explicitly.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum HyperTrustAnchor {
    /// Accept any package — for tests and local development **only**.
    ///
    /// When selected by `Node::from_config_file`, emits a prominent
    /// `tracing::warn!` at load time.
    DevOnly,
    /// Pre-shared Ed25519 public key. See
    /// [`actr_config::TrustAnchor::Static`] for field semantics.
    Static {
        #[serde(default)]
        pubkey_file: Option<std::path::PathBuf>,
        #[serde(default)]
        pubkey_b64: Option<String>,
    },
    /// AIS HTTP registry endpoint. See
    /// [`actr_config::TrustAnchor::Registry`].
    Registry { endpoint: String },
}

/// Top-level wrapper used when parsing `actr.toml` for the `[hyper]`
/// section in isolation. Ignores every other field.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(crate) struct HyperSectionWrapper {
    #[serde(default)]
    pub hyper: HyperSection,
}

/// Load an `actr.toml`-style file and return a [`crate::Node`] in the
/// `Init` state. This function handles the full pipeline:
///
/// 1. Parse the runtime section via [`actr_config::ConfigParser`].
/// 2. Parse the optional `[hyper]` section for data-dir / template /
///    trust overrides.
/// 3. Resolve trust anchors (preferring `[hyper.trust]`, falling back to
///    top-level `[[trust]]`, finally producing an `Err` unless the
///    effective config explicitly opts into dev-only trust).
/// 4. Build a [`crate::Hyper`] and wrap it in `Node<Init>`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn node_from_config_file(
    path: &Path,
) -> crate::error::HyperResult<crate::Node<crate::Init>> {
    node_from_config_file_with_package(path, None).await
}

/// Same as [`node_from_config_file`] but lets the caller supply the
/// [`actr_config::PackageInfo`] that becomes the node's registered
/// `actr_type`. Used by the language bindings, which already parsed
/// `manifest.toml` and don't want their identity collapsed to the
/// placeholder.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn node_from_config_file_with_package(
    path: &Path,
    package_info: Option<actr_config::PackageInfo>,
) -> crate::error::HyperResult<crate::Node<crate::Init>> {
    use crate::error::HyperError;
    use crate::verify::{ChainTrust, RegistryTrust, StaticTrust, TrustProvider};

    // Load both the RuntimeConfig and the raw TOML so we can pick up
    // [hyper.*] overrides that `ConfigParser` doesn't surface.
    let raw_text = std::fs::read_to_string(path).map_err(|e| {
        HyperError::Config(format!(
            "failed to read runtime config `{}`: {e}",
            path.display()
        ))
    })?;

    let raw_runtime: actr_config::RuntimeRawConfig = raw_text.parse().map_err(|e| {
        HyperError::Config(format!(
            "failed to parse runtime config `{}`: {e}",
            path.display()
        ))
    })?;
    // `RuntimeConfig` requires a PackageInfo. When the caller has one
    // already (bindings come in here with the manifest's `[package]`),
    // honour it so the node registers under the real actr_type. Without
    // an explicit one, fall back to the historical `local:Client:0.0.0`
    // placeholder so callers without a sibling manifest still work.
    let package_info = package_info.unwrap_or_else(|| actr_config::PackageInfo {
        name: "client".to_string(),
        actr_type: actr_protocol::ActrType {
            manufacturer: "local".to_string(),
            name: "Client".to_string(),
            version: "0.0.0".to_string(),
        },
        description: None,
        authors: vec![],
        license: None,
    });
    let runtime_config = actr_config::ConfigParser::parse_runtime(raw_runtime, path, package_info)
        .map_err(|e| HyperError::Config(format!("failed to parse runtime config: {e}")))?;

    // Parse the optional [hyper] section.
    let hyper_section: HyperSectionWrapper = toml::from_str(&raw_text).map_err(|e| {
        HyperError::Config(format!(
            "failed to parse [hyper] section of `{}`: {e}",
            path.display()
        ))
    })?;
    let hyper_section = hyper_section.hyper;

    // Resolve the data_dir: [hyper].data_dir > CLI user-config default.
    let data_dir = if let Some(dir) = hyper_section.data_dir.clone() {
        dir
    } else {
        actr_config::user_config::resolve_hyper_data_dir().map_err(|e| {
            HyperError::Config(format!(
                "failed to resolve default hyper data_dir (set `[hyper].data_dir` explicitly): {e}"
            ))
        })?
    };

    // Resolve trust: prefer [hyper.trust], fall back to top-level [[trust]].
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let trust: Arc<dyn TrustProvider> = if let Some(anchor) = hyper_section.trust.clone() {
        match anchor {
            HyperTrustAnchor::DevOnly => {
                tracing::warn!(
                    "[hyper.trust] kind = \"dev_only\" selected; accepting any package — \
                     NEVER use in production"
                );
                Arc::new(StaticTrust::dev_only())
            }
            HyperTrustAnchor::Static {
                pubkey_file,
                pubkey_b64,
            } => {
                let key_bytes = load_static_pubkey_bytes(
                    pubkey_file.as_deref().map(|p| resolve_path(base_dir, p)),
                    pubkey_b64,
                )?;
                Arc::new(StaticTrust::new(key_bytes)?)
            }
            HyperTrustAnchor::Registry { endpoint } => {
                let base = endpoint.trim_end_matches("/ais").to_string();
                Arc::new(RegistryTrust::new(base))
            }
        }
    } else if !runtime_config.trust.is_empty() {
        // Chain fallback from the top-level [[trust]] anchors.
        let mut providers: Vec<Arc<dyn TrustProvider>> =
            Vec::with_capacity(runtime_config.trust.len());
        for anchor in &runtime_config.trust {
            let provider: Arc<dyn TrustProvider> = match anchor {
                actr_config::TrustAnchor::Static {
                    pubkey_file,
                    pubkey_b64,
                } => {
                    let key_bytes =
                        load_static_pubkey_bytes(pubkey_file.clone(), pubkey_b64.clone())?;
                    Arc::new(StaticTrust::new(key_bytes)?)
                }
                actr_config::TrustAnchor::Registry { endpoint } => {
                    let base = endpoint.trim_end_matches("/ais").to_string();
                    Arc::new(RegistryTrust::new(base))
                }
            };
            providers.push(provider);
        }
        if providers.len() == 1 {
            providers.into_iter().next().unwrap()
        } else {
            Arc::new(ChainTrust::new(providers))
        }
    } else {
        return Err(HyperError::Config(
            "no `[hyper.trust]` or `[[trust]]` anchor configured. \
             Every runtime must declare a package-signature trust policy. \
             For dev / tests set `[hyper.trust] kind = \"dev_only\"`; \
             for production use `kind = \"static\"` with a `pubkey_file` \
             or `kind = \"registry\"` with an AIS endpoint."
                .to_string(),
        ));
    };

    // Build HyperConfig with the resolved values.
    let mut hyper_config = HyperConfig::new(&data_dir, trust);
    if let Some(template) = hyper_section.storage_path_template {
        hyper_config = hyper_config.with_storage_template(template);
    }

    // Build Hyper and return Node<Init>. Observability bring-up is left
    // to the caller — bindings and the CLI both want control over when
    // the tracing subscriber gets installed (they may want to layer in
    // their own filters first).
    let hyper = crate::Hyper::new(hyper_config).await?;
    let _ = &base_dir;
    Ok(crate::Node::from_hyper(hyper, runtime_config))
}

#[cfg(not(target_arch = "wasm32"))]
fn resolve_path(base_dir: &Path, path: impl AsRef<Path>) -> std::path::PathBuf {
    let p = path.as_ref();
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base_dir.join(p)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn load_static_pubkey_bytes(
    pubkey_file: Option<std::path::PathBuf>,
    pubkey_b64: Option<String>,
) -> crate::error::HyperResult<Vec<u8>> {
    use crate::error::HyperError;
    use base64::Engine;

    if let Some(b64) = pubkey_b64 {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&b64)
            .map_err(|e| HyperError::Config(format!("invalid pubkey_b64: {e}")))?;
        if bytes.len() != 32 {
            return Err(HyperError::Config(format!(
                "pubkey_b64 must decode to 32 bytes, got {}",
                bytes.len()
            )));
        }
        return Ok(bytes);
    }
    let path = pubkey_file.ok_or_else(|| {
        HyperError::Config("static trust anchor requires `pubkey_file` or `pubkey_b64`".to_string())
    })?;
    let text = std::fs::read_to_string(&path).map_err(|e| {
        HyperError::Config(format!(
            "failed to read pubkey_file `{}`: {e}",
            path.display()
        ))
    })?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
        HyperError::Config(format!(
            "pubkey_file `{}` is not valid JSON: {e}",
            path.display()
        ))
    })?;
    let b64 = value
        .get("public_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            HyperError::Config(format!(
                "pubkey_file `{}` is missing the `public_key` field",
                path.display()
            ))
        })?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| {
            HyperError::Config(format!(
                "pubkey_file `{}` has invalid base64: {e}",
                path.display()
            ))
        })?;
    if bytes.len() != 32 {
        return Err(HyperError::Config(format!(
            "pubkey_file `{}` must contain a 32-byte key, got {}",
            path.display(),
            bytes.len()
        )));
    }
    Ok(bytes)
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
            credential_expiry_warning: DEFAULT_CREDENTIAL_EXPIRY_WARNING,
            mailbox_backpressure_threshold: None,
            dispatch_concurrency: None,
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

    /// Override the credential-expiry warning lead time.
    pub fn with_credential_expiry_warning(mut self, window: Duration) -> Self {
        self.credential_expiry_warning = window;
        self
    }

    /// Set the mailbox backpressure threshold.
    ///
    /// See [`HyperConfig::mailbox_backpressure_threshold`] for semantics.
    pub fn with_mailbox_backpressure_threshold(mut self, threshold: Option<usize>) -> Self {
        self.mailbox_backpressure_threshold = threshold;
        self
    }

    /// Resolve the active mailbox backpressure threshold — explicit
    /// override or the built-in [`DEFAULT_MAILBOX_BACKPRESSURE_THRESHOLD`].
    pub fn resolved_mailbox_backpressure_threshold(&self) -> usize {
        self.mailbox_backpressure_threshold
            .unwrap_or(DEFAULT_MAILBOX_BACKPRESSURE_THRESHOLD)
    }

    /// Set the conflict-key dispatch concurrency config (B2). `None` falls back
    /// to [`DispatchConcurrency::default`] (gate on, strategy A); pass
    /// `Some(DispatchConcurrency { enabled: false, .. })` to force serial.
    pub fn with_dispatch_concurrency(mut self, cfg: Option<DispatchConcurrency>) -> Self {
        self.dispatch_concurrency = cfg;
        self
    }

    /// Resolve the effective dispatch concurrency, applying the
    /// `ACTR_DISPATCH_SERIAL` escape hatch (which forces `enabled = false`).
    pub fn resolved_dispatch_concurrency(&self) -> DispatchConcurrency {
        let mut cfg = self.dispatch_concurrency.unwrap_or_default();
        let raw = std::env::var("ACTR_DISPATCH_SERIAL").ok();
        if dispatch_serial_env_override(raw.as_deref()) {
            if cfg.enabled {
                tracing::warn!(
                    "ACTR_DISPATCH_SERIAL is set; forcing fully-serial dispatch \
                     (dispatch concurrency disabled)"
                );
            }
            cfg.enabled = false;
        }
        cfg
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
#[path = "config_tests.rs"]
mod tests;
