use super::*;
use tempfile::TempDir;

#[test]
fn parses_proto_content_from_real_service_definitions() {
    let generator = TypeScriptGenerator;
    let (package, services) = generator.parse_proto_content(
        r#"
            syntax = "proto3";

            package demo.echo;

            service EchoService {
              rpc Echo(EchoRequest) returns (EchoResponse);
              rpc Ping(demo.echo.PingRequest) returns (demo.echo.PingResponse);
            }
            "#,
    );

    assert_eq!(package, "demo.echo");
    assert_eq!(services.len(), 1);
    assert_eq!(services[0].name, "EchoService");
    assert_eq!(services[0].methods.len(), 2);
    assert_eq!(services[0].methods[0].name, "Echo");
    assert_eq!(services[0].methods[0].input_type_short, "EchoRequest");
    assert_eq!(services[0].methods[1].output_type_short, "PingResponse");
}

#[test]
fn inspects_generated_client_exports() {
    let generator = TypeScriptGenerator;
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("echo_client.ts");
    std::fs::write(
        &path,
        r#"
            export const EchoRequest = {
                routeKey: "demo.echo.Echo",
            } as const;
            "#,
    )
    .unwrap();

    let api = generator.inspect_generated_client_api(&path).unwrap();
    assert!(api.exported_consts.contains("EchoRequest"));
}

#[test]
fn implemented_marker_prevents_overwrite() {
    let generator = TypeScriptGenerator;
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("actr_service.ts");
    std::fs::write(
            &path,
            format!(
                "{IMPLEMENTED_MARKER}\n{UNIMPLEMENTED_MARKER}\nexport default defineWorkload({{ dispatch() {{ throw new Error('custom'); }} }});\n"
            ),
        )
        .unwrap();
    assert!(!generator.should_overwrite_scaffold(&path).unwrap());
}

#[test]
fn recognizes_minimal_unimplemented_scaffold() {
    let generator = TypeScriptGenerator;
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("actr_service.ts");
    std::fs::write(
            &path,
            format!(
                "{UNIMPLEMENTED_MARKER}\n{SCAFFOLD_HINT}\nexport default defineWorkload({{ dispatch() {{ throw new Error('TODO'); }} }});\n"
            ),
        )
        .unwrap();
    assert!(generator.should_overwrite_scaffold(&path).unwrap());
}

#[test]
fn unimplemented_marker_with_scaffold_hint_is_overwritten() {
    let generator = TypeScriptGenerator;
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("actr_service.ts");
    std::fs::write(
        &path,
        format!("{UNIMPLEMENTED_MARKER}\n{SCAFFOLD_HINT}\nconsole.log('custom quick-start');\n"),
    )
    .unwrap();
    assert!(generator.should_overwrite_scaffold(&path).unwrap());

    std::fs::write(&path, "console.log('user code');\n").unwrap();
    assert!(!generator.should_overwrite_scaffold(&path).unwrap());
}

#[test]
fn does_not_treat_echo_templates_as_generated_scaffold() {
    let generator = TypeScriptGenerator;
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("actr_service.ts");

    std::fs::write(
            &path,
            format!(
                "{IMPLEMENTED_MARKER}\nexport default defineWorkload({{ dispatch() {{ throw new Error('template'); }} }});\n"
            ),
        )
        .unwrap();
    assert!(!generator.should_overwrite_scaffold(&path).unwrap());

    std::fs::write(
            &path,
            format!(
                "{IMPLEMENTED_MARKER}\nimport {{ ActrNode }} from '@actrium/actr';\nconsole.log('template');\n"
            ),
        )
        .unwrap();
    assert!(!generator.should_overwrite_scaffold(&path).unwrap());
}

#[test]
fn generates_scaffold_with_local_and_remote_sections() {
    let generator = TypeScriptGenerator;
    let scaffold = generator.generate_scaffold_content(&[
        BoundMethodInfo {
            generated_client_import: "./generated/echo_client".to_string(),
            generated_proto_import: "./generated/echo_pb.js".to_string(),
            generated_workload_import: "./generated/echo_workload.js".to_string(),
            service_name: "EchoService".to_string(),
            handler_interface: "EchoServiceHandler".to_string(),
            dispatcher_type: "EchoServiceDispatcher".to_string(),
            method_name: "Echo".to_string(),
            handler_method_name: "echo".to_string(),
            input_type: "EchoRequest".to_string(),
            output_type: "EchoResponse".to_string(),
            input_type_short: "EchoRequest".to_string(),
            output_type_short: "EchoResponse".to_string(),
            request_companion: Some("EchoRequest".to_string()),
            is_local: true,
        },
        BoundMethodInfo {
            generated_client_import: "./generated/demo/remote_client".to_string(),
            generated_proto_import: "./generated/demo/remote_pb.js".to_string(),
            generated_workload_import: String::new(),
            service_name: "RemoteService".to_string(),
            handler_interface: String::new(),
            dispatcher_type: String::new(),
            method_name: "Ping".to_string(),
            handler_method_name: "ping".to_string(),
            input_type: "PingRequest".to_string(),
            output_type: "PingResponse".to_string(),
            input_type_short: "PingRequest".to_string(),
            output_type_short: "PingResponse".to_string(),
            request_companion: Some("PingRequest".to_string()),
            is_local: false,
        },
    ]);

    assert!(scaffold.contains("import { defineWorkload } from '@actrium/actr-workload';"));
    assert!(scaffold.contains("export default defineWorkload({"));
    assert!(
        scaffold.contains("import { EchoServiceDispatcher } from './generated/echo_workload.js';")
    );
    assert!(scaffold.contains("class EchoServiceHandlerImpl implements EchoServiceHandler"));
    assert!(scaffold.contains("return dispatcher.dispatch(envelope);"));
    assert!(!scaffold.contains("Implement this workload with @actrium/actr-workload"));
    assert!(!scaffold.contains("// - EchoService.Echo (EchoRequest -> EchoResponse)"));
    assert!(scaffold.contains("Remote RPC quick-start examples"));
    assert!(scaffold.contains("PingRequest.encode"));
    assert!(scaffold.contains("PingRequest.routeKey"));
    assert!(scaffold.contains("PingRequest.response.decode"));
    assert!(scaffold.contains(UNIMPLEMENTED_MARKER));
}

#[test]
fn local_workload_imports_imported_types_from_their_owner_module() {
    // A local `data_stream_app` service references `ask.*` message types
    // declared in `remote/ask-service/ask.proto`. The workload dispatcher must
    // import those types from the owner module (`./ask-service/ask_pb.js`),
    // not from the local service's proto stem.
    let module = LocalWorkloadModule {
        name: "data_stream_app_workload".to_string(),
        services: vec![LocalWorkloadService {
            name: "DataStreamAppService".to_string(),
            handler_interface: "DataStreamAppServiceHandler".to_string(),
            dispatcher_type: "DataStreamAppServiceDispatcher".to_string(),
            methods: vec![LocalWorkloadMethod {
                name: "ContinuePromptResultStreams".to_string(),
                handler_method_name: "continuePromptResultStreams".to_string(),
                input_type_short: "ContinuePromptResultStreamsRequest".to_string(),
                output_type_short: "ContinuePromptResultStreamsResponse".to_string(),
                route_key: "data_stream_app.DataStreamAppService.ContinuePromptResultStreams"
                    .to_string(),
                input_pb_import: "./ask-service/ask_pb.js".to_string(),
                output_pb_import: "./ask-service/ask_pb.js".to_string(),
            }],
        }],
    };

    let content = generate_local_workload_content(&module);

    assert!(
        content.contains("from './ask-service/ask_pb.js'"),
        "expected owner module import, got:\n{content}"
    );
    assert!(
        !content.contains("from './data_stream_app_pb.js'"),
        "must not pin imported types to the local service proto stem:\n{content}"
    );
    assert!(content.contains("ContinuePromptResultStreamsRequest"));
    assert!(content.contains("ContinuePromptResultStreamsResponseSchema"));
}
