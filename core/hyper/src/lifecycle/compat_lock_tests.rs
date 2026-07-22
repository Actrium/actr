use super::*;
use tempfile::TempDir;

#[tokio::test]
async fn test_compat_lock_file_roundtrip() {
    let temp_dir = TempDir::new().unwrap();
    let base_path = temp_dir.path();

    // Create and save
    let mut lock_file = CompatLockFile::new();
    lock_file.upsert_entry(NegotiationEntry::new(
        "user-service".to_string(),
        "sha256:old".to_string(),
        "sha256:new".to_string(),
        CompatibilityCheck::BackwardCompatible,
    ));
    lock_file.save(base_path).await.unwrap();

    // Verify file exists
    assert!(CompatLockFile::exists(base_path).await);

    // Reload
    let loaded = CompatLockFile::load(base_path).await.unwrap().unwrap();
    assert_eq!(loaded.negotiation.len(), 1);
    assert_eq!(loaded.negotiation[0].service_name, "user-service");
    assert!(loaded.is_sub_healthy());
}

#[tokio::test]
async fn test_compat_lock_manager() {
    let temp_dir = TempDir::new().unwrap();
    // Use temp directory as project root
    let project_root = temp_dir.path().to_path_buf();

    let mut manager = CompatLockManager::new(project_root.clone());

    // Verify cache directory is under the system temp directory
    let cache_dir = manager.cache_dir().to_path_buf();
    assert!(cache_dir.starts_with(std::env::temp_dir()));
    assert!(cache_dir.to_string_lossy().contains("actr"));

    // Record a compatible match
    manager
        .record_negotiation(
            "user-service",
            "sha256:old",
            "sha256:new",
            false,
            CompatibilityCheck::BackwardCompatible,
        )
        .await
        .unwrap();

    // Verify file exists in the computed cache directory
    assert!(CompatLockFile::exists(&cache_dir).await);

    // Verify file is not in the project directory
    assert!(!project_root.join(COMPAT_LOCK_FILENAME).exists());

    // Find cached entry
    let entry = manager.find_cached_compatible("user-service", "sha256:old");
    assert!(entry.is_some());

    // After exact match, the entry should be removed
    manager
        .record_negotiation(
            "user-service",
            "sha256:exact",
            "sha256:exact",
            true,
            CompatibilityCheck::ExactMatch,
        )
        .await
        .unwrap();

    // File should be removed (no other entries remain)
    assert!(!CompatLockFile::exists(&cache_dir).await);
}

#[test]
fn test_project_hash_deterministic() {
    let path1 = PathBuf::from("/tmp/test-project");
    let path2 = PathBuf::from("/tmp/test-project");
    let path3 = PathBuf::from("/tmp/other-project");

    let hash1 = compute_project_hash(&path1);
    let hash2 = compute_project_hash(&path2);
    let hash3 = compute_project_hash(&path3);

    // Same path should produce the same hash
    assert_eq!(hash1, hash2);
    // Different paths should produce different hashes
    assert_ne!(hash1, hash3);
    // Hash should be 16 hex characters
    assert_eq!(hash1.len(), 16);
}

#[tokio::test]
async fn load_returns_none_when_file_absent() {
    let dir = TempDir::new().unwrap();
    assert!(CompatLockFile::load(dir.path()).await.unwrap().is_none());
    assert!(!CompatLockFile::exists(dir.path()).await);
}

#[tokio::test]
async fn remove_returns_false_when_absent_and_true_when_present() {
    let dir = TempDir::new().unwrap();
    assert!(!CompatLockFile::remove(dir.path()).await.unwrap());

    let mut lf = CompatLockFile::new();
    lf.upsert_entry(NegotiationEntry::new(
        "svc".into(),
        "old".into(),
        "new".into(),
        CompatibilityCheck::BackwardCompatible,
    ));
    lf.save(dir.path()).await.unwrap();
    assert!(CompatLockFile::remove(dir.path()).await.unwrap());
    assert!(!CompatLockFile::exists(dir.path()).await);
}

#[tokio::test]
async fn save_creates_missing_base_dir() {
    let dir = TempDir::new().unwrap();
    let nested = dir.path().join("deeply/nested/missing");
    let mut lf = CompatLockFile::new();
    lf.upsert_entry(NegotiationEntry::new(
        "svc".into(),
        "a".into(),
        "b".into(),
        CompatibilityCheck::BackwardCompatible,
    ));
    lf.save(&nested).await.unwrap();
    assert!(CompatLockFile::exists(&nested).await);
}

#[test]
fn find_entry_and_find_valid_entry_semantics() {
    let mut lf = CompatLockFile::new();
    lf.upsert_entry(NegotiationEntry::new(
        "present".into(),
        "a".into(),
        "b".into(),
        CompatibilityCheck::BackwardCompatible,
    ));
    lf.negotiation.push(NegotiationEntry {
        service_name: "expired".into(),
        requested_fingerprint: "a".into(),
        resolved_fingerprint: "b".into(),
        compatibility_check: CompatibilityCheck::BackwardCompatible,
        negotiated_at: Utc::now() - Duration::hours(48),
        expires_at: Utc::now() - Duration::hours(1),
    });

    assert!(lf.find_entry("present").is_some());
    assert!(lf.find_entry("expired").is_some());
    assert!(lf.find_entry("missing").is_none());

    assert!(lf.find_valid_entry("present").is_some());
    assert!(lf.find_valid_entry("expired").is_none());
}

#[test]
fn upsert_entry_replaces_existing_name() {
    let mut lf = CompatLockFile::new();
    lf.upsert_entry(NegotiationEntry::new(
        "svc".into(),
        "old".into(),
        "v1".into(),
        CompatibilityCheck::BackwardCompatible,
    ));
    lf.upsert_entry(NegotiationEntry::new(
        "svc".into(),
        "new".into(),
        "v2".into(),
        CompatibilityCheck::BreakingChanges,
    ));
    assert_eq!(lf.negotiation.len(), 1);
    assert_eq!(lf.find_entry("svc").unwrap().requested_fingerprint, "new");
    assert_eq!(lf.find_entry("svc").unwrap().resolved_fingerprint, "v2");
}

#[test]
fn cleanup_expired_removes_only_expired_entries() {
    let mut lf = CompatLockFile::new();
    lf.upsert_entry(NegotiationEntry::new(
        "alive".into(),
        "a".into(),
        "b".into(),
        CompatibilityCheck::BackwardCompatible,
    ));
    lf.negotiation.push(NegotiationEntry {
        service_name: "dead".into(),
        requested_fingerprint: "a".into(),
        resolved_fingerprint: "b".into(),
        compatibility_check: CompatibilityCheck::BackwardCompatible,
        negotiated_at: Utc::now() - Duration::hours(48),
        expires_at: Utc::now() - Duration::hours(1),
    });

    let removed = lf.cleanup_expired();
    assert_eq!(removed, 1);
    assert_eq!(lf.negotiation.len(), 1);
    assert_eq!(lf.negotiation[0].service_name, "alive");
}

#[tokio::test]
async fn load_clamps_far_future_expiry_to_negotiated_ttl() {
    let dir = TempDir::new().unwrap();

    // Simulate a corrupted or clock-jumped write: expires_at far beyond the
    // TTL that was in force when the entry was negotiated.
    let negotiated_at = Utc::now() - Duration::hours(48);
    let mut lf = CompatLockFile::new();
    lf.negotiation.push(NegotiationEntry {
        service_name: "svc".into(),
        requested_fingerprint: "a".into(),
        resolved_fingerprint: "b".into(),
        compatibility_check: CompatibilityCheck::BackwardCompatible,
        negotiated_at,
        expires_at: negotiated_at + Duration::days(3650),
    });
    lf.save(dir.path()).await.unwrap();

    let loaded = CompatLockFile::load(dir.path()).await.unwrap().unwrap();
    let entry = loaded.find_entry("svc").unwrap();
    assert_eq!(
        entry.expires_at,
        negotiated_at + Duration::hours(DEFAULT_TTL_HOURS),
        "expiry must be clamped to negotiated_at + TTL on load"
    );
    // negotiated_at is 48h old with a 24h TTL, so the clamped entry is expired.
    assert!(entry.is_expired());
    assert!(loaded.find_valid_entry("svc").is_none());

    // An in-range expiry is left untouched by load.
    let mut fresh = CompatLockFile::new();
    fresh.upsert_entry(NegotiationEntry::new(
        "fresh".into(),
        "a".into(),
        "b".into(),
        CompatibilityCheck::BackwardCompatible,
    ));
    let expected_expires_at = fresh.find_entry("fresh").unwrap().expires_at;
    fresh.save(dir.path()).await.unwrap();
    let reloaded = CompatLockFile::load(dir.path()).await.unwrap().unwrap();
    assert_eq!(
        reloaded.find_entry("fresh").unwrap().expires_at,
        expected_expires_at
    );
    assert!(reloaded.find_valid_entry("fresh").is_some());
}

#[test]
fn is_expired_reflects_expires_at() {
    let fresh = NegotiationEntry::new(
        "s".into(),
        "a".into(),
        "b".into(),
        CompatibilityCheck::BackwardCompatible,
    );
    assert!(!fresh.is_expired());

    let stale = NegotiationEntry {
        service_name: "s".into(),
        requested_fingerprint: "a".into(),
        resolved_fingerprint: "b".into(),
        compatibility_check: CompatibilityCheck::BackwardCompatible,
        negotiated_at: Utc::now() - Duration::hours(48),
        expires_at: Utc::now() - Duration::hours(1),
    };
    assert!(stale.is_expired());
}

#[test]
fn compatibility_check_display_covers_all_variants() {
    assert_eq!(CompatibilityCheck::ExactMatch.to_string(), "exact_match");
    assert_eq!(
        CompatibilityCheck::BackwardCompatible.to_string(),
        "backward_compatible"
    );
    assert_eq!(
        CompatibilityCheck::BreakingChanges.to_string(),
        "breaking_changes"
    );
}
