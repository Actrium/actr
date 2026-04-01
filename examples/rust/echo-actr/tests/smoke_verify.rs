mod common;

#[test]
fn smoke_verify_package() {
    let Some(package_path) = common::package_path() else {
        eprintln!("skipping smoke_verify_package because PACKAGE_PATH is not set");
        return;
    };

    let manifest = common::verify_package(&package_path, &common::pubkey_path())
        .expect("package should verify");
    assert_eq!(manifest.manufacturer, "actrium");
    assert_eq!(manifest.name, "EchoService");
}
