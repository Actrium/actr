//! systemctl operations for `update`/`rollback` restarts and health checks.
//!
//! Used after a version switch to restart an existing service and confirm it
//! came up. `update` never creates or edits systemd units — it only restarts
//! already-deployed services.

use anyhow::{Result, bail};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

/// Restart a systemd service.
pub fn restart(service: &str) -> Result<()> {
    println!("🔄 Restarting service '{service}' ...");
    let out = Command::new("sudo")
        .args(["systemctl", "restart", service])
        .output()?;
    if !out.status.success() {
        bail!(
            "systemctl restart {service} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Whether a service is currently active.
///
/// This is *liveness* only — the unit's main process is running — not
/// *readiness*: a process that is up but not yet serving traffic (or serving
/// errors) still reports active. Use [`wait_ready`] with a health URL when you
/// need to confirm the service is actually accepting requests.
pub fn is_active(service: &str) -> bool {
    Command::new("sudo")
        .args(["systemctl", "is-active", "--quiet", service])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Poll `is_active` once per second for up to `seconds`.
pub fn wait_active(service: &str, seconds: u32) -> Result<()> {
    for _ in 0..seconds {
        if is_active(service) {
            return Ok(());
        }
        sleep(Duration::from_secs(1));
    }
    bail!("service '{service}' did not become active within {seconds}s")
}

/// Wait for a restarted service to be ready.
///
/// When `health_url` is given (e.g. `http://127.0.0.1:8080/health`), the
/// service is only considered ready once it is `active` AND the endpoint
/// returns a 2xx — catching processes that are alive but broken, which a
/// liveness-only check would miss. Without a URL this falls back to
/// [`wait_active`].
pub fn wait_ready(service: &str, seconds: u32, health_url: Option<&str>) -> Result<()> {
    match health_url {
        Some(url) => wait_ready_with_probe(service, seconds, url),
        None => wait_active(service, seconds),
    }
}

fn wait_ready_with_probe(service: &str, seconds: u32, health_url: &str) -> Result<()> {
    for _ in 0..seconds {
        if is_active(service) {
            match http_ok(health_url) {
                Ok(true) => return Ok(()),
                Ok(false) => {} // active but not yet serving 2xx
                Err(err) => {
                    // Don't abort the whole wait on a transient curl error;
                    // keep polling until the window expires.
                    eprintln!("⚠️  health probe error for {health_url}: {err}");
                }
            }
        }
        sleep(Duration::from_secs(1));
    }
    bail!("service '{service}' did not become ready within {seconds}s (health url: {health_url})")
}

/// `true` if `url` returns an HTTP 2xx status.
fn http_ok(url: &str) -> Result<bool> {
    let out = Command::new("curl")
        .args(["-sS", "--fail", "--max-time", "3", "-o", "/dev/null", url])
        .output()?;
    Ok(out.status.success())
}

/// Health-check wait window, overridable via `ACTRIX_HEALTH_WAIT_SECONDS`.
pub fn health_wait_seconds() -> u32 {
    const DEFAULT: u32 = 5;
    const MAX: u32 = 300;
    std::env::var("ACTRIX_HEALTH_WAIT_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .map(|n: u32| n.min(MAX))
        .unwrap_or(DEFAULT)
}
