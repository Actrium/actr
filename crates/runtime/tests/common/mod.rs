//! Common test utilities for WebRTC and signaling tests
//!
//! This module provides shared infrastructure for integration tests:
//! - WebSocket-based signaling server (TestSignalingServer)
//! - Helper functions for creating peers and credentials
//! - Common test utilities

pub mod signaling;
pub mod utils;

pub use signaling::TestSignalingServer;
pub use utils::*;
