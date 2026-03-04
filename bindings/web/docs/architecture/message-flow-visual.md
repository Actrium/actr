# 消息流可视化全景图

## 一图看懂：所有消息如何在 SW 和 DOM 间穿梭

```
                             🌍 远程节点
                                  │
                   ┌──────────────┼──────────────┐
                   │              │              │
              WebSocket      WebRTC DC     WebRTC Track
                   │              │              │
═══════════════════════════════════════════════════════════════════════════════
                   │              │              │
                   ↓              │              │
        ┏━━━━━━━━━━━━━━━━━━━━━━━━┓│              │
        ┃    Service Worker      ┃│              │
        ┃    (主控 + State)      ┃│              │
        ┗━━━━━━━━━━━━━━━━━━━━━━━━┛│              │
                   │              │              │
         ┌─────────┴────────┐     │              │
         │ 消息类型判断      │     │              │
         └─────┬────────┬───┘     │              │
               │        │         │              │
           RPC/*    STREAM_*      │              │
               │        │         │              │
               │        │         │              │
    ┌──────────┘        └─────────┼──────┐      │
    │                              │      │      │
    ↓                              ↓      │      │
┌─────────────────────┐    ┌──────────────┼──────┼───────┐
│ 📬 State Path       │    │  PostMessage │      │       │
│  (慢车道 30-40ms)   │    │      ↓       │      │       │
│                     │    └──────────────┼──────┼───────┘
│ InboundDispatcher   │                   │      │
│        ↓            │                   │      │
│ Mailbox (IndexedDB) │                   │      │
│        ↓            │                   │      │
│ MailboxProcessor    │                   │      │
│        ↓            │                   │      │
│ Scheduler           │                   │      │
│        ↓            │                   │      │
│ Actor (WebContext)  │                   │      │
│        ↓            │                   │      │
│ OutGate 发送 ───────┼───────────────────┘      │
│                     │                          │
└─────────────────────┘                          │
                                                 │
═══════════════════════════════════════════════════════════════════════════════
                                                 │
        ┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓│
        ┃           DOM/Window                  ┃│
        ┃      (辅助 + Fast Path)               ┃│
        ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛│
                                                 │
                 ┌───────────────────────────────┘
                 │
                 ↓
        ┌─────────────────┐
        │ 消息类型判断     │
        └────┬──────┬─────┘
             │      │
         STREAM_* MEDIA_RTP
             │      │
             ↓      ↓
    ┌────────────────────────────────┐
    │ ⚡ Fast Path                   │
    │   (快车道 1-3ms)               │
    │                                │
    │  DomSystem                     │
    │    ├─ StreamHandlerRegistry    │
    │    │    stream_id → callback   │
    │    │         ↓                 │
    │    │    callback(data) 并发    │
    │    │                           │
    │    └─ MediaFrameHandlerRegistry│
    │         track_id → callback    │
    │              ↓                 │
    │         callback(rtp) 零拷贝   │
    │                                │
    └────────────────────────────────┘
                   │
                   ↓
            用户应用处理
         (视频播放/文件处理)


═══════════════════════════════════════════════════════════════════════════════
                          核心流程对比
═══════════════════════════════════════════════════════════════════════════════

流程 A: RPC 消息（State Path - 可靠慢速）
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

🌍 远程节点
   │ WebSocket (RPC_REQUEST)
   ↓
SW │ InboundDispatcher (判断类型)
   ↓
SW │ Mailbox.enqueue() → IndexedDB 💾
   ↓
SW │ MailboxProcessor.dequeue() (批量)
   ↓
SW │ Scheduler (串行化)
   ↓
SW │ Actor.handle_call(ctx) (业务逻辑)
   ↓
SW │ ctx.call() → OutGate
   ↓
SW │ OutprocTransportManager
   ↓
SW │ WebSocket.send() (RPC_RESPONSE)
   ↓
🌍 远程节点

延迟: ~30-40ms
特点: ✅ 持久化 ✅ 有序 ✅ 可靠


流程 B: Stream 消息 via WebRTC（Fast Path - 极速）
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

🌍 远程节点
   │ WebRTC DataChannel (STREAM_RELIABLE)
   ↓
DOM│ WebRtcReceiver
   ↓
DOM│ DomSystem.dispatch_stream(stream_id, data)
   ↓
DOM│ StreamHandlerRegistry.dispatch()
   ↓
DOM│ callback(data) ⚡ 直接回调
   ↓
👤 用户代码处理

延迟: ~1-2ms
特点: ⚡ 极快 ⚡ 高吞吐 ⚠️ 无持久化


流程 C: Stream 消息 via WebSocket（跨域转发）
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

🌍 远程节点
   │ WebSocket (STREAM_RELIABLE)
   ↓
SW │ WebSocket Lane 接收
   ↓
SW │ 判断类型 = STREAM_* (无法在 SW 处理)
   ↓
SW │ PostMessage → DOM
   │ { type: "forward", stream_id, data }
   ↓
DOM│ onmessage handler
   ↓
DOM│ DomSystem.dispatch_stream()
   ↓
DOM│ callback(data)
   ↓
👤 用户代码处理

延迟: ~3-4ms (多了 PostMessage)
特点: ⚡ 快 ⚠️ 比 WebRTC 稍慢


流程 D: MediaTrack（超快速路径）
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

🌍 远程节点
   │ WebRTC MediaTrack (RTP packet)
   ↓
DOM│ PeerConnection.ontrack
   ↓
DOM│ DomSystem.dispatch_media_frame()
   ↓
DOM│ MediaFrameHandlerRegistry.dispatch()
   ↓
DOM│ callback(rtp) 🎯 零拷贝
   ↓
📺 <video>/<audio> 直接播放

延迟: < 1ms
特点: 🚀 最快 🎯 零拷贝 🎬 实时


流程 E: 控制平面（State Path 控制 Fast Path）
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

👤 用户调用 OpenStream
   ↓
SW │ RPC: OpenStreamRequest (走 State Path)
   ↓
SW │ Actor.handle_open_stream()
   │   - 生成 stream_id = "video_123"
   │   - 创建回调函数
   ↓
SW │ PostMessage → DOM
   │ { cmd: "register_stream", stream_id, ... }
   ↓
DOM│ DomSystem.register_stream_handler(stream_id, callback)
   │ (注册到 Registry)
   ↓
SW │ RPC Response { stream_id: "video_123" }
   ↓
👤 用户收到 stream_id

═══ 现在 Fast Path 轨道已建好 ═══

🌍 远程开始发送流数据
   │ STREAM_DATA { stream_id: "video_123", data }
   ↓
   (走流程 B 或 C，直接到 callback)

═══ 关闭流 ═══

👤 用户调用 CloseStream
   ↓
SW │ RPC: CloseStreamRequest (走 State Path)
   ↓
SW │ Actor.handle_close_stream()
   ↓
SW │ PostMessage → DOM
   │ { cmd: "unregister_stream", stream_id }
   ↓
DOM│ DomSystem.unregister_stream_handler(stream_id)
   ↓
SW │ RPC Response { ok: true }

═══════════════════════════════════════════════════════════════════════════════
                           设计精髓
═══════════════════════════════════════════════════════════════════════════════

1. 职责分离
   ├─ SW:  主控 + State Path (可靠、有序、持久化)
   └─ DOM: 辅助 + Fast Path (快速、高吞吐、实时)

2. 双路径协作
   ├─ State Path 负责"建轨道"（OpenStream/CloseStream）
   └─ Fast Path 负责"跑火车"（Stream Data）

3. 智能路由
   ├─ RPC/*     → 必走 SW State Path
   ├─ STREAM_*  → 优先 DOM Fast Path
   └─ MEDIA_RTP → 只能 DOM Fast Path

4. 零轮询设计
   └─ 全程事件驱动 (watch + notify)

5. 性能优化
   ├─ WebRTC > WebSocket (P2P 优先)
   ├─ MediaTrack 零拷贝
   └─ 批量处理 (Mailbox dequeue)

═══════════════════════════════════════════════════════════════════════════════
                           符号说明
═══════════════════════════════════════════════════════════════════════════════

🌍 = 远程节点
👤 = 用户代码
SW = Service Worker
DOM = DOM/Window
📬 = State Path (慢车道)
⚡ = Fast Path (快车道)
💾 = 持久化存储
🎯 = 零拷贝
🚀 = 最快路径
📺 = 媒体播放

RPC/*      = RPC_REQUEST, RPC_RESPONSE, RPC_ERROR
STREAM_*   = STREAM_RELIABLE, STREAM_LATENCY_FIRST
MEDIA_RTP  = WebRTC MediaTrack 的 RTP 包
