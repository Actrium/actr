# bindings/web 技术债登记

收录浏览器侧已发现、但当前不立即处理的技术债。每条含：现象、证据、暂缓理由、修复选项。

---

## TD-001：SW↔DOM 的 Rust `DataLane` 桥接层有 5 个 zero-call setter

**发现时间**：2026-04-24
**发现路径**：T18 诊断 spike（定位 `handle_dom_control` 为什么没 fire 时顺手观察到）

### 现象

下列 5 个公有方法在整个仓里 `grep` 不到任何调用点：

| 文件 | 行号 | 方法 |
|------|------|------|
| `bindings/web/crates/sw-host/src/transport/wire_builder.rs` | 45 | `WebWireBuilder::set_dom_channel` |
| `bindings/web/crates/sw-host/src/transport/sw_transport.rs` | 66 | `SwTransport::set_dom_channel` |
| `bindings/web/crates/sw-host/src/inbound/dispatcher.rs` | 47 | `InboundPacketDispatcher::set_dom_lane` |
| `bindings/web/crates/dom-bridge/src/transport/dom_transport.rs` | 100 | `DomTransport::set_sw_channel` |
| `bindings/web/crates/dom-bridge/src/webrtc/coordinator.rs` | 45 | `WebRtcCoordinator::set_sw_channel` |

每个 setter 都把一个 `DataLane` 塞进对应结构体的 `Arc<Mutex<Option<DataLane>>>` 字段；部分 setter 还会启动一个 `start_sw_receiver` / `start_dom_receiver` 接收循环。**Setter 从未被调用，意味着这些接收循环和发送代码路径在生产运行时全是死代码**。

### 证据

```
$ grep -rn "set_dom_channel\|set_dom_lane\|set_sw_channel" \
    bindings/web/crates/sw-host/src/ bindings/web/crates/dom-bridge/src/
```

仅命中上面 5 个定义位置，**无外部调用者**。

### 设计意图 vs 现状

原设计（从代码注释与字段命名推断）：

- SW 和 DOM 之间的 MessagePort 在 Rust 侧被包成 `DataLane`
- `WebWireBuilder` / `SwTransport` / `InboundPacketDispatcher` / `DomTransport` / `WebRtcCoordinator` 通过 setter 注入 `DataLane`
- 后续 send / recv 统一走 `DataLane` 抽象

现状（548ad7d9 后的实际链路）：

- `actor.sw.js` 和 web-sdk / actr-dom 包直接在 JS 层用 MessagePort 路由 control / data 消息
- SW 侧 control 消息直接进入 `wasm_bindgen.handle_dom_control`（`sw-host/src/runtime.rs:3011` 附近）
- 不经过 Rust 侧的 `DataLane` 抽象

也就是说，**原 Rust 侧的桥接层在 548ad7d9 的 Component Model bridge 落地时被绕开**，但对应的接口保留未删。

### 暂缓理由

1. **不阻塞任何功能**：没有任何外部代码在调用这些 setter；它们是安静的死代码
2. **删除有风险**：不确定"残留 setter 是错误"还是"未来 MediaFrame / 大 payload fast-path 计划复用 Rust 侧 DataLane 架构"。需要作者确认语义
3. **T18 主线更重要**：当前优先级是解决"`handle_dom_control` 为什么没 fire"，清理死代码不是 blocker

### 修复选项（决策时拿出来看）

- **A. 删**：五个 setter 及其 `Arc<Mutex<Option<DataLane>>>` 字段、对应 receive loop 全删。最干净，但如果未来 MediaFrame 重新需要 Rust 侧 Lane，还得重新造
- **B. 标记保留**：`#[allow(dead_code)]` + 注释说明"Reserved for future Rust-side DataLane path"，或者降级为 `pub(crate)`，不让它出现在公开面
- **C. 连上**：真的在 `actor.sw.js` 初始化时调用 setter，把 MessagePort 真的包成 `DataLane` 注入。**然后评估是否把部分 JS 路由逻辑下沉到 Rust**。工作量最大

### 相关文件（可能一并处理）

- `bindings/web/packages/web-sdk/src/actor.sw.js` — 真实的 SW 入口
- `cli/assets/web-runtime/actor.sw.js` — 548ad7d9 同步的副本（一改两处）
- `bindings/web/docs/architecture/message-flow-complete.zh.md` — 官方文档描述的消息流

### 不修复就应该知道的事

- 这五个 setter 里 `log::info!` 永远不会打印 —— 看到 `[SW][SwTransport]`、`[WebWireBuilder]` 之类的静默，别误以为它们已被触达
- public API 公开面（T5.5 Batch 5 bindings/web 扫描时）要注意是否把它们降级 —— 以免把未使用的 pub 暴露成外部承诺
