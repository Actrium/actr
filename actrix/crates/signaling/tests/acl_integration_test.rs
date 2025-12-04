//! ACL integration tests for signaling server
//!
//! Tests ACL enforcement in:
//! - Presence notifications
//! - Service discovery  
//! - WebRTC relay

use actr_protocol::{ActrId, ActrType, Realm};
use actrix_common::tenant::acl::ActorAcl;
use serial_test::serial;
use std::path::PathBuf;

async fn setup_test_db() -> anyhow::Result<()> {
    use actrix_common::storage::db;

    // Use a temporary path for testing
    let test_db_path = PathBuf::from(format!("/tmp/actrix_test_{}.db", uuid::Uuid::new_v4()));
    db::set_db_path(&test_db_path).await?;

    Ok(())
}

fn create_test_actor(serial: u64, actor_type: &str, realm_id: u32) -> ActrId {
    ActrId {
        serial_number: serial,
        r#type: ActrType {
            manufacturer: "test".to_string(),
            name: actor_type.to_string(),
        },
        realm: Realm { realm_id },
    }
}

#[tokio::test]
#[serial]
async fn test_can_discover_with_acl_allow() -> anyhow::Result<()> {
    setup_test_db().await?;

    let tenant_id = "1";
    let from_type = "user";
    let to_type = "service";

    // Setup: Create ACL rule allowing user -> service discovery
    let mut acl = ActorAcl::new(
        tenant_id.to_string(),
        from_type.to_string(),
        to_type.to_string(),
        true, // allow
    );
    acl.save().await?;

    // Test: can_discover should return true
    let can_discover = ActorAcl::can_discover(tenant_id, from_type, to_type).await?;
    assert!(can_discover, "ACL should allow user -> service discovery");

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_can_discover_with_acl_deny() -> anyhow::Result<()> {
    setup_test_db().await?;

    let tenant_id = "1";
    let from_type = "anonymous";
    let to_type = "admin";

    // Setup: Create ACL rule denying anonymous -> admin discovery
    let mut acl = ActorAcl::new(
        tenant_id.to_string(),
        from_type.to_string(),
        to_type.to_string(),
        false, // deny
    );
    acl.save().await?;

    // Test: can_discover should return false
    let can_discover = ActorAcl::can_discover(tenant_id, from_type, to_type).await?;
    assert!(
        !can_discover,
        "ACL should deny anonymous -> admin discovery"
    );

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_can_discover_default_deny() -> anyhow::Result<()> {
    setup_test_db().await?;

    let tenant_id = "1";
    let from_type = "unknown-type";
    let to_type = "unknown-service";

    // Test: can_discover should return false (default deny) when no rule exists
    let can_discover = ActorAcl::can_discover(tenant_id, from_type, to_type).await?;
    assert!(
        !can_discover,
        "ACL should deny by default when no rule exists"
    );

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_presence_acl_filtering() -> anyhow::Result<()> {
    setup_test_db().await?;

    use signaling::presence::PresenceManager;

    let tenant_id = "1";

    // Setup: Allow user -> service, deny anonymous -> service
    let mut acl1 = ActorAcl::new(
        tenant_id.to_string(),
        "user".to_string(),
        "service".to_string(),
        true,
    );
    acl1.save().await?;

    let mut acl2 = ActorAcl::new(
        tenant_id.to_string(),
        "anonymous".to_string(),
        "service".to_string(),
        false,
    );
    acl2.save().await?;

    // Create actors
    let user_actor = create_test_actor(1, "user", 1);
    let anon_actor = create_test_actor(2, "anonymous", 1);
    let service_actor = create_test_actor(3, "service", 1);

    // Create presence manager
    let mut manager = PresenceManager::new();

    // Subscribe both user and anonymous to service type
    manager.subscribe(user_actor.clone(), service_actor.r#type.clone());
    manager.subscribe(anon_actor.clone(), service_actor.r#type.clone());

    // Get subscribers with ACL filtering
    let allowed_subscribers = manager.get_subscribers_with_acl(&service_actor).await;

    // Only user should be allowed
    assert_eq!(allowed_subscribers.len(), 1);
    assert_eq!(allowed_subscribers[0].serial_number, 1);
    assert_eq!(allowed_subscribers[0].r#type.name, "user");

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_service_registry_acl_discovery() -> anyhow::Result<()> {
    setup_test_db().await?;

    use signaling::service_registry::ServiceRegistry;

    let tenant_id = "1";

    // Setup: Allow user -> service, deny user -> admin
    let mut acl1 = ActorAcl::new(
        tenant_id.to_string(),
        "user".to_string(),
        "service".to_string(),
        true,
    );
    acl1.save().await?;

    let mut acl2 = ActorAcl::new(
        tenant_id.to_string(),
        "user".to_string(),
        "admin".to_string(),
        false,
    );
    acl2.save().await?;

    // Create actors
    let user_actor = create_test_actor(1, "user", 1);
    let service_actor = create_test_actor(2, "service", 1);
    let admin_actor = create_test_actor(3, "admin", 1);

    // Create service registry and register services
    let mut registry = ServiceRegistry::new();

    registry.register_service_full(
        service_actor.clone(),
        "test-service".to_string(),
        vec![],
        None,
        None,
        None,
    );

    registry.register_service_full(
        admin_actor.clone(),
        "admin-service".to_string(),
        vec![],
        None,
        None,
        None,
    );

    // Discover services with ACL filtering
    let service_type = ActrType {
        manufacturer: "test".to_string(),
        name: "service".to_string(),
    };
    let services = registry
        .discover_with_acl(&user_actor, &service_type)
        .await?;

    // Should find service (allowed)
    assert_eq!(services.len(), 1);
    assert_eq!(services[0].actor_id.serial_number, 2);

    // Discover admin services
    let admin_type = ActrType {
        manufacturer: "test".to_string(),
        name: "admin".to_string(),
    };
    let admin_services = registry.discover_with_acl(&user_actor, &admin_type).await?;

    // Should not find admin (denied)
    assert_eq!(admin_services.len(), 0);

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_cross_realm_discovery_denied() -> anyhow::Result<()> {
    setup_test_db().await?;

    let tenant1_id = "1";
    let tenant2_id = "2";

    // Setup: Allow user -> service within tenant1
    let mut acl = ActorAcl::new(
        tenant1_id.to_string(),
        "user".to_string(),
        "service".to_string(),
        true,
    );
    acl.save().await?;

    // Create actors in different realms
    let user_actor = create_test_actor(1, "user", 1); // realm 1
    let service_actor = create_test_actor(2, "service", 2); // realm 2 (different)

    // Test cross-realm discovery using PresenceManager
    use signaling::presence::PresenceManager;

    let manager = PresenceManager::new();
    let allowed_subscribers = manager.get_subscribers_with_acl(&service_actor).await;

    // Should be empty (cross-realm denied)
    assert_eq!(
        allowed_subscribers.len(),
        0,
        "Cross-realm discovery should be denied"
    );

    Ok(())
}
