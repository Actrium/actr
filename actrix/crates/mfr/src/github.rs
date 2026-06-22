//! GitHub repository verification for manufacturer identity.
//!
//! Flow:
//! 1. User creates a public repo `{login}/actr-mfr-verify` with a file
//!    named `{actrix_domain}.txt` containing the challenge token
//! 2. User triggers verification via the API
//! 3. We fetch the file via GitHub Contents API and check that it contains the token
//!
//! Using a public repo because GitHub orgs cannot create Gists.

use crate::MfrError;
use serde::Deserialize;

/// Fixed repo name the user must create under their GitHub account/org.
pub const VERIFY_REPO: &str = "actr-mfr-verify";

#[derive(Deserialize)]
struct ContentsResponse {
    content: Option<String>,
}

/// Build the verify filename from the actrix domain.
/// e.g. `actrix.s15.kookyleo.space` → `actrix.s15.kookyleo.space.txt`
pub fn verify_filename(domain: &str) -> String {
    format!("{domain}.txt")
}

/// Verify that `{github_login}/{VERIFY_REPO}/{domain}.txt` contains the expected token.
pub async fn verify_repo(
    github_login: &str,
    expected_token: &str,
    domain: &str,
) -> Result<bool, MfrError> {
    let filename = verify_filename(domain);
    let url = format!(
        "https://api.github.com/repos/{}/{}/contents/{}",
        github_login, VERIFY_REPO, filename,
    );

    let client = reqwest::Client::builder()
        .user_agent("actrix-mfr/0.1")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| MfrError::GitHub(format!("http client error: {e}")))?;

    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| MfrError::GitHub(format!("failed to fetch verification file: {e}")))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(MfrError::VerificationFailed(format!(
            "Repo '{github_login}/{VERIFY_REPO}' or file '{filename}' not found — ensure the repo is public"
        )));
    }

    // Read rate limit headers
    let remaining: Option<u64> = resp
        .headers()
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    let reset: Option<i64> = resp
        .headers()
        .get("x-ratelimit-reset")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());

    // Handle rate limit exceeded (403)
    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        let reset_info = reset
            .map(|ts| format!(", resets at unix timestamp {ts}"))
            .unwrap_or_default();
        return Err(MfrError::GitHub(format!(
            "rate limit exceeded{reset_info} — consider using a GitHub token"
        )));
    }

    // Warn when quota is low
    if let Some(rem) = remaining {
        if rem < 5 {
            platform::recording::warn!(
                "GitHub API rate limit low: {rem} requests remaining (resets at {:?})",
                reset
            );
        }
    }

    if !resp.status().is_success() {
        return Err(MfrError::GitHub(format!(
            "GitHub API returned {}",
            resp.status()
        )));
    }

    let file: ContentsResponse = resp
        .json()
        .await
        .map_err(|e| MfrError::GitHub(format!("failed to parse response: {e}")))?;

    let content_b64 = file.content.unwrap_or_default();
    // GitHub base64 contains embedded newlines — strip whitespace before decoding
    let clean: String = content_b64.chars().filter(|c| !c.is_whitespace()).collect();

    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&clean)
        .map_err(|e| MfrError::GitHub(format!("failed to decode file content: {e}")))?;

    let text = String::from_utf8_lossy(&bytes);
    Ok(text.contains(expected_token))
}

/// Build the public repo URL for record-keeping.
pub fn repo_url(github_login: &str) -> String {
    format!("https://github.com/{}/{}", github_login, VERIFY_REPO)
}
