mod peer;

// Re-export ActrId from actr-protocol (new naming convention)
pub use actr_protocol::ActrId;
pub use peer::PeerId;

/// Realm ID type - simple u32 wrapper
pub type RealmId = u32;
