/// MFR 模块集成测试
///
/// 所有需要数据库的测试使用 in-process SQLite 内存库，每个测试独立 pool，互不干扰。
use sqlx::SqlitePool;

use crate::{
    MfrError, crypto,
    manager::{MfrManager, PublishRequest, lookup_package},
    model::{ActrPackage, GitHubRepoChallenge, Manufacturer, MfrStatus, PkgStatus},
    reserved::{is_reserved, validate_github_login},
};

// ─── 测试辅助 ────────────────────────────────────────────────────────────────

async fn setup_test_pool() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("failed to create in-memory sqlite pool");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS mfr (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            public_key TEXT NOT NULL DEFAULT '',
            contact TEXT,
            status TEXT NOT NULL DEFAULT 'pending',
            created_at INTEGER NOT NULL,
            updated_at INTEGER,
            verified_at INTEGER,
            suspended_at INTEGER,
            revoked_at INTEGER,
            key_expires_at INTEGER
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create mfr table");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS mfr_challenge (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            mfr_id INTEGER NOT NULL REFERENCES mfr(id),
            token TEXT NOT NULL,
            verify_url TEXT NOT NULL DEFAULT '',
            expires_at INTEGER NOT NULL,
            verified_at INTEGER,
            created_at INTEGER NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create mfr_challenge table");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS mfr_package (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            mfr_id INTEGER NOT NULL REFERENCES mfr(id),
            manufacturer TEXT NOT NULL,
            name TEXT NOT NULL,
            version TEXT NOT NULL,
            type_str TEXT NOT NULL,
            manifest TEXT NOT NULL,
            signature TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'active',
            published_at INTEGER NOT NULL,
            revoked_at INTEGER,
            UNIQUE(manufacturer, name, version)
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create mfr_package table");

    pool
}

// ─── reserved.rs 纯单元测试 ──────────────────────────────────────────────────

#[test]
fn test_reserved_names_exact() {
    assert!(is_reserved("self"));
    assert!(is_reserved("acme"));
    assert!(is_reserved("actrix"));
}

#[test]
fn test_reserved_names_case_insensitive() {
    assert!(is_reserved("SELF"));
    assert!(is_reserved("Acme"));
    assert!(is_reserved("ACTRIX"));
}

#[test]
fn test_non_reserved_names() {
    assert!(!is_reserved("mycompany"));
    assert!(!is_reserved("apple"));
    assert!(!is_reserved("openai"));
}

#[test]
fn test_validate_github_login_reserved() {
    assert!(matches!(
        validate_github_login("self"),
        Err(MfrError::ReservedName(_))
    ));
    assert!(matches!(
        validate_github_login("acme"),
        Err(MfrError::ReservedName(_))
    ));
    assert!(matches!(
        validate_github_login("actrix"),
        Err(MfrError::ReservedName(_))
    ));
}

#[test]
fn test_validate_github_login_too_long() {
    let long = "a".repeat(40);
    assert!(matches!(
        validate_github_login(&long),
        Err(MfrError::InvalidName(_))
    ));
}

#[test]
fn test_validate_github_login_empty() {
    assert!(matches!(
        validate_github_login(""),
        Err(MfrError::InvalidName(_))
    ));
}

#[test]
fn test_validate_github_login_hyphen_boundary() {
    assert!(matches!(
        validate_github_login("-abc"),
        Err(MfrError::InvalidName(_))
    ));
    assert!(matches!(
        validate_github_login("abc-"),
        Err(MfrError::InvalidName(_))
    ));
}

#[test]
fn test_validate_github_login_consecutive_hyphens() {
    assert!(matches!(
        validate_github_login("a--b"),
        Err(MfrError::InvalidName(_))
    ));
}

#[test]
fn test_validate_github_login_invalid_chars() {
    assert!(matches!(
        validate_github_login("my_company"),
        Err(MfrError::InvalidName(_))
    ));
    assert!(matches!(
        validate_github_login("my company"),
        Err(MfrError::InvalidName(_))
    ));
    assert!(matches!(
        validate_github_login("com.example"),
        Err(MfrError::InvalidName(_))
    ));
}

#[test]
fn test_validate_github_login_valid() {
    assert!(validate_github_login("octocat").is_ok());
    assert!(validate_github_login("my-company").is_ok());
    assert!(validate_github_login("user123").is_ok());
    assert!(validate_github_login("a").is_ok());
    let max = "a".repeat(39);
    assert!(validate_github_login(&max).is_ok());
}

// ─── crypto.rs 纯单元测试 ────────────────────────────────────────────────────

#[test]
fn test_generate_keypair_roundtrip() {
    use base64::Engine as _;

    let (private_b64, public_b64) = crypto::generate_keypair();
    assert!(!private_b64.is_empty());
    assert!(!public_b64.is_empty());

    let priv_bytes = base64::engine::general_purpose::STANDARD
        .decode(&private_b64)
        .expect("private key should be valid base64");
    assert_eq!(
        priv_bytes.len(),
        32,
        "Ed25519 private key should be 32 bytes"
    );

    let pub_bytes = base64::engine::general_purpose::STANDARD
        .decode(&public_b64)
        .expect("public key should be valid base64");
    assert_eq!(pub_bytes.len(), 32, "Ed25519 public key should be 32 bytes");
}

#[test]
fn test_generate_keypair_unique() {
    let (priv1, pub1) = crypto::generate_keypair();
    let (priv2, pub2) = crypto::generate_keypair();
    assert_ne!(priv1, priv2);
    assert_ne!(pub1, pub2);
}

#[test]
fn test_verify_signature_valid() {
    use base64::Engine as _;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();
    let message = b"hello mfr";
    let sig = signing_key.sign(message);

    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());
    let pub_b64 = base64::engine::general_purpose::STANDARD.encode(verifying_key.to_bytes());

    let result = crypto::verify_signature(message, &sig_b64, &pub_b64)
        .expect("verify_signature should not error on valid inputs");
    assert!(result, "valid signature should verify as true");
}

#[test]
fn test_verify_signature_invalid_zeros() {
    use base64::Engine as _;
    let (_, pub_b64) = crypto::generate_keypair();
    let bad_sig = base64::engine::general_purpose::STANDARD.encode([0u8; 64]);
    let result = crypto::verify_signature(b"message", &bad_sig, &pub_b64);
    assert!(
        matches!(result, Ok(false) | Err(MfrError::Crypto(_))),
        "all-zero signature should fail: {result:?}"
    );
}

#[test]
fn test_verify_signature_wrong_key() {
    use base64::Engine as _;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    let key1 = SigningKey::generate(&mut OsRng);
    let key2 = SigningKey::generate(&mut OsRng);
    let message = b"test message";
    let sig = key1.sign(message);

    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());
    let wrong_pub_b64 =
        base64::engine::general_purpose::STANDARD.encode(key2.verifying_key().to_bytes());

    let result = crypto::verify_signature(message, &sig_b64, &wrong_pub_b64)
        .expect("should not return error for valid encoding");
    assert!(!result, "signature with wrong key should verify as false");
}

#[test]
fn test_verify_signature_bad_pubkey_encoding() {
    let bad_pub = "not-valid-base64!!!";
    let result = crypto::verify_signature(b"msg", "anysig", bad_pub);
    assert!(
        matches!(result, Err(MfrError::Crypto(_))),
        "bad base64 pubkey should return Crypto error"
    );
}

// ─── model/manufacturer.rs 测试（需 DB）─────────────────────────────────────

#[tokio::test]
async fn test_manufacturer_create_and_get() {
    let pool = setup_test_pool().await;
    let mfr = Manufacturer::create(&pool, "testco", Some("admin@testco.com"))
        .await
        .expect("create should succeed");

    assert_eq!(mfr.name, "testco");
    assert_eq!(mfr.status, MfrStatus::Pending);
    assert!(mfr.verified_at.is_none());
    assert_eq!(mfr.contact.as_deref(), Some("admin@testco.com"));

    let found = Manufacturer::get(&pool, mfr.id)
        .await
        .expect("get should succeed")
        .expect("should find created manufacturer");
    assert_eq!(found.name, "testco");
    assert_eq!(found.id, mfr.id);
}

#[tokio::test]
async fn test_manufacturer_get_nonexistent() {
    let pool = setup_test_pool().await;
    let found = Manufacturer::get(&pool, 9999).await.unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn test_manufacturer_get_by_name() {
    let pool = setup_test_pool().await;
    let mfr = Manufacturer::create(&pool, "namedco", None).await.unwrap();

    let found = Manufacturer::get_by_name(&pool, "namedco")
        .await
        .unwrap()
        .expect("should find by name");
    assert_eq!(found.id, mfr.id);

    let missing = Manufacturer::get_by_name(&pool, "nobody").await.unwrap();
    assert!(missing.is_none());
}

#[tokio::test]
async fn test_manufacturer_duplicate_name() {
    let pool = setup_test_pool().await;
    Manufacturer::create(&pool, "dupco", None).await.unwrap();
    let result = Manufacturer::create(&pool, "dupco", None).await;
    assert!(
        matches!(result, Err(MfrError::AlreadyExists(_))),
        "duplicate name should return AlreadyExists"
    );
}

#[tokio::test]
async fn test_manufacturer_activate() {
    let pool = setup_test_pool().await;
    let mut mfr = Manufacturer::create(&pool, "activeco", None).await.unwrap();

    mfr.activate(&pool, "pubkey_base64".to_string())
        .await
        .expect("activate from pending should succeed");

    assert_eq!(mfr.status, MfrStatus::Active);
    assert_eq!(mfr.public_key, "pubkey_base64");
    assert!(mfr.verified_at.is_some());
    assert!(mfr.updated_at.is_some());

    let from_db = Manufacturer::get(&pool, mfr.id).await.unwrap().unwrap();
    assert_eq!(from_db.status, MfrStatus::Active);
    assert_eq!(from_db.public_key, "pubkey_base64");
}

#[tokio::test]
async fn test_manufacturer_lifecycle_full() {
    let pool = setup_test_pool().await;
    let mut mfr = Manufacturer::create(&pool, "lifecycle", None)
        .await
        .unwrap();

    mfr.activate(&pool, "pubkey123".to_string()).await.unwrap();
    assert_eq!(mfr.status, MfrStatus::Active);

    mfr.suspend(&pool).await.unwrap();
    assert_eq!(mfr.status, MfrStatus::Suspended);
    assert!(mfr.suspended_at.is_some());

    mfr.reinstate(&pool).await.unwrap();
    assert_eq!(mfr.status, MfrStatus::Active);

    mfr.revoke(&pool).await.unwrap();
    assert_eq!(mfr.status, MfrStatus::Revoked);
    assert!(mfr.revoked_at.is_some());
}

#[tokio::test]
async fn test_manufacturer_invalid_transitions_from_pending() {
    let pool = setup_test_pool().await;
    let mut mfr = Manufacturer::create(&pool, "transco", None).await.unwrap();

    let err = mfr.suspend(&pool).await.unwrap_err();
    assert!(matches!(err, MfrError::InvalidStatus(_)));

    let err = mfr.reinstate(&pool).await.unwrap_err();
    assert!(matches!(err, MfrError::InvalidStatus(_)));
}

#[tokio::test]
async fn test_manufacturer_cannot_activate_twice() {
    let pool = setup_test_pool().await;
    let mut mfr = Manufacturer::create(&pool, "twiceco", None).await.unwrap();

    mfr.activate(&pool, "key1".to_string()).await.unwrap();
    let err = mfr.activate(&pool, "key2".to_string()).await.unwrap_err();
    assert!(matches!(err, MfrError::InvalidStatus(_)));
}

#[tokio::test]
async fn test_manufacturer_list_all() {
    let pool = setup_test_pool().await;
    Manufacturer::create(&pool, "list1", None).await.unwrap();
    Manufacturer::create(&pool, "list2", None).await.unwrap();

    let all = Manufacturer::list(&pool, None).await.unwrap();
    assert_eq!(all.len(), 2);
}

#[tokio::test]
async fn test_manufacturer_list_by_status() {
    let pool = setup_test_pool().await;
    Manufacturer::create(&pool, "statuslist1", None)
        .await
        .unwrap();
    let mut mfr2 = Manufacturer::create(&pool, "statuslist2", None)
        .await
        .unwrap();
    mfr2.activate(&pool, "pk".to_string()).await.unwrap();

    let active = Manufacturer::list(&pool, Some(MfrStatus::Active))
        .await
        .unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].name, "statuslist2");

    let pending = Manufacturer::list(&pool, Some(MfrStatus::Pending))
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].name, "statuslist1");
}

#[tokio::test]
async fn test_manufacturer_delete() {
    let pool = setup_test_pool().await;
    let mfr = Manufacturer::create(&pool, "delco", None).await.unwrap();
    let id = mfr.id;

    Manufacturer::delete(&pool, id).await.unwrap();

    let found = Manufacturer::get(&pool, id).await.unwrap();
    assert!(found.is_none());
}

// ─── model/challenge.rs 测试（需 DB）─────────────────────────────────────────

#[tokio::test]
async fn test_challenge_create() {
    let pool = setup_test_pool().await;
    let mfr = Manufacturer::create(&pool, "chco", None).await.unwrap();

    let ch = GitHubRepoChallenge::create(&pool, mfr.id).await.unwrap();

    assert!(
        ch.token.starts_with("actrix-verify="),
        "token should start with 'actrix-verify=', got: {}",
        ch.token
    );
    assert!(ch.verify_url.is_empty());
    assert!(ch.verified_at.is_none());
    assert!(ch.expires_at > ch.created_at);
    assert_eq!(ch.mfr_id, mfr.id);
}

#[tokio::test]
async fn test_challenge_get_active_found() {
    let pool = setup_test_pool().await;
    let mfr = Manufacturer::create(&pool, "activech", None).await.unwrap();
    let ch = GitHubRepoChallenge::create(&pool, mfr.id).await.unwrap();

    let active = GitHubRepoChallenge::get_active(&pool, mfr.id)
        .await
        .unwrap();
    assert!(active.is_some());
    assert_eq!(active.unwrap().id, ch.id);
}

#[tokio::test]
async fn test_challenge_get_active_none_when_empty() {
    let pool = setup_test_pool().await;
    let mfr = Manufacturer::create(&pool, "nochco", None).await.unwrap();

    let active = GitHubRepoChallenge::get_active(&pool, mfr.id)
        .await
        .unwrap();
    assert!(active.is_none());
}

#[tokio::test]
async fn test_challenge_mark_verified() {
    let pool = setup_test_pool().await;
    let mfr = Manufacturer::create(&pool, "verch", None).await.unwrap();
    let mut ch = GitHubRepoChallenge::create(&pool, mfr.id).await.unwrap();

    ch.mark_verified(&pool, "https://github.com/verch/actr-mfr-verify")
        .await
        .unwrap();
    assert!(ch.verified_at.is_some());
    assert_eq!(ch.verify_url, "https://github.com/verch/actr-mfr-verify");

    let active = GitHubRepoChallenge::get_active(&pool, mfr.id)
        .await
        .unwrap();
    assert!(
        active.is_none(),
        "verified challenge should not appear in get_active"
    );
}

#[tokio::test]
async fn test_challenge_token_unique() {
    let pool = setup_test_pool().await;
    let mfr = Manufacturer::create(&pool, "tokenco", None).await.unwrap();

    let ch1 = GitHubRepoChallenge::create(&pool, mfr.id).await.unwrap();
    let ch2 = GitHubRepoChallenge::create(&pool, mfr.id).await.unwrap();

    assert_ne!(
        ch1.token, ch2.token,
        "each challenge should have a unique token"
    );
}

// ─── model/package.rs 测试（需 DB）──────────────────────────────────────────

#[tokio::test]
async fn test_package_publish_and_get() {
    let pool = setup_test_pool().await;
    let mut mfr = Manufacturer::create(&pool, "pkgco", None).await.unwrap();
    mfr.activate(&pool, "pubkey".to_string()).await.unwrap();

    let pkg = ActrPackage::publish(
        &pool,
        mfr.id,
        "pkgco",
        "client",
        "1.0.0",
        "manifest content",
        "sig123",
    )
    .await
    .unwrap();

    assert_eq!(pkg.type_str, "pkgco:client:1.0.0");
    assert_eq!(pkg.status, PkgStatus::Active);
    assert_eq!(pkg.manufacturer, "pkgco");
    assert_eq!(pkg.name, "client");
    assert_eq!(pkg.version, "1.0.0");

    let found = ActrPackage::get_by_type(&pool, "pkgco:client:1.0.0")
        .await
        .unwrap()
        .expect("should find published package");
    assert_eq!(found.id, pkg.id);
}

#[tokio::test]
async fn test_package_get_by_type_not_found() {
    let pool = setup_test_pool().await;
    let found = ActrPackage::get_by_type(&pool, "nobody:nothing:v0")
        .await
        .unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn test_package_duplicate_rejected() {
    let pool = setup_test_pool().await;
    let mut mfr = Manufacturer::create(&pool, "dupkg", None).await.unwrap();
    mfr.activate(&pool, "pk".to_string()).await.unwrap();

    ActrPackage::publish(&pool, mfr.id, "dupkg", "svc", "1.0.0", "m", "s")
        .await
        .unwrap();
    let result = ActrPackage::publish(&pool, mfr.id, "dupkg", "svc", "1.0.0", "m2", "s2").await;
    assert!(
        matches!(result, Err(MfrError::PackageAlreadyPublished)),
        "duplicate publish should return PackageAlreadyPublished"
    );
}

#[tokio::test]
async fn test_package_revoke() {
    let pool = setup_test_pool().await;
    let mut mfr = Manufacturer::create(&pool, "revpkg", None).await.unwrap();
    mfr.activate(&pool, "pk".to_string()).await.unwrap();

    let mut pkg = ActrPackage::publish(&pool, mfr.id, "revpkg", "svc", "1.0.0", "m", "s")
        .await
        .unwrap();
    pkg.revoke(&pool).await.unwrap();

    assert_eq!(pkg.status, PkgStatus::Revoked);
    assert!(pkg.revoked_at.is_some());

    let found = ActrPackage::get_by_type(&pool, "revpkg:svc:1.0.0")
        .await
        .unwrap();
    assert!(
        found.is_none(),
        "revoked package should not be found by get_by_type"
    );
}

#[tokio::test]
async fn test_package_list_by_mfr() {
    let pool = setup_test_pool().await;
    let mut mfr = Manufacturer::create(&pool, "listpkg", None).await.unwrap();
    mfr.activate(&pool, "pk".to_string()).await.unwrap();

    ActrPackage::publish(&pool, mfr.id, "listpkg", "alpha", "1.0.0", "m", "s")
        .await
        .unwrap();
    ActrPackage::publish(&pool, mfr.id, "listpkg", "beta", "1.0.0", "m", "s")
        .await
        .unwrap();

    let pkgs = ActrPackage::list_by_mfr(&pool, mfr.id).await.unwrap();
    assert_eq!(pkgs.len(), 2);
}

#[tokio::test]
async fn test_package_get_by_id() {
    let pool = setup_test_pool().await;
    let mut mfr = Manufacturer::create(&pool, "idpkg", None).await.unwrap();
    mfr.activate(&pool, "pk".to_string()).await.unwrap();

    let pkg = ActrPackage::publish(&pool, mfr.id, "idpkg", "svc", "1.0.0", "m", "s")
        .await
        .unwrap();

    let found = ActrPackage::get_by_id(&pool, pkg.id).await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().type_str, "idpkg:svc:1.0.0");
}

// ─── manager.rs 测试（需 DB）─────────────────────────────────────────────────

#[tokio::test]
async fn test_lookup_package_reserved() {
    let pool = setup_test_pool().await;
    assert!(lookup_package(&pool, "self:anything:1.0.0").await.unwrap());
    assert!(lookup_package(&pool, "acme:client:1.0.0").await.unwrap());
    assert!(lookup_package(&pool, "actrix:core:1.0.0").await.unwrap());
    assert!(lookup_package(&pool, "SELF:svc:1.0.0").await.unwrap());
}

#[tokio::test]
async fn test_lookup_package_not_registered() {
    let pool = setup_test_pool().await;
    assert!(!lookup_package(&pool, "unknown:svc:1.0.0").await.unwrap());
}

#[tokio::test]
async fn test_lookup_package_active() {
    let pool = setup_test_pool().await;
    let mut mfr = Manufacturer::create(&pool, "lookco", None).await.unwrap();
    mfr.activate(&pool, "pk".to_string()).await.unwrap();
    ActrPackage::publish(&pool, mfr.id, "lookco", "svc", "1.0.0", "m", "s")
        .await
        .unwrap();

    assert!(lookup_package(&pool, "lookco:svc:1.0.0").await.unwrap());
    assert!(!lookup_package(&pool, "lookco:svc:v2").await.unwrap());
}

#[tokio::test]
async fn test_lookup_package_revoked() {
    let pool = setup_test_pool().await;
    let mut mfr = Manufacturer::create(&pool, "revokedlook", None)
        .await
        .unwrap();
    mfr.activate(&pool, "pk".to_string()).await.unwrap();
    let mut pkg = ActrPackage::publish(&pool, mfr.id, "revokedlook", "svc", "1.0.0", "m", "s")
        .await
        .unwrap();
    pkg.revoke(&pool).await.unwrap();

    assert!(
        !lookup_package(&pool, "revokedlook:svc:1.0.0")
            .await
            .unwrap(),
        "revoked package should not be found"
    );
}

#[tokio::test]
async fn test_manager_apply_reserved_rejected() {
    let pool = setup_test_pool().await;
    let manager = MfrManager::new(pool);
    let result = manager.apply("acme", None).await;
    assert!(matches!(result, Err(MfrError::ReservedName(_))));
}

#[tokio::test]
async fn test_manager_apply_valid() {
    let pool = setup_test_pool().await;
    let manager = MfrManager::new(pool);
    let (mfr, challenge) = manager
        .apply("octocat", Some("admin@octocat.dev"))
        .await
        .unwrap();

    assert_eq!(mfr.name, "octocat");
    assert_eq!(mfr.status, MfrStatus::Pending);
    assert!(challenge.token.starts_with("actrix-verify="));
    assert!(challenge.verify_url.is_empty());
}

#[tokio::test]
async fn test_manager_apply_invalid_login() {
    let pool = setup_test_pool().await;
    let manager = MfrManager::new(pool);
    // Underscore is not allowed in GitHub logins
    let result = manager.apply("my_company", None).await;
    assert!(matches!(result, Err(MfrError::InvalidName(_))));
}

#[tokio::test]
async fn test_manager_get_status() {
    let pool = setup_test_pool().await;
    let manager = MfrManager::new(pool);
    let (mfr, _) = manager.apply("statusco", None).await.unwrap();

    let status = manager.get_status(mfr.id).await.unwrap();
    assert_eq!(status.name, "statusco");
    assert_eq!(status.status, MfrStatus::Pending);
}

#[tokio::test]
async fn test_manager_get_status_not_found() {
    let pool = setup_test_pool().await;
    let manager = MfrManager::new(pool);
    let result = manager.get_status(9999).await;
    assert!(matches!(result, Err(MfrError::NotFound)));
}

#[tokio::test]
async fn test_manager_admin_approve() {
    let pool = setup_test_pool().await;
    let manager = MfrManager::new(pool);
    let (mfr, _) = manager.apply("approveco", None).await.unwrap();

    let keychain = manager.admin_approve(mfr.id).await.unwrap();
    assert_eq!(keychain.certificate.mfr_name, "approveco");
    assert!(!keychain.private_key.is_empty());
    assert!(!keychain.certificate.mfr_pubkey.is_empty());
    assert!(keychain.certificate.expires_at > keychain.certificate.issued_at);
}

#[tokio::test]
async fn test_manager_admin_suspend_reinstate() {
    let pool = setup_test_pool().await;
    let manager = MfrManager::new(pool);
    let (mfr, _) = manager.apply("suspco", None).await.unwrap();
    manager.admin_approve(mfr.id).await.unwrap();

    manager.admin_suspend(mfr.id).await.unwrap();
    let status = manager.get_status(mfr.id).await.unwrap();
    assert_eq!(status.status, MfrStatus::Suspended);

    manager.admin_reinstate(mfr.id).await.unwrap();
    let status = manager.get_status(mfr.id).await.unwrap();
    assert_eq!(status.status, MfrStatus::Active);
}

#[tokio::test]
async fn test_manager_admin_delete() {
    let pool = setup_test_pool().await;
    let manager = MfrManager::new(pool);
    let (mfr, _) = manager.apply("deleteco", None).await.unwrap();
    let id = mfr.id;

    manager.admin_delete(id).await.unwrap();
    let result = manager.get_status(id).await;
    assert!(matches!(result, Err(MfrError::NotFound)));
}

#[tokio::test]
async fn test_manager_publish_invalid_signature() {
    use base64::Engine as _;

    let pool = setup_test_pool().await;
    let manager = MfrManager::new(pool);
    let (mfr, _) = manager.apply("sigco", None).await.unwrap();
    manager.admin_approve(mfr.id).await.unwrap();

    let bad_sig = base64::engine::general_purpose::STANDARD.encode([0u8; 64]);
    let result = manager
        .publish_package(PublishRequest {
            manufacturer: "sigco".to_string(),
            name: "svc".to_string(),
            version: "1.0.0".to_string(),
            manifest: "manifest content".to_string(),
            signature: bad_sig,
        })
        .await;
    assert!(
        matches!(
            result,
            Err(MfrError::InvalidSignature) | Err(MfrError::Crypto(_))
        ),
        "invalid signature should be rejected: {result:?}"
    );
}

#[tokio::test]
async fn test_manager_publish_valid_signature() {
    use base64::Engine as _;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    let pool = setup_test_pool().await;

    let signing_key = SigningKey::generate(&mut OsRng);
    let pub_b64 =
        base64::engine::general_purpose::STANDARD.encode(signing_key.verifying_key().to_bytes());

    let mut mfr = Manufacturer::create(&pool, "validpub", None).await.unwrap();
    mfr.activate(&pool, pub_b64).await.unwrap();

    let manifest = "type = \"validpub:client:1.0.0\"\nbinary_hash = \"sha256:abc\"";
    let sig = signing_key.sign(manifest.as_bytes());
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());

    let manager = MfrManager::new(pool);
    let pkg = manager
        .publish_package(PublishRequest {
            manufacturer: "validpub".to_string(),
            name: "client".to_string(),
            version: "1.0.0".to_string(),
            manifest: manifest.to_string(),
            signature: sig_b64,
        })
        .await
        .unwrap();

    assert_eq!(pkg.type_str, "validpub:client:1.0.0");
    assert_eq!(pkg.status, PkgStatus::Active);
}

#[tokio::test]
async fn test_manager_publish_inactive_mfr() {
    let pool = setup_test_pool().await;
    Manufacturer::create(&pool, "pendingmfr", None)
        .await
        .unwrap();

    let manager = MfrManager::new(pool);
    let result = manager
        .publish_package(PublishRequest {
            manufacturer: "pendingmfr".to_string(),
            name: "svc".to_string(),
            version: "1.0.0".to_string(),
            manifest: "m".to_string(),
            signature: "s".to_string(),
        })
        .await;
    assert!(
        matches!(result, Err(MfrError::InvalidStatus(_))),
        "publishing for pending MFR should fail with InvalidStatus"
    );
}

#[tokio::test]
async fn test_manager_resolve_by_name() {
    let pool = setup_test_pool().await;
    let manager = MfrManager::new(pool);
    let (mfr, _) = manager.apply("resolveco", None).await.unwrap();
    manager.admin_approve(mfr.id).await.unwrap();

    let info = manager.resolve_by_name("resolveco").await.unwrap();
    assert_eq!(info.name, "resolveco");
    assert!(!info.public_key.is_empty());
}

#[tokio::test]
async fn test_manager_resolve_by_name_not_active() {
    let pool = setup_test_pool().await;
    let manager = MfrManager::new(pool);
    manager.apply("pendingres", None).await.unwrap();

    let result = manager.resolve_by_name("pendingres").await;
    assert!(matches!(result, Err(MfrError::InvalidStatus(_))));
}

#[tokio::test]
async fn test_manager_get_and_revoke_package() {
    use base64::Engine as _;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    let pool = setup_test_pool().await;
    let key = SigningKey::generate(&mut OsRng);
    let pub_b64 = base64::engine::general_purpose::STANDARD.encode(key.verifying_key().to_bytes());

    let mut mfr = Manufacturer::create(&pool, "revmgr", None).await.unwrap();
    mfr.activate(&pool, pub_b64).await.unwrap();

    let manifest = "type = \"revmgr:svc:1.0.0\"";
    let sig = key.sign(manifest.as_bytes());
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());

    let manager = MfrManager::new(pool);
    let pkg = manager
        .publish_package(PublishRequest {
            manufacturer: "revmgr".to_string(),
            name: "svc".to_string(),
            version: "1.0.0".to_string(),
            manifest: manifest.to_string(),
            signature: sig_b64,
        })
        .await
        .unwrap();

    let found = manager.get_package("revmgr:svc:1.0.0").await.unwrap();
    assert_eq!(found.id, pkg.id);

    manager.revoke_package(pkg.id).await.unwrap();

    let result = manager.get_package("revmgr:svc:1.0.0").await;
    assert!(matches!(result, Err(MfrError::NotFound)));
}

#[tokio::test]
async fn test_manager_admin_list() {
    let pool = setup_test_pool().await;
    let manager = MfrManager::new(pool);

    manager.apply("adminlist1", None).await.unwrap();
    let (mfr2, _) = manager.apply("adminlist2", None).await.unwrap();
    manager.admin_approve(mfr2.id).await.unwrap();

    let all = manager.admin_list(None).await.unwrap();
    assert_eq!(all.len(), 2);

    let active = manager.admin_list(Some(MfrStatus::Active)).await.unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].name, "adminlist2");
}

#[tokio::test]
async fn test_manager_list_packages_by_mfr() {
    use base64::Engine as _;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    let pool = setup_test_pool().await;
    let key = SigningKey::generate(&mut OsRng);
    let pub_b64 = base64::engine::general_purpose::STANDARD.encode(key.verifying_key().to_bytes());

    let mut mfr = Manufacturer::create(&pool, "listmgr", None).await.unwrap();
    mfr.activate(&pool, pub_b64).await.unwrap();

    let manager = MfrManager::new(pool);

    for pkg_name in &["alpha", "beta"] {
        let manifest = format!("type = \"listmgr:{pkg_name}:1.0.0\"");
        let sig = key.sign(manifest.as_bytes());
        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());
        manager
            .publish_package(PublishRequest {
                manufacturer: "listmgr".to_string(),
                name: pkg_name.to_string(),
                version: "1.0.0".to_string(),
                manifest,
                signature: sig_b64,
            })
            .await
            .unwrap();
    }

    let pkgs = manager.list_packages(Some("listmgr")).await.unwrap();
    assert_eq!(pkgs.len(), 2);

    let all = manager.list_packages(None).await.unwrap();
    assert_eq!(all.len(), 2);
}
