use super::*;

#[test]
fn extract_signaling_host_parses_all_variants() {
    assert_eq!(
        extract_signaling_host("ws://10.30.3.206:8081/signaling/ws"),
        "10.30.3.206"
    );
    assert_eq!(
        extract_signaling_host("wss://actrix1.develenv.com/signaling/ws"),
        "actrix1.develenv.com"
    );
    assert_eq!(extract_signaling_host("ws://localhost:8080"), "localhost");
    // Empty string falls through splits and returns the empty string itself.
    assert_eq!(extract_signaling_host(""), "");
    assert!(
        extract_signaling_host("not-a-url").contains("not-a-url"),
        "{:?}",
        extract_signaling_host("not-a-url")
    );
}

#[test]
fn derive_ais_endpoint_no_suffix_uses_slash_ais() {
    assert_eq!(
        derive_ais_endpoint_url("ws://example.com:8080/signaling/ws"),
        "http://example.com:8080/ais"
    );
    assert_eq!(
        derive_ais_endpoint_url("wss://example.com/signaling"),
        "https://example.com/ais"
    );
    assert_eq!(
        derive_ais_endpoint_url("http://example.com/ws"),
        "http://example.com/ais"
    );
    assert!(derive_ais_endpoint_url("").is_empty());
}

#[test]
fn to_pascal_case_handles_kebab_and_snake() {
    assert_eq!(to_pascal_case("echo-service"), "EchoService");
    assert_eq!(to_pascal_case("my_app"), "MyApp");
    assert_eq!(to_pascal_case("hello-world_app"), "HelloWorldApp");
}

#[test]
fn to_package_name_generates_android_style() {
    let pkg = to_package_name("my-echo-client");
    assert!(pkg.starts_with("io.actrium."));
    assert!(!pkg.contains('-'));
}

#[test]
fn apply_placeholders_substitutes_all_keys() {
    let result = apply_placeholders(
        "{{NAME}} at {{PLACE}}",
        &[
            ("{{NAME}}".into(), "X".into()),
            ("{{PLACE}}".into(), "Y".into()),
        ],
    );
    assert_eq!(result, "X at Y");
}
