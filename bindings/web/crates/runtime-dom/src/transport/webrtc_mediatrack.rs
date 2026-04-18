//! WebRTC MediaTrack lane for DOM-side MediaStreamTrack transport.
//!
//! Used by the DOM side to transport media data over WebRTC MediaStreamTrack.
//! Supports only `MEDIA_RTP`.
//!
//! ## Notes
//! - MediaTrack can only be used in the DOM environment because Service Workers do not support WebRTC
//! - MediaTrack uses the Fast Path and bypasses the mailbox
//! - Supports both audio and video tracks
//! - RTP extraction requires MediaStreamTrackProcessor, WebRTC Stats, or similar APIs

use super::lane::{DataLane, LaneResult};
use actr_web_common::WebError;
use bytes::Bytes;
use futures::channel::mpsc;
use parking_lot::Mutex;
use std::sync::Arc;

/// MediaTrack type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaTrackType {
    /// Audio track.
    Audio,
    /// Video track.
    Video,
}

/// WebRTC MediaTrack lane builder.
///
/// Creates and configures a WebRTC MediaTrack lane.
pub struct WebRtcMediaTrackLaneBuilder {
    track_id: String,
    track_type: MediaTrackType,
    buffer_size: usize,
}

impl WebRtcMediaTrackLaneBuilder {
    /// Create a new WebRTC MediaTrack lane builder.
    ///
    /// # Parameters
    /// - `track_id`: Unique MediaStreamTrack identifier
    /// - `track_type`: Track type (`Audio` or `Video`)
    pub fn new(track_id: impl Into<String>, track_type: MediaTrackType) -> Self {
        Self {
            track_id: track_id.into(),
            track_type,
            buffer_size: 512, // Media frame buffers are larger by default.
        }
    }

    /// Set the receive buffer size.
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }

    /// Build the WebRTC MediaTrack lane.
    ///
    /// # Note
    /// MediaTrack lanes support only `MEDIA_RTP`.
    /// Actual media extraction must be implemented with MediaStreamTrackProcessor or similar APIs.
    pub fn build(self) -> LaneResult<DataLane> {
        // Create the receive channel.
        let (_tx, rx) = mpsc::unbounded();
        let rx = Arc::new(Mutex::new(rx));

        log::info!(
            "WebRTC MediaTrack Lane created successfully: track_id={}, track_type={:?}",
            self.track_id,
            self.track_type
        );

        // Note: actual MediaTrack reception requires one of the following:
        // 1. MediaStreamTrackProcessor API (Insertable Streams)
        // 2. WebRTC Stats API
        // 3. WebRTC Transform API
        //
        // This builder only creates the lane structure. Actual data flow must be wired at a higher layer.

        Ok(DataLane::WebRtcMediaTrack {
            track_id: self.track_id,
            rx,
        })
    }
}

/// MediaTrack processor.
///
/// Extracts media frame data from MediaStreamTrack and forwards it into the lane.
///
/// ## Implementation options
///
/// There are several ways to extract RTP or media data in the browser:
///
/// 1. **Insertable Streams (recommended)**:
///    ```javascript
///    const receiver = peerConnection.getReceivers()[0];
///    const readableStream = receiver.readable;
///    const reader = readableStream.getReader();
///
///    while (true) {
///      const {value: encodedFrame, done} = await reader.read();
///      if (done) break;
///      // Forward encodedFrame into Rust
///    }
///    ```
///
/// 2. **WebCodecs API**:
///    ```javascript
///    const processor = new MediaStreamTrackProcessor({track: videoTrack});
///    const reader = processor.readable.getReader();
///
///    while (true) {
///      const {value: videoFrame, done} = await reader.read();
///      if (done) break;
///      // Process VideoFrame
///    }
///    ```
///
/// 3. **Canvas + ImageData (video)**:
///    ```javascript
///    const video = document.createElement('video');
///    video.srcObject = new MediaStream([track]);
///    const canvas = document.createElement('canvas');
///    const ctx = canvas.getContext('2d');
///
///    setInterval(() => {
///      ctx.drawImage(video, 0, 0);
///      const imageData = ctx.getImageData(0, 0, canvas.width, canvas.height);
///      // Send imageData
///    }, 1000/30); // 30fps
///    ```
pub struct MediaTrackProcessor {
    track_id: String,
    track_type: MediaTrackType,
    tx: mpsc::UnboundedSender<Bytes>,
}

impl MediaTrackProcessor {
    /// Create a new MediaTrack processor.
    ///
    /// # Parameters
    /// - `track_id`: MediaStreamTrack ID
    /// - `track_type`: Track type
    /// - `tx`: Send channel connected to the lane receiver
    pub fn new(
        track_id: String,
        track_type: MediaTrackType,
        tx: mpsc::UnboundedSender<Bytes>,
    ) -> Self {
        Self {
            track_id,
            track_type,
            tx,
        }
    }

    /// Process one media frame.
    ///
    /// Sends the media frame to the receiving side of the lane.
    ///
    /// # Parameters
    /// - `frame_data`: Media frame data, such as an RTP packet or encoded frame
    ///
    /// # Returns
    /// - `Ok(())`: Send succeeded
    /// - `Err`: Send failed because the channel is closed
    pub fn process_frame(&self, frame_data: Bytes) -> LaneResult<()> {
        self.tx.unbounded_send(frame_data.clone()).map_err(|_| {
            WebError::Transport(format!(
                "MediaTrack Lane receiver is closed: track_id={}",
                self.track_id
            ))
        })?;

        log::trace!(
            "MediaTrack processed frame: track_id={}, track_type={:?}, size={} bytes",
            self.track_id,
            self.track_type,
            frame_data.len()
        );

        Ok(())
    }

    /// Process a batch of media frames.
    pub fn process_frames(&self, frames: Vec<Bytes>) -> LaneResult<()> {
        for frame in frames {
            self.process_frame(frame)?;
        }
        Ok(())
    }

    /// Return the track ID.
    pub fn track_id(&self) -> &str {
        &self.track_id
    }

    /// Return the track type.
    pub fn track_type(&self) -> MediaTrackType {
        self.track_type
    }
}

/// Helper for creating a MediaTrack lane and processor.
///
/// Creates the lane and processor structures associated with a `MediaStreamTrack`.
///
/// # Note
/// This function only creates the base structures. Actual media extraction must
/// still be implemented on the JavaScript side.
pub fn create_mediatrack_lane_with_processor(
    track_id: impl Into<String>,
    track_type: MediaTrackType,
) -> LaneResult<(DataLane, MediaTrackProcessor)> {
    let track_id = track_id.into();
    let (tx, rx) = mpsc::unbounded();
    let rx = Arc::new(Mutex::new(rx));

    let processor = MediaTrackProcessor::new(track_id.clone(), track_type, tx);

    let lane = DataLane::WebRtcMediaTrack {
        track_id: track_id.clone(),
        rx,
    };

    log::info!(
        "Created MediaTrack Lane and processor: track_id={}, track_type={:?}",
        track_id,
        track_type
    );

    Ok((lane, processor))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn test_mediatrack_lane_builder() {
        let lane = WebRtcMediaTrackLaneBuilder::new("test-track-id", MediaTrackType::Video)
            .build()
            .unwrap();

        match lane {
            DataLane::WebRtcMediaTrack { track_id, .. } => {
                assert_eq!(track_id, "test-track-id");
            }
            _ => panic!("Expected WebRtcMediaTrack variant"),
        }
    }

    #[wasm_bindgen_test]
    fn test_create_mediatrack_lane_with_processor() {
        let (lane, processor) =
            create_mediatrack_lane_with_processor("test-track", MediaTrackType::Audio).unwrap();

        assert_eq!(processor.track_id(), "test-track");
        assert_eq!(processor.track_type(), MediaTrackType::Audio);

        match lane {
            DataLane::WebRtcMediaTrack { track_id, .. } => {
                assert_eq!(track_id, "test-track");
            }
            _ => panic!("Expected WebRtcMediaTrack variant"),
        }
    }

    #[wasm_bindgen_test]
    fn test_mediatrack_processor_process_frame() {
        let (tx, mut rx) = mpsc::unbounded();
        let processor =
            MediaTrackProcessor::new("test-track".to_string(), MediaTrackType::Video, tx);

        let frame_data = Bytes::from_static(b"test frame data");
        processor.process_frame(frame_data.clone()).unwrap();

        // Verify the data was sent to the channel.
        let received = rx.try_recv().unwrap();
        assert_eq!(received, frame_data);
    }
}
