//! Per-service health and metrics endpoints
//!
//! Provides 16 fixed API endpoints:
//! - `/{service}/health` and `/{service}/metrics` for each service group
//! - `/health` (aggregate) and `/metrics` (all registries merged) at root

use axum::{
    Json, Router,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::get,
};
use base64::Engine;
use platform::{ServiceCollector, config::ActrixConfig};
use serde_json::{Value, json};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

/// Known service names accepted in `/{service}/health` and `/{service}/metrics`.
const KNOWN_SERVICES: &[&str] = &[
    "signaling",
    "ais",
    "signer",
    "control",
    "admin",
    "stun",
    "turn",
    "daemon",
];

/// Shared state for observability endpoints.
pub struct ObservabilityState {
    pub collector: ServiceCollector,
    pub config: ActrixConfig,
    pub version: String,
}

/// Services that already have their own `/health` route in their nested router.
/// We skip registering observability `/health` for these to avoid duplicate routes.
const SERVICES_WITH_OWN_HEALTH: &[&str] = &["signaling", "ais"];

/// Build the observability sub-router with auth middleware.
///
/// Uses explicit routes for each service (not a `{service}` path param)
/// so that concrete routes take priority over nested service routers.
///
/// Services that already define their own `/health` in their nested router
/// (signaling, ais, signer) keep their own health endpoint; we only register
/// `/{service}/metrics` for those.
pub fn build_observability_router(state: Arc<ObservabilityState>) -> Router {
    let mut router = Router::new()
        .route("/health", get(root_health))
        .route("/metrics", get(root_metrics));

    for &svc in KNOWN_SERVICES {
        // Always register /{service}/metrics (no conflicts)
        let metrics_path = format!("/{svc}/metrics");
        router = router.route(&metrics_path, get(service_metrics));

        // Only register /{service}/health if the service doesn't have its own
        if !SERVICES_WITH_OWN_HEALTH.contains(&svc) {
            let health_path = format!("/{svc}/health");
            router = router.route(&health_path, get(service_health));
        }
    }

    router
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            monitoring_auth,
        ))
        .with_state(state)
}

// ── Handlers ────────────────────────────────────────────────────

/// GET /health — aggregate health across all enabled services
async fn root_health(State(state): State<Arc<ObservabilityState>>) -> Json<Value> {
    let services_map = build_services_health(&state).await;

    let overall = if services_map
        .values()
        .all(|v| v.as_str() == Some("healthy") || v.as_str() == Some("disabled"))
    {
        "healthy"
    } else {
        "unhealthy"
    };

    Json(json!({
        "status": overall,
        "node": state.config.name,
        "version": state.version,
        "services": services_map,
    }))
}

/// GET /{service}/health — per-service health
async fn service_health(
    State(state): State<Arc<ObservabilityState>>,
    request: axum::extract::Request,
) -> Json<Value> {
    let service = extract_service_from_path(request.uri().path()).unwrap_or("daemon");
    let status = resolve_service_status(service, &state).await;

    Json(json!({
        "status": status,
        "service": service,
    }))
}

/// GET /metrics — all registries merged (global + per-service)
async fn root_metrics() -> String {
    platform::metrics::SERVICE_REGISTRIES.export_all()
}

/// GET /{service}/metrics — single service registry
async fn service_metrics(request: axum::extract::Request) -> String {
    let service = extract_service_from_path(request.uri().path()).unwrap_or("daemon");
    platform::metrics::SERVICE_REGISTRIES.export_for(service)
}

// ── Auth middleware ──────────────────────────────────────────────

/// IP whitelist + HTTP Basic Auth middleware for monitoring endpoints.
///
/// Auth logic (OR — either match allows):
/// 1. If no auth configured → allow all
/// 2. If client IP matches `allowed_ips` → allow
/// 3. If Basic Auth credentials match `htpasswd_file` → allow
/// 4. Otherwise → 401
///
/// Per-service overrides replace global config when the request targets
/// a specific service (extracted from the URI path).
async fn monitoring_auth(
    State(state): State<Arc<ObservabilityState>>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let monitoring = &state.config.monitoring;

    // Extract service name from path (e.g. "/signaling/health" → "signaling")
    let path = request.uri().path();
    let service_name = extract_service_from_path(path);

    // Get effective config for this service (per-service override or global)
    let (allowed_ips, htpasswd_file) = match service_name {
        Some(svc) => monitoring.effective_for(svc),
        None => (
            monitoring.allowed_ips.as_slice(),
            monitoring.htpasswd_file.as_str(),
        ),
    };

    // If no auth configured, allow all
    if allowed_ips.is_empty() && htpasswd_file.is_empty() {
        return next.run(request).await;
    }

    // Check IP whitelist
    if !allowed_ips.is_empty() {
        let client_ip = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ci| ci.0.ip());
        if let Some(ip) = client_ip
            && ip_matches_any(ip, allowed_ips)
        {
            return next.run(request).await;
        }
    }

    // Check HTTP Basic Auth
    if !htpasswd_file.is_empty()
        && let Some(creds) = extract_basic_auth(&headers)
        && check_htpasswd(htpasswd_file, &creds.0, &creds.1)
    {
        return next.run(request).await;
    }

    // Neither matched → 401
    let mut response = (
        StatusCode::UNAUTHORIZED,
        Json(json!({"error": "unauthorized"})),
    )
        .into_response();
    if !htpasswd_file.is_empty() {
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            "Basic realm=\"actrix-monitoring\"".parse().unwrap(),
        );
    }
    response
}

/// Extract service name from path like "/signaling/health" → Some("signaling").
/// Returns None for root paths like "/health" or "/metrics".
fn extract_service_from_path(path: &str) -> Option<&str> {
    let trimmed = path.strip_prefix('/').unwrap_or(path);
    if trimmed == "health" || trimmed == "metrics" {
        return None;
    }
    trimmed
        .split('/')
        .next()
        .filter(|s| KNOWN_SERVICES.contains(s))
}

/// Check if an IP address matches any CIDR in the whitelist.
fn ip_matches_any(ip: IpAddr, cidrs: &[String]) -> bool {
    cidrs.iter().any(|cidr| ip_matches_cidr(ip, cidr))
}

/// Check if an IP matches a single CIDR entry (e.g. "10.0.0.0/8" or "127.0.0.1").
fn ip_matches_cidr(ip: IpAddr, cidr: &str) -> bool {
    let (network_str, prefix_len) = if let Some((net, len)) = cidr.split_once('/') {
        let Ok(len) = len.parse::<u32>() else {
            return false;
        };
        (net, len)
    } else {
        // Bare IP — exact match
        let max_prefix = match ip {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        (cidr, max_prefix)
    };

    let Ok(network_ip) = network_str.parse::<IpAddr>() else {
        return false;
    };

    match (ip, network_ip) {
        (IpAddr::V4(ip4), IpAddr::V4(net4)) => {
            if prefix_len > 32 {
                return false;
            }
            if prefix_len == 0 {
                return true;
            }
            let mask = u32::MAX.checked_shl(32 - prefix_len).unwrap_or(0);
            (u32::from(ip4) & mask) == (u32::from(net4) & mask)
        }
        (IpAddr::V6(ip6), IpAddr::V6(net6)) => {
            if prefix_len > 128 {
                return false;
            }
            if prefix_len == 0 {
                return true;
            }
            let mask = u128::MAX.checked_shl(128 - prefix_len).unwrap_or(0);
            (u128::from(ip6) & mask) == (u128::from(net6) & mask)
        }
        _ => false, // v4/v6 mismatch
    }
}

/// Extract username:password from an HTTP Basic Auth header.
fn extract_basic_auth(headers: &HeaderMap) -> Option<(String, String)> {
    let auth = headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Basic ")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(auth)
        .ok()?;
    let decoded_str = String::from_utf8(decoded).ok()?;
    let (user, pass) = decoded_str.split_once(':')?;
    Some((user.to_string(), pass.to_string()))
}

/// Check credentials against an htpasswd file (plaintext format: user:password).
fn check_htpasswd(path: &str, username: &str, password: &str) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((user, pass)) = line.split_once(':')
            && user == username
            && pass == password
        {
            return true;
        }
    }
    false
}

// ── Helpers ─────────────────────────────────────────────────────

/// Build a map of service name → health status string for all service groups.
async fn build_services_health(state: &ObservabilityState) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    for &svc in KNOWN_SERVICES {
        let status = resolve_service_status(svc, state).await;
        map.insert(svc.to_string(), Value::String(status));
    }
    map
}

/// Determine a service's health status: "healthy", "unhealthy", or "disabled".
async fn resolve_service_status(service: &str, state: &ObservabilityState) -> String {
    let cfg = &state.config;

    // Check if service is enabled
    let enabled = match service {
        "signaling" => cfg.is_signaling_enabled(),
        "ais" => cfg.is_ais_enabled(),
        "signer" => cfg.is_signer_enabled(),
        "stun" => cfg.is_stun_enabled(),
        "turn" => cfg.is_turn_enabled(),
        // control, admin, daemon are always enabled
        "control" | "admin" | "daemon" => true,
        _ => false,
    };

    if !enabled {
        return "disabled".to_string();
    }

    // Map service name to ServiceCollector key
    let collector_key = match service {
        "signaling" => Some("Signaling Service"),
        "ais" => Some("AIS Service"),
        "signer" => Some("Signer Service"),
        "stun" => Some("STUN Server"),
        "turn" => Some("TURN Server"),
        // control/admin/daemon don't have collector entries — always healthy
        _ => None,
    };

    if let Some(key) = collector_key {
        match state.collector.get(key).await {
            Some(info) if info.is_running() => "healthy".to_string(),
            Some(_) => "unhealthy".to_string(),
            None => "unhealthy".to_string(),
        }
    } else {
        "healthy".to_string()
    }
}
