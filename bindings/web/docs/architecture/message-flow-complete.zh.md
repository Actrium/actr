# Actor-RTC Web 完整消息流图

> **历史/已被取代的详细图**：本文早期把 Fast Path 描述为 DOM 本地 `DomSystem` / Registry callback。当前源码已经走 Option U + wasm-bindgen：DOM 侧通过 `FastPathForwarder` 发送 `fast_path_data`，SW 侧 `handle_dom_fast_path` 调用 runtime fast path 并派发 stream handlers。若本文下方细节与 [架构总览](./overview.zh.md) 冲突，以架构总览和源码为准。

## 全览图：消息在 SW 和 DOM 间的完整流转

```
                        远程节点 (Remote Node)
                               │
                               │
              ┌────────────────┼────────────────┐
              │                │                │
         WebSocket          WebRTC         WebRTC
         (Server)        (DataChannel)  (MediaTrack)
              │                │                │
              │                │                │
═══════════════════════════════════════════════════════════════
              │                │                │
              ↓                ↓                ↓
┌─────────────────────────────────────────────────────────────┐
│                    Service Worker (SW)                       │
│                     sw-host crate                         │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐ │
│  │           📥 INBOUND (接收流程)                        │ │
│  └────────────────────────────────────────────────────────┘ │
│                                                              │
│  WebSocket Lane ──┐                                         │
│         │         │                                          │
│         │         ↓                                          │
│         │   ┌──────────────────┐                            │
│         │   │ 消息类型判断      │                            │
│         │   └──────┬───────────┘                            │
│         │          │                                         │
│         │    ┌─────┴──────┐                                 │
│         │    │            │                                  │
│         │  RPC/*      STREAM_*                              │
│         │    │            │                                  │
│         │    │            │ (转发到 DOM)                     │
│         │    │            └──────────────────────┐          │
│         │    │                                   │           │
│         │    ↓                                   │           │
│         │  ┌─────────────────────┐              │           │
│         │  │ InboundPacket       │              │           │
│         │  │   Dispatcher        │              │           │
│         │  └──────┬──────────────┘              │           │
│         │         │                              │           │
│         │         ↓                              │           │
│         │  ┌─────────────────────┐              │           │
│         │  │ Mailbox (IndexedDB) │              │           │
│         │  │   - enqueue()       │              │           │
│         │  │   - 优先级队列      │              │           │
│         │  └──────┬──────────────┘              │           │
│         │         │                              │           │
│         │         ↓                              │           │
│         │  ┌─────────────────────┐              │           │
│         │  │ MailboxProcessor    │              │           │
│         │  │   - dequeue()       │              │           │
│         │  │   - ack()           │              │           │
│         │  └──────┬──────────────┘              │           │
│         │         │                              │           │
│         │         ↓                              │           │
│         │  ┌─────────────────────┐              │           │
│         │  │ Scheduler           │              │           │
│         │  │   (串行调度)        │              │           │
│         │  └──────┬──────────────┘              │           │
│         │         │                              │           │
│         │         ↓                              │           │
│         │  ┌─────────────────────┐              │           │
│         │  │ Actor 实例          │              │           │
│         │  │   - handle_call()   │              │           │
│         │  │   - WebContext      │              │           │
│         │  └─────────────────────┘              │           │
│         │                                        │           │
│         │  延迟: 需 benchmark 确认 (State Path) │           │
│         │                                        │           │
│  ┌────────────────────────────────────────────────────────┐ │
│  │           📤 OUTBOUND (发送流程)                       │ │
│  └────────────────────────────────────────────────────────┘ │
│                                                              │
│         ┌─────────────────────┐                             │
│         │ Gate                │                             │
│         │  - HostGate         │ (同 SW 内部)                │
│         │  - PeerGate         │ (跨节点)                    │
│         └──────┬──────────────┘                             │
│                │                                             │
│                ↓                                             │
│         ┌─────────────────────┐                             │
│         │ PeerTransport       │                             │
│         │ / TransportManager  │                             │
│         └──────┬──────────────┘                             │
│                │                                             │
│                ↓                                             │
│         ┌─────────────────────┐                             │
│         │ DestTransport       │                             │
│         │  (每个目标节点)     │                             │
│         └──────┬──────────────┘                             │
│                │                                             │
│                ↓                                             │
│         ┌─────────────────────┐                             │
│         │ WirePool            │                             │
│         │  - WebSocket Wire   │                             │
│         │  - (等待 WebRTC)    │                             │
│         └──────┬──────────────┘                             │
│                │                                             │
│                ↓                                             │
│           WebSocket Lane                                     │
│                │                                             │
└────────────────┼─────────────────────────────────────────────┘
                 │                 ↑
                 │  PostMessage    │  PostMessage
                 │  (STREAM_* 转发)│  (控制信令)
                 ↓                 │
═══════════════════════════════════════════════════════════════
┌────────────────┴─────────────────┬───────────────────────────┐
│                  DOM/Window                                   │
│                  dom-bridge crate                            │
│                                                               │
│  ┌────────────────────────────────────────────────────────┐  │
│  │           📥 INBOUND (快速接收)                        │  │
│  └────────────────────────────────────────────────────────┘  │
│                                                               │
│  WebRTC DataChannel ─┐   WebRTC MediaTrack ─┐               │
│         │             │          │            │               │
│         │             ↓          │            ↓               │
│         │      ┌──────────────┐ │     ┌──────────────┐      │
│         │      │ 消息类型判断 │ │     │ RTP 解包     │      │
│         │      └──────┬───────┘ │     └──────┬───────┘      │
│         │             │          │            │               │
│         │       ┌─────┴──────┐  │            │               │
│         │       │            │  │            │               │
│         │     RPC/*      STREAM_*│            │               │
│         │       │            │  │            │               │
│         │       │ (转发 SW)  │  │            │               │
│         │       │            ↓  │            ↓               │
│         │       │      ┌──────────────────────────┐         │
│         │       │      │ FastPathForwarder        │         │
│         │       │      │  ┌────────────────────┐  │         │
│         │       │      │  │ ServiceWorker      │  │         │
│         │       │      │  │   Bridge           │  │         │
│         │       │      │  │  fast_path_data    │  │         │
│         │       │      │  └─────┬──────────────┘  │         │
│         │       │      │        │                 │         │
│         │       │      │  ┌─────▼──────────────┐  │         │
│         │       │      │  │ SW handle_dom      │  │         │
│         │       │      │  │   _fast_path       │  │         │
│         │       │      │  │  stream handlers   │  │         │
│         │       │      │  └─────┬──────────────┘  │         │
│         │       │      └────────┼─────────────────┘         │
│         │       │               │                            │
│         │       │               ↓                            │
│         │       │         转发给 SW runtime                  │
│         │       │         绕过 Mailbox/Scheduler            │
│         │       │                                            │
│         │       │         延迟:                              │
│         │       │         - DataChannel: 当前需实测         │
│         │       │         - MediaTrack: 当前需实测          │
│         │       │                                            │
│         │       └────────────────┐                          │
│         │                        │                           │
│         └─────PostMessage────────┤                          │
│                 (转发到 SW)      │                           │
│                                  │                           │
│  ┌────────────────────────────────────────────────────────┐ │
│  │           📤 OUTBOUND (发送准备)                       │ │
│  └────────────────────────────────────────────────────────┘ │
│                                                              │
│         ┌─────────────────────┐                             │
│         │ WebRtcCoordinator   │                             │
│         │  - createOffer()    │                             │
│         │  - addIceCandidate()│                             │
│         │  - onDataChannel()  │                             │
│         │  - onTrack()        │                             │
│         └──────┬──────────────┘                             │
│                │                                             │
│                ├─ 创建 PeerConnection                        │
│                ├─ 协商 SDP                                   │
│                ├─ 建立 ICE 连接                              │
│                └─ 通知 SW "P2P Ready"                        │
│                                                              │
│                   ↓  (信令通过 PostMessage)                  │
└──────────────────┼──────────────────────────────────────────┘
                   │
═══════════════════════════════════════════════════════════════
                   ↓
              远程节点 (Remote Node)


═══════════════════════════════════════════════════════════════
                      图例说明
═══════════════════════════════════════════════════════════════

📥 INBOUND  = 接收流程（从远程 → 本地）
📤 OUTBOUND = 发送流程（从本地 → 远程）

RPC/*      = State Path 消息（RPC_REQUEST, RPC_RESPONSE 等）
STREAM_*   = Fast Path 消息（STREAM_RELIABLE, STREAM_LATENCY_FIRST）
MEDIA_RTP  = MediaTrack 消息（音视频 RTP 包）

PostMessage = SW ↔ DOM 的浏览器 API 通信
WebSocket   = SW ↔ Server 的长连接
WebRTC      = DOM ↔ Peer 的 P2P 连接

═══════════════════════════════════════════════════════════════
```

## 关键消息流详解

### 流程 1️⃣：RPC 消息接收（State Path）

```
远程节点
   │ WebSocket
   ↓
SW: WebSocket Lane
   │
   ↓
SW: InboundPacketDispatcher
   │ (判断类型 = RPC_REQUEST)
   ↓
SW: Mailbox.enqueue()
   │ (持久化到 IndexedDB)
   ↓
SW: MailboxProcessor.dequeue()
   │
   ↓
SW: Scheduler (串行调度)
   │
   ↓
SW: Actor.handle_call(ctx: &WebContext)
   │ (业务逻辑处理)
   ↓
SW: Gate.send_request()
   │ (发送响应)
   ↓
SW: WebSocket Lane
   │
   ↓
远程节点

延迟: 需当前 benchmark 确认
特点: 可靠、有序、持久化
```

### 流程 2️⃣：Stream 消息接收（Fast Path）

```
远程节点
   │ WebRTC DataChannel
   ↓
DOM: WebRtcDataChannelReceiver
   │ (判断类型 = STREAM_RELIABLE)
   ↓
DOM: FastPathForwarder.forward(stream_id, data)
   │
   ↓
DOM: ServiceWorkerBridge.sendToSW({ type: "fast_path_data" }, [buffer])
   ↓
SW: actor.sw.js → wasm_bindgen.handle_dom_fast_path(client_id, payload)
   ↓
SW: runtime.handle_fast_path()
   ↓
SW: stream handler callback(data)
   ↓
用户代码处理

延迟: 当前不声明固定数字，需以 e2e/benchmark 为准
特点: 绕过 Mailbox/Scheduler、Transferable 转发、处理点在 SW
```

### 流程 3️⃣：MediaTrack 接收（超快速路径）

```
远程节点
   │ WebRTC MediaTrack (RTP)
   ↓
DOM: ontrack event
   │
   ↓
DOM: WebRTC coordinator / media handling
   │
   ↓
DOM/SW: 按当前实现路径转发或处理
   ↓
DOM: callback(rtp_packet)
   │ (零拷贝，直接播放)
   ↓
<video> / <audio> 元素

延迟: 当前需实测
特点: 浏览器原生优化，零拷贝
```

### 流程 4️⃣：跨域消息转发（WebSocket → DOM）

```
远程节点
   │ WebSocket (发送 STREAM_*)
   ↓
SW: WebSocket Lane
   │ (判断类型 = STREAM_RELIABLE)
   │ (SW 无法处理 Stream，需转发 DOM)
   ↓
SW/DOM: 按当前 fast_path_data / handler 路径处理
   │ { type: "fast_path_data", payload }
   ↓
DOM: onmessage handler
   │
   ↓
DOM: FastPathForwarder.forward()
   │
   ↓
DOM: callback(data)

延迟: 当前需实测（包含 PostMessage/MessagePort 开销）
```

### 流程 5️⃣：发送消息（Actor → 远程）

```
SW: Actor 内部
   │
   ↓
SW: ctx.call(target_id, request)
   │ (WebContext trait)
   ↓
SW: Gate.send_request()
   │
   ↓
SW: PeerTransport
   │ (查找目标节点)
   ↓
SW: DestTransport
   │ (选择最佳连接)
   ↓
SW: WirePool
   │ (优先级: WebRTC > WebSocket)
   ↓
SW: WireHandle::send()
   │
   ├─ WebSocket Lane → 远程节点
   │
   └─ (WebRTC 需通过 DOM)
      │
      SW → DOM: PostMessage
      │ { type: "SEND_P2P", data }
      ↓
      DOM: WebRTC DataChannel.send()
      │
      ↓
      远程节点
```

## 性能对比表

| 消息类型 | 路径 | 延迟 | 用途 |
|---------|------|------|------|
| RPC_REQUEST | WebSocket → SW → Mailbox → Actor | 需当前 benchmark 确认 | 业务逻辑、状态变更 |
| RPC_RESPONSE | 同上 | 需当前 benchmark 确认 | RPC 响应 |
| STREAM_RELIABLE (WebSocket) | WebSocket → SW fast path / handlers | 需实测 | 文件传输、可靠流 |
| STREAM_RELIABLE (WebRTC) | WebRTC DC → DOM FastPathForwarder → SW handlers | 需实测 | 低延迟流 |
| MEDIA_RTP | WebRTC Track → DOM/当前媒体路径 | 需实测 | 音视频实时流 |

## 设计亮点

1. **智能路由**：SW 自动判断消息类型，RPC 走 State Path，Stream 转发 DOM
2. **零拷贝**：MediaTrack 直接到 DOM，无序列化
3. **事件驱动**：全程无轮询，使用 watch + notify
4. **职责分离**：SW 管状态，DOM 管快速数据
5. **双保险**：State Path 可靠持久，Fast Path 低延迟

---

**文档版本**: 2026-01-08（拆分完成后）
**相关文档**:
- [架构总览](./overview.zh.md)
- [双层架构](./dual-layer.zh.md)
