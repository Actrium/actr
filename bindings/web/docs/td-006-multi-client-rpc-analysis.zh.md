# TD-006 深度分析：同源 SW 下 2+ client 场景 RPC round-trip 挂起

**状态**：未解决（2026-04-24）
**上下文**：[tech-debt.zh.md §TD-006](./tech-debt.zh.md#td-006)；[Option U Phase 6 收尾](./option-u-wit-compile-web.zh.md)
**前置已修**：[TD-003](./tech-debt.zh.md#td-003) (guest dispatch ctx 单例)、[TD-004](./tech-debt.zh.md#td-004) (cred namespace 按 actr-type)
**优先级**：中（阻塞 MultiTab 6-2/6-3/6-4/6-5；不阻塞单 client 生产路径）

---

## 0. TL;DR

Phase 6 落地 + TD-003 γ + TD-004 α' 之后，浏览器 e2e 单 client 路径完全跑通（BasicFunction 6/6 ✓）。但**一旦同一 SW 下存在 2 个 client actor，任何一个 client 发 echo RPC 都拿不到回复**。

- 症状：client 在 DOM 上显示 `📤 Sending: "..."`、button re-enable（~60ms）、但 DOM 永远收不到 `📥` 回复；60s 超时后报 `Echo not working within 60000ms`
- 触发条件：**仅 "同源 SW + ≥ 2 个 client actor 登记"**。无论 `default` 还是 `incognito` BrowserContext、无论顺序登记还是并发
- 不影响：1 client（BasicFunction 6/6）、多 client 但不做 RPC（MultiTab 6-1 Two Client Tabs ✓、6-6 Shared SW Isolation ✓）
- 两个已知原因已排除：dispatch ctx 单例（TD-003 γ 修，HashMap 化）、credential namespace 共享（TD-004 α' 修，按 client_id 分区）；**TD-006 是第三层独立问题**

---

## 1. 实证数据

### 1.1 测试覆盖状态（Phase 6 完成后）

完整 e2e 结果（`SUITES="BasicFunction MultiTab" bash start-mock-wbg.sh`）：

| Suite | # | 名称 | 结果 | 触发 RPC | 涉及 client 数 |
|-------|---|------|------|---------|---------|
| BasicFunction | 1-1 | Manual Send | ✓ 62ms | 是 | 1 |
| BasicFunction | 1-2 | Empty Message Send | ✓ 90ms | 是 | 1 |
| BasicFunction | 1-3 | Rapid Consecutive Sends | ✓ 287ms | 是（连发） | 1 |
| BasicFunction | 1-4 | Large Message Send | ✓ 173ms | 是 | 1 |
| BasicFunction | 1-5 | Special Characters | ✓ 68ms | 是 | 1 |
| BasicFunction | 1-6 | Send with Enter Key | ✓ 120ms | 是 | 1 |
| MultiTab | 6-1 | Two Client Tabs | ✓ 11626ms | ❌（仅校验 status） | 2 |
| **MultiTab** | **6-2** | **Concurrent Multi-Client** | **✗ 79662ms** | **是** | **2** |
| **MultiTab** | **6-3** | **Close One Client** | **✗ 79846ms** | **是** | **2** |
| **MultiTab** | **6-4** | **Refresh One Client** | **✗ 79848ms** | **是** | **2** |
| **MultiTab** | **6-5** | **Multiple Server Instances** | **✗ 81905ms** | **是** | **2 server + 1 client** |
| MultiTab | 6-6 | Shared SW Isolation | ✓ 9627ms | ❌（仅校验 SW.scriptURL） | 2 |

**规律**：
- "涉及 RPC + ≥ 2 个 client in same SW" 全挂
- "只涉及 UI 状态 + ≥ 2 个 client" 通
- "涉及 RPC + 1 个 client" 通

### 1.2 6-2 源码流程（test-auto.js:855-903）

```
browser.createBrowserContext()           ← 全新 incognito
  → open server (actrix-register HTTP + WS bind)
  → open client1
  → waitForEchoWorking(client1, 60s)     ← 第一处可能挂
  → open client2
  → waitForEchoWorking(client2, 60s)     ← 第二处可能挂
  → sendEchoMessage(client1, "from-c1-first")
  → verify client1 logs contain 📥
  → sendEchoMessage(client2, "from-c2")
  → verify client2 logs contain 📥
```

测试设计里的注释自证有问题："**Sequential warm-up prevents WebRTC contention in the shared SW**"（test-auto.js:864）—— 已经把并发改成顺序，还挂。

### 1.3 现象关键时间线（γ-validate spike，default context）

我跑过的最小复现（非 incognito）：

```
T0   server ready
T1   client1 opened (tab navigates to localhost:5173)
T2   client1 registers in SW, actor_id=4@.../echo-client-app
T3   mock-actrix: 4@... bound to WS (serial=4)
T4   client2 opened
T5   client2 registers, actor_id=5@.../echo-client-app (TD-004 fix: 独立 serial ✓)
T6   mock-actrix: 5@... bound to WS (serial=5)
T7   test calls waitForEchoWorking(client1, 60s)
     loop: sendEcho "echo-probe-<ts>" → click → button disable → button re-enable after ~60ms
         → NO `📥` response after 60s
T8   TIMEOUT → test FAIL
```

mock-actrix log 期间有大量 `route candidates response count=1 target_type=acme.EchoService` —— 说明 client 做了 **discover** 请求，mock 返回了 server。但后续 RPC 包是否真的经 WebRTC 传达 / server 是否收到，**从 mock-actrix log 看不出来**（mock 只看到信令流量）。

### 1.4 TD-004 诊断 agent 的"次要发现"

> "client1 warmup 在 **client2 还没出现前**就已经失败（~30s 无回复）"

这条值得高度重视。如果 client2 尚未创建、而 client1 的 warmup 已经挂，那故障**不一定是"2 个 client 相互干扰"**，而可能是**"incognito 环境下首个 client 的 RPC 路径就坏"**。

但我的 γ-validate（default context，非 incognito）**也**挂在 client1 warmup —— 所以至少 default context 下 2-client 场景确实坏了。两种场景的失败模式是否完全一致还没对照验证（SW console CDP 抓不到，见 [feedback](../../../../home/l/.claude/projects/-ext-actor-rtc-actr/memory/feedback_puppeteer_sw_console.md)）。

---

## 2. 已排除的假说

下列假说已由 TD-003 γ 和 TD-004 α' 的修复证伪（这两项修完，MultiTab 的失败模式没变化，说明它们不是 TD-006 根因）：

### ❌ 假说 1：guest dispatch ctx 单例互相覆盖

- 原以为：`thread_local GUEST_CTX: Option<Ctx>` 被 2 个并发 dispatch 互相覆盖
- 已修（γ）：改为 `HashMap<RequestId, Ctx>`（`sw-host/src/guest_bridge.rs:102`）
- 验证：MultiTab 6-2 行为不变；`install_ctx called while another context is active` 日志不再 fire

### ❌ 假说 2：cred namespace 按 actr-type 共享

- 原以为：第二个 client 从 IndexedDB 复用第一个的 credential，两 ws 绑到同 actor_id
- 已修（α'）：namespace 改为 `{actr_type}_{client_id}`（`sw-host/src/runtime.rs:1001-1006`）
- 验证：mock-actrix log 显示两 client 拿到独立 serial（4 和 5）；`WS actor rebound` WARN 不再 fire

### ❌ 假说 3：CLIENTS / ClientContext 单例

- 原以为：SW 里只留一个 client 状态
- 代码审计：**已是多实例**
  - `runtime.rs:2490` `CLIENTS: HashMap<String, Rc<ClientContext>>` ← multi-tab aware
  - `runtime.rs:2354` `ClientContext` 每 tab 独立 `runtime` / `system` / `dispatcher` / `peer_gate` / `transport_manager` / `stream_handlers`
  - `runtime.rs:487` 每 `SwRuntime` 自带 `signaling: SignalingClient`、独立 `credential`、独立 `pending_rpcs`、独立 `discovered_targets`

### ❌ 假说 4：actor-wbg.sw.js 的 DOM Port 管理错配

- 原以为：SW 里 `clientPorts` Map 错把两个 tab 的 port 混了
- 已验证（TD-004 diagnostic agent §假说 3）：`clientPorts: Map<clientId, port>` 每 tab 独立，port.onmessage 闭包捕获各自 clientId

---

## 3. 嫌疑空间（根因假说 + 代码指针）

### 🟡 假说 A：`WORKLOAD` 单例下的 dispatch 竞争

**状态**：理论可能，未验证

`runtime.rs:2492`：
```rust
thread_local! {
    static WORKLOAD: RefCell<Option<WasmWorkload>> = const { RefCell::new(None) };
}
```

- 每个 SW 只一个 `WORKLOAD`
- **server 端**：`EchoService` 的 workload，全进程只一个
- **client 端**：2 个 client-guest 共享一份 `WORKLOAD`，它们都发 dispatch 进入同一个 workload 实例

对 server 端的影响：2 个 client 同时向 server 发 RPC，server 的 dispatch 会被并发调用两次。每次 dispatch 构造独立 Context（via γ HashMap）应该 OK。但 `RefCell::borrow()` on `WORKLOAD` 是否可能在高并发下跨 yield 点冲突？理论上单线程 wasm 不会，但**借用期如果跨 `.await`** 就会 panic/错乱。

**验证方法**：在 `register_workload`（runtime.rs:2513）和 dispatch entry 点打 `borrow().as_ref()` 时的 RefCell state。

### 🟡 假说 B：server 端的 inbound packet dispatcher 单 peer 假设

**状态**：未验证，但嫌疑度高

`bindings/web/crates/sw-host/src/inbound/` 是 inbound dispatch 模块。server 收到 WebRTC DataChannel 数据时走这条。

**关键问题**：sw-host 的 **server side** 对"多个 inbound peer 同时发 RPC"是否有状态隔离？

- `SwRuntime.pending_rpcs: HashMap<String, PendingRpcTarget>` 是 **outbound** 的 pending 表（我发出的请求等回复）
- **inbound** 的对应是什么？让我看 `inbound/dispatcher.rs`
- 如果 inbound 侧有"正在处理的 request"的单例 / hash 键错、或者响应信道被覆盖，就会导致 server 处理 client1 请求时 client2 的上下文污染

**验证方法**：在 inbound/dispatcher.rs 的 dispatch 入口加 `log::info!` 打印 request_id / caller_id / target route，跑 2 客户端场景看是否有错配。

### 🟡 假说 C：WebRTC coordinator / peer connection 共享

**状态**：未验证，嫌疑度高

每个 `SwRuntime` 有自己的 signaling，但 **WebRTC peer connection 是浏览器 API**，在 Service Worker 全局作用域里。问题：

- 一个 SW 里能同时维护多少 RTCPeerConnection？每个 client 对每个 peer 都建一个独立 connection 吗？
- client1 和 server 建了一个 RTCPeerConnection。client2 登记后，它是否也要建一个独立的 RTCPeerConnection 到同一个 server？
- 如果 sw-host 的 WebRTC 层在 `SwRuntime` 实例内是隔离的（好），但 **server 端的 WebRtcCoordinator 被两个 SwRuntime 共享**（如是 singleton），那 server 看到的 inbound 可能混在一起

**代码指针**：
- sw-host 没有 `wire/webrtc/` 目录；`bindings/web/crates/dom-bridge/src/webrtc/coordinator.rs:34` 是 DOM 侧
- sw-host 侧的 WebRTC 操作通过 MessagePort 转发到 DOM 的 coordinator
- 如果 `DomTransport::sw_channel` 或相关 channel 是单实例（[TD-001](./tech-debt.zh.md#td-001) 说的 zero-call setter 家族之一），两个 client 的 outbound WebRTC 请求可能打到同一个 DOM-side coordinator，产生 peer_id 错配

**验证方法**：
- 读 `bindings/web/packages/actr-dom/` 下的 DOM-side WebRTC 协调代码
- 在 sw-host 的 `PeerGate` / `PeerTransport` 建 p2p 请求点加 trace
- 观察 DOM 侧 `window.__webrtcCoordinator`（test-auto.js:751 用过）在 2-client 场景下看到几个 peer

### 🟡 假说 D：mock-actrix 只保留一条 ws per actor_id

**状态**：TD-004 α' 修前是这个 bug；修后已 confirmed OK（`WS actor rebound` WARN 不 fire）；但——

**后置风险**：client1 和 client2 有独立 actor_id 之后，mock-actrix 给两个 ws 都注册正常。但**当 client1 发 discover 找 server 时**，mock 只返 server 的 ws。响应走到 server 的 ws 后，server 怎么路由回 client1 vs client2？

- 看 `testing/mock-actrix/src/signaling.rs:523` `handle_actr_relay`：按 target actor_id 找 ws 转发
- server → client 的 relay：server 在 envelope 里填目标 client 的 actor_id，mock 转发给对应 ws
- 如果 server 错把 client1 的 caller_id 当成 client2，relay 就走偏
- **server 的 `caller_id` 追踪是否 per-dispatch？** 若 guest_bridge 里有 "last caller" 单例，并发 dispatch 会错乱

**代码指针**：
- `sw-host/src/guest_bridge.rs:462` `host_get_caller_id` —— ctx_get 查 HashMap 后从 RuntimeContext 取 caller_id
- 这步 γ 已修；但 **RuntimeContext 构造时 caller_id 字段怎么填进去的？**
- `sw-host/src/runtime.rs` 某处 inbound dispatch 为每次 request 新建 RuntimeContext —— 需要核查它读的是 request 真正的 caller_id 还是 SwRuntime 当前的某个单例 field

### 🟡 假说 E：SignalingClient 的 ws 复用与多 peer discovery 冲突

**状态**：未验证

每 SwRuntime 一个 SignalingClient（`runtime.rs:498`），所以 client1 和 client2 各自有独立 ws 到 mock-actrix。

但 server 也只有一个 SwRuntime，一条 ws 到 mock-actrix。server 收到两条 client 的 signaling 消息时：
- client1 的 SDP offer → server 处理 → 回 answer
- client2 的 SDP offer → server 处理 → 回 answer
- server 端要同时维持两个 WebRTC peer connection 状态

如果 server 侧的 `SwRuntime.known_peers` / `open_channels` / `role_assignments`（`runtime.rs:511-519`）在处理第二个 peer 的信令时覆盖第一个，就会挂。

**代码指针**：
- `SwRuntime.known_peers: HashSet<String>` / `open_channels: HashMap<String, HashSet<u32>>` / `role_negotiated: HashSet<String>` / `role_assignments: HashMap<String, bool>` / `ice_restart_inflight` / `ice_restart_attempts` / `peer_connection_states` —— 看起来都是 per-peer（key 是 peer_id 字符串），**不是单例**
- 但这些 state 在 `SwRuntime::on_signaling_message` 等入口的读/写是否正确区分 peer？需要核查

**验证方法**：启动 server + client1 + client2，在 server 的 SwRuntime 状态变化点加 trace，观察 2 个 peer 的 ICE / DataChannel 建联是否都完成。

### 🟡 假说 F：`DomTransport.sw_channel` / `WebRtcCoordinator.sw_channel` 单实例

**状态**：TD-001 遗留

[TD-001](./tech-debt.zh.md#td-001) 记录的 5 个 zero-call setter 里有：
- `sw-host/src/transport/sw_transport.rs:66` `SwTransport::set_dom_channel`
- `dom-bridge/src/transport/dom_transport.rs:100` `DomTransport::set_sw_channel`
- `dom-bridge/src/webrtc/coordinator.rs:45` `WebRtcCoordinator::set_sw_channel`

这些虽然是 zero-call，但说明**结构上设计了 SW↔DOM 各一个 channel**。如果在 WBG 路径下实际 SW↔DOM 的通信靠 `actor-wbg.sw.js` 的 MessagePort 走 per-client port（TD-004 已验证），那**多 client 的 WebRTC 信息怎么在 DOM 侧区分**？

- DOM 侧只有**一个** `WebRtcCoordinator`（dom-bridge）维护一个 peer_connections Map
- 当 client1 和 client2 都向 DOM 发 "create peer to server" 请求时，DOM 怎么区分哪个请求来自哪个 client？
- 如果 DOM 合并两个请求、共享一条 RTCPeerConnection，就出 bug

**验证方法**：
- 在 DOM 的 `WebRtcCoordinator` 的"接受 SW 请求"入口加 console.log（puppeteer 能抓到，因为是 page console）
- 看 2 client 场景下 DOM 收到几个 create-p2p 请求、建了几个 RTCPeerConnection

这条**非常值得立刻验证**，因为 DOM 侧代码 puppeteer 能直接抓 console（不像 SW 那样盲）。

---

## 4. 建议诊断方法

### 4.1 非侵入式：抓 DOM 侧的 WebRTC 状态

puppeteer 能直接抓 DOM console。在 test-auto.js 的 `suiteMultiTab` 入口前加一段：

```js
page.evaluate(() => {
    const coord = window.__webrtcCoordinator;
    if (coord) {
        console.log('[TD006] WebRtcCoordinator instance:', coord);
        console.log('[TD006] peers:', coord.getAllPeers?.() || 'N/A');
    }
});
```

跑 6-2 场景：
- 期望：两个 client 各自对 server 建一个 peer → coord 看到 2 个 peers
- 如果实际只有 1 个 peer，或 2 个但某个状态不对，就锁定假说 C/F

### 4.2 侵入式：sw-host trace

在以下点加 `log::info!`（SW 侧 log 走 emitSwLog 到 DOM log 缓冲，puppeteer 能看到）：

1. `sw-host/src/runtime.rs:524` `SwRuntime::new` 入口 + 出口（每 tab 一次）
2. `sw-host/src/runtime.rs:579` `SignalingClient::connect_with_retries` 开始 / 完成
3. `sw-host/src/runtime.rs` 的 discover / p2p-setup 路径（具体 grep `request_p2p` / `NegotiateRole`）
4. `sw-host/src/outbound/peer_gate.rs` 的 send_request 入口
5. inbound dispatcher entry（`sw-host/src/inbound/dispatcher.rs:dispatch`）

跑 2 client 场景观察：
- client1 是否完成 signaling + discovery
- client1 是否尝试 create peer to server
- server 是否收到 client1 的 inbound dispatch
- server 是否返回 response
- 如回 client1，relay 是否到了

任意一环断掉就锁定根因层。

### 4.3 工具化改进（长期）

- 现在 `page.on('console')` 抓不到 SW console（[feedback](../../../../../home/l/.claude/projects/-ext-actor-rtc-actr/memory/feedback_puppeteer_sw_console.md)）
- `test-auto.js` 的 `CAPTURE_SW_CONSOLE=1` 环境变量默认用 `browser.on('targetcreated')`，但在 headless + default / incognito 下经常不触发
- 长期应该用 CDP `Target.setDiscoverTargets` + `Target.attachToTarget` 手动 attach SW target，详见 [TD-004](./tech-debt.zh.md#td-004) §诊断入口第 2 条

这本身是另一个待完成项（TD-004 诊断 agent 提出过）。

---

## 5. 修复选项（待诊断后细化）

| 选项 | 描述 | 风险 | 适用假说 |
|------|------|------|---------|
| **a''** | 找到下一处单例，加 client_id 分区（和 γ/α' 同样模式） | 低 | A / B / E |
| **b''** | 重构 DOM 侧 WebRtcCoordinator 支持"per sw-client peer" | 中 | C / F |
| **c''** | 放弃"1 SW 多 client"模型；改为"1 SW 1 client"，MultiTab 6-2/6-3/6-4/6-5 正式 skip | 高（战略让步） | 全部 |
| **d''** | 重新设计 SW↔DOM transport protocol（带 client_id 前缀） | 高 | F |

**推荐路径**：先诊断（§4.1 + §4.2）锁定根因，再决定 a''-d''。

---

## 6. 决策点（留给认真分析后作答）

- **Q1**：是否愿意支持 "**1 SW 多 client**" 模型？
  - 支持 → 需要 a''/b''/d'' 路线
  - 不支持 → c'' 路线（把 MultiTab 6-2/6-3/6-4/6-5 转为"架构不支持"明文 skip）
  - 这是一个架构层的产品决策；影响浏览器多 tab 用户 UX

- **Q2**：MultiTab 6-5 "Multiple Server Instances" 本质是 "1 SW 多 workload"，与多 client 不同。架构层面：
  - [`register_workload` 是 `OnceLock`](./tech-debt.zh.md#td-003) — 硬性 panic
  - 修它需要 `HashMap<actr_type, Workload>` 重构
  - 和 6-2/6-3/6-4 的根因分开处理（或一起放弃）

- **Q3**：诊断成本 vs 收益
  - 6-2/6-3/6-4 只影响浏览器 e2e 测试覆盖面，不影响单 client 生产路径
  - 诊断至少 1-2 个 spike（先 DOM coordinator inspection 再 sw-host trace）
  - 如果答 c''（战略 skip），可以直接免去诊断投入

---

## 7. 附录：相关工件索引

### 7.1 相关 commit

- TD-003 γ：`b268b376`（HashMap<RequestId, Ctx>）、`8dd82fa3`（host_*_async 加 request_id）
- TD-004 α'：`eb034d94`（cred namespace）、`b658d4f0`（mock-actrix WARN）、`1ac5afd4`（cli/assets sync）
- Phase 6c：`87b3401d`、`a4ec581b`、`cff7f71a`（echo 一份化）

### 7.2 mock-actrix 验证日志（TD-004 α' 生效，multi-client 各独立 serial）

```
mock-actrix: registered actor (http) serial=3 name="EchoService"
mock-actrix: WS bound to HTTP-registered actor actor_id=3@.../acme:EchoService:0.1.0
mock-actrix: registered actor (http) serial=4 name="echo-client-app"
mock-actrix: WS bound to HTTP-registered actor actor_id=4@.../acme:echo-client-app:0.1.0
mock-actrix: registered actor (http) serial=5 name="echo-client-app"   ← 独立 client
mock-actrix: WS bound to HTTP-registered actor actor_id=5@.../acme:echo-client-app:0.1.0
```

**无** `WS actor rebound` WARN —— 证明 TD-004 的 bug 不再复现。

### 7.3 关键代码位置（继续诊断时优先读这些）

| 文件 | 行 | 内容 |
|------|-----|------|
| `sw-host/src/runtime.rs` | 487 | `SwRuntime` 定义（52 字段） |
| `sw-host/src/runtime.rs` | 2354 | `ClientContext` 定义 |
| `sw-host/src/runtime.rs` | 2487-2493 | 三个 thread_local：CLIENTS / GLOBAL_INITIALIZED / WORKLOAD |
| `sw-host/src/guest_bridge.rs` | 102 | DISPATCH_CTXS HashMap (γ fix) |
| `sw-host/src/outbound/peer_gate.rs` | — | PeerGate 实现（outbound RPC 路径） |
| `sw-host/src/inbound/dispatcher.rs` | — | inbound packet dispatcher |
| `sw-host/src/transport/peer_transport.rs` | — | PeerTransport，每 ClientContext 一个 Arc |
| `dom-bridge/src/webrtc/coordinator.rs` | 34 | DOM 侧 WebRtcCoordinator |
| `dom-bridge/src/transport/dom_transport.rs` | 100 | DomTransport::set_sw_channel (zero-call, TD-001) |
| `testing/mock-actrix/src/signaling.rs` | 523 | handle_actr_relay（按 target actor_id 转发） |
| `bindings/web/examples/echo/test-auto.js` | 831 | suiteMultiTab 起始 |
| `bindings/web/examples/echo/test-auto.js` | 856 | 6-2 测试实现 |

### 7.4 重现环境

```bash
cd bindings/web/examples/echo
NODE_PATH=/home/l/.local/n/lib/node_modules \
    SUITES="MultiTab" \
    CAPTURE_SW_CONSOLE=1 \
    bash start-mock-wbg.sh
```

前置：
- main HEAD ≥ `eda786ed`（Phase 6 完成后）
- `target/debug/actr` 必须是最新 build（重 build 规则见 [TD-002](./tech-debt.zh.md#td-002)）
- `cli/assets/web-runtime/actr_sw_host_bg.wasm` 必须是最新（同 TD-002）

### 7.5 历史教训

两轮 spike 踩过的相同坑（记忆在 `feedback_puppeteer_sw_console.md`）：
- puppeteer `page.on('console')` 抓不到 SW console；必须用 CDP `Target.setDiscoverTargets` 手动 attach SW target
- 浏览器 e2e 改 sw-host Rust 代码后，`cli/assets/web-runtime/` 必须手动同步，否则 `include_bytes!` 还是旧 wasm（[TD-002](./tech-debt.zh.md#td-002)）

---

## 8. 下一步我需要的输入

要你决策的几件事（回答后才派 agent）：

1. Q1 的答案（支持 vs 不支持"1 SW 多 client"）
2. 如果支持，先做 §4.1（DOM coordinator inspection，非侵入）还是 §4.2（sw-host trace，侵入式）？
3. Q2：6-5 另案处理 / 或和 6-2/6-3/6-4 一起 skip？
4. 容忍投入：一个 spike（~2-3 小时）接受锁定一个假说 / 两个 spike 接受锁定根因 / 不投入直接跳？
