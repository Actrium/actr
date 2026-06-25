//! Artifact source normalization.
//!
//! Three binary sources (`--tag`, `--latest`, `--binary-path`) are normalized
//! into a single [`ResolvedArtifact`]: a verified local file plus its version
//! string. Release sources download + SHA-256 verify; local sources verify
//! against an explicit `--sha256-path` unless `--skip-verify` is given.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

use crate::checksum::{parse_sha256_sum, verify_file};
use crate::release::{AssetKind, TagTarget, download_asset, fetch_release};

/// Where the actrix binary comes from.
#[derive(Debug, Clone)]
pub enum Source {
    /// A specific GitHub Release tag, e.g. `v0.4.3`.
    Tag(String),
    /// The latest stable GitHub Release.
    Latest,
    /// A pre-downloaded local binary file.
    BinaryPath(PathBuf),
}

/// A resolved, verified binary ready to install.
#[derive(Debug)]
pub struct ResolvedArtifact {
    /// Path to the verified binary file.
    pub path: PathBuf,
    /// Version string (release tag or explicit `--version`), e.g. `v0.4.3`.
    pub version: String,
    /// Whether `path` is a temporary file the caller should remove after use.
    pub is_temp: bool,
}

/// Resolve a [`Source`] into a verified [`ResolvedArtifact`].
///
/// - `version` is required for `BinaryPath` (ignored for `Tag`/`Latest`).
/// - `sha256_path` is required for `BinaryPath` unless `skip_verify` is set.
/// - `repo` and `token` are used only for `Tag`/`Latest`.
pub fn resolve(
    source: &Source,
    version: Option<&str>,
    sha256_path: Option<&Path>,
    skip_verify: bool,
    repo: &str,
    token: Option<&str>,
) -> Result<ResolvedArtifact> {
    match source {
        Source::BinaryPath(path) => {
            let version =
                version.ok_or_else(|| anyhow::anyhow!("--binary-path requires --version"))?;
            if !path.exists() {
                bail!("binary path does not exist: {}", path.display());
            }

            if skip_verify {
                warn_skip_verify();
            } else {
                let sha256_path = sha256_path.ok_or_else(|| {
                    anyhow::anyhow!("--binary-path requires --sha256-path (or --skip-verify)")
                })?;
                let text = std::fs::read_to_string(sha256_path).with_context(|| {
                    format!("failed to read sha256 file: {}", sha256_path.display())
                })?;
                let expected = parse_sha256_sum(&text)?;
                verify_file(path, &expected)?;
            }

            Ok(ResolvedArtifact {
                path: path.clone(),
                version: version.to_string(),
                is_temp: false,
            })
        }
        Source::Tag(_) | Source::Latest => {
            let target = match source {
                Source::Latest => TagTarget::Latest,
                Source::Tag(t) => TagTarget::Tag(t.clone()),
                Source::BinaryPath(_) => unreachable!(),
            };

            println!("📦 Fetching release info for {repo} ({target:?})...");
            let info = fetch_release(repo, &target, token)?;
            let version = info.tag_name.clone();
            let kind = AssetKind::from_host_arch()?;

            let bin_asset = info.find_asset(kind.asset_name()).ok_or_else(|| {
                anyhow::anyhow!("release {} has no asset {}", version, kind.asset_name())
            })?;

            let dest =
                std::env::temp_dir().join(format!("actrix-deploy-{version}-{}", kind.asset_name()));
            println!("⬇️  Downloading {} ...", bin_asset.name);
            download_asset(bin_asset, token, &dest)?;

            if skip_verify {
                warn_skip_verify();
            } else {
                let sha_name = kind.sha256_asset_name();
                let sha_asset = info.find_asset(&sha_name).ok_or_else(|| {
                    anyhow::anyhow!(
                        "release {version} has no `{sha_name}` sidecar (required for verification; use --skip-verify to bypass)"
                    )
                })?;
                let sha_dest =
                    std::env::temp_dir().join(format!("actrix-deploy-{version}-{sha_name}"));
                download_asset(sha_asset, token, &sha_dest)?;
                let text = std::fs::read_to_string(&sha_dest).with_context(|| {
                    format!("failed to read downloaded sha256: {}", sha_dest.display())
                })?;
                let expected = parse_sha256_sum(&text)?;
                verify_file(&dest, &expected)?;
                let _ = std::fs::remove_file(&sha_dest);
                println!("✅ Checksum verified for {}", bin_asset.name);
            }

            Ok(ResolvedArtifact {
                path: dest,
                version,
                is_temp: true,
            })
        }
    }
}

fn warn_skip_verify() {
    eprintln!("⚠️  --skip-verify: SHA-256 verification bypassed. Not safe for production.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn resolves_local_binary_with_checksum() {
        let dir = std::env::temp_dir();
        let bin = dir.join("actrix-deploy-artifact-test.bin");
        let sha = dir.join("actrix-deploy-artifact-test.bin.sha256");
        std::fs::write(&bin, b"hello world").unwrap();
        // sha256sum of "hello world" (no trailing newline)
        let digest = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        let mut f = std::fs::File::create(&sha).unwrap();
        writeln!(f, "{digest}  actrix-deploy-artifact-test.bin").unwrap();

        let art = resolve(
            &Source::BinaryPath(bin.clone()),
            Some("v9.9.9"),
            Some(&sha),
            false,
            "Actrium/actr",
            None,
        )
        .unwrap();
        assert_eq!(art.version, "v9.9.9");
        assert_eq!(art.path, bin);
        assert!(!art.is_temp);

        let _ = std::fs::remove_file(&bin);
        let _ = std::fs::remove_file(&sha);
    }

    #[test]
    fn local_binary_requires_version() {
        let dir = std::env::temp_dir();
        let bin = dir.join("actrix-deploy-artifact-test2.bin");
        std::fs::write(&bin, b"x").unwrap();
        let err = resolve(
            &Source::BinaryPath(bin.clone()),
            None,
            None,
            true,
            "Actrium/actr",
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("--version"));
        let _ = std::fs::remove_file(&bin);
    }

    #[test]
    fn local_binary_requires_sha256_without_skip() {
        let dir = std::env::temp_dir();
        let bin = dir.join("actrix-deploy-artifact-test3.bin");
        std::fs::write(&bin, b"x").unwrap();
        let err = resolve(
            &Source::BinaryPath(bin.clone()),
            Some("v1.0.0"),
            None,
            false,
            "Actrium/actr",
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("--sha256-path"));
        let _ = std::fs::remove_file(&bin);
    }
}
