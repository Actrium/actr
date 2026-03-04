# Actor-RTC Web 架构总览

**版本**: 2025-11-11 (更新: 2026-01-08)
**状态**: 已实现（Transport + Message 处理）

## 架构全览图

> **💡 核心理念**：出入的总控制器都在 SW，但只要涉及 WebRTC 就转发 DOM

### SVG 版本（推荐）

![架构全览图](./architecture-overview.svg)

### Mermaid 版本（交互式）

```mermaid
graph TB
    subgraph Remote["🌍 远程节点 Remote Node"]
        WS_Server["WebSocket Server"]
        RTC_Peer["WebRTC Peer<br/>(DataChannel + MediaTrack)"]
    end

    subgraph SW["Service Worker - 总控制器"]
        direction LR

        subgraph SW_IN["📥 INBOUND 入口<br/>(State Path ~30-40ms)"]
            direction TB
            IN1["InboundDispatcher<br/>类型判断"]
            IN2["Mailbox<br/>(IndexedDB)"]
            IN3["MailboxProcessor<br/>dequeue + ack"]
            IN4["Scheduler<br/>串行调度"]
            IN5["Actor Logic<br/>业务处理"]

            IN1 --> IN2 --> IN3 --> IN4 --> IN5
        end

        subgraph SW_OUT["📤 OUTBOUND 出口"]
            direction TB
            OUT1["OutGate"]
            OUT2["TransportManager"]
            OUT3["WirePool<br/>(优先级管理)"]
            OUT4A["WebSocket Wire<br/>(Priority 0)"]
            OUT4B["WebRTC Wire<br/>(Priority 1)<br/>via MessagePort"]

            OUT1 --> OUT2 --> OUT3
            OUT3 --> OUT4A
            OUT3 --> OUT4B
        end

        IN5 -.->|"ctx.call()<br/>ctx.tell()"| OUT1
    end

    subgraph DOM["DOM/Window - WebRTC 专用 & Fast Path"]
        direction LR

        subgraph DOM_RTC["📡 WebRTC 管理"]
            direction TB
            RTC1["WebRtcCoordinator<br/>创建 P2P"]
            RTC2["PeerConnection<br/>SDP + ICE"]
            RTC3["DataChannel<br/>双向通信"]

            RTC1 --> RTC2 --> RTC3
        end

        subgraph DOM_FAST["⚡ Fast Path<br/>(~1-3ms)"]
            direction TB
            FAST1["DomSystem<br/>分发器"]
            FAST2["StreamRegistry<br/>stream_id → callback"]
            FAST3["MediaRegistry<br/>track_id → callback"]

            FAST1 --> FAST2
            FAST1 --> FAST3
        end
    end

    %% 跨域连接
    WS_Server -->|"① WebSocket 入"| IN1
    RTC_Peer -->|"② WebRTC DC 入"| FAST1
    RTC_Peer -->|"③ WebRTC Track 入"| FAST3
    FAST1 -.->|"④ RPC 消息<br/>PostMessage 转发"| IN1
    FAST2 -.->|"⑤ Stream 本地处理<br/>直接回调"| FAST1
    RTC1 -.->|"⑥ P2P Ready<br/>连接就绪通知"| OUT3
    OUT4B -.->|"⑦ 发送请求<br/>PostMessage 转发"| RTC1
    OUT4A -->|"⑧ WebSocket 出"| WS_Server
    RTC3 -->|"⑨ WebRTC 出"| RTC_Peer

    %% 样式定义
    classDef swClass fill:#e3f2fd,stroke:#1976d2,stroke-width:3px,color:#000
    classDef domClass fill:#f3e5f5,stroke:#7b1fa2,stroke-width:3px,color:#000
    classDef remoteClass fill:#fff3e0,stroke:#f57c00,stroke-width:3px,color:#000
    classDef fastClass fill:#c8e6c9,stroke:#2e7d32,stroke-width:2px,color:#000

    class SW swClass
    class DOM domClass
    class Remote remoteClass
    class DOM_FAST fastClass

    %% 线条样式（按照图例说明）
    linkStyle 0 stroke:#2196f3,stroke-width:3px
    linkStyle 1 stroke:#9c27b0,stroke-width:3px
    linkStyle 2 stroke:#9c27b0,stroke-width:3px
    linkStyle 3 stroke:#ff9800,stroke-width:2px,stroke-dasharray:5
    linkStyle 4 stroke:#4caf50,stroke-width:2px,stroke-dasharray:5
    linkStyle 5 stroke:#4caf50,stroke-width:2px,stroke-dasharray:5
    linkStyle 6 stroke:#ff9800,stroke-width:2px,stroke-dasharray:5
    linkStyle 7 stroke:#2196f3,stroke-width:3px
    linkStyle 8 stroke:#9c27b0,stroke-width:3px
```

**流程编号说明**：
- ① WebSocket 入：远程 → SW InboundDispatcher
- ② WebRTC DC 入：远程 → DOM Fast Path（DataChannel）
- ③ WebRTC Track 入：远程 → DOM Fast Path（MediaTrack）
- ④ RPC 转发：DOM → SW（PostMessage 转发 RPC 消息）
- ⑤ Stream 本地处理：DOM Fast Path 内部（直接回调，超低延迟）
- ⑥ P2P Ready 通知：DOM → SW（连接建立完成信号）
- ⑦ 发送转发：SW → DOM（通过 MessagePort 发送 WebRTC 数据）
- ⑧ WebSocket 出：SW → 远程（直接发送）
- ⑨ WebRTC 出：DOM → 远程（P2P 发送）

**线条颜色说明**：
- 🔵 **蓝色实线**：WebSocket 路径（①⑧）- SW 可直接使用
- 🟣 **紫色实线**：WebRTC 路径（②③⑨）- 必须通过 DOM
- 🟠 **橙色虚线**：PostMessage 转发（④⑦）- SW ↔ DOM 跨域通信
- 🟢 **绿色虚线**：控制与优化（⑤⑥）- 本地处理和状态通知

**关键要点**：
1. **总控在 SW**：所有入口（InboundDispatcher）、出口（OutGate）、路由决策都在 Service Worker
2. **WebRTC 转发**：SW 无法直接访问 WebRTC API，通过 MessagePort 桥接到 DOM
3. **优先级管理**：WirePool 自动选择，WebRTC (P2P) 优先级高于 WebSocket (C/S)
4. **Fast Path**：Stream 和 Media 在 DOM 本地处理，绕过 SW Mailbox，延迟 < 3ms

---

## 核心架构（详细说明）

### 双进程模型

```
┌────────────────────────────────────────────────────┐
│            Service Worker (主控)                   │
│                                                    │
│  ┌─────────────── 发送 (Transport) ──────────┐   │
│  │  OutprocTransportManager                   │   │
│  │    → DestTransport                         │   │
│  │      → WirePool (WebSocket + WebRTC)       │   │
│  └────────────────────────────────────────────┘   │
│                                                    │
│  ┌─────────────── 接收 (State Path) ─────────┐   │
│  │  InboundPacketDispatcher                   │   │
│  │    → Mailbox (IndexedDB)                   │   │
│  │      → MailboxProcessor                    │   │
│  │        → Scheduler → Actor                 │   │
│  └────────────────────────────────────────────┘   │
│                                                    │
└─────────────────┬──────────────────────────────────┘
                  │ PostMessage
┌─────────────────┴──────────────────────────────────┐
│                 DOM (辅助 + Fast Path)             │
│                                                    │
│  ┌─────────── WebRTC 管理 ──────────┐             │
│  │  WebRtcCoordinator                │             │
│  │    → 创建 PeerConnection          │             │
│  │    → 通知 SW P2P 就绪             │             │
│  └───────────────────────────────────┘             │
│                                                    │
│  ┌─────────── 接收 (Fast Path) ──────┐            │
│  │  WebRtcDataChannelReceiver         │            │
│  │    → StreamHandlerRegistry         │            │
│  │    → MediaFrameHandlerRegistry     │            │
│  └───────────────────────────────────┘             │
│                                                    │
└────────────────────────────────────────────────────┘
```

## 核心组件

### Transport 层（发送）

| 组件 | 职责 |
|------|------|
| OutprocTransportManager | 统一发送接口，管理所有 Dest |
| WireBuilder | 工厂模式创建连接 |
| DestTransport | 单 Dest 管理，事件驱动发送 |
| WirePool | 连接池，优先级选择 |
| WireHandle | WebSocket/WebRTC 统一封装 |

### Message 处理（接收）

#### State Path (RPC 消息)

```
接收 → InboundPacketDispatcher
  ↓
Mailbox.enqueue(from, data, priority)
  ↓
MailboxProcessor (dequeue 循环)
  ├─ dequeue(batch)
  ├─ handler(msg)
  └─ ack(msg_id)  ✅ 可靠处理
      ↓
Scheduler → Actor
```

**延迟**: ~30-40ms（持久化 + 调度）

#### Fast Path (Stream 消息)

```
接收 → Registry.dispatch(stream_id, data)
  ↓
callback(data)  ⭐ 直接回调，绕过 Mailbox
```

**延迟**:
- WebSocket→DOM: ~3-4ms
- WebRTC (DOM 本地): ~1-2ms ⭐
- WebRTC Track: < 1ms

## 关键设计模式

### 事件驱动（零轮询）

```rust
// ✅ 正确（事件驱动）
let mut watcher = wire_pool.subscribe_changes();
loop {
    if ready { break; }
    watcher.changed().await;  ⭐
}
```

### SW 主控

- **SW (主控)**:
  - OutprocTransportManager (发送)
  - InboundPacketDispatcher (接收 RPC)
  - Mailbox + Scheduler (Actor)

- **DOM (辅助)**:
  - WebRtcCoordinator (创建 P2P)
  - WebRtcDataChannelReceiver (接收 Stream)
  - 快车道 Registry (Fast Path)

### 双路径

- **State Path (RPC)**:
  - 持久化 Mailbox
  - 可靠处理（dequeue-ack）
  - 优先级队列
  - 延迟 ~30-40ms

- **Fast Path (Stream/Media)**:
  - 零持久化
  - 直接回调
  - 绕过 Mailbox 和 Scheduler
  - 延迟 < 5ms

## 与 actr 对比

| 组件 | actr | actr-web | 一致性 |
|------|------|----------|--------|
| OutprocTransportManager | ✓ | ✓ | 100% |
| DestTransport | ✓ | ✓ | 100% |
| WirePool | ✓ | ✓ | 95% |
| InboundPacketDispatcher | ✓ | ✓ | 100% |
| Mailbox | SQLite | IndexedDB | 95% |
| DataStreamRegistry | ✓ | ✓ | 100% |

**总体一致性**: 95%+

差异仅在存储和并发模型（WASM 限制），功能完全等价。

## 性能指标

| 指标 | 值 |
|------|-----|
| 首次发送 | 50ms (WebSocket 立即) |
| State Path | ~30-40ms |
| Fast Path (WebSocket) | ~3-4ms |
| Fast Path (WebRTC 本地) | ~1-2ms |
| MediaTrack | < 1ms |

## 后续工作

- [x] Actor Scheduler 完整实现
- [x] WebRTC 连接逻辑完善（完整传输栈 + ICE restart + MessagePort 桥接）
- [ ] 端到端测试
- [ ] 性能优化（批量、压缩）

---

**相关文档**:
- [双层架构设计](./dual-layer.md) - State Path vs Fast Path 详细设计
- [API 层设计](./api-layer.md) - OutGate/Context/ActrRef 实现
- [完成度评估](./completion-status.md) - 相对 actr 的完成度分析
