# Web 环境生命周期恢复机制设计

**日期**: 2026-01-08
**状态**: 部分实现（DomLifecycleManager 已实现 DOM 侧生命周期管理）
**优先级**: P0（阻塞生产使用）

---

## 一、背景

Web 环境的特殊性：
- **DOM 可能随时重启**（页面刷新、标签页切换）
- **SW 可能被浏览器回收**（资源不足、长时间空闲）
- **MessagePort 会失效**（DOM 重启时）
- **WebRTC 连接会断开**（DOM 重启时）
- **Registry 会清空**（DOM 进程销毁时）

**当前状态**：DOM 侧已部分实现（~40%）。`DomLifecycleManager`（`crates/runtime-dom/src/lifecycle.rs`，277 行）已实现 DOM_READY 事件、beforeunload 监听、visibility change 监听和 SW 健康检查。SW 侧恢复机制（SW 唤醒后重建连接、Registry 恢复等）尚未实现。

---

## 二、核心场景

### 场景 1：页面刷新

```
用户按 F5 或 Ctrl+R 刷新页面

1. 浏览器行为：
   - DOM 进程销毁
   - SW 继续运行（不受影响）
   - 所有 WebRTC 连接断开
   - MessagePort 对象失效

2. 当前问题：
   - SW 不知道 DOM 已重启
   - WirePool 仍保留失效的 WebRTC 连接
   - 尝试发送消息时 postMessage() 报错
   - Registry 清空，Stream/Media 处理器丢失

3. 期望行为：
   - DOM 重启后主动通知 SW："我回来了"
   - SW 清理旧的 WebRTC 连接状态
   - DOM 重新建立 WebRTC 连接
   - 用户重新注册 Stream/Media 处理器
```

### 场景 2：关闭标签页再打开

```
用户关闭标签页，稍后重新打开

1. 浏览器行为：
   - DOM 完全销毁
   - SW 可能继续运行（后台活跃）
   - 重新打开时，DOM 是全新的进程

2. 当前问题：
   - 同场景 1

3. 期望行为：
   - 同场景 1
```

### 场景 3：SW 被浏览器回收

```
SW 长时间空闲，被浏览器终止

1. 浏览器行为：
   - SW 进程销毁
   - 下次访问时，SW 重新启动
   - 所有内存状态丢失（WirePool、TransportManager 等）

2. 当前问题：
   - Mailbox 在 IndexedDB（持久化）✅
   - 但 WirePool、连接状态等都在内存中 ❌
   - 需要重新建立所有连接

3. 期望行为：
   - SW 重启后，从 IndexedDB 恢复关键状态
   - DOM 检测到 SW 重启，重新建立通信
```

---

## 三、详细设计

### 3.1 DOM 重启检测机制

#### 方案：DOM 主动通知 + SW 被动检测

```rust
// ========== DOM 侧 ==========
// crates/runtime-dom/src/lifecycle.rs (新建)

use web_sys::{window, MessageEvent, Navigator};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

pub struct DomLifecycleManager {
    session_id: String,  // 本次 DOM 会话 ID
}

impl DomLifecycleManager {
    pub fn new() -> Self {
        let session_id = generate_session_id();  // UUID
        Self { session_id }
    }

    /// 初始化生命周期管理
    pub fn init(&self) -> WebResult<()> {
        // 1. 监听页面加载完成
        self.setup_load_listener()?;

        // 2. 监听页面即将卸载
        self.setup_beforeunload_listener()?;

        // 3. 监听可见性变化
        self.setup_visibility_listener()?;

        Ok(())
    }

    /// 页面加载完成时
    fn setup_load_listener(&self) -> WebResult<()> {
        let window = window().unwrap();
        let session_id = self.session_id.clone();

        let callback = Closure::wrap(Box::new(move || {
            log::info!("[DomLifecycle] Page loaded, session_id={}", session_id);

            // 通知 SW："我启动了"
            if let Some(controller) = navigator()
                .service_worker()
                .controller()
            {
                let msg = js_sys::Object::new();
                js_sys::Reflect::set(&msg, &"type".into(), &"DOM_READY".into()).unwrap();
                js_sys::Reflect::set(&msg, &"session_id".into(), &session_id.clone().into()).unwrap();

                controller.post_message(&msg).ok();
            }
        }) as Box<dyn FnMut()>);

        window.add_event_listener_with_callback(
            "load",
            callback.as_ref().unchecked_ref()
        )?;
        callback.forget();

        Ok(())
    }

    /// 页面即将卸载时
    fn setup_beforeunload_listener(&self) -> WebResult<()> {
        let window = window().unwrap();
        let session_id = self.session_id.clone();

        let callback = Closure::wrap(Box::new(move |_event: web_sys::BeforeUnloadEvent| {
            log::info!("[DomLifecycle] Page unloading, session_id={}", session_id);

            // 通知 SW："我要关闭了"
            if let Some(controller) = navigator()
                .service_worker()
                .controller()
            {
                let msg = js_sys::Object::new();
                js_sys::Reflect::set(&msg, &"type".into(), &"DOM_UNLOADING".into()).unwrap();
                js_sys::Reflect::set(&msg, &"session_id".into(), &session_id.clone().into()).unwrap();

                // 使用 sendBeacon (更可靠)
                navigator().send_beacon_with_str(
                    &format!("/api/lifecycle/unload?session={}", session_id),
                    ""
                ).ok();

                controller.post_message(&msg).ok();
            }
        }) as Box<dyn FnMut(web_sys::BeforeUnloadEvent)>);

        window.add_event_listener_with_callback(
            "beforeunload",
            callback.as_ref().unchecked_ref()
        )?;
        callback.forget();

        Ok(())
    }

    /// 可见性变化时（标签页切换）
    fn setup_visibility_listener(&self) -> WebResult<()> {
        let document = window().unwrap().document().unwrap();
        let session_id = self.session_id.clone();

        let callback = Closure::wrap(Box::new(move || {
            let document = window().unwrap().document().unwrap();
            let hidden = document.hidden();

            log::debug!("[DomLifecycle] Visibility changed: hidden={}", hidden);

            if !hidden {
                // 标签页变为可见，检查 SW 是否还活着
                check_sw_alive();
            }
        }) as Box<dyn FnMut()>);

        document.add_event_listener_with_callback(
            "visibilitychange",
            callback.as_ref().unchecked_ref()
        )?;
        callback.forget();

        Ok(())
    }
}

/// 生成会话 ID
fn generate_session_id() -> String {
    use js_sys::Math;
    format!("dom-{}-{}",
        js_sys::Date::now() as u64,
        (Math::random() * 1_000_000.0) as u64
    )
}

/// 检查 SW 是否活跃
fn check_sw_alive() {
    // 发送 ping，期望收到 pong
    // 如果超时，说明 SW 可能被回收，需要重新注册
}

// ========== SW 侧 ==========
// crates/runtime-sw/src/lifecycle.rs (新建)

use wasm_bindgen::prelude::*;
use web_sys::ExtendableMessageEvent;
use std::sync::Arc;
use parking_lot::Mutex;

pub struct SwLifecycleManager {
    /// 当前活跃的 DOM 会话
    active_sessions: Arc<Mutex<HashSet<String>>>,
}

impl SwLifecycleManager {
    pub fn new() -> Self {
        Self {
            active_sessions: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// 监听 message 事件
    pub fn setup_message_listener(&self) -> WebResult<()> {
        let active_sessions = Arc::clone(&self.active_sessions);

        // 在 SW 的 global scope 上监听
        let callback = Closure::wrap(Box::new(move |event: ExtendableMessageEvent| {
            if let Ok(data) = event.data().dyn_into::<js_sys::Object>() {
                if let Ok(msg_type) = js_sys::Reflect::get(&data, &"type".into()) {
                    let msg_type = msg_type.as_string().unwrap_or_default();

                    match msg_type.as_str() {
                        "DOM_READY" => {
                            // DOM 重启了
                            if let Ok(session_id) = js_sys::Reflect::get(&data, &"session_id".into()) {
                                let session_id = session_id.as_string().unwrap();
                                log::info!("[SwLifecycle] DOM ready: {}", session_id);

                                active_sessions.lock().insert(session_id.clone());

                                // 清理旧的 WebRTC 连接（重要！）
                                Self::cleanup_stale_webrtc_connections(&session_id);
                            }
                        }

                        "DOM_UNLOADING" => {
                            // DOM 即将关闭
                            if let Ok(session_id) = js_sys::Reflect::get(&data, &"session_id".into()) {
                                let session_id = session_id.as_string().unwrap();
                                log::info!("[SwLifecycle] DOM unloading: {}", session_id);

                                active_sessions.lock().remove(&session_id);

                                // 标记该会话的 WebRTC 连接为待清理
                                Self::mark_webrtc_for_cleanup(&session_id);
                            }
                        }

                        _ => {}
                    }
                }
            }
        }) as Box<dyn FnMut(ExtendableMessageEvent)>);

        // 注册到 SW 的 message 事件
        js_sys::Reflect::get(&js_sys::global(), &"self".into())
            .unwrap()
            .dyn_into::<web_sys::ServiceWorkerGlobalScope>()
            .unwrap()
            .add_event_listener_with_callback(
                "message",
                callback.as_ref().unchecked_ref()
            )?;

        callback.forget();
        Ok(())
    }

    /// 清理失效的 WebRTC 连接
    fn cleanup_stale_webrtc_connections(session_id: &str) {
        log::info!("[SwLifecycle] Cleaning up stale WebRTC for session: {}", session_id);

        // TODO: 通知 WirePool 清理失效连接
        // wire_pool.remove_stale_connections();
    }

    /// 标记 WebRTC 连接为待清理
    fn mark_webrtc_for_cleanup(session_id: &str) {
        log::info!("[SwLifecycle] Marking WebRTC for cleanup: {}", session_id);
        // TODO: 设置延迟清理（给 beforeunload 一些时间）
    }
}
```

---

### 3.2 MessagePort 失效检测

#### 方案：发送时捕获错误 + 心跳检测

```rust
// crates/runtime-sw/src/transport/lane.rs

impl DataLane {
    /// 发送消息（增强版，带失效检测）
    pub async fn send(&self, data: Bytes) -> LaneResult<()> {
        match self {
            DataLane::PostMessage { port, payload_type, .. } => {
                let mut msg = Vec::with_capacity(5 + data.len());
                msg.push(*payload_type as u8);
                msg.extend_from_slice(&(data.len() as u32).to_be_bytes());
                msg.extend_from_slice(&data);

                let js_array = js_sys::Uint8Array::from(&msg[..]);

                // 尝试发送
                match port.post_message(&js_array.into()) {
                    Ok(_) => {
                        log::trace!("PostMessage sent: {} bytes", data.len());
                        Ok(())
                    }
                    Err(e) => {
                        // ❌ MessagePort 失效了！
                        log::error!("PostMessage failed (port dead?): {:?}", e);

                        // 🔥 关键：通知连接失效
                        if let Some(failure_notifier) = &self.failure_notifier {
                            failure_notifier.notify_port_failed();
                        }

                        Err(WebError::Transport(format!("MessagePort failed: {:?}", e)))
                    }
                }
            }
            // ... WebSocket 同理
        }
    }
}

// 失效通知器
pub struct PortFailureNotifier {
    wire_pool: Arc<WirePool>,
    conn_type: ConnType,
}

impl PortFailureNotifier {
    pub fn notify_port_failed(&self) {
        log::warn!("[PortFailureNotifier] Notifying port failure: {:?}", self.conn_type);
        self.wire_pool.mark_connection_failed(self.conn_type);
    }
}
```

---

### 3.3 WirePool 连接清理与重建

#### 新增方法：移除失效连接

```rust
// crates/runtime-sw/src/transport/wire_pool.rs

impl WirePool {
    /// 标记连接为失效
    pub fn mark_connection_failed(&self, conn_type: ConnType) {
        let mut conns = self.connections.lock();
        conns[conn_type.as_index()] = Some(WireStatus::Failed);

        log::warn!("[WirePool] Connection marked as failed: {:?}", conn_type);

        // 从就绪集合移除
        {
            let mut ready = self.ready_set.lock();
            ready.remove(&conn_type);
        }

        // 通知等待者（让他们知道状态变化了）
        Self::notify_all(&self.change_notifiers);
    }

    /// 移除失效连接（为重建做准备）
    pub fn remove_connection(&self, conn_type: ConnType) {
        let mut conns = self.connections.lock();
        conns[conn_type.as_index()] = None;

        log::info!("[WirePool] Connection removed: {:?}", conn_type);

        // 从就绪集合移除
        {
            let mut ready = self.ready_set.lock();
            ready.remove(&conn_type);
        }
    }

    /// 重新添加连接（用于恢复）
    pub fn reconnect(&self, connection: WireHandle) {
        let conn_type = connection.conn_type();

        log::info!("[WirePool] Reconnecting: {:?}", conn_type);

        // 先移除旧的
        self.remove_connection(conn_type);

        // 再添加新的
        self.add_connection(connection);
    }

    /// 检查连接是否存活
    pub async fn health_check(&self) -> HashMap<ConnType, bool> {
        let mut results = HashMap::new();

        for conn_type in [ConnType::WebSocket, ConnType::WebRTC] {
            if let Some(handle) = self.get_connection(conn_type).await {
                let alive = handle.is_connected();
                results.insert(conn_type, alive);
            } else {
                results.insert(conn_type, false);
            }
        }

        results
    }
}
```

---

### 3.4 WebRTC 重建流程

#### 完整流程设计

```
1. DOM 重启检测到
   ↓
2. DOM → SW: "DOM_READY" message
   ↓
3. SW 清理旧 WebRTC 连接
   ↓
4. SW → DOM: "REQUEST_WEBRTC_REBUILD" message
   ↓
5. DOM 创建新的 PeerConnection
   ↓
6. DOM 建立 DataChannel
   ↓
7. DOM 创建 MessageChannel
   ↓
8. DOM Transfer MessagePort → SW
   ↓
9. SW 接收 MessagePort，创建新 WireHandle
   ↓
10. SW 添加到 WirePool
   ↓
11. 完成恢复
```

#### 代码实现

```rust
// crates/runtime-sw/src/webrtc_recovery.rs (新建)

pub struct WebRtcRecoveryManager {
    wire_pool: Arc<WirePool>,
    transport_manager: Arc<PeerTransport>,
}

impl WebRtcRecoveryManager {
    /// 处理 DOM 重启事件
    pub async fn handle_dom_restart(&self, session_id: String) -> WebResult<()> {
        log::info!("[WebRtcRecovery] Handling DOM restart: {}", session_id);

        // 1. 清理所有 WebRTC 连接
        self.wire_pool.remove_connection(ConnType::WebRTC);

        // 2. 请求 DOM 重新建立 WebRTC
        self.request_webrtc_rebuild().await?;

        Ok(())
    }

    /// 请求 DOM 重建 WebRTC
    async fn request_webrtc_rebuild(&self) -> WebResult<()> {
        // 通过 DOM lane 发送重建请求
        // TODO: 需要一个专门的控制通道
        log::info!("[WebRtcRecovery] Requesting WebRTC rebuild from DOM");
        Ok(())
    }

    /// 接收新的 MessagePort（DOM 重建完成后）
    pub fn register_new_port(&self, peer_id: String, port: web_sys::MessagePort) {
        log::info!("[WebRtcRecovery] Received new MessagePort for: {}", peer_id);

        // 创建新的 WebRTC 连接
        let mut rtc_conn = WebRtcConnection::new(peer_id);
        rtc_conn.set_datachannel_port(port);

        // 添加到 WirePool
        self.wire_pool.reconnect(WireHandle::WebRTC(rtc_conn));

        log::info!("[WebRtcRecovery] WebRTC connection rebuilt successfully");
    }
}
```

---

### 3.5 Registry 重建机制

#### 方案：用户手动重注册 + 可选持久化

```rust
// crates/runtime-dom/src/fastpath.rs

// 增强 Registry，支持状态通知
pub struct StreamHandlerRegistry {
    handlers: DashMap<String, StreamCallback>,

    // 新增：注册变更通知
    on_cleared: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl StreamHandlerRegistry {
    /// 设置清空回调（用于通知用户需要重新注册）
    pub fn on_cleared<F>(&mut self, callback: F)
    where
        F: Fn() + Send + Sync + 'static
    {
        self.on_cleared = Some(Arc::new(callback));
    }

    /// 清空所有处理器（DOM 重启时调用）
    pub fn clear_all(&self) {
        self.handlers.clear();
        log::warn!("[StreamRegistry] All handlers cleared");

        // 通知用户
        if let Some(callback) = &self.on_cleared {
            callback();
        }
    }

    /// 导出当前注册信息（用于持久化，可选）
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

// 用户代码示例
// examples/stream/src/main.rs

use actr_runtime_dom::{DomSystem, StreamCallback};

#[wasm_bindgen(start)]
pub async fn start() {
    let system = DomSystem::new();

    // 设置清空回调
    system.stream_registry().on_cleared(|| {
        log::warn!("⚠️ Registry cleared! Please re-register your handlers.");
        // 可选：自动重新注册
        re_register_handlers(&system);
    });

    // 初始注册
    register_handlers(&system);

    // 监听 DOM 生命周期
    setup_lifecycle_listener(&system);
}

fn register_handlers(system: &DomSystem) {
    system.register_stream_handler(
        "video-1".to_string(),
        Arc::new(|data| {
            log::info!("Received video data: {} bytes", data.len());
        })
    );

    log::info!("Stream handlers registered: {}", system.stream_registry().count());
}

fn re_register_handlers(system: &DomSystem) {
    log::info!("Re-registering stream handlers after DOM restart...");
    register_handlers(system);
}

fn setup_lifecycle_listener(system: &DomSystem) {
    // 监听页面加载
    window().add_event_listener_with_callback("load", /* ... */);

    // 在 load 事件中检查 Registry 是否为空
    // 如果为空，自动重新注册
}
```

---

## 四、实现计划

### Phase 1: 基础生命周期（P0）

**文件**：
- `crates/runtime-dom/src/lifecycle.rs` (新建)
- `crates/runtime-sw/src/lifecycle.rs` (新建)

**功能**：
- ✅ DOM 加载时发送 "DOM_READY"
- ✅ DOM 卸载时发送 "DOM_UNLOADING"
- ✅ SW 监听这些事件
- ✅ 页面刷新后能检测到 DOM 重启

**工作量**：~200 行代码，2-3 天

---

### Phase 2: MessagePort 失效检测（P0）

**文件**：
- `crates/runtime-sw/src/transport/lane.rs` (修改)
- `crates/runtime-sw/src/transport/wire_pool.rs` (扩展)

**功能**：
- ✅ `port.post_message()` 失败时通知 WirePool
- ✅ WirePool 标记连接为 Failed
- ✅ 提供 `mark_connection_failed()` API

**工作量**：~150 行代码，1-2 天

---

### Phase 3: WebRTC 重建流程（P0）

**文件**：
- `crates/runtime-sw/src/webrtc_recovery.rs` (新建)
- `crates/runtime-sw/src/transport/wire_pool.rs` (扩展)
- `crates/runtime-dom/src/webrtc/coordinator.rs` (修改)

**功能**：
- ✅ DOM 重启后，清理旧 WebRTC 连接
- ✅ DOM 重新创建 PeerConnection
- ✅ 新 MessagePort 传递给 SW
- ✅ SW 重新添加 WebRTC 到 WirePool

**工作量**：~300 行代码，3-4 天

---

### Phase 4: Registry 重建提示（P1）

**文件**：
- `crates/runtime-dom/src/fastpath.rs` (修改)

**功能**：
- ✅ Registry 提供清空回调
- ✅ 用户可监听并重新注册
- ⚠️ 可选：持久化到 localStorage

**工作量**：~100 行代码，1 天

---

### Phase 5: 健康检查（P1）

**文件**：
- `crates/runtime-sw/src/transport/wire_pool.rs` (扩展)

**功能**：
- ✅ 定期检查连接存活
- ✅ 提供 `health_check()` API
- ⚠️ 可选：自动重连

**工作量**：~100 行代码，1 天

---

## 五、测试计划

### 测试用例 1：页面刷新

```
前置条件：
- SW 已启动
- WebRTC 连接已建立
- Registry 已注册 handlers

操作：
1. 用户按 F5 刷新页面

期望结果：
1. DOM 发送 "DOM_READY" 到 SW ✅
2. SW 清理旧 WebRTC 连接 ✅
3. DOM 重新建立 WebRTC ✅
4. MessagePort 传递给 SW ✅
5. 用户收到重新注册提示 ✅
6. 用户重新注册 handlers ✅
7. 系统恢复正常通信 ✅
```

### 测试用例 2：标签页切换

```
操作：
1. 切换到其他标签页（visibilitychange: hidden=true）
2. 等待 30 秒
3. 切回来（visibilitychange: hidden=false）

期望结果：
1. 连接保持活跃 ✅
2. 数据流正常 ✅
```

### 测试用例 3：MessagePort 失效

```
操作：
1. 模拟 MessagePort 失效（通过关闭 port）
2. 尝试发送消息

期望结果：
1. send() 返回错误 ✅
2. WirePool 标记连接为 Failed ✅
3. 日志输出警告 ✅
```

---

## 六、注意事项

### 6.1 Service Worker 的生命周期限制

- SW 可能在 30 秒后被浏览器终止（如果空闲）
- 使用 `event.waitUntil()` 延长生命周期
- 关键操作需要在 `activate` 事件中完成

### 6.2 MessagePort 的 Transferable 特性

- MessagePort 只能 transfer 一次
- Transfer 后，原始端失效
- 必须重新创建 MessageChannel

### 6.3 IndexedDB 的异步特性

- 不能在 `beforeunload` 中可靠地写入
- 建议定期持久化，而非关闭时才写入

### 6.4 用户体验

- 页面刷新后，明确提示用户"连接已重置，请重新订阅"
- 提供便捷的批量重新注册 API
- 可选：自动重新注册（保存注册信息）

---

## 七、后续优化

### 7.1 自动重连（可选）

```rust
// 自动重连策略
pub struct AutoReconnectStrategy {
    max_retries: u32,
    backoff: Duration,
}

impl AutoReconnectStrategy {
    pub async fn reconnect_with_backoff(&self) {
        for attempt in 1..=self.max_retries {
            log::info!("Reconnect attempt {}/{}", attempt, self.max_retries);

            if self.try_reconnect().await.is_ok() {
                log::info!("Reconnect successful");
                return;
            }

            sleep(self.backoff * attempt).await;
        }

        log::error!("Reconnect failed after {} attempts", self.max_retries);
    }
}
```

### 7.2 Registry 持久化（可选）

```rust
// 持久化到 localStorage
pub async fn save_registry_state(&self) -> WebResult<()> {
    let state = self.export_state();
    let json = serde_json::to_string(&state)?;

    window()
        .local_storage()?
        .unwrap()
        .set_item("actr_registry_state", &json)?;

    Ok(())
}

pub async fn restore_registry_state(&self) -> WebResult<Vec<String>> {
    let json = window()
        .local_storage()?
        .unwrap()
        .get_item("actr_registry_state")?
        .unwrap_or_default();

    let state: Vec<String> = serde_json::from_str(&json)?;
    Ok(state)
}
```

### 7.3 多标签页协调（高级）

- 使用 BroadcastChannel API
- 避免多个标签页重复建立连接
- 指定一个"主标签页"管理连接

---

## 八、总结

### 核心目标

让 actr-web 在 Web 环境下具备**生产级的健壮性**：

1. ✅ 页面刷新后自动恢复通信
2. ✅ MessagePort 失效能被检测和处理
3. ✅ WebRTC 连接能重建
4. ✅ 用户代码能感知生命周期变化

### 实现优先级

- **P0（必须）**: Phase 1-3（生命周期、失效检测、WebRTC 重建）
- **P1（重要）**: Phase 4-5（Registry 提示、健康检查）
- **P2（可选）**: 自动重连、持久化、多标签页协调

### 预估工作量

- **核心功能（P0+P1）**: ~850 行代码，7-10 天
- **测试和文档**: 2-3 天
- **总计**: 2 周左右

**当前状态**: 设计完成 ✅
**下一步**: 创建 TODO 并开始实现
