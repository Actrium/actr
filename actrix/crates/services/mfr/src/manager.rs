use crate::{
    MfrError, crypto, github,
    model::{ActrPackage, GitHubGistChallenge, Manufacturer, MfrStatus, PkgStatus},
    reserved,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Serialize, Deserialize)]
pub struct MfrKeychain {
    /// Ed25519 private key, base64. Returned ONCE — never stored by actrix.
    pub private_key: String,
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
    /// Full content of actr.toml (with binary_hash field populated)
    pub manifest: String,
    /// base64 Ed25519 signature by MFR private key over manifest bytes
    pub signature: String,
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
    /// Returns a challenge token that the user must place in a public Gist.
    pub async fn apply(
        &self,
        github_login: &str,
        contact: Option<&str>,
    ) -> Result<(Manufacturer, GitHubGistChallenge), MfrError> {
        let login = github_login.to_ascii_lowercase();
        reserved::validate_github_login(&login)?;
        let mfr = Manufacturer::create(&self.pool, &login, contact).await?;
        let challenge = GitHubGistChallenge::create(&self.pool, mfr.id).await?;
        platform::recording::info!("MFR application received: github_login={}", login,);
        Ok((mfr, challenge))
    }

    /// Step 2: Verify ownership by checking a public GitHub repo.
    ///
    /// Looks for `{mfr.name}/actr-mfr-verify/{domain}.txt` containing the challenge token.
    pub async fn verify_github(&self, mfr_id: i64) -> Result<MfrKeychain, MfrError> {
        let mut mfr = Manufacturer::get(&self.pool, mfr_id)
            .await?
            .ok_or(MfrError::NotFound)?;

        if mfr.status != MfrStatus::Pending {
            return Err(MfrError::InvalidStatus(format!(
                "cannot verify MFR with status: {}",
                mfr.status
            )));
        }

        let mut challenge = GitHubGistChallenge::get_active(&self.pool, mfr_id)
            .await?
            .ok_or(MfrError::ChallengeNotFound)?;

        let filename = github::verify_filename(&self.domain);
        let verified = github::verify_repo(&mfr.name, &challenge.token, &self.domain).await?;
        if !verified {
            return Err(MfrError::VerificationFailed(format!(
                "{filename} does not contain the expected challenge token"
            )));
        }

        let (private_key, public_key) = crypto::generate_keypair();
        let url = github::repo_url(&mfr.name);
        challenge.mark_verified(&self.pool, &url).await?;
        mfr.activate(&self.pool, public_key.clone()).await?;

        let keychain = self.build_keychain(&mfr, private_key);
        platform::recording::info!(
            "MFR verified via GitHub repo and keychain issued: mfr_id={}, name={}",
            mfr_id,
            mfr.name
        );
        Ok(keychain)
    }

    /// Admin: manually approve without GitHub verification (for private deployments).
    pub async fn admin_approve(&self, mfr_id: i64) -> Result<MfrKeychain, MfrError> {
        let mut mfr = Manufacturer::get(&self.pool, mfr_id)
            .await?
            .ok_or(MfrError::NotFound)?;
        let (private_key, public_key) = crypto::generate_keypair();
        mfr.activate(&self.pool, public_key).await?;
        platform::recording::info!(
            "MFR manually approved by admin: mfr_id={}, name={}",
            mfr_id,
            mfr.name
        );
        Ok(self.build_keychain(&mfr, private_key))
    }

    fn build_keychain(&self, mfr: &Manufacturer, private_key: String) -> MfrKeychain {
        use chrono::Utc;
        let now = Utc::now().timestamp();
        let expires_at = now + 365 * 24 * 3600; // 1 year
        MfrKeychain {
            private_key,
            certificate: MfrCertificate {
                mfr_name: mfr.name.clone(),
                mfr_pubkey: mfr.public_key.clone(),
                issued_at: now,
                expires_at,
            },
        }
    }

    /// Get the active (unexpired, unverified) challenge for a pending MFR.
    pub async fn get_challenge(&self, mfr_id: i64) -> Result<GitHubGistChallenge, MfrError> {
        let mfr = Manufacturer::get(&self.pool, mfr_id)
            .await?
            .ok_or(MfrError::NotFound)?;
        if mfr.status != MfrStatus::Pending {
            return Err(MfrError::InvalidStatus(format!(
                "MFR is not pending (status: {})",
                mfr.status
            )));
        }
        GitHubGistChallenge::get_active(&self.pool, mfr_id)
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
        let cert = {
            use chrono::Utc;
            let now = Utc::now().timestamp();
            MfrCertificate {
                mfr_name: mfr.name.clone(),
                mfr_pubkey: mfr.public_key.clone(),
                issued_at: now,
                expires_at: now + 365 * 24 * 3600,
            }
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

        // Verify signature: MFR's Ed25519 private key signed the manifest bytes
        let valid =
            crypto::verify_signature(req.manifest.as_bytes(), &req.signature, &mfr.public_key)?;
        if !valid {
            return Err(MfrError::InvalidSignature);
        }

        let pkg = ActrPackage::publish(
            &self.pool,
            mfr.id,
            &req.manufacturer,
            &req.name,
            &req.version,
            &req.manifest,
            &req.signature,
        )
        .await?;
        platform::recording::info!(
            "actor package published: type_str={}, mfr_id={}",
            pkg.type_str,
            mfr.id
        );
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
pub async fn lookup_package(pool: &SqlitePool, type_str: &str) -> Result<bool, MfrError> {
    // Extract manufacturer from "manufacturer:name:version"
    let manufacturer = type_str.split(':').next().unwrap_or("");
    if reserved::is_reserved(manufacturer) {
        return Ok(true);
    }
    let pkg = ActrPackage::get_by_type(pool, type_str).await?;
    Ok(pkg.map(|p| p.status == PkgStatus::Active).unwrap_or(false))
}
