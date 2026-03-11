# 消息入口和出口详解

## ❌ 常见误解

> "所有消息入口都在 SW" - **错误！**

实际上：
- **入口**：分布在 SW 和 DOM
- **出口控制**：在 SW（Gate）
- **出口执行**：分布在 SW 和 DOM

## 📥 消息入口（INBOUND）

```
                      远程节点
                         │
        ┌────────────────┼────────────────┐
        │                │                │
   WebSocket        WebRTC DC       WebRTC Track
        │                │                │
        │                │                │
═══════════════════════════════════════════════════════
        │                │                │
        ↓                ↓                ↓
   ┌────────┐      ┌──────────────────────────┐
   │   SW   │      │         DOM              │
   └────────┘      └──────────────────────────┘
```

### 入口 1️⃣：WebSocket → SW

```
远程节点
   │ WebSocket
   ↓
SW: WebSocket Lane (web_sys::WebSocket)
   │ onmessage event
   ↓
SW: DataLane.recv()
   │
   ↓
SW: 消息类型判断
   ├─ RPC_* → InboundDispatcher → Mailbox
   └─ STREAM_* → PostMessage转发 → DOM

位置: crates/runtime-sw/src/transport/lane.rs:20-25
```

**特点**：
- ✅ SW 可以直接接收 WebSocket 消息
- ✅ RPC 消息直接进入 State Path（最快）
- ⚠️ Stream 消息需要转发到 DOM

### 入口 2️⃣：WebRTC DataChannel → DOM

```
远程节点
   │ WebRTC DataChannel
   ↓
DOM: RtcDataChannel (web_sys::RtcDataChannel)
   │ onmessage event
   ↓
DOM: WebRtcDataChannelReceiver
   │
   ↓
DOM: 消息类型判断
   ├─ RPC_* → PostMessage转发 → SW Mailbox
   └─ STREAM_* → DomSystem.dispatch_stream() → 本地callback

位置: crates/runtime-dom/src/inbound/webrtc_receiver.rs:1
```

**特点**：
- ❌ SW 无法访问 WebRTC API
- ✅ Stream 消息本地处理（最快）
- ⚠️ RPC 消息需要转发到 SW

### 入口 3️⃣：WebRTC MediaTrack → DOM

```
远程节点
   │ WebRTC MediaTrack (RTP)
   ↓
DOM: PeerConnection.ontrack
   │ MediaStreamTrack
   ↓
DOM: DomSystem.dispatch_media_frame()
   │
   ↓
DOM: MediaFrameHandlerRegistry.dispatch()
   │
   ↓
用户 callback(rtp_packet)

位置: crates/runtime-dom/src/transport/webrtc_mediatrack.rs:1
```

**特点**：
- 🚀 最快路径（< 1ms）
- 🎯 零拷贝
- ❌ 只能在 DOM 接收

### 入口 4️⃣：PostMessage（双向）

```
SW ←──→ DOM
   PostMessage

用途：
- SW → DOM: 转发 STREAM_* 消息
- DOM → SW: 转发 RPC_* 消息
- DOM → SW: WebRTC 控制信令
```

## 📤 消息出口（OUTBOUND）

### 出口控制层：全在 SW

```
SW: Actor 内部
   │
   ↓
SW: ctx.call(target_id, request)
   │ WebContext trait
   ↓
SW: Gate.send_request()
   │
   ├─ HostGate (同 SW 内)
   │    └─ 直接调用 handler
   │
   └─ PeerGate (跨节点)
        │
        ↓
     PeerTransport
        │
        ↓
     DestTransport
        │
        ↓
     WirePool (选择最佳连接)
        │
        ├─ WebSocket Wire
        └─ WebRTC Wire
```

**位置**：
- `crates/runtime-sw/src/outbound/mod.rs:1` - Gate
- `crates/runtime-sw/src/transport/outproc_transport_manager.rs:1`
- `crates/runtime-sw/src/transport/dest_transport.rs:1`

### 出口执行层：分布在 SW 和 DOM

#### 出口 A：WebSocket（SW 直接发送）

```
SW: PeerTransport
   │
   ↓
SW: DestTransport.send()
   │
   ↓
SW: WireHandle::WebSocket
   │
   ↓
SW: WebSocketConnection.get_lane()
   │
   ↓
SW: DataLane::WebSocket.send()
   │ web_sys::WebSocket.send()
   ↓
远程节点

位置: crates/runtime-sw/src/transport/lane.rs:45-60
```

**特点**：
- ✅ SW 可以直接发送
- ✅ 无需 DOM 参与
- ✅ 发送路径最简单

#### 出口 B：WebRTC（通过 MessagePort 桥接）

```
SW: PeerTransport
   │
   ↓
SW: DestTransport.send()
   │
   ↓
SW: WireHandle::WebRTC
   │ (持有 DOM 转移过来的 MessagePort)
   ↓
SW: WebRtcConnection.get_lane()
   │
   ↓
SW: DataLane::MessagePort.send()
   │ messagePort.postMessage(data)  ← Transferable!
   ↓
   ┊ (零拷贝传输)
   ↓
DOM: MessagePort.onmessage
   │
   ↓
DOM: RtcDataChannel.send()
   ↓
远程节点

当前状态: ✅ 已实现 (WireHandle::get_lane 返回 DataLane::PostMessage)
设计详解: sw-webrtc-design.md
```

**特点**：
- ✅ SW 通过 MessagePort 间接使用 WebRTC
- ✅ MessagePort 是 Transferable（零拷贝）
- ⚠️ 需要 DOM 先创建 PeerConnection
- ⚠️ 核心逻辑待实现（TODO）

**关键设计**：
1. DOM 创建 PeerConnection + DataChannel
2. DOM 创建 MessageChannel，绑定 DataChannel
3. DOM Transfer MessagePort 到 SW
4. SW 通过 MessagePort 收发数据
5. DOM 在后台转发 MessagePort ↔ DataChannel

## 📊 入口/出口对比表

| 传输方式 | 入口位置 | 出口位置 | RPC 延迟 | Stream 延迟 |
|---------|---------|---------|----------|------------|
| **WebSocket** | SW | SW | 最佳 (~30ms) | 较慢 (~4ms，需转发DOM) |
| **WebRTC DC** | DOM | DOM | 较慢 (~35ms，需转发SW) | 最佳 (~1-2ms) |
| **WebRTC Track** | DOM | DOM | N/A | 最快 (< 1ms) |

## 🎯 最佳实践

### 场景 1：RPC 消息

**最佳方案**：WebSocket（SW → SW）

```
发送: SW Gate → WebSocket → 远程
接收: 远程 → WebSocket → SW InboundDispatcher
```

**优势**：
- ✅ 无需跨进程转发
- ✅ 直接进入 Mailbox

**避免**：WebRTC（需要 2 次 PostMessage）

### 场景 2：Stream 消息

**最佳方案**：WebRTC DataChannel（DOM → DOM）

```
发送: DOM → WebRTC DC → 远程
接收: 远程 → WebRTC DC → DOM Fast Path
```

**优势**：
- ✅ 本地处理，延迟最低
- ✅ 无需跨进程转发

**避免**：WebSocket（需要 SW → DOM 转发）

### 场景 3：媒体流

**唯一方案**：WebRTC MediaTrack（DOM → DOM）

```
发送: DOM → MediaStreamTrack → 远程
接收: 远程 → MediaStreamTrack → DOM Fast Path
```

**优势**：
- 🚀 浏览器原生优化
- 🎯 零拷贝
- 🎬 实时性最佳

## 🔍 架构设计要点

### 为什么入口分布？

1. **WebSocket**：SW 可访问
   - Service Worker 支持 WebSocket API
   - 最适合 RPC（控制平面）

2. **WebRTC**：只有 DOM 可访问
   - Service Worker **无法创建** PeerConnection
   - 最适合 Stream/Media（数据平面）

### 为什么出口控制在 SW？

1. **统一管理**：
   - Actor 逻辑在 SW
   - Gate 在 SW
   - 路由决策在 SW
   - WirePool 优先级管理在 SW

2. **实际执行**：
   - WebSocket → SW 直接发送
   - WebRTC → SW 通过 MessagePort 发送（DOM 后台转发）

### SW 如何使用 WebRTC？

**巧妙设计**：通过 **MessagePort** 桥接

```
1. DOM 创建 PeerConnection 和 DataChannel
2. DOM 创建 MessageChannel
3. DOM 绑定: DataChannel ↔ MessagePort
4. DOM Transfer MessagePort 到 SW (Transferable!)
5. SW 持有 MessagePort，通过它收发数据
6. DOM 在后台透明转发
```

**详细设计**：参见 [SW WebRTC 设计](./sw-webrtc-design.zh.md)

## 💡 关键洞察

1. **入口不全在 SW**
   - WebSocket ✅ SW
   - WebRTC ❌ DOM only

2. **出口控制在 SW，执行分布**
   - 决策：SW（Gate + WirePool）
   - WebSocket 发送：SW 直接
   - WebRTC 发送：SW 通过 MessagePort（DOM 后台转发）

3. **SW 确实涉及 WebRTC**
   - ✅ WireHandle 包含 WebRTC
   - ✅ WirePool 管理 WebRTC 连接
   - ✅ 优先级：WebRTC > WebSocket
   - ⚠️ 通过 MessagePort 间接使用

4. **最优路径**
   - RPC：WebSocket（SW → SW）
   - Stream：WebRTC DC（SW → MessagePort → DOM → DC）
   - Media：WebRTC Track（DOM → DOM）

5. **跨进程开销**
   - MessagePort Transferable：零拷贝 ✅
   - PostMessage 普通数据：序列化开销 ⚠️

---

**相关文档**：
- [消息流全览图](./message-flow-visual.zh.md) - 完整流程图
- [SW WebRTC 设计](./sw-webrtc-design.zh.md) - MessagePort 桥接详解 🔥
- [架构总览](./overview.zh.md) - 整体设计
