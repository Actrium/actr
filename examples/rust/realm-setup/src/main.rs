//! realm-setup - CLI tool to setup realms in actrix for actr-examples
//!
//! This tool reads actr.toml configuration files, creates realms in actrix
//! via the Admin UI REST API, and writes back the assigned realm_id and
//! realm_secret into each actr.toml file.

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;
use std::path::PathBuf;
use tracing::{debug, info, warn};

/// CLI arguments
#[derive(Parser, Debug)]
#[command(name = "realm-setup")]
#[command(about = "Setup realms in actrix from actr.toml configuration files")]
#[command(version)]
struct Args {
    /// Path to actrix-config.toml file
    #[arg(short = 'c', long, default_value = "actrix-config.toml")]
    actrix_config: PathBuf,

    /// Paths to actr.toml files to read realm info from
    #[arg(short = 'a', long = "actr-toml", required = true, num_args = 1..)]
    actr_tomls: Vec<PathBuf>,

    /// Override admin API base URL (default: derived from actrix-config bind.http)
    #[arg(short = 'u', long)]
    admin_url: Option<String>,

    /// Override admin password (default: read from actrix-config)
    #[arg(short = 'p', long)]
    password: Option<String>,

    /// Realm name for the created realm
    #[arg(long, default_value = "example-realm")]
    realm_name: String,

    /// Enable verbose output
    #[arg(short = 'v', long)]
    verbose: bool,
}

// ─── Actrix config parsing ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ActrixConfig {
    bind: Option<BindSection>,
    control: Option<ControlSection>,
}

#[derive(Debug, Deserialize)]
struct BindSection {
    http: Option<HttpBind>,
}

#[derive(Debug, Deserialize)]
struct HttpBind {
    #[allow(dead_code)]
    ip: Option<String>,
    port: Option<u16>,
    advertised_ip: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ControlSection {
    admin_ui: Option<AdminUiConfig>,
}

#[derive(Debug, Deserialize)]
struct AdminUiConfig {
    password: Option<String>,
}

// ─── actr.toml parsing (read-only, for displaying info) ────────────────────

#[derive(Debug, Deserialize)]
struct ActrConfig {
    system: Option<SystemSection>,
}

#[derive(Debug, Deserialize)]
struct SystemSection {
    deployment: Option<DeploymentSection>,
}

#[derive(Debug, Deserialize)]
struct DeploymentSection {
    #[serde(alias = "realm")]
    realm_id: Option<u32>,
}

// ─── Admin API response types ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct LoginResponse {
    token: Option<String>,
    #[allow(dead_code)]
    expires_in: Option<u64>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateRealmResponse {
    success: bool,
    error_message: Option<String>,
    realm: Option<RealmInfo>,
    realm_secret: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RealmInfo {
    realm_id: u32,
    #[allow(dead_code)]
    name: String,
}

#[derive(Debug, Deserialize)]
struct ListRealmsResponse {
    #[allow(dead_code)]
    success: bool,
    realms: Vec<RealmInfo>,
}

// ─── Implementation ─────────────────────────────────────────────────────────

fn parse_actrix_config(path: &PathBuf) -> Result<(String, String)> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read actrix-config.toml: {}", path.display()))?;

    let config: ActrixConfig = toml::from_str(&content)
        .with_context(|| format!("Failed to parse actrix-config.toml: {}", path.display()))?;

    // Derive admin URL from bind.http
    let bind = config.bind.unwrap_or(BindSection { http: None });
    let http = bind.http.unwrap_or(HttpBind {
        ip: None,
        port: None,
        advertised_ip: None,
    });
    let ip = http
        .advertised_ip
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let port = http.port.unwrap_or(8080);
    let admin_url = format!("http://{}:{}", ip, port);

    // Get admin password
    let password = config
        .control
        .and_then(|c| c.admin_ui)
        .and_then(|a| a.password)
        .ok_or_else(|| {
            anyhow::anyhow!("Missing [control.admin_ui].password in actrix-config.toml")
        })?;

    Ok((admin_url, password))
}

/// Login to Admin API and get JWT token
async fn admin_login(client: &reqwest::Client, base_url: &str, password: &str) -> Result<String> {
    let url = format!("{}/admin/api/auth/login", base_url);
    debug!("Logging in to Admin API at {}", url);

    let resp = client
        .post(&url)
        .json(&serde_json::json!({ "password": password }))
        .send()
        .await
        .with_context(|| format!("Failed to connect to Admin API at {}", url))?;

    let status = resp.status();
    let body: LoginResponse = resp
        .json()
        .await
        .with_context(|| "Failed to parse login response")?;

    if !status.is_success() {
        let err_msg = body.error.unwrap_or_else(|| format!("HTTP {}", status));
        return Err(anyhow::anyhow!("Admin login failed: {}", err_msg));
    }

    body.token
        .ok_or_else(|| anyhow::anyhow!("Login response missing token"))
}

/// List existing realms
async fn list_realms(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
) -> Result<Vec<RealmInfo>> {
    let url = format!("{}/admin/api/realms", base_url);
    let resp = client
        .get(&url)
        .bearer_auth(token)
        .send()
        .await
        .with_context(|| "Failed to list realms")?;

    let body: ListRealmsResponse = resp
        .json()
        .await
        .with_context(|| "Failed to parse list realms response")?;

    Ok(body.realms)
}

/// Create a new realm
async fn create_realm(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    name: &str,
) -> Result<(u32, String)> {
    let url = format!("{}/admin/api/realms", base_url);
    let resp = client
        .post(&url)
        .bearer_auth(token)
        .json(&serde_json::json!({
            "name": name,
            "enabled": true,
            "expires_at": 0
        }))
        .send()
        .await
        .with_context(|| "Failed to create realm")?;

    let status = resp.status();
    let body: CreateRealmResponse = resp
        .json()
        .await
        .with_context(|| "Failed to parse create realm response")?;

    if !body.success || !status.is_success() {
        let err_msg = body
            .error_message
            .unwrap_or_else(|| format!("HTTP {}", status));
        return Err(anyhow::anyhow!("Failed to create realm: {}", err_msg));
    }

    let realm = body
        .realm
        .ok_or_else(|| anyhow::anyhow!("Create realm response missing realm info"))?;

    let secret = body
        .realm_secret
        .ok_or_else(|| anyhow::anyhow!("Create realm response missing realm_secret"))?;

    Ok((realm.realm_id, secret))
}

/// Update an actr.toml file with the new realm_id and realm_secret
fn update_actr_toml(path: &PathBuf, realm_id: u32, realm_secret: &str) -> Result<()> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read actr.toml: {}", path.display()))?;

    let mut doc = content
        .parse::<toml_edit::DocumentMut>()
        .with_context(|| format!("Failed to parse actr.toml as TOML: {}", path.display()))?;

    // Ensure [system.deployment] exists
    if doc.get("system").is_none() {
        doc["system"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    let system = doc["system"]
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[system] is not a table in {}", path.display()))?;

    if system.get("deployment").is_none() {
        system["deployment"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    let deployment = system["deployment"]
        .as_table_mut()
        .ok_or_else(|| {
            anyhow::anyhow!("[system.deployment] is not a table in {}", path.display())
        })?;

    // Update realm_id
    deployment["realm_id"] = toml_edit::value(realm_id as i64);

    // Update realm_secret
    deployment["realm_secret"] = toml_edit::value(realm_secret);

    std::fs::write(path, doc.to_string())
        .with_context(|| format!("Failed to write actr.toml: {}", path.display()))?;

    info!("Updated {} (realm_id={})", path.display(), realm_id);

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize logging
    let filter = if args.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    // ── Determine admin URL and password ────────────────────────────────
    let (cfg_url, cfg_password) = parse_actrix_config(&args.actrix_config)?;
    let admin_url = args.admin_url.unwrap_or(cfg_url);
    let password = args.password.unwrap_or(cfg_password);

    info!("Admin API: {}", admin_url);

    // ── Parse existing realm info from actr.toml files (for logging) ────
    for path in &args.actr_tomls {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read: {}", path.display()))?;
        let config: ActrConfig = toml::from_str(&content)
            .with_context(|| format!("Failed to parse: {}", path.display()))?;
        let old_realm = config
            .system
            .and_then(|s| s.deployment)
            .and_then(|d| d.realm_id);
        info!(
            "Found actr.toml: {} (current realm_id: {:?})",
            path.display(),
            old_realm
        );
    }

    // ── Login ───────────────────────────────────────────────────────────
    let http = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()?;

    info!("Logging in to Admin API...");
    let token = admin_login(&http, &admin_url, &password).await?;
    info!("Login successful");

    // ── Check existing realms ───────────────────────────────────────────
    let existing = list_realms(&http, &admin_url, &token).await?;
    debug!("Existing realms: {:?}", existing);

    let (realm_id, realm_secret) = if let Some(realm) = existing.first() {
        // Reuse existing realm — but we need the secret.
        // Admin API only shows secret at creation time, so if a realm exists
        // and we don't know its secret, we create a new one.
        warn!(
            "Found existing realm {} (id={}), creating a new realm for this test run",
            realm.name, realm.realm_id
        );
        let (id, secret) = create_realm(&http, &admin_url, &token, &args.realm_name).await?;
        info!("Created realm: id={}, name={}", id, args.realm_name);
        (id, secret)
    } else {
        let (id, secret) = create_realm(&http, &admin_url, &token, &args.realm_name).await?;
        info!("Created realm: id={}, name={}", id, args.realm_name);
        (id, secret)
    };

    // ── Update all actr.toml files ──────────────────────────────────────
    for path in &args.actr_tomls {
        update_actr_toml(path, realm_id, &realm_secret)?;
    }

    // ── Summary ─────────────────────────────────────────────────────────
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("Realm setup complete:");
    info!("  Created realm_id: {}", realm_id);
    info!(
        "  Updated {} actr.toml file(s)",
        args.actr_tomls.len()
    );
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    Ok(())
}
