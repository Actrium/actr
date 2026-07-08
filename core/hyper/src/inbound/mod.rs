//! Inbound Layer 3: Inbound dispatch layer
//!
//! Responsible for inbound message routing and dispatching:
//! - DataChunkRegistry: LatencyFirst type message registration and callback (streaming data chunks)
//! - MediaFrameRegistry: MediaTrack type message registration and callback (media streams)

mod data_chunk_registry;
mod media_frame_registry;

pub(crate) use data_chunk_registry::DataChunkRegistry;
pub use media_frame_registry::MediaFrameRegistry;
#[cfg(feature = "test-utils")]
pub use media_frame_registry::MediaTrackCallback;

// MediaSample and MediaType are now re-exported from actr-framework, not here
