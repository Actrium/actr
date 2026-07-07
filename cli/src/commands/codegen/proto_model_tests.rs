use super::*;
use actr_config::ConfigParser;
use tempfile::TempDir;

#[test]
fn classify_proto_side_is_remote_when_first_component_is_remote() {
    assert_eq!(
        classify_proto_side(Path::new("remote/echo.proto")),
        ProtoSide::Remote
    );
    assert_eq!(
        classify_proto_side(Path::new("local/echo.proto")),
        ProtoSide::Local
    );
    assert_eq!(
        classify_proto_side(Path::new("echo.proto")),
        ProtoSide::Local
    );
}

#[test]
fn infer_remote_actr_type_uses_dependency_actr_types() {
    let mut deps: HashMap<String, String> = HashMap::new();
    deps.insert("echo-echo-server".into(), "acme:Echo:1.0.0".into());
    assert_eq!(
        infer_remote_actr_type(
            Path::new("remote/echo-echo-server/echo.proto"),
            &deps,
            "default",
            None,
        ),
        Some("acme:Echo:1.0.0".into())
    );
    // No matching dependency, falls back to constructed ActrType.
    assert_eq!(
        infer_remote_actr_type(
            Path::new("remote/unknown/svc.proto"),
            &deps,
            "mfr",
            Some("Svc"),
        ),
        Some("mfr:Svc:1.0.0".into())
    );
    assert_eq!(
        infer_remote_actr_type(Path::new("remote/unknown/svc.proto"), &deps, "mfr", None),
        None
    );
}

#[test]
fn normalize_proto_type_trims_leading_dot_and_whitespace() {
    assert_eq!(normalize_proto_type("  .EchoRequest "), "EchoRequest");
    assert_eq!(normalize_proto_type("EchoResponse"), "EchoResponse");
    assert_eq!(normalize_proto_type(""), "");
}

#[test]
fn extract_declared_type_name_finds_message_and_enum() {
    assert!(extract_declared_type_name("message EchoRequest {", "message ").is_some());
    assert!(extract_declared_type_name("enum Status {", "enum ").is_some());
    assert!(extract_declared_type_name("not matching", "message ").is_none());
}

#[test]
fn parse_rpc_method_parses_stream_and_unary_signatures() {
    let m = parse_rpc_method(
        "Echo(EchoRequest) returns (EchoResponse);",
        "echo",
        "EchoService",
    )
    .unwrap();
    assert_eq!(m.name, "Echo");
    assert_eq!(m.snake_name, "echo");
    assert_eq!(m.input_type, "EchoRequest");
    assert_eq!(m.output_type, "EchoResponse");
    assert_eq!(m.route_key, "echo.EchoService.Echo");
}

#[test]
fn parse_rejects_proto_path_escaping_proto_root() {
    let tmp = TempDir::new().unwrap();
    let proto_root = tmp.path().join("protos");
    std::fs::create_dir_all(proto_root.join("local")).unwrap();
    let config_path = tmp.path().join("manifest.toml");
    std::fs::write(
        &config_path,
        "edition = 1\nexports = []\n[package]\nname = \"Demo\"\nmanufacturer = \"acme\"\nversion = \"0.1.0\"\n[system.signaling]\nurl = \"ws://127.0.0.1:8080\"\n[system.ais_endpoint]\nurl = \"http://127.0.0.1:8080/ais\"\n[system.deployment]\nrealm_id = 1001\n",
    )
    .unwrap();
    let config = ConfigParser::from_manifest_file(&config_path).unwrap();

    // A proto file path that escapes the proto root via `..` must be rejected
    // so traversal sequences cannot reach generated import/module paths.
    let escaping = proto_root.join("local/../evil.proto");
    let result = ProtoModel::parse(&[escaping], &proto_root, &config);
    assert!(
        result.is_err(),
        "proto path escaping the root should error, got: {:?}",
        result
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("escapes the proto root"),
        "unexpected error message: {msg}"
    );
}

#[test]
fn type_owner_index_does_not_bare_resolve_unmatched_qualified_type() {
    let files = vec![
        ProtoFileModel {
            proto_file: PathBuf::from("local/app.proto"),
            relative_path: PathBuf::from("local/app.proto"),
            package: "app".to_string(),
            side: ProtoSide::Local,
            declared_type_names: vec!["Empty".to_string()],
            services: vec![],
        },
        ProtoFileModel {
            proto_file: PathBuf::from("remote/other.proto"),
            relative_path: PathBuf::from("remote/other.proto"),
            package: "other".to_string(),
            side: ProtoSide::Remote,
            declared_type_names: vec!["Request".to_string()],
            services: vec![],
        },
    ];
    let index = TypeOwnerIndex::from_files(&files);

    assert_eq!(
        index.resolve("google.protobuf.Empty", &files[0]),
        Ok(None),
        "unmatched qualified external types must not resolve by bare name"
    );
    assert_eq!(
        index.resolve("vendor.pkg.Request", &files[0]),
        Ok(None),
        "unmatched qualified external types must not resolve to a same-bare imported owner"
    );
}
