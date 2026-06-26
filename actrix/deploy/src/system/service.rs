//! systemctl operations for `update`/`rollback` restarts and health checks.
//!
//! Used after a version switch to restart an existing service and confirm it
//! came up. `update` never creates or edits systemd units — it only restarts
//! already-deployed services.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
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

/// Return the executable path currently running as a systemd service's MainPID.
pub fn running_binary(service: &str) -> Result<PathBuf> {
    let pid = main_pid(service)?;
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .with_context(|| format!("failed to inspect running binary for service '{service}'"))
}

/// Verify that systemd restarted the service onto the expected release binary.
pub fn assert_running_binary(service: &str, expected_binary: &Path) -> Result<()> {
    let actual = canonicalize_or_self(running_binary(service)?);
    let expected = std::fs::canonicalize(expected_binary).with_context(|| {
        format!(
            "failed to canonicalize expected binary {}",
            expected_binary.display()
        )
    })?;

    if actual != expected {
        bail!(
            "service '{service}' is running {}, expected {}. Check that ExecStart points at the managed bin/actrix path and restart the service again.",
            actual.display(),
            expected.display()
        );
    }

    Ok(())
}

fn main_pid(service: &str) -> Result<u32> {
    let out = Command::new("systemctl")
        .args(["show", service, "-p", "MainPID", "--value"])
        .output()
        .with_context(|| format!("failed to query MainPID for service '{service}'"))?;
    if !out.status.success() {
        bail!(
            "systemctl show {service} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    parse_main_pid(&out.stdout, service)
}

fn parse_main_pid(output: &[u8], service: &str) -> Result<u32> {
    let text = String::from_utf8_lossy(output);
    let trimmed = text.trim();
    let pid = trimmed.parse::<u32>().with_context(|| {
        format!("systemctl returned invalid MainPID '{trimmed}' for service '{service}'")
    })?;
    if pid == 0 {
        bail!("service '{service}' has no running MainPID");
    }
    Ok(pid)
}

fn canonicalize_or_self(path: PathBuf) -> PathBuf {
    std::fs::canonicalize(&path).unwrap_or(path)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_main_pid() {
        assert_eq!(parse_main_pid(b"1234\n", "actrix").unwrap(), 1234);
    }

    #[test]
    fn rejects_zero_main_pid() {
        let err = parse_main_pid(b"0\n", "actrix").unwrap_err();
        assert!(err.to_string().contains("no running MainPID"));
    }

    #[test]
    fn rejects_invalid_main_pid() {
        let err = parse_main_pid(b"abc\n", "actrix").unwrap_err();
        assert!(err.to_string().contains("invalid MainPID"));
    }
}
