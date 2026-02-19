use serde::{Deserialize, Serialize};

/// Peer ID type
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PeerId([u8; 16]);

impl PeerId {
    pub fn from_array(array: [u8; 16]) -> Self {
        Self(array)
    }
}
