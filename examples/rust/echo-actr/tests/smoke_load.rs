#![cfg(not(target_arch = "wasm32"))]

mod common;

#[tokio::test]
async fn smoke_load_package() {
    let Some(package_path) = common::package_path() else {
        eprintln!("skipping smoke_load_package because PACKAGE_PATH is not set");
        return;
    };

    let backend = common::load_package(&package_path, &common::pubkey_path())
        .await
        .expect("package should load");
    assert!(!backend.is_empty());
}
