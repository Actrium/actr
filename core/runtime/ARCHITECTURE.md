# actr-runtime 内部架构

**文档目的**：为贡献者和高级用户提供 runtime 相关 crate 的内部架构视图

**最后更新**：2026-03-11
**对应版本**：actr v0.9.x

---

## 1. 模块概览

Actor-RTC 的运行时由两个 crate 协同构成：

- **actr-runtime**：纯业务分发层（ACL + dispatch + 生命周期钩子），无 IO 依赖，可在 native 和 `wasm32-unknown-unknown` 目标上编译。
- **actr-hyper**：基础设施层 + 平台层，承载传输、Wire、信令、WASM/Dynclib 引擎，以及 Actor 沙箱管理。

```
actr-hyper   ← 基础设施层（transport, wire, signaling, WASM engine, dynclib engine …）
actr-runtime ← 业务分发层（ACL + dispatch + lifecycle hooks）
actr-framework ← SDK 接口层（trait 定义：Workload, Context, MessageDispatcher）
actr-protocol  ← 数据定义层（protobuf 类型）
```

### actr-runtime 目录结构

```
actr-runtime/
├── acl.rs              # ACL 权限检查（纯函数，无 IO）
├── dispatch.rs         # ActrDispatch：ACL → 路由 → handler 执行
└── lib.rs              # 转导出 Workload, Context, MessageDispatcher
```

### actr-hyper 目录结构（运行时基础设施部分）

```
actr-hyper/
├── lifecycle/          # Actor 生命周期管理
│   ├── actr_system.rs  # ActrSystem（泛型无关基础设施）
│   └── actr_node.rs    # ActrNode<W>（完整节点）
├── inbound/            # 入站消息处理
│   ├── data_stream_registry.rs     # DataStream 快车道注册表
│   └── media_frame_registry.rs     # MediaFrame 快车道注册表
├── outbound/           # 出站消息处理
│   ├── host_gate.rs    # 进程内出站门（Shell ↔ Workload）
│   └── peer_gate.rs    # 跨进程出站门（Actor ↔ Actor）
├── transport/          # 传输层抽象
│   ├── lane.rs              # DataLane 统一抽象
│   ├── route_table.rs       # PayloadType 路由表
│   ├── manager.rs           # TransportManager trait
│   ├── inproc_manager.rs    # HostTransport 进程内传输管理
│   ├── dest_transport.rs    # 目标传输抽象
│   ├── wire_pool.rs         # Wire 连接池
│   └── wire_handle.rs       # Wire 句柄
├── wire/               # 底层传输协议
│   ├── webrtc/              # WebRTC 实现
│   │   ├── coordinator.rs   # WebRTC 协调器
│   │   ├── gate.rs          # WebRTC 门（入站）
│   │   ├── connection.rs    # WebRTC 连接
│   │   ├── negotiator.rs    # SDP 协商器
│   │   └── signaling.rs     # 信令客户端
│   └── websocket/           # WebSocket 实现
│       ├── connection.rs    # WebSocket 连接
│       ├── gate.rs          # WebSocket 入站门
│       └── server.rs        # WebSocket 服务端
├── executor.rs         # ExecutorAdapter trait（WASM/Dynclib 统一分发接口）
├── wasm/               # WASM 引擎（feature: wasm-engine）
├── dynclib/            # Dynclib 引擎（feature: dynclib-engine）
├── context.rs          # Context 实现
├── context_factory.rs  # Context 工厂
├── actr_ref.rs         # ActrRef（Actor 引用）
└── runtime_error.rs    # 错误类型定义
```

---

## 2. 三种 Actor 集成模式

Actor-RTC 支持三种 Workload 集成方式，覆盖从最高性能到最高隔离的完整光谱：

### 2.1 Source 集成（静态编译）

Workload trait 编译进同一 binary，通过泛型 `ActrNode<W>` 实现静态分发。

- **分发路径**：`ActrNode::handle_incoming` → `ActrDispatch<W>::dispatch` → `W::Dispatcher::dispatch`
- **调度方式**：编译时单态化，零虚函数调用
- **性能特征**：最优——无序列化、无 IPC、完全内联
- **使用场景**：性能敏感服务、同一团队维护的 Actor

```rust
// Source 模式：Workload 直接编译进宿主 binary
let system = ActrSystem::from_config("actr.toml").await?;
let _ref = system.attach(MyWorkload::new()).start().await?;
```

### 2.2 WASM 集成

.wasm 模块由 WasmHost 加载，通过 asyncify suspend/resume 实现异步 I/O。

- **分发路径**：`ActrNode::handle_incoming` → `ExecutorAdapter::dispatch` → `WasmInstance` (wasmtime)
- **调度方式**：通过 `Box<dyn ExecutorAdapter>` 动态分发
- **feature 门控**：`wasm-engine`
- **I/O 模型**：Guest 调用 `hyper_send`/`hyper_recv` host import → asyncify 挂起 → 宿主完成 I/O → 恢复执行
- **使用场景**：第三方 Actor 沙箱隔离、跨平台分发

### 2.3 Dynclib 集成（规划中）

.so / .dylib / .dll 原生共享库由 DynclibHost 加载，通过 C ABI + VTable 调用。

- **分发路径**：`ActrNode::handle_incoming` → `ExecutorAdapter::dispatch` → DynclibInstance (dlopen)
- **调度方式**：通过 `Box<dyn ExecutorAdapter>` 动态分发
- **feature 门控**：`dynclib-engine`
- **性能特征**：接近 Source 模式（原生代码），但有 FFI 边界开销
- **使用场景**：需要原生性能同时保持独立部署的 Actor

### 分发路径汇总

```
handle_incoming(envelope)
    │
    ├── self.executor == Some(adapter)
    │   └── adapter.dispatch(bytes, ctx, call_executor)     // WASM / Dynclib
    │
    └── self.executor == None
        └── ActrDispatch<W>::dispatch(self_id, caller, envelope, ctx)  // Source
```

---

## 3. 两种传输策略

传输策略与集成模式正交——任意集成模式都可使用任一传输策略。

### 3.1 HostGate + HostTransport（Shell ↔ Workload）

- **职责**：同进程内 Shell 与 Workload 之间的双向通信
- **底层实现**：`tokio::sync::mpsc` 通道，零序列化
- **延迟**：~10μs
- **双向设计**：两个独立的 HostTransport 实例
  - Shell → Workload（REQUEST）
  - Workload → Shell（RESPONSE）

### 3.2 PeerGate + PeerTransport（Actor ↔ Actor）

- **职责**：跨进程 / 跨网络的 Actor 间通信
- **底层实现**：WebRTC DataChannel / WebSocket
- **序列化**：Protobuf
- **延迟**：1-50ms（取决于网络）
- **pending_requests**：管理 RPC 请求-响应匹配

---

## 4. ExecutorAdapter trait

`ExecutorAdapter` 是 WASM 和 Dynclib 的统一分发接口。Source 模式不经过此 trait，直接走泛型静态分发。

```rust
pub trait ExecutorAdapter: Send {
    fn dispatch<'a>(
        &'a mut self,
        request_bytes: &[u8],
        ctx: DispatchContext,
        call_executor: &'a CallExecutorFn,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, ...>> + Send + 'a>>;
}
```

### 共享类型

- **DispatchContext**：每次请求的上下文（self_id, caller_id, request_id）
- **PendingCall**：Guest 发起的出站调用枚举（Call / Tell / Discover / CallRaw）
- **IoResult**：出站 I/O 操作结果（Bytes / Done / Error）
- **CallExecutorFn**：宿主侧执行出站调用的闭包

---

## 5. 分层架构

运行时采用 4 层架构设计：

```
┌─────────────────────────────────────────────┐
│  Layer 3: Application (Workload)            │  用户业务逻辑
│    Inbound: DataStreamRegistry              │  快车道回调
│             MediaFrameRegistry              │
├─────────────────────────────────────────────┤
│  Layer 2: Outbound Gate                     │  出站门抽象
│    Gate::Host(Arc<HostGate>)                │  Shell ↔ Workload
│    Gate::Peer(Arc<PeerGate>)                │  Actor ↔ Actor
├─────────────────────────────────────────────┤
│  Layer 1: Transport (DataLane)              │  传输通道抽象
│    HostTransport                            │  进程内 mpsc
│    PeerTransport                            │  WebRTC/WebSocket
│    WirePool / WireHandle                    │
├─────────────────────────────────────────────┤
│  Layer 0: Wire (Protocol)                   │  物理传输协议
│    WebRTC (DataChannel + RTP)               │
│    WebSocket                                │
│    tokio::sync::mpsc                        │
└─────────────────────────────────────────────┘
```

### Gate enum

```rust
pub enum Gate {
    Host(Arc<HostGate>),   // 进程内传输（零序列化）
    Peer(Arc<PeerGate>),   // 跨进程传输（Protobuf 序列化）
}
```

设计优势：enum dispatch 静态分发，零虚函数调用开销，CPU 分支预测命中率 >95%。

### 关键设计原则

1. **层次分离**：每层只依赖下一层，不跨层调用
2. **统一抽象**：通过 DataLane 统一 Host 和 Peer 路径
3. **语义与能力分离**：PayloadType 决定顶层语义，具体执行策略由 resolver 根据语义和后端能力共同决议
4. **零成本抽象**：Host 路径零序列化，Peer 路径零拷贝（`Bytes` 浅拷贝）

---

## 6. 核心组件职责

### 6.1 业务分发层（actr-runtime）

**ActrDispatch<W>**：
- 职责：纯业务分发器——ACL 检查 → 路由 → handler 执行 → panic 捕获
- 无 IO 依赖，可在 native 和 wasm32 目标上编译
- 关键方法：`dispatch()`, `on_start()`, `on_stop()`

**check_acl_permission**：
- 职责：纯函数 ACL 权限判定
- 评估规则：本地调用放行 → 无 ACL 放行 → 空规则拒绝 → Deny-first → Allow 命中放行 → 默认拒绝

### 6.2 生命周期管理（actr-hyper/lifecycle/）

**ActrSystem**：
- 职责：提供无泛型的基础设施（Mailbox、SignalingClient、ContextFactory）
- 生命周期：从创建到 attach(workload) 转换为 ActrNode
- 关键方法：`from_config()`, `new()`, `attach<W>()`

**ActrNode<W>**：
- 职责：泛型化的完整节点，持有 Workload 和运行时组件
- 分发路径：根据 `executor` 字段选择 Source 直接分发或 ExecutorAdapter 动态分发
- 关键方法：`start()`, `handle_incoming()`, `shutdown()`
- 关键字段：`executor: Option<Mutex<Box<dyn ExecutorAdapter>>>`

### 6.3 入站处理（actr-hyper/inbound/）

**WebRtcGate**：
- 职责：消费 `WebRtcCoordinator` 聚合的入站数据，直接根据 PayloadType 分发
- 路由规则：
  - RpcReliable/RpcSignal → 先检查 pending_requests；命中则视为 Response 并唤醒 continuation，否则按优先级入 Mailbox
  - StreamReliable/StreamLatencyFirst → DataStreamRegistry（快车道回调）
  - MediaRtp → 直接丢弃并提示应走 WebRTC Track（MediaFrameRegistry 由 PeerConnection 注册）

**Inproc 接收循环**：
- 职责：`ActrNode` 内的两个 tokio 循环（Shell→Workload、Workload→Shell）直接从 `HostTransport` 的 `DataLane::Mpsc` 收包
- Shell→Workload：解出 `RpcEnvelope` 后调用 `handle_incoming()`
- Workload→Shell：根据 `request_id` 调用 `complete_response()` 唤醒请求方

**DataStreamRegistry**：
- 职责：管理 DataStream 回调注册表（stream_id → callback）
- 并发安全：使用 DashMap 支持多线程并发访问
- 回调签名：`FnMut(DataStream, ActrId) -> BoxFuture<ActorResult<()>>`

**MediaFrameRegistry**：
- 职责：管理 MediaTrack 回调注册表（track_id → callback）
- 并发安全：使用 DashMap
- 回调签名：`FnMut(MediaSample, ActrId) -> BoxFuture<ActorResult<()>>`

#### 语义决策模型（当前约束与演进方向）

> 注：本节描述 runtime 内部推荐的统一决策模型，用于解释当前实现并指导后续 `wasm` / `StateSync` 扩展；其中 `StateSync` 和部分 backend-specific policy 仍属于设计方向，尚未完全落地到代码。

- `PayloadType` 负责表达顶层数据语义，不单独决定本地执行方式：
  - `RpcReliable`
  - `RpcSignal`
  - `StreamReliable`
  - `StreamLatencyFirst`
  - `MediaRtp`
  - `StateSync`（planned）
- `MessageRole` 负责表达交互角色：
  - `Request`
  - `Response`
  - `Notify`
  - `Data`
  - `Snapshot`
  - `Delta`
- 后端能力拆成两个正交维度，而不是混成单一 `BackendProfile`：
  - `RuntimeKind`: `native` | `wasm`
  - `TransportKind`: `inproc` | `webrtc` | `websocket`
- `ExecutionPolicy` 是 resolver 的输出，而不是输入维度：
  - `Mailbox`
  - `PendingContinuation`
  - `OrderedStreamQueue`
  - `CoalescingQueue`
  - `MediaPipeline`
  - `LatestValueStore`

推荐求解形式：

```rust
ExecutionPlan = resolve(payload_type, message_role, runtime_kind, transport_kind, hints)
```

- `hints` 只用于调参，不应用来覆写核心语义。典型字段包括：
  - `priority`
  - `queue_depth`
  - `batch_size`
  - `ttl/deadline`
  - `drop_policy`
  - `persistence`
- resolver 需要先做组合合法性检查；并非所有组合都有效，例如：
  - `MediaRtp + Response`：通常无效
  - `RpcReliable + Data`：通常无效
  - `StateSync + Request`：通常不应作为默认组合

推荐默认映射：

| 语义组合 | 默认 ExecutionPolicy | 备注 |
| --- | --- | --- |
| `RpcReliable + Request/Notify` | `Mailbox` | 正常优先级，进入 actor 状态路径 |
| `RpcSignal + Request/Notify` | `Mailbox` | 高优先级控制消息 |
| `RpcReliable/RpcSignal + Response` | `PendingContinuation` | 默认服务于 `call().await`；若需要"晚点按 actor event 处理"，应建模为显式 split-phase API，而不是改写普通 response 语义 |
| `StreamReliable + Data` | `OrderedStreamQueue` | 当前实现近似为 `DataStreamRegistry` fast path，后续可补齐 bounded queue / batching |
| `StreamLatencyFirst + Data` | `CoalescingQueue` | 当前实现与 `StreamReliable` 共用 registry，目标语义应是 latest-first / coalescing |
| `MediaRtp + Data` | `MediaPipeline` | 应走 WebRTC Track fast path；`websocket` 通常不是合法承载路径 |
| `StateSync + Snapshot/Delta` | `LatestValueStore` | planned；旧值可被新值覆盖，不应强行复用 RPC/mailbox 语义 |

设计约束：

- `Response -> PendingContinuation` 是普通 `call().await` 的默认语义，不是绝对禁止 split-phase。
- 如果业务需要"response 到达后延后处理 / 排队处理 / 持久化处理"，应使用显式 split-phase API（例如 response 转 self-notify / workflow event），而不是把普通 RPC response 全部改为 Mailbox 事件。
- `wasm` 与 `native` 的差异应体现在 resolver 产出的 `ExecutionPlan` 上，而不是体现在开发者 API 或 `PayloadType` 分叉上。特别是 `wasm` 后端应优先减少 host/guest crossing 次数，倾向 batch / coalescing，而不是复制 native 的细粒度调度。

### 6.4 出站处理（actr-hyper/outbound/）

**Gate** enum：
- `Host(Arc<HostGate>)`：进程内出站
- `Peer(Arc<PeerGate>)`：跨进程出站
- 设计优势：静态分发，零虚拟调用开销

**HostGate**：
- 职责：通过 HostTransport 发送进程内消息
- 特点：零序列化，直接传递 RpcEnvelope 对象
- 延迟：~10μs

**PeerGate**：
- 职责：通过 PeerTransport 发送跨进程消息
- 特点：Protobuf 序列化，通过 WebRTC/WebSocket 传输
- 延迟：1-50ms（取决于网络）
- pending_requests：管理 RPC 请求-响应匹配

### 6.5 传输层（actr-hyper/transport/）

**DataLane** enum：
- `Mpsc { payload_type, tx, rx }`：进程内 tokio mpsc 通道
- `WebRtcDataChannel { data_channel, rx }`：WebRTC DataChannel
- `WebSocket { sink, payload_type, rx }`：WebSocket 连接

**PayloadTypeExt** trait：
- 核心方法：`data_lane_types() -> &'static [DataLaneType]`
- 作用：提供 PayloadType 到 DataLaneType 的静态路由表
- 优势：编译时确定，零运行时开销
- 注意：`PayloadTypeExt` 只解决"走哪条 lane"，不单独决定本地 `Mailbox / PendingContinuation / Registry / MediaPipeline` 执行策略；后者由上面的语义 resolver 决定

**TransportManager** trait：
- 职责：管理传输通道的生命周期（创建、缓存、复用）
- 实现：
  - `HostTransport`：管理进程内 mpsc 通道
  - `PeerTransport`：管理 WebRTC/WebSocket 连接

### 6.6 Wire 层（actr-hyper/wire/）

**WebRtcCoordinator**：
- 职责：管理所有 WebRTC peer connections 的生命周期
- 关键功能：
  - 启动多 PayloadType 接收循环（RpcReliable, RpcSignal, StreamReliable, StreamLatencyFirst）
  - 聚合所有 peer 的消息到统一的 message_rx
  - 提供 `send_message()` 和 `receive_message()` 接口

**WebRtcGate**：
- 职责：WebRTC 入站消息路由（Coordinator → Mailbox/Registry）
- 路由逻辑：
  - 根据 PayloadType 分发消息
  - RPC 消息先检查 pending_requests：命中则完成 continuation，否则按优先级进入 Mailbox
  - DataStream 消息直接派发到 DataStreamRegistry

**WebRtcConnection**：
- 职责：封装单个 RTCPeerConnection，管理 DataChannel 和 MediaTrack
- 关键方法：`create_data_channel()`, `get_lane()`, `add_track()`

---

## 7. 关键数据流

### 7.1 RPC 请求-响应流程

**发送端（PeerGate）**：
```rust
1. send_request(target, envelope)
2. 生成 request_id，注册 oneshot::Sender 到 pending_requests
3. 序列化 RpcEnvelope → Bytes
4. TransportManager → DataLane → WebRTC
```

**接收端（WebRtcGate）**：
```rust
1. Coordinator.receive_message() → (from, data, RpcReliable)
2. 反序列化 Bytes → RpcEnvelope
3. 检查 request_id 是否在 pending_requests 中
4. 如果是响应：唤醒 oneshot::Sender
5. 如果是请求：enqueue(Mailbox)
```

### 7.2 DataStream 快车道流程

**发送端**：
```rust
1. ctx.send_data_stream(target, stream_id, chunk)
2. Gate::send_data_stream(target, StreamReliable, data)
3. TransportManager → DataLane(StreamReliable) → WebRTC
```

**接收端**：
```rust
1. Coordinator.receive_message() → (from, data, StreamReliable)
2. WebRtcGate 识别 PayloadType::StreamReliable
3. 反序列化 Bytes → DataStream
4. DataStreamRegistry.dispatch(chunk, sender_id)
5. 调用注册的回调函数
```

### 7.3 WASM Actor 分发流程

```rust
1. handle_incoming(envelope)
2. 检测 self.executor == Some(adapter)
3. 序列化 envelope → bytes
4. adapter.dispatch(bytes, DispatchContext, call_executor)
5. WASM guest 执行 → 遇到 hyper_send → asyncify 挂起
6. call_executor(PendingCall::Call { ... }) → IoResult::Bytes(response)
7. asyncify 恢复 → guest 继续执行 → 返回结果 bytes
```

---

## 8. 性能优化设计

### 8.1 零拷贝设计

- **Host 路径**：直接传递 `RpcEnvelope` 对象，无序列化
- **Peer 路径**：使用 `Bytes` 类型（Arc<Vec<u8>>），浅拷贝
- **MediaTrack**：WebRTC 原生 RTP 通道，绕过 Protobuf 序列化

### 8.2 编译时路由

- **PayloadTypeExt**：路由表在编译时确定，无运行时查表
- **Source 模式**：`ActrDispatch<W>` 通过泛型单态化，完全内联
- **Gate enum**：静态分发，优于 trait object

### 8.3 细粒度并发

- **DashMap**：用于 Registry，支持高并发读写
- **独立接收循环**：每个 PayloadType 独立 tokio 任务
- **无锁设计**：尽可能使用 mpsc/oneshot 避免锁竞争

---

## 9. 错误处理策略

### 9.1 错误类型层次

```rust
RuntimeError
├── TransportError      # 传输层错误（连接断开、超时）
├── ProtocolError       # 协议错误（反序列化失败）
└── Other(anyhow::Error)  # 其他错误
```

### 9.2 错误传播

- **传输层错误**：自动重试（带 exponential backoff）
- **协议错误**：记录日志，丢弃消息
- **应用层错误**：通过 RpcEnvelope.error 返回给调用方

---

## 10. 测试策略

### 10.1 单元测试

- `actr-runtime/acl.rs`：ACL 权限检查测试
- `actr-runtime/dispatch.rs`：分发器 panic 捕获测试
- `transport/lane.rs`：DataLane 创建和收发测试
- `inbound/data_stream_registry.rs`：回调注册和触发测试
- `outbound/host_gate.rs`：进程内消息发送测试

### 10.2 集成测试

- `actr-hyper/tests/wasm_actor_e2e.rs`：WASM actor 端到端测试
- `actr-hyper/tests/asyncify_poc.rs`：asyncify suspend/resume 验证
- `actr-hyper/tests/wasm_host.rs`：WasmHost/WasmInstance 测试

---

## 11. 依赖图

```
actr-hyper
├── actr-runtime       (业务分发层)
├── actr-framework     (trait 定义)
├── actr-protocol      (协议定义)
├── actr-runtime-mailbox (持久化 Mailbox)
├── actr-config        (配置解析)
├── tokio              (异步运行时)
├── webrtc             (WebRTC 实现)
├── tokio-tungstenite  (WebSocket 实现)
├── wasmtime           (WASM 引擎，可选 feature: wasm-engine)
├── dashmap            (并发哈希表)
└── anyhow             (错误处理)

actr-runtime
├── actr-framework     (trait 定义)
├── actr-protocol      (协议定义)
├── bytes              (零拷贝 buffer)
├── futures-util       (catch_unwind)
└── tracing            (日志)
```

---

## 12. 贡献指南

### 12.1 代码组织原则

1. **单一职责**：每个模块只负责一个清晰的功能
2. **依赖倒置**：高层模块依赖抽象（trait），不依赖具体实现
3. **开闭原则**：通过 enum 和 trait 扩展功能，而非修改现有代码

### 12.2 命名约定

- **Manager**：管理生命周期的组件（如 TransportManager）
- **Registry**：管理回调注册的组件（如 DataStreamRegistry）
- **Gate**：消息出入口抽象（如 WebRtcGate, Gate）
- **Coordinator**：协调多个相关组件的组件（如 WebRtcCoordinator）
- **Adapter**：统一接口适配器（如 ExecutorAdapter）

### 12.3 提交 PR 前检查清单

- [ ] 单元测试通过
- [ ] 集成测试通过
- [ ] 更新相关文档（README, ARCHITECTURE.md）
- [ ] 代码符合 rustfmt 和 clippy 规范
- [ ] 性能敏感路径无明显回归

---

## 13. 参考资料

- [用户文档：Runtime 设计](../../actor-rtc.github.io/zh-hans/appendix-runtime-design.zh.md)
- [用户文档：术语表](../../actor-rtc.github.io/zh-hans/appendix-glossary.zh.md)
- [用户文档：Lane 选择策略](../../actor-rtc.github.io/zh-hans/appendix-lane-selection-strategy.zh.md)
- [actr-protocol README](../protocol/README.md)
- [actr-framework README](../framework/README.md)

---

**维护者**：actr 核心团队
**问题反馈**：https://github.com/actor-rtc/actr/issues
