use super::*;
use actr_protocol::Realm;

fn sample_context(realm_secret: Option<&str>) -> DiscoveryContext {
    DiscoveryContext {
        package_actr_type: ActrType {
            manufacturer: "acme".to_string(),
            name: "cli-client".to_string(),
            version: "1.0.0".to_string(),
        },
        signaling_url: Url::parse("ws://localhost:8081/signaling/ws").unwrap(),
        ais_endpoint: "http://localhost:8081/ais".to_string(),
        realm: Realm { realm_id: 1001 },
        realm_secret: realm_secret.map(str::to_string),
    }
}

fn sample_actor_id() -> ActrId {
    ActrId {
        serial_number: 42,
        r#type: ActrType {
            manufacturer: "acme".to_string(),
            name: "echo".to_string(),
            version: "1.0.0".to_string(),
        },
        realm: Realm { realm_id: 1001 },
    }
}

fn sample_credential() -> AIdCredential {
    AIdCredential {
        key_id: 7,
        claims: vec![1, 2, 3, 4].into(),
        signature: vec![5, 6, 7, 8].into(),
    }
}

#[test]
fn build_signaling_url_with_identity_appends_auth_query() {
    let signaling_url = Url::parse("ws://localhost:8081/signaling/ws?existing=1").unwrap();
    let actor_id = sample_actor_id();
    let credential = sample_credential();

    let authenticated_url = NetworkServiceDiscovery::build_signaling_url_with_identity(
        &signaling_url,
        &actor_id,
        &credential,
    );
    let query_pairs: std::collections::HashMap<_, _> =
        authenticated_url.query_pairs().into_owned().collect();

    assert_eq!(query_pairs.get("existing"), Some(&"1".to_string()));
    assert_eq!(
        query_pairs.get("actor_id"),
        Some(&actor_id.to_string_repr())
    );
    assert_eq!(query_pairs.get("key_id"), Some(&"7".to_string()));
    assert_eq!(
        query_pairs.get("claims"),
        Some(&base64::engine::general_purpose::STANDARD.encode([1, 2, 3, 4]))
    );
    assert_eq!(
        query_pairs.get("signature"),
        Some(&base64::engine::general_purpose::STANDARD.encode([5, 6, 7, 8]))
    );
}

#[test]
fn cli_discovery_register_request_uses_linked_auth_mode() {
    let discovery = NetworkServiceDiscovery::new(sample_context(Some("rs_test_secret")));
    let request = discovery.build_linked_register_request();

    assert_eq!(request.auth_mode, Some(RegisterAuthMode::Linked as i32));
    assert_eq!(request.manifest_raw, None);
    assert_eq!(request.mfr_signature, None);
    assert_eq!(request.target, None);
    assert_eq!(request.actr_type.name, "cli-client");
    assert_eq!(request.realm.realm_id, 1001);
}

#[test]
fn cli_discovery_requires_realm_secret() {
    let missing = NetworkServiceDiscovery::new(sample_context(None));
    let err = missing.required_realm_secret().unwrap_err();
    assert!(err.to_string().contains("network.realm_secret is required"));

    let blank = NetworkServiceDiscovery::new(sample_context(Some("   ")));
    let err = blank.required_realm_secret().unwrap_err();
    assert!(err.to_string().contains("network.realm_secret is required"));
}
