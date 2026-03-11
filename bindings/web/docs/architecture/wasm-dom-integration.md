# WASM + DOM 集成架构设计

**日期**: 2025-11-11
**状态**: 已决策
**决策者**: 架构团队

---

## 一、问题背景

### 1.1 核心诉求

在 actr-web 项目开发中，我们希望：

1. **用户代码全部 WASM 化**：业务逻辑（包括 Fast Path 处理）统一用 Rust/Go/C++ 编写，编译为 WASM
2. **支持多语言**：不限于 JS/TS，拓宽语言选项
3. **体验统一**：避免用户需要同时写 Rust（Service Worker）和 JS（DOM）的割裂体验
4. **保持性能**：Fast Path 需要低延迟（目标 <15ms）

### 1.2 技术约束

- **WASM 限制**：WASM 无法直接访问 DOM API 和 WebRTC API
- **浏览器限制**：Service Worker 无法访问 WebRTC API（只有 DOM/Window 上下文可以）
- **性能要求**：
  - State Path (RPC): ~30-40ms（持久化 + 调度）
  - Fast Path (Stream): 目标 <15ms（低延迟数据流）

### 1.3 架构冲突

```
需求：所有用户代码在 WASM（包括 Fast Path）
              ↓
约束：WASM 无法访问 WebRTC API
              ↓
现实：WebRTC 必须在 DOM 侧创建和管理
              ↓
问题：Fast Path 处理逻辑应该在哪里实现？
```

---

## 二、方案对比分析

### 方案 A：分层实现（割裂体验）

```
Service Worker (WASM)         DOM 侧 (手工 JS)
├─ State Path 业务逻辑 ✅      ├─ WebRTC 管理
└─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─        └─ Fast Path 回调 ⚠️
```

**评估**：
- ✅ 性能最优（Fast Path ~1-3ms）
- ❌ **体验割裂**：用户需要写两种语言
- ❌ 无法支持其他语言（Go/C++）

---

### 方案 B：WASM + JS 绑定层

```rust
// 用户代码（全 Rust）
impl FastPathHandler {
    fn on_video_frame(&self, data: &[u8]) {
        // 通过 wasm-bindgen 调用 JS 封装的 WebRTC API
        self.peer_connection.send(data); // ← JS 绑定
    }
}
```

**评估**：
- ✅ 用户代码统一（全 Rust）
- ✅ 支持多语言
- ❌ **绑定层封装极度复杂**（Rust 包装 JS 异步 API 的"生命周期地狱"）
- ⚠️ 性能开销（Rust ↔ JS 频繁调用）

**现实困难**：
```rust
// Rust 包装 JS 异步 API 的痛点
use web_sys::{RtcPeerConnection, RtcDataChannel};

let pc = RtcPeerConnection::new(...)?; // JS 对象
let dc = pc.create_data_channel("label"); // 返回 Promise
dc.set_onmessage(Closure::wrap(...)); // Closure 生命周期管理困难
```

---

### 方案 C：统一在 Service Worker（DOM 固定转发层）✅ **采纳**

```
Service Worker (WASM)         DOM 侧 (框架固定 JS)
├─ 所有用户业务逻辑 ✅          ├─ WebRTC Coordinator (框架提供)
├─ State Path                 │   ├─ 创建 PeerConnection
├─ Fast Path 回调 ✅          │   ├─ 接收 WebRTC 数据
└─ ← PostMessage ← ← ← ← ← ─┤   └─ 转发数据 → SW WASM
```

**核心思想**：
- **DOM 侧是"硬件抽象层"**（HAL），完全由框架提供，用户不需要写代码
- 所有 Fast Path 数据从 DOM 转发到 Service Worker（WASM 处理）
- DOM 侧就像"网卡驱动"，用户只需关心"应用代码"

**评估**：
- ✅ 用户代码统一（100% WASM）
- ✅ 支持多语言（Rust/Go/C++ → WASM）
- ✅ DOM 侧用户无需写代码
- ⚠️ **Fast Path 延迟增加**：~1-3ms → ~6-13ms（+5-10ms 转发开销）
- ✅ **仍然是 "Fast Path"**：6-13ms << State Path 30-40ms

---

## 三、最终架构设计

### 3.1 整体架构图

```
┌──────────────────────────────────────────────┐
│           Browser Window (UI 进程)            │
│                                              │
│  ┌────────────────────────────────────────┐ │
│  │  React/Vue Component (用户 UI 代码)    │ │
│  │                                        │ │
│  │  - 用户操作 ──────┐                    │ │
│  │  - 状态展示 ←─────┼── EventEmitter     │ │
│  └───────────────────┼────────────────────┘ │
│                      │ call/subscribe/on    │
│  ┌───────────────────▼────────────────────┐ │
│  │  ActorRef Proxy (框架胶水层)          │ │
│  └───────────────────┬────────────────────┘ │
│                      │ PostMessage          │
│  ┌───────────────────▼────────────────────┐ │
│  │  DOM WebRTC Coordinator (框架固定)     │ │
│  │  - 创建 PeerConnection                │ │
│  │  - 接收 WebRTC 数据                   │ │
│  │  - 转发到 Service Worker              │ │
│  └───────────────────┬────────────────────┘ │
└──────────────────────┼───────────────────────┘
                       │ PostMessage
┌──────────────────────▼───────────────────────┐
│        Service Worker (WASM 进程)            │
│                                              │
│  ┌────────────────────────────────────────┐ │
│  │  用户业务逻辑 (Rust/Go/C++)            │ │
│  │                                        │ │
│  │  ┌──────────────────────────────────┐ │ │
│  │  │  State Path 逻辑                 │ │ │
│  │  │  - RPC 处理                      │ │ │
│  │  │  - 状态管理                      │ │ │
│  │  └──────────────────────────────────┘ │ │
│  │                                        │ │
│  │  ┌──────────────────────────────────┐ │ │
│  │  │  Fast Path 回调                  │ │ │
│  │  │  - on_video_frame(data)          │ │ │
│  │  │  - on_audio_sample(data)         │ │ │
│  │  │  - 可选: emit('topic', data)     │ │ │
│  │  └──────────────────────────────────┘ │ │
│  └────────────────────────────────────────┘ │
│                                              │
│  ┌────────────────────────────────────────┐ │
│  │  ActorRuntime (框架 Rust 代码)         │ │
│  │  - Mailbox + Scheduler                 │ │
│  │  - Fast Path Registry                  │ │
│  │  - Transport Manager                   │ │
│  └────────────┬───────────────────────────┘ │
└───────────────┼──────────────────────────────┘
                │ WebSocket/WebRTC
                ▼
       Remote Actor Services
```

### 3.2 数据流路径

#### State Path（RPC 消息）
```
UI → actorRef.call()
  → PostMessage → Service Worker WASM
    → Mailbox.enqueue()
      → Scheduler.dequeue()
        → Actor 业务逻辑
          → 返回值 → PostMessage → UI
```

**延迟**: ~30-40ms

#### Fast Path（Stream 消息）
```
WebRTC 数据到达 DOM
  → DOM Coordinator 接收
    → PostMessage → Service Worker WASM
      → Fast Path Registry.dispatch()
        → 用户回调（Rust）
          → (可选) emit() → PostMessage → UI
```

**延迟**: ~6-13ms（转发模式）

---

## 四、UI 交互 API 设计

### 4.1 核心 API 三元组

| API | 方向 | 频率 | 延迟 | 使用场景 |
|-----|------|------|------|---------|
| **`call()`** | UI → WASM → UI | 单次 | 30-40ms | RPC 调用、命令执行 |
| **`subscribe()`** | WASM → UI | 持续 | 6-13ms | 视频流、消息流、实时数据 |
| **`on()`** | WASM → UI | 不定 | <5ms | 状态变化、错误、系统事件 |

### 4.2 API 语义说明

#### `call()` - 请求-响应（State Path）

```typescript
// 类型签名
call<TRequest, TResponse>(
    service: string,
    method: string,
    request: TRequest
): Promise<TResponse>

// 使用示例
const response = await actorRef.call('video-service', 'startCall', {
    peerId: 'user-123',
    codec: 'h264',
});
```

**特点**：
- ✅ 双向通信（有返回值）
- ✅ 走 State Path（持久化、可靠）
- ⏱️ 延迟：30-40ms

#### `subscribe()` - 订阅数据流（Fast Path）

```typescript
// 类型签名
subscribe<T>(
    topic: string,
    callback: (data: T) => void
): Promise<() => void> // 返回取消订阅函数

// 使用示例
const unsubscribe = await actorRef.subscribe(
    'video-stream-remote',
    (frame: VideoFrame) => {
        // 持续接收视频帧
        renderToCanvas(frame);
    }
);

// 取消订阅
await unsubscribe();
```

**特点**：
- ✅ 持续数据流（不是单次）
- ✅ 走 Fast Path（低延迟）
- ⏱️ 延迟：6-13ms
- 📊 适合：视频帧、音频块、实时指标

#### `on()` - 事件监听（系统事件）

```typescript
// 类型签名
on(
    event: string,
    callback: (data: any) => void
): () => void // 返回取消监听函数

// 使用示例
actorRef.on('connection-state-changed', (state) => {
    console.log('Connection:', state);
});

actorRef.on('error', (error) => {
    showErrorToast(error.message);
});
```

**特点**：
- ✅ 框架级事件（生命周期、状态变化）
- ✅ 低延迟（直接 PostMessage）
- 📋 适合：连接状态、错误、系统事件

### 4.3 API 使用示例

```typescript
// 完整的视频通话应用示例
async function VideoCallApp() {
    // 1. 创建 Actor 引用
    const actorRef = await createActorRef({
        wasmUrl: '/video-call-actor.wasm',
        signalingUrl: 'wss://signal.example.com',
    });

    // 2. 监听连接状态（on - 系统事件）
    actorRef.on('connection-state-changed', (state) => {
        setConnectionStatus(state);
    });

    // 3. 启动通话（call - RPC）
    const handleStartCall = async () => {
        await actorRef.call('video-service', 'startCall', {
            peerId: 'user-123',
        });
    };

    // 4. 订阅视频流（subscribe - Fast Path）
    const unsubscribe = await actorRef.subscribe(
        'video-stream-remote',
        (frame: VideoFrame) => {
            renderToCanvas(frame);
        }
    );

    // 5. 结束通话
    const handleEndCall = async () => {
        await unsubscribe();
        await actorRef.call('video-service', 'endCall', {});
    };
}
```

---

## 五、WASM 侧实现

### 5.1 用户代码示例

```rust
#[wasm_bindgen]
pub struct VideoCallActor {
    runtime: ActorRuntime,
    event_emitter: EventEmitter,
}

#[wasm_bindgen]
impl VideoCallActor {
    pub async fn new() -> Self {
        let runtime = ActorRuntime::new();
        let event_emitter = EventEmitter::new();

        // 注册 Fast Path 回调（在 WASM 中处理）
        runtime.register_fast_path("video-in", {
            let emitter = event_emitter.clone();
            move |frame_data: &[u8]| {
                // Fast Path 处理视频帧（Rust 实现）
                let decoded_frame = decode_h264(frame_data);

                // 推送到 UI（通过 subscribe）
                emitter.publish("video-stream-remote", decoded_frame);
            }
        });

        Self { runtime, event_emitter }
    }

    // UI 通过 call() 调用的方法
    #[wasm_bindgen]
    pub async fn start_call(&self, peer_id: &str) -> Result<JsValue, JsValue> {
        // State Path 逻辑
        let result = self.runtime
            .call("video-service", "startCall", peer_id)
            .await?;

        // 发出系统事件（UI 通过 on() 监听）
        self.event_emitter.emit("connection-state-changed", "connected");

        Ok(result)
    }
}
```

### 5.2 编译产物

```bash
# 编译用户代码 + 框架代码 → WASM
cargo build --target wasm32-unknown-unknown --release

# 使用 wasm-bindgen 生成 JS 胶水
wasm-bindgen --out-dir pkg --web video_call_actor.wasm

# 产物
pkg/
├── video_call_actor.wasm       # WASM 主体（用户代码 + 框架）
├── video_call_actor.js         # JS 胶水层
└── video_call_actor.d.ts       # TypeScript 类型
```

---

## 六、DOM 侧固定实现

### 6.1 设计原则

**DOM 侧是框架提供的固定实现，用户无需修改**

```
actr-web-dom/
├── src/
│   ├── webrtc_coordinator.ts    # WebRTC 管理（框架代码）
│   ├── fast_path_forwarder.ts   # 数据转发到 SW（框架代码）
│   └── sw_bridge.ts              # PostMessage 通信（框架代码）
└── dist/
    └── actr-dom.min.js           # 打包后的固定 JS
```

**用户只需要引入**：
```html
<script src="https://cdn.actr.dev/actr-dom.min.js"></script>
```

### 6.2 核心实现

```javascript
// DOM 侧：框架提供，用户无需修改
class FastPathForwarder {
    constructor(swPort) {
        this.swPort = swPort;
        this.peerConnection = null;
    }

    // WebRTC 数据到达时
    onDataChannelMessage(streamId, data) {
        // 使用 Transferable 避免拷贝（零拷贝转移）
        this.swPort.postMessage({
            type: 'fast_path_data',
            streamId,
            data: data.buffer, // Transferable!
        }, [data.buffer]); // ← 关键：零拷贝
    }

    // 创建 WebRTC 连接（框架负责）
    async createPeerConnection(config) {
        this.peerConnection = new RTCPeerConnection(config);
        // ... 标准 WebRTC 逻辑
    }
}
```

---

## 七、关键设计原则

### 7.1 职责划分

| 组件 | 职责 | 实现者 |
|------|------|--------|
| **UI 层** | 视图渲染、用户交互 | 用户（React/Vue） |
| **WASM 层** | 所有业务逻辑（State Path + Fast Path） | 用户（Rust/Go/C++） |
| **DOM 固定层** | WebRTC 管理、数据转发 | 框架提供（固定 JS） |

### 7.2 数据流原则

1. **UI 是启动者**：页面加载 → 初始化 WASM → 启动 UI
2. **WASM 是服务层**：提供 API，响应调用
3. **Fast Path 在 WASM**：所有业务逻辑统一在 WASM
4. **按需通知 UI**：WASM 通过 `publish()` 推送数据到 UI（仅必要时）

### 7.3 性能优化

1. **使用 Transferable**：PostMessage 时零拷贝转移 ArrayBuffer
2. **批量处理**：Fast Path 数据批量转发（减少 PostMessage 次数）
3. **采样推送**：不是每帧都推送给 UI（如每 10 帧推 1 帧）
4. **按需订阅**：UI 只订阅真正需要的数据流

---

## 八、多语言支持路径

### 8.1 统一接口规范

定义统一的 WASM 接口（类似 WASI），让不同语言都能编译到这个接口：

```
用户代码（Go）          用户代码（Rust）        用户代码（C++）
    ↓                      ↓                       ↓
TinyGo → WASM          rustc → WASM          Emscripten → WASM
    ↓                      ↓                       ↓
        统一的 WASM Interface (wasm-bindgen)
                ↓
        Service Worker Runtime
```

### 8.2 接口定义（待完善）

```rust
// 定义统一的 WASM 接口
#[wasm_bindgen]
pub trait ActorInterface {
    fn init() -> Self;
    fn on_message(msg: &[u8]) -> Result<Vec<u8>, String>;
    fn on_fast_path(stream_id: &str, data: &[u8]);
}
```

---

## 九、性能指标

| 路径 | 延迟 | 说明 |
|------|------|------|
| **State Path (RPC)** | 30-40ms | Mailbox 持久化 + Scheduler 调度 |
| **Fast Path (转发)** | 6-13ms | DOM → PostMessage → SW WASM |
| **Fast Path (原生 DOM)** | 1-3ms | 对比参考（未采纳） |
| **系统事件 (on)** | <5ms | 直接 PostMessage |

**关键结论**：
- Fast Path 转发延迟（6-13ms）仍远快于 State Path（30-40ms）
- 对于大多数应用场景（视频通话、文件传输、数据同步），6-13ms 是可接受的
- 换取的是**用户代码 100% 统一**和**多语言支持**

---

## 十、实施路线图

### Phase 1: Pure Client Mode（✅ 已完成）
- ✅ Service Worker WASM Runtime
- ✅ WebSocket Transport
- ✅ State Path (Mailbox + Dispatcher)
- ✅ Scheduler（串行调度 + 优先级 + 事件驱动）
- ✅ 完整传输栈 (PeerGate → PeerTransport → DestTransport → WirePool)
- ✅ RouteTable (~300 行)
- ✅ ActrRef (call/tell/shutdown)
- **目标**：支持 90% 客户端场景，用户代码 100% WASM ✅

### Phase 2: DOM Forwarder（✅ 基础完成）
- ✅ DOM 侧 TypeScript 实现 (MessageChannel 桥接)
- ✅ PostMessage 转发机制 (register_datachannel_port)
- ✅ WebRTC Coordinator (DOM 侧 TS 实现)
- ⚠️ Fast Path Registry（SW 侧 handle_fast_path 已实现，register 回调待完善）
- **目标**：Fast Path 统一模式，延迟 ~6-13ms

### Phase 3: 性能优化（规划中）
- ⏳ Transferable 优化
- ⏳ 批量处理
- ⏳ 采样推送
- **目标**：延迟优化到 <10ms

### Phase 4: 多语言支持（长期目标）
- ⏳ 定义 WASM 接口规范
- ⏳ TinyGo 示例
- ⏳ Emscripten (C++) 示例
- **目标**：验证多语言可行性

---

## 十一、决策总结

### 11.1 最终决策

**采用方案 C：统一在 Service Worker WASM，DOM 侧固定转发层**

### 11.2 核心优势

1. **用户代码统一**：100% WASM，无需写 JS
2. **支持多语言**：Rust/Go/C++ → WASM
3. **DOM 侧透明**：框架提供固定实现，用户无感知
4. **性能可接受**：Fast Path 6-13ms，仍是"快车道"
5. **架构清晰**：UI = 视图层，WASM = 服务层，DOM = HAL

### 11.3 权衡取舍

**得到**：
- ✅ 统一开发体验
- ✅ 多语言支持
- ✅ 架构清晰简洁

**付出**：
- ⚠️ Fast Path 延迟增加 5-10ms（1-3ms → 6-13ms）
- ⚠️ PostMessage 序列化开销（通过 Transferable 缓解）

### 11.4 关键心智模型

> **把 DOM 侧当作"网卡驱动"，Service Worker 侧当作"应用代码"。**
>
> 没人要求驱动程序也用 Rust 写 —— **抽象的边界就是价值所在**。

---

## 十二、后续行动

### 立即执行
1. 完善 DOM 侧固定实现（WebRTC Coordinator + Fast Path Forwarder）
2. 实现 PostMessage 转发机制（使用 Transferable）
3. 实现 SW 侧 Fast Path Registry

### 近期计划
4. 完成 `call/subscribe/on` 三个 API 的完整实现
5. 编写端到端测试（验证延迟和正确性）
6. 编写用户文档和示例代码

### 长期跟踪
7. 性能优化（批量、采样、压缩）
8. 多语言支持验证（TinyGo, Emscripten）
9. 定制运行时（减少 WASM 体积）

---

**文档维护**: 本文档记录了 WASM + DOM 集成的核心技术决策，应随架构演进持续更新。

**相关文档**:
- [架构总览](./overview.md)
- [双层架构设计](./dual-layer.md)
- [技术决策记录](./decisions.md)
