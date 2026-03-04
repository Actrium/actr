# SW 如何使用 WebRTC 发送消息

## 核心设计：MessagePort 桥接

虽然 Service Worker **无法直接访问 WebRTC API**，但可以通过 **MessagePort** 间接使用 WebRTC。

## 🏗️ 架构设计

```
┌─────────────────────────────────────────────────────┐
│              Service Worker (SW)                     │
│                                                      │
│  OutGate → OutprocTransportManager                  │
│                ↓                                     │
│           DestTransport                             │
│                ↓                                     │
│           WirePool                                  │
│                ↓                                     │
│      ┌─────────┴─────────┐                         │
│      │                   │                          │
│  WebSocket          WebRTC Wire                     │
│  (直接发送)         (通过 MessagePort)              │
│      │                   │                          │
│      │                   ↓                          │
│      │            MessagePort.postMessage()         │
│      │                   │                          │
└──────┼───────────────────┼──────────────────────────┘
       │                   │
       │                   │ Transferable MessagePort
       │                   ↓
┌──────┼───────────────────┼──────────────────────────┐
│      │                   │         DOM              │
│      │                   ↓                          │
│      │           MessagePort.onmessage              │
│      │                   ↓                          │
│      │           RtcDataChannel.send()              │
│      │                   │                          │
│      ↓                   ↓                          │
│   WebSocket          WebRTC DC                      │
│      │                   │                          │
└──────┼───────────────────┼──────────────────────────┘
       │                   │
       ↓                   ↓
    🌍 远程节点
```

## 📋 完整流程

### 阶段 1️⃣：连接建立

```
1. SW: DestTransport 创建连接
   │
   ↓
2. SW: WireBuilder.create_connections()
   │
   ├─ 创建 WebSocket fallback (立即可用)
   │
   └─ 请求 DOM 创建 P2P
      │ PostMessage: { cmd: "create_p2p", peer_id }
      ↓
3. DOM: 收到请求
   │
   ├─ 创建 PeerConnection
   ├─ 创建 DataChannel
   ├─ 执行 SDP 交换
   └─ ICE 连接建立
      │
      ↓
4. DOM: P2P 就绪后
   │
   ├─ 创建 MessageChannel
   ├─ 绑定 DataChannel ↔ MessagePort
   └─ Transfer MessagePort 到 SW
      │ PostMessage: {
      │   cmd: "p2p_ready",
      │   port: messagePort  // Transferable!
      │ }
      ↓
5. SW: 收到 MessagePort
   │
   ├─ 创建 WebRtcConnection(messagePort)
   ├─ 包装为 WireHandle::WebRTC
   └─ 添加到 WirePool
      │
      ↓
6. WirePool 通知 DestTransport
   │
   ↓
7. WebRTC 连接就绪，优先级高于 WebSocket
```

**代码位置**：
- SW 请求: `crates/runtime-sw/src/transport/wire_builder.rs:58-94`
- SW 创建连接: `crates/runtime-sw/src/transport/wire_builder.rs:116-132`

### 阶段 2️⃣：消息发送

```
SW: Actor 调用
   │ ctx.call(peer_id, request)
   ↓
SW: OutGate.send_request()
   ↓
SW: OutprocTransportManager.send()
   ↓
SW: DestTransport.send()
   │
   ├─ 优先级判断：WebRTC > WebSocket ✅
   ↓
SW: WirePool.get_connection(WebRTC)
   ↓
SW: WireHandle::WebRTC.get_lane()
   ↓
SW: WebRtcConnection.get_lane()
   │ ✅ 已实现：基于 datachannel_port 创建 DataLane::PostMessage
   │ 并缓存到 lane_cache (DashMap<PayloadType, DataLane>)
   ↓
SW: DataLane::MessagePort
   │ messagePort.postMessage(data)
   ↓
   ┊ (transferable 传输)
   ↓
DOM: MessagePort.onmessage
   │
   ↓
DOM: RtcDataChannel.send(data)
   │
   ↓
🌍 远程节点
```

**代码位置**：
- WirePool 管理: `crates/runtime-sw/src/transport/wire_pool.rs:20-29`
- WebRtcConnection: `crates/runtime-sw/src/transport/wire_handle.rs:14-88`
- get_lane TODO: `crates/runtime-sw/src/transport/wire_handle.rs` (✅ 已实现)

## 🔑 关键技术点

### 1. MessagePort 的 Transferable 特性

```javascript
// DOM 侧
const channel = new MessageChannel();
const port1 = channel.port1;
const port2 = channel.port2;

// 绑定 DataChannel
dataChannel.onmessage = (e) => {
  port1.postMessage(e.data);
};

port1.onmessage = (e) => {
  dataChannel.send(e.data);
};

// Transfer port2 到 SW
navigator.serviceWorker.controller.postMessage(
  { cmd: 'p2p_ready', port: port2 },
  [port2]  // ← Transferable，所有权转移
);
```

```rust
// SW 侧 (Rust/WASM)
let port: MessagePort = /* 从 PostMessage 接收 */;

// 创建 WebRTC 连接
let mut rtc_conn = WebRtcConnection::new(peer_id);
rtc_conn.set_datachannel_port(port);

// 后续通过 port 收发数据
port.post_message(&data)?;
```

### 2. WirePool 的优先级机制

```rust
// WireHandle 优先级
impl WireHandle {
    pub fn priority(&self) -> u8 {
        match self {
            WireHandle::WebSocket(_) => 0,
            WireHandle::WebRTC(_) => 1,  // ← 更高优先级
        }
    }
}

// DestTransport 会优先使用 WebRTC
for &conn_type in &conn_types {
    if ready_set.contains(&conn_type) {
        if let Some(wire) = wire_pool.get_connection(conn_type).await {
            return wire.get_lane(payload_type).await;
        }
    }
}
```

**代码位置**：
- 优先级: `crates/runtime-sw/src/transport/wire_handle.rs:135-140`
- 选择逻辑: `crates/runtime-sw/src/transport/dest_transport.rs:1`

## 📊 与 WebSocket 对比

| 特性 | WebSocket (SW) | WebRTC (SW 通过 MessagePort) |
|-----|----------------|------------------------------|
| **创建** | SW 直接创建 | DOM 创建，SW 持有 MessagePort |
| **发送** | `ws.send()` | `messagePort.postMessage()` → DOM → `dc.send()` |
| **优先级** | 低 (0) | 高 (1) |
| **建立时间** | 快 (~100ms) | 慢 (~1-2s，需要 ICE) |
| **延迟** | 中 (~10-20ms) | 低 (~1-2ms，P2P) |
| **适用场景** | 客户端-服务器 | P2P 点对点 |

## ✅ 当前实现状态

### 已完成 ✅
- WireHandle 支持 WebSocket 和 WebRTC
- WirePool 管理两种连接（优先级：WebRTC > WebSocket）
- WebWireBuilder 异步请求 DOM 创建 P2P
- **WebRtcConnection::get_lane()** — 通过 lane_cache + DataLane::PostMessage 实现
- **完整出站传输栈已接入**：
  - OutGate::OutprocOut → OutprocOutGate → OutprocTransportManager → DestTransport → WirePool → DataLane::PostMessage
  - System.MessageHandler → OutGate.send_message()
  - 响应路由：OutGate.try_handle_response → InprocOutGate.handle_response
- **DOM 侧 MessageChannel 桥接已实现**：
  - DataChannel open 时自动创建 MessageChannel
  - port1 留在 DOM（DataChannel ↔ port1 双向转发）
  - port2 通过 Transferable 转移给 SW
  - SW 通过 `register_datachannel_port` 注入 WirePool
- **OutprocTransportManager.inject_connection()** 支持动态注入连接
- **DestTransport.wire_pool()** 公开访问器
- **ICE restart** 完整实现（检测 + 重试 + 回退）

### 待完善 ⚠️

- **WebWireBuilder Peer fallback URL** 当前为硬编码占位符
- **WebRtcRecoveryManager** DOM 通知重建回路未完全闭合

**代码位置**：
- get_lane 实现: `crates/runtime-sw/src/transport/wire_handle.rs`
- OutGate 路由: `crates/runtime-sw/src/system.rs:init_message_handler`
- OutprocOutGate: `crates/runtime-sw/src/outbound/outproc_out_gate.rs`
- Register port: `crates/runtime-sw/src/client_runtime.rs:register_datachannel_port`
- DOM 桥接: `packages/actr-dom/src/webrtc-coordinator.ts:attachDataChannel`

## 🎯 设计优势

1. **绕过浏览器限制**
   - SW 无法直接用 WebRTC
   - 通过 MessagePort 间接使用

2. **统一接口**
   - OutGate 层无感知
   - WireHandle 统一抽象

3. **优先级自动切换**
   - WebSocket fallback 立即可用
   - WebRTC 就绪后自动切换

4. **性能最优**
   - P2P 直连，延迟最低
   - Transferable 零拷贝

## 💡 总结

**问题**：SW 出口涉及 WebRTC 吗？

**答案**：**是的！** 但通过特殊设计：

1. **架构上**：SW 的 WirePool 包含 WebRTC Wire
2. **实现上**：通过 MessagePort 桥接
3. **当前状态**：SW 端和 DOM 端均已完成（完整传输栈 + MessageChannel 桥接 + register_datachannel_port 注入）

---

**相关文档**：
- [消息 I/O 入口出口](./message-io-entry-exit.md)
- [架构总览](./overview.md)
