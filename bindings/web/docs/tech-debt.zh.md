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

---

## TD-002：sw-host wasm 产物到 `cli/assets/web-runtime/` 无自动同步

**发现时间**：2026-04-24
**发现路径**：T18 H_X/H_Y bisect spike（诊断 agent 把 HX-PROBE 加进 `guest_bridge.rs` 并编译成功，但跑测试时零命中，排查后发现服务端 actr 读到的是老 wasm）

### 现象

`bindings/web/crates/sw-host/build.sh` 产出：
```
bindings/web/dist/sw/actr_sw_host_bg.wasm
bindings/web/dist/sw/actr_sw_host.js
```

`cli/assets/web-runtime/` 需要同名两个文件，通过 `cli/src/web_assets.rs:14` 的 `include_bytes!("../assets/web-runtime/actr_sw_host_bg.wasm")` 嵌入 actr binary。但仓内**没有自动同步脚本或 build rule**把 dist/sw 拷贝到 cli/assets。

### 证据

```
$ grep -rn "cli/assets/web-runtime\|include_bytes.*actr_sw_host" cli/ bindings/web/
cli/src/web_assets.rs:14:pub const RUNTIME_WASM: &[u8] = include_bytes!(".../actr_sw_host_bg.wasm");
cli/src/commands/run.rs:728:    .route("/packages/actr_sw_host_bg.wasm", get(serve_runtime_wasm))
bindings/web/crates/sw-host/build.sh:24:  --out-dir ../../dist/sw \
```

`bindings/web/crates/sw-host/build.sh` 只写 `dist/sw/`，不碰 `cli/assets/`。

### 后果

改动 sw-host 的 Rust 代码后，`bash bindings/web/crates/sw-host/build.sh` + `cargo build -p actr-cli --bin actr` **两步都做了仍会失败** ——
仅当 cli/assets 里的 wasm 是旧内容时，`include_bytes!` 也是旧的，无论 actr 怎么重编都一样。

commit 548ad7d9 的 commit message 第一条明确提到过这点：
> "CLI-embedded assets were stale. `cli/assets/web-runtime/actor.sw.js` still used the legacy `loadWithGuestBridge` dynclib path while `web-sdk/src` already switched to `loadWithComponentBridge`... Re-sync the SDK source and the freshly-built `sw-host` wasm-bindgen output into cli/assets."

也就是说：548ad7d9 把 "再同步一次" 作为一次性修复做了，但没把这个同步动作固化到任何构建脚本里，所以下次又会踩。

### 暂缓理由

加一个 sync 脚本工作量很小，但决定"谁调它"要看整体 web 构建流水线的责任划分（`bindings/web/scripts/build-wasm.sh` 是顶层入口，但当前为空壳）。T18 主线更紧，先登记。

### 修复选项

- **A. 在 `sw-host/build.sh` 末尾直接 `cp` 到 `cli/assets/web-runtime/`**：最直接；坏处是绑定 sw-host 的 build.sh 知道 cli 的目录结构，跨 crate 耦合
- **B. 新增 `bindings/web/scripts/sync-cli-assets.sh`**：把"同步"单独成一步；让上层 build-wasm.sh 或 CI 显式调用
- **C. `build.rs` in cli/actr-cli**：让 cargo 在编译 cli 时自动从 `../bindings/web/dist/sw/` 拉最新；但这样 cli 会变成弱依赖 web/dist 存在
- **D. 软链**：`cli/assets/web-runtime/actr_sw_host_bg.wasm` 不入库，改成指向 `bindings/web/dist/sw/`；但 `include_bytes!` 需要存在的文件，clone 新仓库时会断

推荐 **B**：显式、职责清晰、失败可见。

### 如果不修复就应该知道的事

- 任何改了 sw-host 代码的 spike 都必须**手动**执行：
  ```
  cp bindings/web/dist/sw/actr_sw_host_bg.wasm cli/assets/web-runtime/
  cp bindings/web/dist/sw/actr_sw_host.js     cli/assets/web-runtime/
  cargo build -p actr-cli --bin actr
  ```
- 浏览器 e2e 出现"SW 侧 Rust 新加的日志没打印"时，**先怀疑 cli/assets 没同步**，而不是先怀疑代码逻辑
- diagnostic agent prompt 里要显式提醒这一步（已更新 memory `feedback_puppeteer_sw_console.md` 范畴外的另一个 feedback 点可以考虑单独记）
