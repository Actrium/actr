//! Inbound Layer 3: Inbound dispatch layer
//!
//! Responsible for inbound message routing and dispatching:
//! - DataStreamRegistry: LatencyFirst type message registration and callback (streaming data chunks)
//! - MediaFrameRegistry: MediaTrack type message registration and callback (media streams)

mod data_stream_registry;
mod media_frame_registry;

pub(crate) use data_stream_registry::DataStreamRegistry;
pub use media_frame_registry::MediaFrameRegistry;

// MediaSample and MediaType are now re-exported from actr-framework, not here
