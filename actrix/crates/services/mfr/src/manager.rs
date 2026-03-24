use crate::{
    MfrError, crypto, github,
    model::{ActrPackage, GitHubRepoChallenge, Manufacturer, MfrStatus, PkgStatus},
    reserved,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum KeySource {
    /// Server generated the keypair; private_key is present.
    Generated,
    /// User uploaded their own public key; private_key is absent.
    Uploaded,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ActivateResponse {
    /// How the key was provisioned.
    pub key_source: KeySource,
    /// Ed25519 private key, base64. Present ONLY when key_source == Generated.
    /// Returned ONCE — never stored by actrix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_key: Option<String>,
    pub certificate: MfrCertificate,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MfrCertificate {
    pub mfr_name: String,
    pub mfr_pubkey: String,
    pub issued_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PublishRequest {
    pub manufacturer: String,
    pub name: String,
    pub version: String,
    /// Target platform (e.g. "wasm32-wasip1", "x86_64-unknown-linux-gnu")
    #[serde(default = "default_target")]
    pub target: String,
    /// Full content of actr.toml (with binary_hash field populated)
    pub manifest: String,
    /// base64 Ed25519 signature by MFR private key over manifest bytes
    pub signature: String,
    /// Proto files JSON for filing/audit (optional)
    #[serde(default)]
    pub proto_files: Option<serde_json::Value>,
}

fn default_target() -> String {
    "wasm32-wasip1".to_string()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MfrPublicInfo {
    pub id: i64,
    pub name: String,
    pub public_key: String,
    pub certificate: MfrCertificate,
}

pub struct MfrManager {
    pool: SqlitePool,
    /// Domain of this actrix node, used as the verification filename.
    domain: String,
}

impl MfrManager {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            domain: String::new(),
        }
    }

    pub fn with_domain(mut self, domain: String) -> Self {
        self.domain = domain;
        self
    }

    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// Step 1: Apply for manufacturer registration via GitHub identity.
    /// The GitHub login (user or org) becomes the manufacturer name.
    /// Returns a challenge token that the user must place in a public repo.
    pub async fn apply(
        &self,
        github_login: &str,
        contact: Option<&str>,
    ) -> Result<(Manufacturer, GitHubRepoChallenge), MfrError> {
        let login = github_login.to_ascii_lowercase();
        reserved::validate_github_login(&login)?;
        let mfr = Manufacturer::create(&self.pool, &login, contact).await?;
        let challenge = GitHubRepoChallenge::create(&self.pool, mfr.id).await?;
        platform::recording::info!("MFR application received: github_login={}", login,);
        Ok((mfr, challenge))
    }

    /// Step 2: Verify ownership by checking a public GitHub repo.
    ///
    /// Looks for `{mfr.name}/actr-mfr-verify/{domain}.txt` containing the challenge token.
    ///
    /// If `user_public_key` is Some, the user's own Ed25519 public key is used (uploaded mode).
    /// If None, a new keypair is generated and the private key is returned once (generated mode).
    pub async fn verify_github(
        &self,
        mfr_id: i64,
        user_public_key: Option<&str>,
    ) -> Result<ActivateResponse, MfrError> {
        let mut mfr = Manufacturer::get(&self.pool, mfr_id)
            .await?
            .ok_or(MfrError::NotFound)?;

        if mfr.status != MfrStatus::Pending {
            return Err(MfrError::InvalidStatus(format!(
                "cannot verify MFR with status: {}",
                mfr.status
            )));
        }

        let mut challenge = GitHubRepoChallenge::get_active(&self.pool, mfr_id)
            .await?
            .ok_or(MfrError::ChallengeNotFound)?;

        let filename = github::verify_filename(&self.domain);
        let verified = github::verify_repo(&mfr.name, &challenge.token, &self.domain).await?;
        if !verified {
            return Err(MfrError::VerificationFailed(format!(
                "{filename} does not contain the expected challenge token"
            )));
        }

        let url = github::repo_url(&mfr.name);
        challenge.mark_verified(&self.pool, &url).await?;

        let response = self.activate_with_key(&mut mfr, user_public_key).await?;
        platform::recording::info!(
            "MFR verified via GitHub repo: mfr_id={}, name={}, key_source={:?}",
            mfr_id,
            mfr.name,
            response.key_source
        );
        Ok(response)
    }

    /// Admin: manually approve without GitHub verification (for private deployments).
    ///
    /// If `user_public_key` is Some, the user's own Ed25519 public key is used (uploaded mode).
    /// If None, a new keypair is generated and the private key is returned once (generated mode).
    pub async fn admin_approve(
        &self,
        mfr_id: i64,
        user_public_key: Option<&str>,
    ) -> Result<ActivateResponse, MfrError> {
        let mut mfr = Manufacturer::get(&self.pool, mfr_id)
            .await?
            .ok_or(MfrError::NotFound)?;

        let response = self.activate_with_key(&mut mfr, user_public_key).await?;
        platform::recording::info!(
            "MFR manually approved by admin: mfr_id={}, name={}, key_source={:?}",
            mfr_id,
            mfr.name,
            response.key_source
        );
        Ok(response)
    }

    /// Common key provisioning logic for both verify_github and admin_approve.
    ///
    /// - `user_public_key = None` → generate a new Ed25519 keypair, return private key.
    /// - `user_public_key = Some(b64)` → validate and use the provided public key, no private key returned.
    async fn activate_with_key(
        &self,
        mfr: &mut Manufacturer,
        user_public_key: Option<&str>,
    ) -> Result<ActivateResponse, MfrError> {
        let (key_source, private_key, public_key) = match user_public_key {
            Some(pk) => {
                crypto::validate_public_key(pk)?;
                (KeySource::Uploaded, None, pk.to_string())
            }
            None => {
                let (priv_key, pub_key) = crypto::generate_keypair();
                (KeySource::Generated, Some(priv_key), pub_key)
            }
        };

        mfr.activate(&self.pool, public_key).await?;

        let expires_at = mfr.key_expires_at.ok_or(MfrError::CertificateExpired)?;
        Ok(ActivateResponse {
            key_source,
            private_key,
            certificate: MfrCertificate {
                mfr_name: mfr.name.clone(),
                mfr_pubkey: mfr.public_key.clone(),
                issued_at: mfr.verified_at.unwrap_or(mfr.created_at),
                expires_at,
            },
        })
    }

    /// Get the active (unexpired, unverified) challenge for a pending MFR.
    pub async fn get_challenge(&self, mfr_id: i64) -> Result<GitHubRepoChallenge, MfrError> {
        let mfr = Manufacturer::get(&self.pool, mfr_id)
            .await?
            .ok_or(MfrError::NotFound)?;
        if mfr.status != MfrStatus::Pending {
            return Err(MfrError::InvalidStatus(format!(
                "MFR is not pending (status: {})",
                mfr.status
            )));
        }
        GitHubRepoChallenge::get_active(&self.pool, mfr_id)
            .await?
            .ok_or(MfrError::ChallengeNotFound)
    }

    pub async fn get_status(&self, mfr_id: i64) -> Result<Manufacturer, MfrError> {
        Manufacturer::get(&self.pool, mfr_id)
            .await?
            .ok_or(MfrError::NotFound)
    }

    pub async fn resolve_by_name(&self, name: &str) -> Result<MfrPublicInfo, MfrError> {
        let mfr = Manufacturer::get_by_name(&self.pool, name)
            .await?
            .ok_or(MfrError::NotFound)?;
        if mfr.status != MfrStatus::Active {
            return Err(MfrError::InvalidStatus(format!(
                "MFR '{}' is not active",
                name
            )));
        }
        let expires_at = mfr.key_expires_at.ok_or(MfrError::CertificateExpired)?;
        let cert = MfrCertificate {
            mfr_name: mfr.name.clone(),
            mfr_pubkey: mfr.public_key.clone(),
            issued_at: mfr.verified_at.unwrap_or(mfr.created_at),
            expires_at,
        };
        Ok(MfrPublicInfo {
            id: mfr.id,
            name: mfr.name,
            public_key: mfr.public_key,
            certificate: cert,
        })
    }

    pub async fn publish_package(&self, req: PublishRequest) -> Result<ActrPackage, MfrError> {
        let mfr = Manufacturer::get_by_name(&self.pool, &req.manufacturer)
            .await?
            .ok_or(MfrError::NotFound)?;
        if mfr.status != MfrStatus::Active {
            return Err(MfrError::InvalidStatus(format!(
                "MFR '{}' is not active",
                req.manufacturer
            )));
        }

        // Check if signing key has expired — None means key_expires_at was not set,
        // which is treated as invalid (not "never expires")
        let key_expires = mfr.key_expires_at.ok_or(MfrError::CertificateExpired)?;
        if chrono::Utc::now().timestamp() > key_expires {
            return Err(MfrError::CertificateExpired);
        }

        // Verify signature: MFR's Ed25519 private key signed the manifest bytes
        let valid =
            crypto::verify_signature(req.manifest.as_bytes(), &req.signature, &mfr.public_key)?;
        if !valid {
            return Err(MfrError::InvalidSignature);
        }

        // Serialize proto_files JSON to string for storage
        let proto_files_str = req.proto_files.as_ref().map(|v| v.to_string());

        let pkg = ActrPackage::publish(
            &self.pool,
            mfr.id,
            &req.manufacturer,
            &req.name,
            &req.version,
            &req.target,
            &req.manifest,
            &req.signature,
            proto_files_str.as_deref(),
        )
        .await?;

        if req.proto_files.is_some() {
            platform::recording::info!(
                "actor package published with proto filing: type_str={}, mfr_id={}",
                pkg.type_str,
                mfr.id
            );
        } else {
            platform::recording::info!(
                "actor package published: type_str={}, mfr_id={}",
                pkg.type_str,
                mfr.id
            );
        }
        Ok(pkg)
    }

    pub async fn get_package(&self, type_str: &str) -> Result<ActrPackage, MfrError> {
        ActrPackage::get_by_type(&self.pool, type_str)
            .await?
            .ok_or(MfrError::NotFound)
    }

    pub async fn list_packages(
        &self,
        mfr_name: Option<&str>,
    ) -> Result<Vec<ActrPackage>, MfrError> {
        if let Some(name) = mfr_name {
            let mfr = Manufacturer::get_by_name(&self.pool, name)
                .await?
                .ok_or(MfrError::NotFound)?;
            ActrPackage::list_by_mfr(&self.pool, mfr.id).await
        } else {
            Ok(sqlx::query_as::<_, ActrPackage>(
                "SELECT * FROM mfr_package ORDER BY published_at DESC",
            )
            .fetch_all(&self.pool)
            .await?)
        }
    }

    pub async fn revoke_package(&self, pkg_id: i64) -> Result<(), MfrError> {
        let mut pkg = ActrPackage::get_by_id(&self.pool, pkg_id)
            .await?
            .ok_or(MfrError::NotFound)?;
        pkg.revoke(&self.pool).await?;
        platform::recording::warn!(
            "actor package revoked: pkg_id={}, type_str={}",
            pkg_id,
            pkg.type_str
        );
        Ok(())
    }

    // Admin methods
    pub async fn admin_list(
        &self,
        status: Option<MfrStatus>,
    ) -> Result<Vec<Manufacturer>, MfrError> {
        Manufacturer::list(&self.pool, status).await
    }

    pub async fn admin_suspend(&self, mfr_id: i64) -> Result<(), MfrError> {
        let mut mfr = Manufacturer::get(&self.pool, mfr_id)
            .await?
            .ok_or(MfrError::NotFound)?;
        mfr.suspend(&self.pool).await?;
        platform::recording::warn!(
            "MFR suspended by admin: mfr_id={}, name={}",
            mfr_id,
            mfr.name
        );
        Ok(())
    }

    pub async fn admin_reinstate(&self, mfr_id: i64) -> Result<(), MfrError> {
        let mut mfr = Manufacturer::get(&self.pool, mfr_id)
            .await?
            .ok_or(MfrError::NotFound)?;
        mfr.reinstate(&self.pool).await?;
        platform::recording::info!(
            "MFR reinstated by admin: mfr_id={}, name={}",
            mfr_id,
            mfr.name
        );
        Ok(())
    }

    pub async fn admin_delete(&self, mfr_id: i64) -> Result<(), MfrError> {
        Manufacturer::delete(&self.pool, mfr_id).await?;
        platform::recording::warn!("MFR deleted by admin: mfr_id={}", mfr_id);
        Ok(())
    }
}

/// Public API for AIS integration: check if a type_str is a valid, active package.
/// Reserved names always return true.
///
/// When `target` is provided, lookup is narrowed to that specific platform.
/// When `manifest_hash` is provided, the stored manifest is SHA-256 compared for content integrity.
pub async fn lookup_package(
    pool: &SqlitePool,
    type_str: &str,
    target: Option<&str>,
    manifest_hash: Option<&[u8]>,
) -> Result<bool, MfrError> {
    let pkg = if let Some(t) = target {
        ActrPackage::get_by_type_and_target(pool, type_str, t).await?
    } else {
        ActrPackage::get_by_type(pool, type_str).await?
    };

    match pkg {
        Some(p) if p.status == PkgStatus::Active => {
            // C1: manifest hash comparison (defense-in-depth)
            if let Some(expected_hash) = manifest_hash {
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(p.manifest.as_bytes());
                let stored_hash = hasher.finalize();
                if stored_hash.as_slice() != expected_hash {
                    platform::recording::warn!(
                        "manifest hash mismatch for type_str={}, target={:?}",
                        type_str,
                        target
                    );
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}
