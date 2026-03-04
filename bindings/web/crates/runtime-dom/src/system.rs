//! DOM System Module
//!
//! DOM 端的运行时系统
//! 负责 Fast Path：Stream Handler Registry + MediaFrame Handler Registry
//!
//! # SW ↔ DOM 连接
//!
//! DomSystem 通过 PostMessage 与 Service Worker 通信：
//! - SW → DOM: Fast Path 数据 (STREAM_*, MEDIA_RTP)
//! - DOM → SW: RPC 消息转发

use crate::fastpath::{
    MediaFrameCallback, MediaFrameHandlerRegistry, StreamCallback, StreamHandlerRegistry,
};
use crate::transport::DataLane;
use bytes::Bytes;
use parking_lot::Mutex;
use std::sync::Arc;

/// DOM 运行时系统
///
/// 管理 DOM 端的 Fast Path 注册表：
/// - Stream Handler Registry：处理流数据（STREAM_*）
/// - Media Frame Handler Registry：处理媒体帧（MEDIA_RTP）
pub struct DomSystem {
    /// 流处理器注册表
    stream_registry: Arc<StreamHandlerRegistry>,

    /// 媒体帧处理器注册表
    media_registry: Arc<MediaFrameHandlerRegistry>,

    /// SW 通信通道
    sw_lane: Arc<Mutex<Option<DataLane>>>,
}

impl DomSystem {
    /// 创建新的 DOM 系统
    pub fn new() -> Self {
        Self {
            stream_registry: Arc::new(StreamHandlerRegistry::new()),
            media_registry: Arc::new(MediaFrameHandlerRegistry::new()),
            sw_lane: Arc::new(Mutex::new(None)),
        }
    }

    // ========== SW 连接管理 ==========

    /// 设置 SW 通信通道
    ///
    /// 用于向 SW 转发 RPC 消息
    pub fn set_sw_lane(&self, lane: DataLane) {
        let mut sw_lane = self.sw_lane.lock();
        *sw_lane = Some(lane);
        log::info!("[DomSystem] SW lane connected");
    }

    /// 向 SW 发送消息
    ///
    /// 用于转发 RPC 消息到 SW 的 Mailbox
    pub async fn send_to_sw(&self, data: Bytes) -> Result<(), String> {
        let sw_lane = self.sw_lane.lock();
        if let Some(ref lane) = *sw_lane {
            lane.send(data)
                .await
                .map_err(|e| format!("Failed to send to SW: {}", e))
        } else {
            Err("SW lane not connected".to_string())
        }
    }

    // ========== Stream 处理器管理 ==========

    /// 注册流处理器
    ///
    /// # 参数
    /// - `stream_id`: 流 ID
    /// - `callback`: 处理回调函数
    ///
    /// # 示例
    /// ```ignore
    /// system.register_stream_handler("video_stream".to_string(), Arc::new(|data| {
    ///     // 处理流数据
    /// }));
    /// ```
    pub fn register_stream_handler(&self, stream_id: String, callback: StreamCallback) {
        self.stream_registry.register(stream_id, callback);
    }

    /// 注销流处理器
    pub fn unregister_stream_handler(&self, stream_id: &str) {
        self.stream_registry.unregister(stream_id);
    }

    /// 派发流数据
    ///
    /// 由 Transport 层调用，将接收到的流数据派发给注册的回调
    pub fn dispatch_stream(&self, stream_id: &str, data: Bytes) {
        self.stream_registry.dispatch(stream_id, data);
    }

    // ========== Media 处理器管理 ==========

    /// 注册媒体帧处理器
    ///
    /// # 参数
    /// - `track_id`: Track ID
    /// - `callback`: 处理回调函数
    ///
    /// # 示例
    /// ```ignore
    /// system.register_media_handler("audio_track".to_string(), Arc::new(|frame| {
    ///     // 处理媒体帧
    /// }));
    /// ```
    pub fn register_media_handler(&self, track_id: String, callback: MediaFrameCallback) {
        self.media_registry.register(track_id, callback);
    }

    /// 注销媒体帧处理器
    pub fn unregister_media_handler(&self, track_id: &str) {
        self.media_registry.unregister(track_id);
    }

    /// 派发媒体帧
    ///
    /// 由 Transport 层调用，将接收到的媒体帧派发给注册的回调
    pub fn dispatch_media_frame(&self, track_id: &str, frame: Bytes) {
        self.media_registry.dispatch(track_id, frame);
    }

    // ========== 获取注册表引用 ==========

    /// 获取流处理器注册表的引用
    ///
    /// 供 Transport 层使用
    pub fn stream_registry(&self) -> Arc<StreamHandlerRegistry> {
        Arc::clone(&self.stream_registry)
    }

    /// 获取媒体帧处理器注册表的引用
    ///
    /// 供 Transport 层使用
    pub fn media_registry(&self) -> Arc<MediaFrameHandlerRegistry> {
        Arc::clone(&self.media_registry)
    }
}

impl Default for DomSystem {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dom_system_creation() {
        let _system = DomSystem::new();
    }

    #[test]
    fn test_stream_handler_registration() {
        let system = DomSystem::new();
        let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_clone = Arc::clone(&called);

        system.register_stream_handler(
            "test_stream".to_string(),
            Arc::new(move |_data| {
                called_clone.store(true, std::sync::atomic::Ordering::SeqCst);
            }),
        );

        system.dispatch_stream("test_stream", Bytes::from("test"));
        assert!(called.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn test_media_handler_registration() {
        let system = DomSystem::new();
        let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_clone = Arc::clone(&called);

        system.register_media_handler(
            "test_track".to_string(),
            Arc::new(move |_frame| {
                called_clone.store(true, std::sync::atomic::Ordering::SeqCst);
            }),
        );

        system.dispatch_media_frame("test_track", Bytes::from("test"));
        assert!(called.load(std::sync::atomic::Ordering::SeqCst));
    }
}
