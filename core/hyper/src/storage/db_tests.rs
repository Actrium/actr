use super::*;
use tempfile::TempDir;

async fn open_test_store(dir: &TempDir) -> ActorStore {
    let db_path = dir.path().join("test.db");
    ActorStore::open(&db_path).await.unwrap()
}

#[tokio::test]
async fn kv_set_and_get() {
    let dir = TempDir::new().unwrap();
    let store = open_test_store(&dir).await;

    store.kv_set("hello", b"world").await.unwrap();
    let val = store.kv_get("hello").await.unwrap();
    assert_eq!(val, Some(b"world".to_vec()));
}

#[tokio::test]
async fn kv_get_missing_returns_none() {
    let dir = TempDir::new().unwrap();
    let store = open_test_store(&dir).await;

    let val = store.kv_get("nonexistent").await.unwrap();
    assert_eq!(val, None);
}

#[tokio::test]
async fn kv_delete_removes_key() {
    let dir = TempDir::new().unwrap();
    let store = open_test_store(&dir).await;

    store.kv_set("key1", b"value1").await.unwrap();
    let deleted = store.kv_delete("key1").await.unwrap();
    assert!(
        deleted,
        "should return true indicating a record was actually deleted"
    );

    let val = store.kv_get("key1").await.unwrap();
    assert_eq!(val, None, "get should return None after deletion");
}

#[tokio::test]
async fn kv_delete_nonexistent_returns_false() {
    let dir = TempDir::new().unwrap();
    let store = open_test_store(&dir).await;

    let deleted = store.kv_delete("ghost").await.unwrap();
    assert!(!deleted, "deleting a non-existent key should return false");
}

#[tokio::test]
async fn kv_list_keys_returns_all() {
    let dir = TempDir::new().unwrap();
    let store = open_test_store(&dir).await;

    store.kv_set("b", b"2").await.unwrap();
    store.kv_set("a", b"1").await.unwrap();
    store.kv_set("c", b"3").await.unwrap();

    let keys = store.kv_list_keys(None).await.unwrap();
    assert_eq!(
        keys,
        vec!["a", "b", "c"],
        "should return all keys in lexicographic order"
    );
}

#[tokio::test]
async fn kv_list_keys_prefix_filter() {
    let dir = TempDir::new().unwrap();
    let store = open_test_store(&dir).await;

    store.kv_set("prefix:a", b"1").await.unwrap();
    store.kv_set("prefix:b", b"2").await.unwrap();
    store.kv_set("other:c", b"3").await.unwrap();

    let keys = store.kv_list_keys(Some("prefix:")).await.unwrap();
    assert_eq!(keys, vec!["prefix:a", "prefix:b"]);
}

#[tokio::test]
async fn kv_batch_atomic() {
    let dir = TempDir::new().unwrap();
    let store = open_test_store(&dir).await;

    store.kv_set("existing", b"old").await.unwrap();

    store
        .kv_batch(vec![
            KvOp::Set {
                key: "new_key".to_string(),
                value: b"new_value".to_vec(),
            },
            KvOp::Set {
                key: "existing".to_string(),
                value: b"updated".to_vec(),
            },
            KvOp::Delete {
                key: "existing".to_string(),
            },
        ])
        .await
        .unwrap();

    // new_key should exist
    let val = store.kv_get("new_key").await.unwrap();
    assert_eq!(val, Some(b"new_value".to_vec()));

    // existing was updated then deleted in the batch, should not exist
    let val = store.kv_get("existing").await.unwrap();
    assert_eq!(val, None);
}

#[tokio::test]
async fn data_persists_across_reopen() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("persist.db");

    {
        let store = ActorStore::open(&db_path).await.unwrap();
        store
            .kv_set("persistent_key", b"persistent_value")
            .await
            .unwrap();
    }

    // reopen the same file
    let store2 = ActorStore::open(&db_path).await.unwrap();
    let val = store2.kv_get("persistent_key").await.unwrap();
    assert_eq!(
        val,
        Some(b"persistent_value".to_vec()),
        "data should persist after reopening the database"
    );
}
