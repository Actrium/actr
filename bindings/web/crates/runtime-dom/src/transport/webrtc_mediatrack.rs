//! WebRTC MediaTrack Lane - DOM 端的 WebRTC MediaStreamTrack 传输通道
//!
//! WebRTC MediaTrack Lane 用于 DOM 端通过 WebRTC MediaStreamTrack 传输媒体数据。
//! 仅支持 PayloadType：MEDIA_RTP
//!
//! ## 注意事项
//! - MediaTrack 只能在 DOM 环境中使用（Service Worker 不支持 WebRTC）
//! - MediaTrack 使用 Fast Path，直接回调处理，不经过 Mailbox
//! - 支持音频和视频两种类型的 Track
//! - 需要使用 MediaStreamTrackProcessor API 或 WebRTC Stats API 来提取 RTP 数据

use super::lane::{DataLane, LaneResult};
use actr_web_common::WebError;
use bytes::Bytes;
use futures::channel::mpsc;
use parking_lot::Mutex;
use std::sync::Arc;

/// MediaTrack 类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaTrackType {
    /// 音频轨道
    Audio,
    /// 视频轨道
    Video,
}

/// WebRTC MediaTrack Lane 构建器
///
/// 用于创建和配置 WebRTC MediaTrack Lane
pub struct WebRtcMediaTrackLaneBuilder {
    track_id: String,
    track_type: MediaTrackType,
    buffer_size: usize,
}

impl WebRtcMediaTrackLaneBuilder {
    /// 创建新的 WebRTC MediaTrack Lane 构建器
    ///
    /// # 参数
    /// - `track_id`: MediaStreamTrack 的唯一标识符
    /// - `track_type`: Track 类型（Audio 或 Video）
    pub fn new(track_id: impl Into<String>, track_type: MediaTrackType) -> Self {
        Self {
            track_id: track_id.into(),
            track_type,
            buffer_size: 512, // 媒体帧缓冲区默认更大
        }
    }

    /// 设置接收缓冲区大小
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }

    /// 构建 WebRTC MediaTrack Lane
    ///
    /// # 注意
    /// MediaTrack Lane 只支持 MEDIA_RTP PayloadType。
    /// 实际的媒体数据提取需要通过 MediaStreamTrackProcessor 或其他 API 完成。
    pub fn build(self) -> LaneResult<DataLane> {
        // 创建接收通道
        let (_tx, rx) = mpsc::unbounded();
        let rx = Arc::new(Mutex::new(rx));

        log::info!(
            "WebRTC MediaTrack Lane 创建成功: track_id={}, track_type={:?}",
            self.track_id,
            self.track_type
        );

        // 注意：MediaTrack Lane 的实际数据接收需要通过：
        // 1. MediaStreamTrackProcessor API (Insertable Streams)
        // 2. WebRTC Stats API
        // 3. 或者通过 WebRTC Transform API
        //
        // 这里只创建了 Lane 结构，实际的数据流需要在上层集成时配置

        Ok(DataLane::WebRtcMediaTrack {
            track_id: self.track_id,
            rx,
        })
    }
}

/// MediaTrack 处理器
///
/// 用于从 MediaStreamTrack 提取媒体帧数据并发送到 Lane
///
/// ## 实现方式
///
/// Web 环境下提取 RTP 数据有几种方式：
///
/// 1. **Insertable Streams (推荐)**：
///    ```javascript
///    const receiver = peerConnection.getReceivers()[0];
///    const readableStream = receiver.readable;
///    const reader = readableStream.getReader();
///
///    while (true) {
///      const {value: encodedFrame, done} = await reader.read();
///      if (done) break;
///      // 将 encodedFrame 发送到 Rust 端
///    }
///    ```
///
/// 2. **WebCodecs API**：
///    ```javascript
///    const processor = new MediaStreamTrackProcessor({track: videoTrack});
///    const reader = processor.readable.getReader();
///
///    while (true) {
///      const {value: videoFrame, done} = await reader.read();
///      if (done) break;
///      // 处理 VideoFrame
///    }
///    ```
///
/// 3. **Canvas + ImageData (视频)**：
///    ```javascript
///    const video = document.createElement('video');
///    video.srcObject = new MediaStream([track]);
///    const canvas = document.createElement('canvas');
///    const ctx = canvas.getContext('2d');
///
///    setInterval(() => {
///      ctx.drawImage(video, 0, 0);
///      const imageData = ctx.getImageData(0, 0, canvas.width, canvas.height);
///      // 发送 imageData
///    }, 1000/30); // 30fps
///    ```
pub struct MediaTrackProcessor {
    track_id: String,
    track_type: MediaTrackType,
    tx: mpsc::UnboundedSender<Bytes>,
}

impl MediaTrackProcessor {
    /// 创建新的 MediaTrack 处理器
    ///
    /// # 参数
    /// - `track_id`: MediaStreamTrack ID
    /// - `track_type`: Track 类型
    /// - `tx`: 发送通道（连接到 Lane 的接收端）
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

    /// 处理媒体帧数据
    ///
    /// 将媒体帧数据发送到 Lane 的接收端
    ///
    /// # 参数
    /// - `frame_data`: 媒体帧数据（RTP packet 或 编码后的帧）
    ///
    /// # 返回
    /// - `Ok(())`: 发送成功
    /// - `Err`: 发送失败（通道已关闭）
    pub fn process_frame(&self, frame_data: Bytes) -> LaneResult<()> {
        self.tx.unbounded_send(frame_data.clone()).map_err(|_| {
            WebError::Transport(format!(
                "MediaTrack Lane 接收端已关闭: track_id={}",
                self.track_id
            ))
        })?;

        log::trace!(
            "MediaTrack 处理媒体帧: track_id={}, track_type={:?}, size={} bytes",
            self.track_id,
            self.track_type,
            frame_data.len()
        );

        Ok(())
    }

    /// 批量处理媒体帧
    pub fn process_frames(&self, frames: Vec<Bytes>) -> LaneResult<()> {
        for frame in frames {
            self.process_frame(frame)?;
        }
        Ok(())
    }

    /// 获取 Track ID
    pub fn track_id(&self) -> &str {
        &self.track_id
    }

    /// 获取 Track 类型
    pub fn track_type(&self) -> MediaTrackType {
        self.track_type
    }
}

/// MediaTrack Lane 辅助函数
///
/// 用于从 web_sys::MediaStreamTrack 创建 Lane 和 Processor
///
/// # 注意
/// 这个函数只创建基础结构，实际的媒体帧提取需要在 JavaScript 层完成
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
        "创建 MediaTrack Lane 和 Processor: track_id={}, track_type={:?}",
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

        // 验证数据已发送到通道
        use futures::stream::StreamExt;
        let received = rx.try_next().unwrap();
        assert_eq!(received, Some(frame_data));
    }
}
