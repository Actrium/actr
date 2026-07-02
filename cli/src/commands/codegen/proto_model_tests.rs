use super::*;

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
