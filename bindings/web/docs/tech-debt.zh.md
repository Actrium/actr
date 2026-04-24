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

---

## TD-003：guest dispatch context 单例，并发场景互相覆盖

**发现时间**：2026-04-24
**发现路径**：Option U Phase 4 跑通后对 MultiTab 压测，1/6 PASS

### 现象

| 测试 | 结果 | 场景 |
|------|------|------|
| BasicFunction 1-1 … 1-6 | 6/6 ✓ | 单 client 顺序 RPC |
| MultiTab 6-1 Two Client Tabs | ✓ | 两 tab 各自轮流 echo（不并发） |
| MultiTab 6-2 Concurrent Multi-Client | ✗ timeout 30s | 两 client 同时发 RPC |
| MultiTab 6-3 Close One Client | ✗ timeout 60s | 关一个 tab 后另一个继续 |
| MultiTab 6-4 Refresh One Client | ✗ timeout 60s | 刷新一个 tab 后重新 RPC |
| MultiTab 6-5 Multiple Server Instances | ✗ timeout 60s | 多 server 共存路由 |
| MultiTab 6-6 Shared SW Isolation | ✗ Connection closed 0ms | 多 client 共享 SW 状态隔离 |

### 根因

`bindings/web/crates/sw-host/src/guest_bridge.rs:90`：

```rust
thread_local! {
    static GUEST_CTX: RefCell<Option<Rc<RuntimeContext>>> = const { RefCell::new(None) };
}

fn install_ctx(ctx: Rc<RuntimeContext>) {
    // 如已有 ctx，仅 log::error! 但仍覆盖
    *slot = Some(ctx);
}
```

dispatch 入口（line 504）流程：

```
install_ctx(ctx)
  → dispatch_fn.call1()                       ← 进入 guest wasm
    → guest 代码 host::discover(...).await    ← 读 current_ctx() 并快照；让出
    → [此时别的 dispatch 可能 install_ctx 覆盖]
    → guest 代码 host::call_raw(...).await    ← 重新读 current_ctx() = 被覆盖的
clear_ctx()
```

单 host import 内部：`current_ctx()` 快照后即使 yield 也用那份 ✓
跨 host import 之间：`current_ctx()` 再次被读时已被覆盖 ✗

echo client 的 dispatch 实现恰好是 `discover(...).await → call_raw(...).await`，精准踩坑。

### 影响

- **不是 WBG 特有**：CM 路径也这个 bug。之前没暴露是因为 CM 挂在更浅的 JSPI 层
- 任何并发 dispatch（多 client 或同 client 快速连发）都会触发
- 顺序单 client 不受影响

### 修法选项

| # | 方案 | 改动规模 | 并发性 |
|---|------|---------|-------|
| α | 加 async mutex 把所有 dispatch 在 sw-host 层串行化 | 一处 | 无（排队） |
| β | 在 actor-wbg.sw.js 的 JS 桥层 serialize（一个 dispatch 结束再派下一个） | 一处 | 无（排队） |
| γ | host imports 签名加 request_id 参数；sw-host 用 HashMap\<RequestId, Ctx\> 查找 | WIT + codegen + sw-host | 真并发 |
| δ | 每次 dispatch 在 JS 层构造独立的 host imports 闭包捕获 ctx，替换全局 `self.actrHost*` | actor-wbg.sw.js 重写；但 wasm 侧 `__host_*` import 查表是 binding-time，不是每次调用 | 不可行或很 hack |
| ε | 每 client 一个独立 SW（无 `navigator.serviceWorker` 单例共享） | 架构大改 | 真并发 |

推荐路径：**α 快速止血 → γ 长期真并发**。α 改一处 `Mutex<()>` 就能让 MultiTab 6-2/6-3/6-4 通，6-5/6-6 另议。

### MultiTab 6-5 "Multiple Server Instances"

**独立问题**：`actr-web-abi::host::set_workload` 是 one-shot（`Box::leak` 到 `'static`）。6-5 要求同一 SW 里能注册多个 workload。方案：
- **set_workload 改 HashMap<ActrType, Box<dyn Workload>>**，lookup 时按 dispatch 里的 target actor type
- 或承认"一 SW 一 workload"，把 6-5 重新分类为 "不适用的测试"（写 note 在 test-auto.js）

### MultiTab 6-6 "Shared SW Isolation"

"Connection closed 0ms" —— 没挂 timeout，是启动阶段就失败。可能是：
- 两 client 同时 register_client，sw-host 的 client 状态撞了
- 或者 puppeteer 的 BrowserContext 隔离带来的 SW 注册冲突

需要单独调试，非本条主线。

### 暂缓理由

- 单 client 路径已通（Option U 核心价值兑现）
- 修法有分歧，要架构拍板 α vs γ
- Phase 5（CI drift + 文档收尾）优先级更高

### 修复后应达到的覆盖面

| 套件 | 修 α 后 | 修 γ 后 |
|------|---------|---------|
| BasicFunction | 6/6（已达） | 6/6 |
| MultiTab 6-1 | ✓（已达） | ✓ |
| MultiTab 6-2 / 6-3 / 6-4 | ✓ | ✓ |
| MultiTab 6-5 | 另修 `set_workload` | 另修 |
| MultiTab 6-6 | 另修 | 另修 |
| Webrtc 5-1 / 5-4 | 待跑 | 待跑 |

---

## TD-004：同源 SW 下第二个 client 注册破坏第一个 client 的 RPC 响应

**发现时间**：2026-04-24
**发现路径**：Option U Phase 6 γ-unified 整合后的 γ 轻量验证尝试

### 现象

三种场景对照实验（全走 WBG 路径）：

| 场景 | 结果 |
|------|------|
| 1 client，default BrowserContext | BasicFunction 6/6 ✓ |
| 2 clients，default BrowserContext（共享 SW） | 首个 client warmup 60s 无 `📥` ✗ |
| 2 clients，incognito BrowserContext（隔离 SW）= MultiTab 6-2/6-3/6-4 | 首个 client 首次 RPC 30s timeout ✗ |

共同模式：**一旦 SW 登记了第 2 个 client**，**第 1 个 client 也失去 echo 响应**。button 能 re-enable（~60ms 正常），但 DOM 里永远收不到 `📥` 回复日志。

### 与 TD-003 / γ 的关系

- **不是 γ 的问题**：γ 已架构修正 thread_local GUEST_CTX 单例（现 HashMap<RequestId, Ctx>）；BasicFunction 6/6 证明 request_id threading 全通
- **更前置的问题**：SW 看到 2 个 client actor 登记时，某条路径失效；不影响 server 端 inbound dispatch
- γ 的真并发效果只能**等 TD-004 修好后间接验证**

### 初步假说（诊断后结论见下节）

1. ~~AIS 注册冲突~~：两 client 用同 pubkey/psk 撞车
2. ~~sw-host 单 client 假设~~：隐含"SW 里只一个 client"
3. ~~actor-wbg.sw.js port 管理~~：MessagePort 表错配
4. ~~signaling/WebSocket 冲突~~：同 SW 只留一条 ws

### 诊断结果（2026-04-24 完成）

经 TD-004 diagnostic agent 通过 mock-actrix 端 trace 锁定根因：

| 假说 | 验证 |
|------|------|
| 1 AIS 注册冲突 | **半证实的变体**：不是"两次 register 撞车"，而是"第 2 个 client **根本没重新 register**，从 IndexedDB 恢复了第 1 个 client 的 credential" |
| 2 sw-host 单 client | **证伪**：`CLIENTS: HashMap<client_id, Rc<ClientContext>>` 已是 per-client，`SwRuntime` 每 tab 独立 |
| 3 actor-wbg.sw.js port 管理 | **证伪**：`clientPorts: Map<clientId, port>` 每 tab 独立 |
| 4 ws 会话冲突 | **证实为下游症状**：因假说 1 导致两 ws 同 actor_id，mock-actrix 的 rebind 语义覆盖前者 |

### 真正的根因

**SW 侧 IndexedDB credential namespace 按 actr-type 而非 per-client**：

- `bindings/web/crates/sw-host/src/runtime.rs:1001-1006` 的 `cred_kv_namespace` 只按 `client_actr_type` 命名
- `runtime.rs:646-657` 的 `obtain_credential_from_ais` 里 `try_restore_credential_static` 命中即 return，跳过重新 AIS 注册
- 第 2 个 client 页加载时，从 IndexedDB 恢复到第 1 个 client 的 `actor_id/credential`
- 两个 ws 会话在 mock-actrix 注册成**同一 actor_id**
- `testing/mock-actrix/src/signaling.rs:78-107` 的 `handle_connection` 用 `entry.client_id = new` 无条件覆盖
- 后连的 ws 抢走 registry binding，第 1 个 client 失去所有回程 relay

### 时间线证据（mock-actrix.log）

```
09:40:36  build_register_ok acme:EchoService     serial=1  (server, HTTP register)
09:40:36  build_register_ok acme:echo-client-app serial=2  (client1, HTTP register)
09:40:36  WS bind OK actor_id=2@...echo-client-app  ws=12a83734  (client1 ws)
09:41:03  WARN  WS actor rebound  actor_id=2  new_ws=0c68d540  stolen_from_ws=12a83734  (client2 抢走)
09:41:30  handle_actr_relay target=2  matched_ws=[0c68d540]  (client1 relay 走 client2 的 ws)
```

服务端**只有 1 次** HTTP register（serial=2），client2 page **没有**独立 AIS 注册 —— 直接复用 IndexedDB 恢复的 cred。

### 修复选项

- **α'（推荐）**：`cred_kv_namespace` 改为 `actr_credentials_{client_actr_type}_{client_id}`。简单、彻底，副作用是每 tab 一条 cred 持久条目（可接受）。echo/multi-tab 场景立即通
- **β' AIS 协议层支持"同 type 多实例"**：真 AIS gate + mock-actrix 改协议。跨层工作量大
- **γ' 接受"1 SW 1 client"**：MultiTab 6-2/6-3/6-4/6-6 正式 skip。与多 tab 观察同 actor 的 UX 冲突，不推荐
- **额外防御补丁**：mock-actrix `handle_connection` 的 rebind 至少 WARN（agent 已验证本次 rebind WARN 能 fire），甚至拒绝 rebind 返 conflict

**推荐路径：α' 为主 + mock-actrix WARN 作为长期防御**。α' 是一处 `runtime.rs` 改动，与 TD-003 的 γ request-id threading 正交。

### 次要发现（独立 TD 候选）

client1 warmup 在 **client2 还没出现前就已经失败**（30s 无 `📥`）。这可能是 puppeteer headless + default context 下 WebRTC DC 建立的**另一个独立问题**（类似 MultiTab 6-6 的 "0ms Connection closed"）。TD-004 锁定的 credential 共享问题解决后，这条独立 bug 是否仍存在需要重新观察。

### 暂缓理由

- γ 架构已落地；γ 正确性独立于 TD-004
- TD-004 是独立 blocker，修它不在 Phase 6 scope
- Phase 6b/6c 不依赖 MultiTab 通过（靠 BasicFunction 回归）

### 诊断入口

- `CAPTURE_SW_CONSOLE=1` 必须开（`feedback_puppeteer_sw_console.md`）
- puppeteer `browser.on('targetcreated')` 在 incognito 下可能不触发 SW target；用 CDP `Target.setDiscoverTargets` + 手动 attach
- 2 client 同时跑时，mock-actrix log 里 client_id / 注册序列是关键
- sw-host `runtime.rs::register_client` + `InboundPacketDispatcher` + client_id 路由是嫌疑区

### 修复选项（占位）

- **α'**：sw-host 改为"N client per SW"原生支持；每 client 独立 state
- **β'**：若是 AIS 会话冲突，actor-wbg.sw.js 给每 client 独立 session
- **γ'**：接受"1 SW 1 client"，MultiTab 6-2/6-3/6-4 正式 skip（与 `set_workload` 单例的 6-5 同类处理）
