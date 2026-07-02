use super::*;

#[test]
fn test_create_service_filter() {
    let cmd = DiscoveryCommand::new(Some("user-*".to_string()), false, false);
    let filter = cmd.create_service_filter();

    assert!(filter.is_some());
    let filter = filter.unwrap();
    assert_eq!(filter.name_pattern, Some("user-*".to_string()));
}

#[test]
fn test_create_service_filter_none() {
    let cmd = DiscoveryCommand::new(None, false, false);
    let filter = cmd.create_service_filter();

    assert!(filter.is_none());
}

#[test]
fn test_required_components() {
    let cmd = DiscoveryCommand::default();
    let components = cmd.required_components();

    // Discovery requires only ServiceDiscovery + UserInterface up front.
    // ConfigManager and validators are obtained lazily when needed.
    assert!(components.contains(&ComponentType::ServiceDiscovery));
    assert!(components.contains(&ComponentType::UserInterface));
    assert!(!components.contains(&ComponentType::ConfigManager));
    assert!(!components.contains(&ComponentType::DependencyResolver));
    assert!(!components.contains(&ComponentType::NetworkValidator));
    assert!(!components.contains(&ComponentType::FingerprintValidator));
}
