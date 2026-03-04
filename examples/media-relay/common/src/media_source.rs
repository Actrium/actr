//! Media source abstractions for testing

use actr_framework::{MediaSample, MediaType};
use bytes::Bytes;

/// Trait for media data sources
pub trait MediaSource: Send + Sync {
    /// Get the next media sample
    fn next_sample(&mut self) -> Option<MediaSample>;

    /// Get the codec string (e.g., "VP8", "H264")
    fn codec(&self) -> &str;

    /// Get the media type
    fn media_type(&self) -> MediaType;
}

/// Test pattern video source (generates colored frames)
pub struct TestPatternSource {
    frame_count: u64,
    fps: u32,
}

impl TestPatternSource {
    pub fn new(fps: u32) -> Self {
        Self {
            frame_count: 0,
            fps,
        }
    }
}

impl MediaSource for TestPatternSource {
    fn next_sample(&mut self) -> Option<MediaSample> {
        let timestamp = (self.frame_count * 90000 / self.fps as u64) as u32;

        // Generate test pattern data (simulated encoded frame)
        // Keep frame size small (< 1KB) to fit in WebRTC DataChannel max message size
        let pattern = match (self.frame_count / 30) % 3 {
            0 => vec![0xFF, 0x00, 0x00], // Red
            1 => vec![0x00, 0xFF, 0x00], // Green
            _ => vec![0x00, 0x00, 0xFF], // Blue
        };

        let data = Bytes::from(pattern.repeat(256)); // 768 bytes (3 * 256)

        self.frame_count += 1;

        Some(MediaSample {
            data,
            timestamp,
            codec: "VP8".to_string(),
            media_type: MediaType::Video,
        })
    }

    fn codec(&self) -> &str {
        "VP8"
    }

    fn media_type(&self) -> MediaType {
        MediaType::Video
    }
}
