use super::*;

#[test]
fn empty_container_validates_no_requirements() {
    let container = ServiceContainer::new();
    assert!(container.validate(&[]).is_ok());
}

#[test]
fn validate_reports_each_missing_required_component() {
    let container = ServiceContainer::new();
    let all = [
        ComponentType::ConfigManager,
        ComponentType::DependencyResolver,
        ComponentType::ServiceDiscovery,
        ComponentType::NetworkValidator,
        ComponentType::FingerprintValidator,
        ComponentType::ProtoProcessor,
        ComponentType::CacheManager,
        ComponentType::UserInterface,
    ];
    for ct in &all {
        let err = container.validate(std::slice::from_ref(ct)).unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains(&format!("{ct:?}")),
            "expected {ct:?} in error: {msg}"
        );
    }
}

#[test]
fn getters_return_error_when_component_not_registered() {
    let container = ServiceContainer::new();
    assert!(container.get_config_manager().is_err());
    assert!(container.get_dependency_resolver().is_err());
    assert!(container.get_service_discovery().is_err());
    assert!(container.get_network_validator().is_err());
    assert!(container.get_fingerprint_validator().is_err());
    assert!(container.get_proto_processor().is_err());
    assert!(container.get_cache_manager().is_err());
    assert!(container.get_user_interface().is_err());
}

#[test]
fn pipelines_fail_without_required_components() {
    let mut container = ServiceContainer::new();
    // ValidationPipeline needs ConfigManager (and friends) first.
    assert!(container.get_validation_pipeline().is_err());
    assert!(container.get_install_pipeline().is_err());
    assert!(container.get_generation_pipeline().is_err());
}

#[test]
fn builder_builds_empty_container_and_carries_config_path() {
    let builder = ContainerBuilder::new().config_path("manifest.toml");
    let container = builder.build().unwrap();
    assert!(container.validate(&[]).is_ok());
}

#[test]
fn defaults_construct() {
    let _ = ServiceContainer::default();
    let _ = ContainerBuilder::default();
}
