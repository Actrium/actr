//! SHA-256 checksum verification for downloaded/local binaries.
//!
//! A `.sha256` sidecar (as produced by `sha256sum`) has the form
//! `<hex>  <filename>`; we also accept a bare hex digest. Verification is
//! mandatory in Release mode and the default for local binaries; it can only
//! be bypassed with an explicit `--skip-verify` plus a strong warning.

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// Parse a `.sha256` sidecar into a lowercase 64-char hex digest.
///
/// Accepts both `sha256sum` output (`<hex>  <file>`) and a bare `<hex>` line.
/// Extra leading/trailing whitespace and blank lines are ignored.
pub fn parse_sha256_sum(text: &str) -> Result<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // First whitespace-delimited token is the digest.
        let token = line.split_whitespace().next().unwrap_or("");
        if is_hex64(token) {
            return Ok(token.to_ascii_lowercase());
        }
    }
    bail!("no valid 64-char hex SHA-256 digest found in checksum sidecar");
}

/// Compute the SHA-256 of a file as a lowercase hex string.
pub fn sha256_of_file(path: &Path) -> Result<String> {
    let file = File::open(path)
        .with_context(|| format!("failed to open file for hashing: {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut buf)
            .with_context(|| format!("failed reading file for hashing: {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Verify a file's SHA-256 against an expected hex digest.
pub fn verify_file(path: &Path, expected_hex: &str) -> Result<()> {
    let expected = expected_hex.trim().to_ascii_lowercase();
    if !is_hex64(&expected) {
        bail!("expected SHA-256 is not a valid 64-char hex digest: {expected_hex}");
    }
    let actual = sha256_of_file(path)?;
    if actual != expected {
        bail!(
            "SHA-256 mismatch for {}\n  expected: {}\n  actual:   {}",
            path.display(),
            expected,
            actual
        );
    }
    Ok(())
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.as_bytes().iter().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_sha256sum_format() {
        let text = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  actrix-linux-x86_64\n";
        assert_eq!(
            parse_sha256_sum(text).unwrap(),
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
    }

    #[test]
    fn parses_bare_hex() {
        let text = "  ABCDEF0123456789abcdef0123456789ABCDEF0123456789abcdef0123456789  \n";
        assert_eq!(
            parse_sha256_sum(text).unwrap(),
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
        );
    }

    #[test]
    fn rejects_invalid_sidecar() {
        assert!(parse_sha256_sum("not a digest\n").is_err());
        assert!(parse_sha256_sum("\n  \n").is_err());
    }

    #[test]
    fn verifies_known_file() {
        let dir = std::env::temp_dir();
        let path = dir.join("actrix-deploy-checksum-test.bin");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"hello world").unwrap();
        // sha256sum of "hello world" (no trailing newline)
        let expected = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        assert!(verify_file(&path, expected).is_ok());
        assert!(
            verify_file(
                &path,
                "0000000000000000000000000000000000000000000000000000000000000000"
            )
            .is_err()
        );
        let _ = std::fs::remove_file(&path);
    }
}
