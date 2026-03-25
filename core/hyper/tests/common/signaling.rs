//! WebSocket-based test signaling server
//!
//! Thin wrapper around `actr_mock_signaling::MockSignalingServer` with
//! backward-compatible API for existing integration tests.

pub use actr_mock_signaling::MockSignalingServer;

/// Controllable test signaling server with real WebSocket.
///
/// This is a compatibility wrapper around [`MockSignalingServer`] that
/// preserves the original `TestSignalingServer` API used by hyper integration
/// tests.
pub struct TestSignalingServer {
    inner: MockSignalingServer,
}

impl TestSignalingServer {
    /// Start the test server on a random available port.
    pub async fn start() -> anyhow::Result<Self> {
        Ok(Self {
            inner: MockSignalingServer::start().await?,
        })
    }

    /// Start the test server on the specified port.
    pub async fn start_on_port(port: u16) -> anyhow::Result<Self> {
        Ok(Self {
            inner: MockSignalingServer::start_on_port(port).await?,
        })
    }

    pub fn url(&self) -> String {
        self.inner.url()
    }

    pub fn port(&self) -> u16 {
        self.inner.port()
    }

    pub async fn shutdown(&mut self) {
        self.inner.shutdown().await;
    }

    pub fn pause_forwarding(&self) {
        self.inner.pause_forwarding();
    }

    pub fn resume_forwarding(&self) {
        self.inner.resume_forwarding();
    }

    pub fn message_count(&self) -> u32 {
        self.inner.message_count()
    }

    pub fn get_ice_restart_count(&self) -> u32 {
        self.inner.ice_restart_count()
    }

    pub fn get_connection_count(&self) -> u32 {
        self.inner.connection_count()
    }

    pub fn get_disconnection_count(&self) -> u32 {
        self.inner.disconnection_count()
    }

    pub fn reset_counters(&self) {
        self.inner.reset_counters();
    }

    pub fn is_running(&self) -> bool {
        self.inner.is_running()
    }
}
