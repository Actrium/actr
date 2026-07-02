use super::*;
use tempfile::TempDir;

#[tokio::test]
async fn get_cached_proto_returns_none_when_path_absent() {
    let dir = TempDir::new().unwrap();
    let mgr = DefaultCacheManager::with_project_root(dir.path().to_path_buf());
    assert!(mgr.get_cached_proto("nonexistent").await.unwrap().is_none());
}

#[tokio::test]
async fn cache_get_invalidate_roundtrip() {
    let dir = TempDir::new().unwrap();
    let mgr = DefaultCacheManager::with_project_root(dir.path().to_path_buf());

    // Cache a proto file.
    let proto = ProtoFile {
        name: "echo.proto".into(),
        path: "echo.proto".into(),
        content: "syntax = \"proto3\";".into(),
        services: vec![],
    };
    mgr.cache_proto("echo", &[proto]).await.unwrap();

    // Now retrieve it.
    let cached = mgr.get_cached_proto("echo").await.unwrap().unwrap();
    assert_eq!(cached.files.len(), 1);
    assert_eq!(cached.files[0].name, "echo.proto");
    assert!(!cached.fingerprint.value.is_empty());

    // Invalidate.
    mgr.invalidate_cache("echo").await.unwrap();
    assert!(mgr.get_cached_proto("echo").await.unwrap().is_none());
}

#[tokio::test]
async fn cache_proto_adds_dot_proto_extension_when_missing() {
    let dir = TempDir::new().unwrap();
    let mgr = DefaultCacheManager::with_project_root(dir.path().to_path_buf());
    let proto = ProtoFile {
        name: "echo".into(),
        path: "echo".into(),
        content: "syntax = \"proto3\";".into(),
        services: vec![],
    };
    mgr.cache_proto("echo", &[proto]).await.unwrap();
    let cached = mgr.get_cached_proto("echo").await.unwrap().unwrap();
    // The name field is unchanged, but the file on disk has .proto appended.
    assert_eq!(cached.files.len(), 1);
    assert!(cached.files[0].name.ends_with(".proto"));
}

#[tokio::test]
async fn clear_cache_removes_protos_directory() {
    let dir = TempDir::new().unwrap();
    let mgr = DefaultCacheManager::with_project_root(dir.path().to_path_buf());
    let proto = ProtoFile {
        name: "echo.proto".into(),
        path: "echo.proto".into(),
        content: "// stub".into(),
        services: vec![],
    };
    mgr.cache_proto("echo", &[proto]).await.unwrap();
    assert!(dir.path().join("protos").exists());

    mgr.clear_cache().await.unwrap();
    assert!(!dir.path().join("protos").exists());
}

#[tokio::test]
async fn get_cache_stats_counts_entries_and_sizes() {
    let dir = TempDir::new().unwrap();
    let mgr = DefaultCacheManager::with_project_root(dir.path().to_path_buf());
    let proto = ProtoFile {
        name: "echo.proto".into(),
        path: "echo.proto".into(),
        content: "// hi".into(),
        services: vec![],
    };
    mgr.cache_proto("echo", &[proto]).await.unwrap();
    let stats = mgr.get_cache_stats().await.unwrap();
    assert_eq!(stats.total_entries, 1);
    assert!(stats.total_size_bytes > 0);
    assert_eq!(stats.hit_rate, 0.0);
    assert_eq!(stats.miss_rate, 0.0);
}
