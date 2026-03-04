//! Service Worker Keepalive 机制
//!
//! 用于定期向 Service Worker 发送保活消息，防止浏览器回收 Service Worker。

use crate::transport::DataLane;
use bytes::Bytes;
use parking_lot::Mutex;
use std::sync::Arc;

/// Service Worker Keepalive 机制
///
/// 只要 DOM 还有活动，就应该保持 Service Worker 活跃。
pub struct ServiceWorkerKeepalive {
    lane: Arc<DataLane>,
    interval_secs: u64,
    running: Arc<Mutex<bool>>,
}

impl ServiceWorkerKeepalive {
    /// 创建新的 Keepalive 实例
    ///
    /// # 参数
    /// - `lane`: PostMessage Lane（指向 Service Worker）
    /// - `interval_secs`: 保活消息发送间隔（秒），默认 20 秒
    pub fn new(lane: Arc<DataLane>, interval_secs: Option<u64>) -> Self {
        Self {
            lane,
            interval_secs: interval_secs.unwrap_or(20),
            running: Arc::new(Mutex::new(false)),
        }
    }

    /// 启动保活机制
    ///
    /// 每隔 `interval_secs` 秒发送一次保活消息到 Service Worker。
    pub fn start(&self) {
        let mut running = self.running.lock();
        if *running {
            log::warn!("ServiceWorkerKeepalive 已经在运行");
            return;
        }
        *running = true;
        drop(running);

        let lane = self.lane.clone();
        let interval_ms = self.interval_secs * 1000;
        let running = self.running.clone();

        wasm_bindgen_futures::spawn_local(async move {
            log::info!(
                "ServiceWorkerKeepalive 已启动: 间隔 {} 秒",
                interval_ms / 1000
            );

            loop {
                // 检查是否应该停止
                if !*running.lock() {
                    log::info!("ServiceWorkerKeepalive 已停止");
                    break;
                }

                // 等待指定间隔
                wasm_bindgen_futures::JsFuture::from(js_sys::Promise::new(&mut |resolve, _| {
                    let window = web_sys::window().unwrap();
                    window
                        .set_timeout_with_callback_and_timeout_and_arguments_0(
                            &resolve,
                            interval_ms as i32,
                        )
                        .unwrap();
                }))
                .await
                .unwrap();

                // 发送保活消息
                let keepalive_msg = Bytes::from_static(b"KEEPALIVE");
                match lane.send(keepalive_msg).await {
                    Ok(_) => {
                        log::trace!("ServiceWorkerKeepalive: 发送保活消息");
                    }
                    Err(e) => {
                        log::error!("ServiceWorkerKeepalive: 发送保活消息失败: {:?}", e);
                    }
                }
            }
        });
    }

    /// 停止保活机制
    pub fn stop(&self) {
        let mut running = self.running.lock();
        *running = false;
        log::info!("ServiceWorkerKeepalive: 请求停止");
    }

    /// 检查保活机制是否正在运行
    pub fn is_running(&self) -> bool {
        *self.running.lock()
    }
}
