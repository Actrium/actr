//! Fast Path Registry
//!
//! DOM 端的 Fast Path 注册表，用于管理流数据和媒体帧的快速处理回调。
//!
//! Fast Path 机制：
//! - Stream 数据（STREAM_*）和媒体帧（MEDIA_RTP）绕过 Mailbox
//! - 直接派发给预先注册的回调函数
//! - 在 I/O 线程并发执行，无需等待 Scheduler 调度
//! - 延迟 ~100µs（相比 State Path 的 ~10-20ms）

// TODO: Phase 2 实现

use bytes::Bytes;
use dashmap::DashMap;
use std::sync::Arc;

/// 流处理回调类型
pub type StreamCallback = Arc<dyn Fn(Bytes) + Send + Sync>;

/// 媒体帧处理回调类型
pub type MediaFrameCallback = Arc<dyn Fn(Bytes) + Send + Sync>;

/// 流处理器注册表
///
/// 管理 STREAM_* 类型的快速处理回调
pub struct StreamHandlerRegistry {
    handlers: DashMap<String, StreamCallback>,

    /// 清空回调（DOM 重启时调用）
    on_cleared: parking_lot::Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl StreamHandlerRegistry {
    /// 创建新的注册表
    pub fn new() -> Self {
        Self {
            handlers: DashMap::new(),
            on_cleared: parking_lot::Mutex::new(None),
        }
    }

    /// 注册流处理回调
    ///
    /// # 参数
    /// - `stream_id`: 流 ID
    /// - `callback`: 处理回调函数
    pub fn register(&self, stream_id: String, callback: StreamCallback) {
        self.handlers.insert(stream_id.clone(), callback);
        log::debug!("Stream handler registered: stream_id={}", stream_id);
    }

    /// 注销流处理回调
    pub fn unregister(&self, stream_id: &str) {
        self.handlers.remove(stream_id);
        log::debug!("Stream handler unregistered: stream_id={}", stream_id);
    }

    /// 派发流数据到回调
    pub fn dispatch(&self, stream_id: &str, data: Bytes) {
        if let Some(handler) = self.handlers.get(stream_id) {
            (handler.value())(data);
        } else {
            log::warn!("No handler found for stream_id={}", stream_id);
        }
    }

    /// 设置清空回调
    ///
    /// 当 Registry 被清空时（DOM 重启），会调用此回调通知用户
    pub fn on_cleared<F>(&self, callback: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        let mut on_cleared = self.on_cleared.lock();
        *on_cleared = Some(Arc::new(callback));
        log::debug!("Stream registry on_cleared callback registered");
    }

    /// 清空所有处理器
    ///
    /// 在 DOM 重启时调用，清空所有注册的回调
    pub fn clear_all(&self) {
        let count = self.handlers.len();
        self.handlers.clear();

        log::warn!("[StreamRegistry] All handlers cleared (count={})", count);

        // 通知用户
        if let Some(callback) = self.on_cleared.lock().as_ref() {
            callback();
        }
    }

    /// 导出当前注册状态
    ///
    /// 返回所有已注册的 stream_id
    pub fn export_state(&self) -> Vec<String> {
        self.handlers
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// 获取注册数量
    pub fn count(&self) -> usize {
        self.handlers.len()
    }
}

impl Default for StreamHandlerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// 媒体帧处理器注册表
///
/// 管理 MEDIA_RTP 类型的快速处理回调
pub struct MediaFrameHandlerRegistry {
    handlers: DashMap<String, MediaFrameCallback>,

    /// 清空回调（DOM 重启时调用）
    on_cleared: parking_lot::Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl MediaFrameHandlerRegistry {
    /// 创建新的注册表
    pub fn new() -> Self {
        Self {
            handlers: DashMap::new(),
            on_cleared: parking_lot::Mutex::new(None),
        }
    }

    /// 注册媒体帧处理回调
    ///
    /// # 参数
    /// - `track_id`: Track ID
    /// - `callback`: 处理回调函数
    pub fn register(&self, track_id: String, callback: MediaFrameCallback) {
        self.handlers.insert(track_id.clone(), callback);
        log::debug!("Media frame handler registered: track_id={}", track_id);
    }

    /// 注销媒体帧处理回调
    pub fn unregister(&self, track_id: &str) {
        self.handlers.remove(track_id);
        log::debug!("Media frame handler unregistered: track_id={}", track_id);
    }

    /// 派发媒体帧到回调
    pub fn dispatch(&self, track_id: &str, frame: Bytes) {
        if let Some(handler) = self.handlers.get(track_id) {
            (handler.value())(frame);
        } else {
            log::warn!("No handler found for track_id={}", track_id);
        }
    }

    /// 设置清空回调
    ///
    /// 当 Registry 被清空时（DOM 重启），会调用此回调通知用户
    pub fn on_cleared<F>(&self, callback: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        let mut on_cleared = self.on_cleared.lock();
        *on_cleared = Some(Arc::new(callback));
        log::debug!("Media registry on_cleared callback registered");
    }

    /// 清空所有处理器
    ///
    /// 在 DOM 重启时调用，清空所有注册的回调
    pub fn clear_all(&self) {
        let count = self.handlers.len();
        self.handlers.clear();

        log::warn!("[MediaRegistry] All handlers cleared (count={})", count);

        // 通知用户
        if let Some(callback) = self.on_cleared.lock().as_ref() {
            callback();
        }
    }

    /// 导出当前注册状态
    ///
    /// 返回所有已注册的 track_id
    pub fn export_state(&self) -> Vec<String> {
        self.handlers
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// 获取注册数量
    pub fn count(&self) -> usize {
        self.handlers.len()
    }
}

impl Default for MediaFrameHandlerRegistry {
    fn default() -> Self {
        Self::new()
    }
}
