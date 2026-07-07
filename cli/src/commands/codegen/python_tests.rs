use std::collections::HashMap;

#[test]
fn test_remote_path_extraction() {
    // Test the logic for extracting remote path after "/remote/"
    let test_cases = vec![
        (
            "protos/remote/server/service.proto",
            Some("server/service.proto"),
        ),
        // "remote/test.proto" will NOT match because split produces ["", "test.proto"]
        // which is only 2 parts, but the first part is empty, not what we want
        ("protos/remote/test.proto", Some("test.proto")),
        ("protos/local.proto", None),
        ("no_remote_here.proto", None),
    ];

    for (input, expected) in test_cases {
        let parts: Vec<&str> = input.split("/remote/").collect();
        let result = if parts.len() == 2 && !parts[0].is_empty() {
            Some(parts[1])
        } else {
            None
        };

        assert_eq!(
            result, expected,
            "Failed for input: {}, expected: {:?}, got: {:?}",
            input, expected, result
        );
    }
}

#[test]
fn test_remote_services_map_construction() {
    // Create a simple mock lock file structure
    let mut remote_services_map: HashMap<String, String> = HashMap::new();

    // Simulate adding entries from lock file
    remote_services_map.insert(
        "server/service.proto".to_string(),
        "acme:TestServer".to_string(),
    );
    remote_services_map.insert(
        "api/v1/api.proto".to_string(),
        "custom:ApiService".to_string(),
    );

    // Verify the mapping
    assert_eq!(remote_services_map.len(), 2);
    assert_eq!(
        remote_services_map.get("server/service.proto"),
        Some(&"acme:TestServer".to_string())
    );
    assert_eq!(
        remote_services_map.get("api/v1/api.proto"),
        Some(&"custom:ApiService".to_string())
    );
}

#[test]
fn test_options_string_building() {
    let remote_file_mappings = [
        "remote/s1.proto=testco:S1".to_string(),
        "remote/s2.proto=other:S2".to_string(),
    ];
    let local_paths = ["local.proto".to_string()];

    let mut options = String::new();

    if !remote_file_mappings.is_empty() {
        options.push_str(&format!(
            "RemoteFileMapping={}",
            remote_file_mappings.join(";")
        ));
    }

    if !local_paths.is_empty() {
        if !options.is_empty() {
            options.push(',');
        }
        options.push_str(&format!("LocalFiles={}", local_paths.join(":")));
    }

    assert!(
        options.contains("RemoteFileMapping=remote/s1.proto=testco:S1;remote/s2.proto=other:S2")
    );
    assert!(options.contains("LocalFiles=local.proto"));
}

#[test]
fn test_actr_type_extraction_logic() {
    let remote_services_map: HashMap<String, String> = [
        (
            "service1/api.proto".to_string(),
            "mfg1:Service1".to_string(),
        ),
        (
            "service2/api.proto".to_string(),
            "mfg2:Service2".to_string(),
        ),
    ]
    .iter()
    .cloned()
    .collect();

    // Test matched path
    let path1 = "service1/api.proto";
    assert_eq!(
        remote_services_map.get(path1),
        Some(&"mfg1:Service1".to_string())
    );

    // Test unmatched path (should return None)
    let path2 = "unknown/api.proto";
    assert_eq!(remote_services_map.get(path2), None);

    // Test that we can handle None gracefully with empty string
    let actr_type = remote_services_map.get(path2).cloned().unwrap_or_default();
    assert_eq!(actr_type, "");
}

#[test]
fn test_empty_lock_file_scenario() {
    // When lock file doesn't exist or has no dependencies
    let remote_services_map: HashMap<String, String> = HashMap::new();

    // Should handle gracefully
    assert_eq!(remote_services_map.len(), 0);
    assert_eq!(remote_services_map.get("any/path.proto"), None);

    // Simulating the warning path
    let _path_str = "remote/service/api.proto";
    let is_in_map = remote_services_map.contains_key("service/api.proto");
    assert!(!is_in_map);
    // In actual code, this triggers warn! and pushes empty string
}

#[test]
fn pb2_alias_and_import_resolves_imported_type_owner() {
    // An imported `ask.*` type declared in `remote/ask-service/ask.proto`
    // resolves to an alias based on its owner proto path, not the local
    // service's package.
    let (alias, import) = super::pb2_alias_and_import("ask", "remote/ask-service/ask.proto");
    assert_eq!(alias, "remote_ask_service_ask_pb2");
    assert_eq!(
        import,
        "from generated.remote.ask_service import ask_pb2 as remote_ask_service_ask_pb2"
    );

    // A locally-declared type keeps its own package alias + module path.
    let (local_alias, local_import) =
        super::pb2_alias_and_import("data_stream_app", "local/data_stream_app.proto");
    assert_eq!(local_alias, "local_data_stream_app_pb2");
    assert_eq!(
        local_import,
        "from generated.local import data_stream_app_pb2 as local_data_stream_app_pb2"
    );
}

#[test]
fn pb2_alias_and_import_distinguishes_same_package_owner_files() {
    let (request_alias, request_import) =
        super::pb2_alias_and_import("shared", "remote/shared/request.proto");
    let (response_alias, response_import) =
        super::pb2_alias_and_import("shared", "remote/shared/response.proto");

    assert_eq!(request_alias, "remote_shared_request_pb2");
    assert_eq!(response_alias, "remote_shared_response_pb2");
    assert_ne!(request_alias, response_alias);
    assert_eq!(
        request_import,
        "from generated.remote.shared import request_pb2 as remote_shared_request_pb2"
    );
    assert_eq!(
        response_import,
        "from generated.remote.shared import response_pb2 as remote_shared_response_pb2"
    );
}
