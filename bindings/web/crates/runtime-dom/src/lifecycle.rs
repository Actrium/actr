//! DOM 生命周期管理
//!
//! 负责检测 DOM 进程的启动、关闭和状态变化，并通知 Service Worker

use crate::{WebError, WebResult};
use parking_lot::Mutex;
use std::sync::Arc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{Navigator, ServiceWorker, window};

/// DOM 生命周期管理器
pub struct DomLifecycleManager {
    /// 当前 DOM 会话 ID
    session_id: String,

    /// 是否已初始化
    initialized: Arc<Mutex<bool>>,
}

impl DomLifecycleManager {
    /// 创建新的生命周期管理器
    pub fn new() -> Self {
        let session_id = generate_session_id();
        log::info!("[DomLifecycle] Created new session: {}", session_id);

        Self {
            session_id,
            initialized: Arc::new(Mutex::new(false)),
        }
    }

    /// 获取会话 ID
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// 初始化生命周期管理
    ///
    /// 设置所有事件监听器并通知 SW 当前状态
    pub fn init(&self) -> WebResult<()> {
        let mut initialized = self.initialized.lock();
        if *initialized {
            log::warn!("[DomLifecycle] Already initialized");
            return Ok(());
        }

        log::info!("[DomLifecycle] Initializing lifecycle management");

        // 1. 监听页面加载完成
        self.setup_load_listener()?;

        // 2. 监听页面即将卸载
        self.setup_beforeunload_listener()?;

        // 3. 监听可见性变化
        self.setup_visibility_listener()?;

        // 4. 立即通知 SW "DOM_READY"（页面已加载）
        self.notify_dom_ready()?;

        *initialized = true;
        log::info!("[DomLifecycle] Lifecycle management initialized");

        Ok(())
    }

    /// 通知 SW：DOM 已就绪
    fn notify_dom_ready(&self) -> WebResult<()> {
        log::info!("[DomLifecycle] Notifying SW: DOM_READY");

        let window = window().ok_or_else(|| WebError::Internal("No window".into()))?;
        let navigator = window.navigator();

        if let Some(controller) = get_sw_controller(&navigator) {
            let msg = create_lifecycle_message("DOM_READY", &self.session_id)?;
            controller
                .post_message(&msg)
                .map_err(|e| WebError::Internal(format!("Failed to post DOM_READY: {:?}", e)))?;

            log::info!("[DomLifecycle] DOM_READY sent successfully");
        } else {
            log::warn!("[DomLifecycle] No SW controller found, skipping DOM_READY");
        }

        Ok(())
    }

    /// 设置 load 事件监听器
    fn setup_load_listener(&self) -> WebResult<()> {
        let window = window().ok_or_else(|| WebError::Internal("No window".into()))?;
        let session_id = self.session_id.clone();

        let callback = Closure::wrap(Box::new(move || {
            log::info!("[DomLifecycle] Page loaded, session_id={}", session_id);

            // 页面加载时的通知已在 init() 中完成
            // 这里主要用于调试和日志记录
        }) as Box<dyn FnMut()>);

        window
            .add_event_listener_with_callback("load", callback.as_ref().unchecked_ref())
            .map_err(|e| WebError::Internal(format!("Failed to add load listener: {:?}", e)))?;

        callback.forget(); // 保持监听器活跃

        Ok(())
    }

    /// 设置 beforeunload 事件监听器
    fn setup_beforeunload_listener(&self) -> WebResult<()> {
        let win = window().ok_or_else(|| WebError::Internal("No window".into()))?;
        let session_id = self.session_id.clone();

        let callback = Closure::wrap(Box::new(move |_event: web_sys::BeforeUnloadEvent| {
            log::info!("[DomLifecycle] Page unloading, session_id={}", session_id);

            // 通知 SW："我要关闭了"
            if let Some(window_obj) = window() {
                let navigator = window_obj.navigator();

                if let Some(controller) = get_sw_controller(&navigator) {
                    if let Ok(msg) = create_lifecycle_message("DOM_UNLOADING", &session_id) {
                        // 尝试发送，但不保证成功（页面即将关闭）
                        let _ = controller.post_message(&msg);

                        log::info!("[DomLifecycle] DOM_UNLOADING sent");
                    }
                }

                // 也可以使用 sendBeacon（更可靠，但需要服务端支持）
                // navigator.send_beacon_with_str(
                //     &format!("/api/lifecycle/unload?session={}", session_id),
                //     ""
                // ).ok();
            }
        }) as Box<dyn FnMut(web_sys::BeforeUnloadEvent)>);

        win.add_event_listener_with_callback("beforeunload", callback.as_ref().unchecked_ref())
            .map_err(|e| {
                WebError::Internal(format!("Failed to add beforeunload listener: {:?}", e))
            })?;

        callback.forget();

        Ok(())
    }

    /// 设置 visibilitychange 事件监听器
    fn setup_visibility_listener(&self) -> WebResult<()> {
        let win = window().ok_or_else(|| WebError::Internal("No window".into()))?;
        let document = win
            .document()
            .ok_or_else(|| WebError::Internal("No document".into()))?;
        let session_id = self.session_id.clone();

        let callback = Closure::wrap(Box::new(move || {
            if let Some(window_obj) = window() {
                if let Some(doc) = window_obj.document() {
                    let hidden = doc.hidden();

                    log::debug!("[DomLifecycle] Visibility changed: hidden={}", hidden);

                    if !hidden {
                        // 标签页变为可见，检查 SW 是否还活着
                        log::debug!("[DomLifecycle] Tab became visible, checking SW health");
                        check_sw_alive(&session_id);
                    }
                }
            }
        }) as Box<dyn FnMut()>);

        document
            .add_event_listener_with_callback("visibilitychange", callback.as_ref().unchecked_ref())
            .map_err(|e| {
                WebError::Internal(format!("Failed to add visibilitychange listener: {:?}", e))
            })?;

        callback.forget();

        Ok(())
    }
}

impl Default for DomLifecycleManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 生成唯一的会话 ID
fn generate_session_id() -> String {
    use js_sys::{Date, Math};

    let timestamp = Date::now() as u64;
    let random = (Math::random() * 1_000_000.0) as u64;

    format!("dom-{}-{}", timestamp, random)
}

/// 获取 Service Worker 控制器
fn get_sw_controller(navigator: &Navigator) -> Option<ServiceWorker> {
    navigator.service_worker().controller()
}

/// 创建生命周期消息
fn create_lifecycle_message(msg_type: &str, session_id: &str) -> WebResult<JsValue> {
    let msg = js_sys::Object::new();

    js_sys::Reflect::set(&msg, &"type".into(), &msg_type.into())
        .map_err(|e| WebError::Internal(format!("Failed to set type: {:?}", e)))?;

    js_sys::Reflect::set(&msg, &"session_id".into(), &session_id.into())
        .map_err(|e| WebError::Internal(format!("Failed to set session_id: {:?}", e)))?;

    Ok(msg.into())
}

/// 检查 SW 是否活跃
///
/// 发送 ping 消息，期望收到 pong
fn check_sw_alive(session_id: &str) {
    if let Some(window) = window() {
        let navigator = window.navigator();

        if let Some(controller) = get_sw_controller(&navigator) {
            if let Ok(msg) = create_lifecycle_message("DOM_PING", session_id) {
                match controller.post_message(&msg) {
                    Ok(_) => {
                        log::debug!("[DomLifecycle] SW ping sent");
                    }
                    Err(e) => {
                        log::warn!(
                            "[DomLifecycle] SW ping failed: {:?}, may need re-registration",
                            e
                        );
                        // TODO: 触发 SW 重新注册流程
                    }
                }
            }
        } else {
            log::warn!("[DomLifecycle] No SW controller, may need re-registration");
            // TODO: 触发 SW 重新注册流程
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn test_generate_session_id() {
        let id1 = generate_session_id();
        let id2 = generate_session_id();

        // 确保生成不同的 ID
        assert_ne!(id1, id2);

        // 确保格式正确
        assert!(id1.starts_with("dom-"));
        assert!(id2.starts_with("dom-"));
    }

    #[wasm_bindgen_test]
    fn test_lifecycle_manager_creation() {
        let manager = DomLifecycleManager::new();

        // 确保有 session_id
        assert!(!manager.session_id().is_empty());
        assert!(manager.session_id().starts_with("dom-"));
    }
}
