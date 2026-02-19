//! Internal integration-test facade.
//!
//! This module is intentionally feature-gated and not part of the default SDK
//! surface. It collects test-oriented protocol and helper exports used by
//! in-repo integration tests.

pub use admin::nonce_auth;
pub use admin::{
    ConfigType, CreateRealmRequest, DeleteRealmRequest, GetConfigRequest, GetNodeInfoRequest,
    GetRealmRequest, ListRealmsRequest, NonceCredential, REALM_ENABLED_KEY, REALM_USE_SERVERS_KEY,
    REALM_VERSION_KEY, ResourceType, ShutdownRequest, SupervisedServiceClient, UpdateConfigRequest,
    UpdateRealmRequest,
};
