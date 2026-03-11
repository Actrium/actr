# Fast Path Demo - 高性能数据流示例

演示 Actor-RTC Web 的 Fast Path 零拷贝数据流处理。

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
    type: 'fastpath_data',
    stream_id: 'peer:123:video',
    data: videoFrame.buffer,  // 发送 ArrayBuffer
    timestamp: Date.now()
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
  if (event.data.type === 'fastpath_data') {
    const { stream_id, data, timestamp } = event.data;

    // 创建 FastPathData（零拷贝包装）
    const uint8Array = new Uint8Array(data);
    const fastpathData = new FastPathData(
      stream_id,
      uint8Array,
      timestamp
    );

    // 转换为 StreamChunk 并分发
    const chunk = fastpathData.to_chunk();
    actorSystem.dispatch_fastpath(chunk);
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
serviceWorkerPort.postMessage({
  type: 'fastpath_data',
  stream_id: 'test:latency',
  data: new Uint8Array([1, 2, 3]).buffer,
  timestamp: start
}, [data]);

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
    type: 'fastpath_data',
    stream_id: 'test:throughput',
    data: frame.buffer,
    timestamp: Date.now()
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

1. **构建 WASM**
   ```bash
   cd crates/runtime-web
   wasm-pack build --target web
   ```

2. **启动服务器**
   ```bash
   python3 -m http.server 8000
   ```

3. **访问示例**
   ```
   http://localhost:8000/examples/fastpath-demo/
   ```

## 相关文档

- [Fast Path 架构文档](../../docs/architecture/fastpath.md)
- [RouteTable 设计文档](../../docs/architecture/routetable.md)
- [性能优化指南](../../docs/performance.md)
