//! GitHub Release asset discovery and download.
//!
//! Queries the GitHub Releases API for a given repo + tag (or latest), selects
//! the architecture-appropriate `actrix-linux-*` asset plus optional checksum
//! metadata, and downloads them via `curl`. A `GITHUB_TOKEN` (Contents: Read)
//! is used for private repositories and is forwarded as a Bearer header.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::Path;
use std::process::Command;

/// Which release to fetch.
#[derive(Debug, Clone)]
pub enum TagTarget {
    Latest,
    Tag(String),
}

/// Architecture-specific asset selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    LinuxX86_64,
    LinuxArm64,
}

impl AssetKind {
    /// Map the host architecture to a release asset kind.
    pub fn from_host_arch() -> Result<Self> {
        match std::env::consts::ARCH {
            "x86_64" => Ok(Self::LinuxX86_64),
            "aarch64" | "arm64" => Ok(Self::LinuxArm64),
            other => bail!("unsupported host architecture for release assets: {other}"),
        }
    }

    /// The binary asset name, e.g. `actrix-linux-x86_64`.
    pub fn asset_name(&self) -> &'static str {
        match self {
            Self::LinuxX86_64 => "actrix-linux-x86_64",
            Self::LinuxArm64 => "actrix-linux-arm64",
        }
    }

    /// The `.sha256` sidecar asset name for this binary asset.
    pub fn sha256_asset_name(&self) -> String {
        format!("{}.sha256", self.asset_name())
    }
}

#[derive(Debug, Deserialize)]
struct ReleaseJson {
    tag_name: String,
    assets: Vec<AssetJson>,
}

#[derive(Debug, Deserialize)]
struct AssetJson {
    name: String,
    url: String,
    browser_download_url: String,
    digest: Option<String>,
}

/// A resolved release asset with its download URLs.
#[derive(Debug, Clone)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub browser_download_url: String,
    pub digest: Option<String>,
}

/// A fetched release: its tag plus all assets.
#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub assets: Vec<ReleaseAsset>,
}

impl ReleaseInfo {
    /// Find an asset by exact name.
    pub fn find_asset(&self, name: &str) -> Option<&ReleaseAsset> {
        self.assets.iter().find(|a| a.name == name)
    }
}

/// Query the GitHub Releases API and parse the response.
pub fn fetch_release(repo: &str, target: &TagTarget, token: Option<&str>) -> Result<ReleaseInfo> {
    let url = match target {
        TagTarget::Latest => format!("https://api.github.com/repos/{repo}/releases/latest"),
        TagTarget::Tag(tag) => format!("https://api.github.com/repos/{repo}/releases/tags/{tag}"),
    };
    let body = curl_text(&url, token, Some("application/vnd.github+json"))
        .context("failed to query GitHub Release")?;
    let json: ReleaseJson =
        serde_json::from_str(&body).context("failed to parse GitHub Release JSON")?;
    Ok(ReleaseInfo {
        tag_name: json.tag_name,
        assets: json
            .assets
            .into_iter()
            .map(|a| ReleaseAsset {
                name: a.name,
                url: a.url,
                browser_download_url: a.browser_download_url,
                digest: a.digest,
            })
            .collect(),
    })
}

/// Download a release asset to `dest`.
///
/// When a token is present (private repo), the API `.url` is used with
/// `Accept: application/octet-stream` + Bearer auth; otherwise the public
/// `browser_download_url` is used directly.
pub fn download_asset(asset: &ReleaseAsset, token: Option<&str>, dest: &Path) -> Result<()> {
    let (url, accept) = if token.is_some() {
        (&asset.url, Some("application/octet-stream"))
    } else {
        (&asset.browser_download_url, None)
    };
    curl_download(url, token, accept, dest)
        .with_context(|| format!("failed to download asset {}", asset.name))?;
    Ok(())
}

fn curl_text(url: &str, token: Option<&str>, accept: Option<&str>) -> Result<String> {
    let (args, stdin_cfg) = curl_args(url, token, accept, None);
    let output = curl_exec(&args, stdin_cfg.as_deref())?;
    if !output.status.success() {
        bail!(
            "curl request failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn curl_download(url: &str, token: Option<&str>, accept: Option<&str>, dest: &Path) -> Result<()> {
    let (args, stdin_cfg) = curl_download_args(url, token, accept, dest);
    let status = curl_exec_streaming(&args, stdin_cfg.as_deref())?;
    if !status.success() {
        bail!("curl download failed with status {status}");
    }
    Ok(())
}

/// Spawn curl with the given args, optionally feeding a curl config (carrying
/// the secret `Authorization` header) over stdin so the token never appears in
/// argv / `ps` / `/proc`.
fn curl_exec(args: &[String], stdin_config: Option<&str>) -> Result<std::process::Output> {
    let mut cmd = Command::new("curl");
    cmd.args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if stdin_config.is_some() {
        cmd.stdin(std::process::Stdio::piped());
    }
    let mut child = cmd.spawn().context("failed to invoke curl")?;
    if let Some(cfg) = stdin_config {
        use std::io::Write;
        if let Some(stdin) = child.stdin.as_mut() {
            stdin.write_all(cfg.as_bytes())?;
        }
    }
    child.wait_with_output().context("failed to wait for curl")
}

/// Spawn curl for a download and let curl's progress bar write to stderr.
fn curl_exec_streaming(
    args: &[String],
    stdin_config: Option<&str>,
) -> Result<std::process::ExitStatus> {
    let mut cmd = Command::new("curl");
    cmd.args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit());
    if stdin_config.is_some() {
        cmd.stdin(std::process::Stdio::piped());
    }
    let mut child = cmd.spawn().context("failed to invoke curl")?;
    if let Some(cfg) = stdin_config {
        use std::io::Write;
        if let Some(stdin) = child.stdin.as_mut() {
            stdin.write_all(cfg.as_bytes())?;
        }
    }
    child.wait().context("failed to wait for curl")
}

/// Build curl args plus an optional stdin config holding the bearer token.
///
/// The non-secret headers (User-Agent, Accept) stay on the argv; the
/// `Authorization` header is moved into a `--config -` blob read from stdin so
/// the token is not visible in the process list.
fn curl_args(
    url: &str,
    token: Option<&str>,
    accept: Option<&str>,
    output: Option<&Path>,
) -> (Vec<String>, Option<String>) {
    let mut args: Vec<String> = vec!["-sSLf".into(), "--retry".into(), "3".into()];
    args.push("-H".into());
    args.push("User-Agent: actrix-deploy".into());
    if let Some(accept) = accept {
        args.push("-H".into());
        args.push(format!("Accept: {accept}"));
    }
    if let Some(output) = output {
        args.push("-o".into());
        args.push(output.to_string_lossy().into_owned());
    }

    let stdin_config = token.map(|t| {
        // curl config file syntax: `header = "value"`. GitHub tokens are
        // alphanumeric, so quoting is safe.
        args.insert(0, "--config".into());
        args.insert(1, "-".into());
        format!("header = \"Authorization: Bearer {t}\"\n")
    });

    args.push(url.into());
    (args, stdin_config)
}

fn curl_download_args(
    url: &str,
    token: Option<&str>,
    accept: Option<&str>,
    output: &Path,
) -> (Vec<String>, Option<String>) {
    let mut args: Vec<String> = vec![
        "-fL".into(),
        "--progress-bar".into(),
        "--retry".into(),
        "3".into(),
    ];
    args.push("-H".into());
    args.push("User-Agent: actrix-deploy".into());
    if let Some(accept) = accept {
        args.push("-H".into());
        args.push(format!("Accept: {accept}"));
    }
    args.push("-o".into());
    args.push(output.to_string_lossy().into_owned());

    let stdin_config = token.map(|t| {
        args.insert(0, "--config".into());
        args.insert(1, "-".into());
        format!("header = \"Authorization: Bearer {t}\"\n")
    });

    args.push(url.into());
    (args, stdin_config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arch_maps_to_asset_names() {
        assert_eq!(AssetKind::LinuxX86_64.asset_name(), "actrix-linux-x86_64");
        assert_eq!(AssetKind::LinuxArm64.asset_name(), "actrix-linux-arm64");
        assert_eq!(
            AssetKind::LinuxX86_64.sha256_asset_name(),
            "actrix-linux-x86_64.sha256"
        );
    }

    #[test]
    fn finds_asset_by_name() {
        let info = ReleaseInfo {
            tag_name: "v0.4.3".into(),
            assets: vec![
                ReleaseAsset {
                    name: "actrix-linux-x86_64".into(),
                    url: "u1".into(),
                    browser_download_url: "b1".into(),
                    digest: Some(
                        "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                            .into(),
                    ),
                },
                ReleaseAsset {
                    name: "actrix-linux-x86_64.sha256".into(),
                    url: "u2".into(),
                    browser_download_url: "b2".into(),
                    digest: None,
                },
            ],
        };
        assert!(info.find_asset("actrix-linux-x86_64").is_some());
        assert!(info.find_asset("actrix-linux-x86_64.sha256").is_some());
        assert!(info.find_asset("missing").is_none());
    }

    #[test]
    fn parses_release_json() {
        let body = r#"{
            "tag_name": "v0.4.3",
            "assets": [
                {
                    "name": "actrix-linux-x86_64",
                    "url": "https://api",
                    "browser_download_url": "https://b",
                    "digest": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                }
            ]
        }"#;
        let json: ReleaseJson = serde_json::from_str(body).unwrap();
        assert_eq!(json.tag_name, "v0.4.3");
        assert_eq!(json.assets.len(), 1);
        assert_eq!(
            json.assets[0].digest.as_deref(),
            Some("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );
    }

    #[test]
    fn download_args_enable_progress_without_leaking_token() {
        let (args, cfg) = curl_download_args(
            "https://example.com/asset",
            Some("example-value"),
            Some("application/octet-stream"),
            Path::new("/tmp/asset"),
        );

        assert!(args.contains(&"--progress-bar".to_string()));
        assert!(!args.iter().any(|arg| arg.contains("example-value")));
        assert!(cfg.unwrap().contains("Authorization: Bearer example-value"));
    }

    #[test]
    fn text_args_remain_silent() {
        let (args, _cfg) = curl_args("https://example.com/api", None, None, None);

        assert!(args.contains(&"-sSLf".to_string()));
        assert!(!args.contains(&"--progress-bar".to_string()));
    }

    #[test]
    fn text_exec_captures_stdout() {
        let path = std::env::temp_dir().join(format!(
            "actrix-deploy-curl-text-test-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, r#"{"ok":true}"#).unwrap();

        let output = curl_exec(
            &["-sSLf".into(), format!("file://{}", path.display())],
            None,
        )
        .unwrap();

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), r#"{"ok":true}"#);

        let _ = std::fs::remove_file(path);
    }
}
