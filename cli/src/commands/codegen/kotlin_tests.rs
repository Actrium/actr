use super::*;

fn remote_echo_service() -> ServiceInfo {
    ServiceInfo {
        service_name: "EchoService".to_string(),
        proto_package: "echo".to_string(),
        proto_file_name: "echo.proto".to_string(),
        is_local: false,
        remote_target_type: Some("acme:EchoService:1.0.0".to_string()),
        methods: vec![MethodInfo {
            name: "Echo".to_string(),
            request_type: "EchoRequest".to_string(),
            response_type: "EchoResponse".to_string(),
        }],
        needs_outer_class_suffix: false,
    }
}

fn local_client_service() -> ServiceInfo {
    ServiceInfo {
        service_name: "ClientService".to_string(),
        proto_package: "client".to_string(),
        proto_file_name: "client.proto".to_string(),
        is_local: true,
        remote_target_type: None,
        methods: vec![MethodInfo {
            name: "Send".to_string(),
            request_type: "SendRequest".to_string(),
            response_type: "SendResponse".to_string(),
        }],
        needs_outer_class_suffix: false,
    }
}

#[test]
fn remote_service_registry_uses_manifest_dependency_aliases() {
    let generator = KotlinGenerator;
    let service = remote_echo_service();
    let mut aliases = HashMap::new();
    aliases.insert(
        "acme:EchoService:1.0.0".to_string(),
        "echo-service".to_string(),
    );

    let content = generator
        .generate_remote_service_registry(&[&service], &aliases)
        .expect("render remote registry");

    assert!(content.contains("val remoteRouteAliases: Map<String, String>"));
    assert!(content.contains("\"echo.Echo\" to \"echo-service\""));
    assert!(content.contains("resolveManifestDependency(manifestPath, alias)"));
    assert!(content.contains(".toSet()"));
    assert!(content.contains(".associateWith { alias ->"));
    assert!(content.contains("targetsByAlias.getValue(alias)"));
    assert!(
        content.contains("getActorType(routeKey: String, remoteTargets: Map<String, ActrType>)")
    );
    assert!(!content.contains("ActrType(manufacturer"));
    assert!(!content.contains("remoteRoutes: Map<String, ActrType>"));
}

#[test]
fn unified_workload_requires_pre_resolved_remote_targets() {
    let services = vec![local_client_service(), remote_echo_service()];
    let content = generate_unified_workload_scaffold(&services, "com.example.generated");

    assert!(content.contains("class UnifiedWorkload("));
    assert!(content.contains("private val remoteTargets: Map<String, ActrType>,"));
    assert!(content.contains("suspend fun onStart(ctx: ActrContext)"));
    assert!(content.contains("suspend fun onReady(ctx: ActrContext)"));
    assert!(content.contains("suspend fun onStop(ctx: ActrContext)"));
    assert!(content.contains("suspend fun onError(ctx: ActrContext, event: ErrorEvent)"));
    assert!(
        content
            .contains("suspend fun dispatch(ctx: ActrContext, envelope: RpcEnvelope): ByteArray")
    );
    assert!(content.contains("UnifiedDispatcher.discoverRemoteServices(ctx, remoteTargets)"));
    assert!(content.contains("UnifiedDispatcher.dispatch(handler, ctx, remoteTargets, envelope)"));

    assert!(!content.contains("WorkloadLifecycleBridge"));
    assert!(content.contains("val lifecycle = UnifiedLifecycleAdapter(workload)"));
    assert!(content.contains("val dynamicWorkload = lifecycle.toDynamicWorkload()"));
    assert!(!content.contains("import io.actrium.actr.DynamicWorkload"));
    assert!(!content.contains("fun toDynamicWorkload(): DynamicWorkload"));
    assert!(!content.contains("private val realmId"));
    assert!(!content.contains("ActrId("));
    assert!(!content.contains("Realm("));
    assert!(!content.contains("manifestPath: String?"));
    assert!(!content.contains("remoteTargets: Map<String, ActrType> = emptyMap()"));
    assert!(!content.contains("manifestPath?.let"));
    assert!(!content.contains("\n\\\n"));
}

#[test]
fn local_only_workload_does_not_require_remote_targets() {
    let content =
        generate_unified_workload_scaffold(&[local_client_service()], "com.example.generated");

    assert!(!content.contains("private val remoteTargets"));
    assert!(!content.contains("resolveRemoteTargets"));
    assert!(content.contains("val workload = UnifiedWorkload(handler)"));
}

#[test]
fn unified_lifecycle_adapter_wraps_unified_workload() {
    let content = generate_unified_lifecycle_adapter_scaffold("com.example.generated");

    assert!(content.contains("package com.example"));
    assert!(content.contains("class UnifiedLifecycleAdapter("));
    assert!(content.contains("private val workload: UnifiedWorkload"));
    assert!(content.contains(") : Workload"));
    assert!(content.contains("import io.actrium.actr.dsl.ActrContext"));
    assert!(content.contains("import io.actrium.actr.dsl.DynamicWorkload"));
    assert!(content.contains("import io.actrium.actr.dsl.ErrorEvent"));
    assert!(content.contains("import io.actrium.actr.dsl.RpcEnvelope"));
    assert!(content.contains("import io.actrium.actr.dsl.Workload"));
    assert!(content.contains("import io.actrium.actr.dsl.dynamicWorkload"));
    assert!(content.contains("override suspend fun onStart(ctx: ActrContext)"));
    assert!(content.contains("workload.onStart(ctx)"));
    assert!(content.contains("override suspend fun onReady(ctx: ActrContext)"));
    assert!(content.contains("workload.onReady(ctx)"));
    assert!(content.contains("override suspend fun onStop(ctx: ActrContext)"));
    assert!(content.contains("workload.onStop(ctx)"));
    assert!(content.contains("override suspend fun onError(ctx: ActrContext, event: ErrorEvent)"));
    assert!(content.contains("workload.onError(ctx, event)"));
    assert!(content.contains(
        "override suspend fun dispatch(ctx: ActrContext, envelope: RpcEnvelope): ByteArray"
    ));
    assert!(content.contains("return workload.dispatch(ctx, envelope)"));
    assert!(content.contains("fun toDynamicWorkload(): DynamicWorkload"));
    assert!(content.contains("return dynamicWorkload("));
    assert!(content.contains("lifecycle = this"));
}

#[test]
fn kotlin_bootstrap_fixtures_inject_manifest_resolved_targets() {
    for fixture in [
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/kotlin/echo/MainActivity.kt"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/kotlin/echo/EchoIntegrationTest.kt"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/kotlin/data-stream/DataStreamIntegrationTest.kt"
        )),
    ] {
        assert!(fixture.contains("RemoteServiceRegistry.resolveRemoteTargets("));
        assert!(fixture.contains("remoteTargets = remoteTargets"));
        assert!(fixture.contains("UnifiedLifecycleAdapter("));
        assert!(fixture.contains("toDynamicWorkload()"));
        assert!(!fixture.contains(concat!("attach", "(clientWorkload)")));
    }
}
