//! Service type definitions
//!
//! Defines the types of services supported by the system

use actrix_proto::ResourceType;
use serde::{Deserialize, Serialize};
use strum::Display;

/// Service type enumeration
#[derive(Debug, Clone, Serialize, Deserialize, Display, PartialEq, Eq)]
pub enum ServiceType {
    Stun,
    Turn,
    Signaling,
    Ais,
    Ks,
}

/// Convert ServiceType to ResourceType
impl From<ServiceType> for ResourceType {
    fn from(service_type: ServiceType) -> Self {
        match service_type {
            ServiceType::Stun => ResourceType::Stun,
            ServiceType::Turn => ResourceType::Turn,
            ServiceType::Signaling => ResourceType::Signaling,
            ServiceType::Ais => ResourceType::Ais,
            ServiceType::Ks => ResourceType::Ks,
        }
    }
}

/// Convert ServiceType to ResourceType (reference version)
impl From<&ServiceType> for ResourceType {
    fn from(service_type: &ServiceType) -> Self {
        Self::from(service_type.clone())
    }
}
