use super::*;

#[test]
fn from_metadata_maps_local_and_remote_to_scaffold_services() {
    let metadata = ActrGenMetadata {
        plugin_version: "actr-cli".into(),
        language: "rust".into(),
        local_services: vec![crate::commands::codegen::metadata::LocalServiceMetadata {
            name: "EchoService".into(),
            package: "echo".into(),
            proto_file: "echo.proto".into(),
            handler_interface: "EchoServiceHandler".into(),
            workload_type: "EchoServiceWorkload".into(),
            dispatcher_type: "EchoServiceDispatcher".into(),
            methods: vec![crate::commands::codegen::metadata::MethodMetadata {
                name: "Echo".into(),
                snake_name: "echo".into(),
                input_type: "EchoRequest".into(),
                output_type: "EchoResponse".into(),
                route_key: "echo.Echo".into(),
                ..Default::default()
            }],
        }],
        remote_services: vec![],
    };
    let catalog = ScaffoldCatalog::from_metadata(&metadata);
    assert_eq!(catalog.local_services.len(), 1);
    assert_eq!(catalog.local_services[0].name, "EchoService");
    assert!(catalog.local_services[0].handler_interface.is_some());
    assert!(catalog.local_services[0].client_type.is_none());
    assert!(catalog.remote_services.is_empty());
    assert!(catalog.has_any_methods());

    assert!(
        !ScaffoldCatalog {
            local_services: vec![],
            remote_services: vec![]
        }
        .has_any_methods()
    );
}
