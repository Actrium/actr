# Fast Path Demo - 高性能数据流示例

> **概念说明 / 非可运行示例**：原 `examples/fastpath-demo/` 只有 README 型材料，已移入 docs。本文保留为 Fast Path 数据流概念说明，不代表仓库里存在可直接访问的 demo URL。当前代码锚点是 `bindings/web/packages/actr-dom/src/fast-path-forwarder.ts` 的 `FastPathForwarder`、`bindings/web/packages/web-sdk/src/actor.sw.js` 的 `fast_path_data` handler，以及 `sw-host` / `dom-bridge` 的 lane 实现。

说明 Actor-RTC Web 的 Fast Path 数据流处理模型。

## 功能特性

### 1. 零拷贝数据传输
- **DOM → Service Worker**: Transferable ArrayBuffer
- **WASM 内部**: Bytes (Arc<Vec<u8>>)
- **单次拷贝**: 仅在 JS heap → WASM linear memory

### 2. 静态分发
- 枚举而非 dyn Trait
- 编译器完全内联
- 零虚函数调用开销

### 3. 性能目标
- **延迟**: 6-13ms (DOM → WASM → Handler)
- **吞吐**: 60fps 视频流 (~16ms/frame)
- **零分配**: 静态路由表 + DashMap 无锁访问

## 架构

```text
┌─────────────────────────────────────────────────────────────┐
│ DOM (JS)                                                     │
│ - 创建 Uint8Array                                            │
│ - 使用 Transferable 发送 ArrayBuffer                         │
└─────────────────────┬───────────────────────────────────────┘
                      │ PostMessage (零拷贝)
                      ↓
┌─────────────────────────────────────────────────────────────┐
│ Service Worker (WASM)                                        │
│ ┌───────────────────────────────────────────────────────┐   │
│ │ FastPathData                                          │   │
│ │ - 接收 Uint8Array (JS 引用)                           │   │
│ │ - to_chunk() 提取到 WASM 内存 (唯一拷贝点)           │   │
│ └───────────────┬───────────────────────────────────────┘   │
│                 │                                            │
│                 ↓                                            │
│ ┌───────────────────────────────────────────────────────┐   │
│ │ FastPathRegistry                                      │   │
│ │ - DashMap 无锁查找                                    │   │
│ │ - dispatch() 静态分发                                 │   │
│ └───────────────┬───────────────────────────────────────┘   │
│                 │                                            │
│                 ↓                                            │
│ ┌───────────────────────────────────────────────────────┐   │
│ │ Handler (静态函数指针)                                │   │
│ │ - 处理 StreamChunk (Bytes 零拷贝共享)                 │   │
│ │ - 编译器完全内联                                      │   │
│ └───────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

## 代码示例

### DOM 端（JS）

```javascript
// 1. 创建数据（模拟视频帧）
const videoFrame = new Uint8Array(1920 * 1080 * 3); // RGB 数据
for (let i = 0; i < videoFrame.length; i++) {
  videoFrame[i] = Math.random() * 255;
}

// 2. 发送到 Service Worker（Transferable 零拷贝）
serviceWorkerPort.postMessage(
  {
    type: 'fast_path_data',
    payload: {
      streamId: 'peer:123:video',
      data: videoFrame,
      timestamp: Date.now()
    }
  },
  [videoFrame.buffer]  // Transferable！所有权转移，零拷贝
);

// 注意：发送后 videoFrame.buffer 被清空（detached）
console.log(videoFrame.byteLength); // 0
```

### Service Worker 端（WASM）

```rust
// worker.js - 接收消息
self.addEventListener('message', async (event) => {
  if (event.data.type === 'fast_path_data') {
    const { streamId, data, timestamp } = event.data.payload;

    // 当前 SW 入口按 fast_path_data 消息分支处理 payload；
    // Rust 侧仍会在 JS heap -> WASM linear memory 边界发生必要拷贝。
    wasm_bindgen.handle_fast_path(streamId, data, timestamp);
  }
});

// 注册 Fast Path 处理器（Rust）
fn video_frame_handler(frame: MediaFrame) {
    // 处理视频帧（零拷贝访问数据）
    let frame_data = frame.data; // Bytes (Arc)

    // 多个处理器可以共享同一数据（零拷贝）
    let decoder = frame_data.clone(); // 仅增加引用计数
    let logger = frame_data.clone();

    // 处理...
    web_sys::console::log_1(
        &format!("Received {} bytes video frame", frame_data.len()).into()
    );
}

// 注册到 ActorSystem
actorSystem.register_fastpath_media(
    "peer:123:video".to_string(),
    MediaFrameType::Video,
    video_frame_handler
);
```

## 性能优化要点

### 1. 零拷贝链路

```text
DOM (ArrayBuffer)
  ↓ Transferable (零拷贝，所有权转移)
Service Worker (ArrayBuffer)
  ↓ Uint8Array 视图 (零拷贝)
WASM (to_vec()) ← 唯一拷贝点（JS heap → WASM linear memory）
  ↓ Bytes::from(Vec) (零拷贝，仅包装)
WASM 内部传递 (Bytes clone) ← Arc 引用计数，零拷贝
  ↓ handler.data.clone() (零拷贝)
多个处理器共享数据 (Arc<Vec<u8>>)
```

### 2. 静态分发优势

```rust
// ❌ 动态分发（虚函数调用）
trait Handler {
    fn handle(&self, chunk: StreamChunk);
}

let handler: Box<dyn Handler> = ...; // 虚函数表查找
handler.handle(chunk); // 无法内联

// ✅ 静态分发（枚举 + match）
enum HandlerType {
    Video(Arc<VideoHandler>),
    Audio(Arc<AudioHandler>),
}

match handler_type {
    HandlerType::Video(h) => h.handle(chunk), // 编译器完全内联
    HandlerType::Audio(h) => h.handle(chunk),
}
```

### 3. DashMap 无锁性能

```rust
// ❌ RwLock + HashMap
let handlers: RwLock<HashMap<String, Handler>> = ...;
let guard = handlers.read().await; // 锁等待
let handler = guard.get(stream_id); // 查找

// ✅ DashMap（无锁并发读）
let handlers: DashMap<String, Handler> = ...;
let handler = handlers.get(stream_id); // 无锁！性能接近 HashMap
```

## 性能测试

### 延迟测试

```javascript
// DOM 端发送
const start = performance.now();
const latencyPayload = new Uint8Array([1, 2, 3]);
serviceWorkerPort.postMessage({
  type: 'fast_path_data',
  payload: {
    streamId: 'test:latency',
    data: latencyPayload,
    timestamp: start
  }
}, [latencyPayload.buffer]);

// WASM 端接收并回应
fn latency_handler(chunk: StreamChunk) {
    let dom_timestamp = chunk.timestamp as f64 / 1000.0;
    let now = js_sys::Date::now();
    let latency = now - dom_timestamp;
    web_sys::console::log_1(&format!("Latency: {:.2}ms", latency).into());
}
```

### 吞吐测试

```javascript
// 模拟 60fps 视频流
let frameCount = 0;
const fps = 60;
const interval = 1000 / fps;

setInterval(() => {
  const frame = new Uint8Array(1920 * 1080 * 3);
  serviceWorkerPort.postMessage({
    type: 'fast_path_data',
    payload: {
      streamId: 'test:throughput',
      data: frame,
      timestamp: Date.now()
    }
  }, [frame.buffer]);

  frameCount++;
  if (frameCount % 60 === 0) {
    console.log(`Sent ${frameCount} frames`);
  }
}, interval);
```

## 与 @actr Native 对比

| 特性 | Native (Rust) | Web (WASM) |
|------|---------------|------------|
| 零拷贝方式 | 共享内存 + Arc | Transferable + Bytes |
| 分发机制 | 枚举 + match | 枚举 + match |
| 并发访问 | DashMap | DashMap |
| 延迟目标 | 1-3ms | 6-13ms |
| 瓶颈 | 网络传输 | PostMessage + WASM边界 |

## 运行步骤

1. **运行当前仓库示例**
   当前没有 `examples/fastpath-demo/` 可运行目录。需要验证 Fast Path 行为时，请从现有 Web 示例和 `FastPathForwarder.forward()` / `forwardBatch()` 的调用链入手。

## 相关文档

- 当前实现：`bindings/web/packages/actr-dom/src/fast-path-forwarder.ts`
- SW 入口：`bindings/web/packages/web-sdk/src/actor.sw.js`
- Rust lane：`bindings/web/crates/sw-host/src/transport/lane.rs`、`bindings/web/crates/dom-bridge/src/transport/lane.rs`
