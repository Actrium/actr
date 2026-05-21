# TD-006 深度分析：同源 SW 下 2+ client 场景 RPC round-trip 挂起

**状态**：已修复（2026-04-25，commit `301c58d6`）  
**上下文**：[tech-debt.zh.md §TD-006](./tech-debt.zh.md#td-006)；[Option U Phase 6 γ-unified](./option-u-phase6-gamma-unified.zh.md)  
**前置已修**：[TD-003](./tech-debt.zh.md#td-003)（dispatch ctx 单例）、[TD-004](./tech-debt.zh.md#td-004)（credential namespace 共享）  
**优先级**：已关闭；以下主体保留 2026-04-24 的修复前分析，结案结果见本页 TL;DR / §5 / §6

---

## §0 TL;DR

TD-006 已经收敛并修复。最后落地的是一组组合修复，而不是单点：`mock-actrix` 不再把 role-negotiation / relay 错发给旁观连接，`RouteCandidates` 优先最新 live actor；`sw-host` 在 stale peer、disconnected/failed/closed、以及 DOM `command_error` 时会真正清 transport、失效 discovered target 并拒绝 pending RPC；DOM/SW 侧补了 `CLIENT_UNREGISTER`、按浏览器窗口 remap 清理旧 client、串行处理同一 `MessagePort`、lane-0 `bindRpcPort` 重绑与 stale peer 替换。修复后验证结果是 `SUITES='BasicFunction MultiTab' bash start-mock.sh` `12/12` 全过，`SUITES=BasicFunction` 复跑 `6/6` 也全过，所以此前剩下的 `1-0 Basic Echo Connectivity` 也一并不再复现。

---

## §1 实证数据

### 1.1 修复前测试矩阵（历史记录）

修复后的重新验证：

- `SUITES='BasicFunction MultiTab' bash start-mock.sh`：`12/12` 全过。
- `SUITES=BasicFunction bash start-mock.sh`：`6/6` 全过。
- 关键通过项：`1-0 Basic Echo Connectivity`、`6-4 Refresh One Client`、`6-5 Multiple Server Instances`。

以 `SUITES="BasicFunction MultiTab" bash start-mock.sh` 为准：

| Suite | # | 名称 | 结果 | 涉及 RPC | 涉及 client 数 |
|-------|---|------|------|---------|---------------|
| BasicFunction | 1-1 | Manual Send | ✓ 62ms | 是 | 1 |
| BasicFunction | 1-2 | Empty Message Send | ✓ 90ms | 是 | 1 |
| BasicFunction | 1-3 | Rapid Consecutive Sends | ✓ 287ms | 是（连发） | 1 |
| BasicFunction | 1-4 | Large Message Send | ✓ 173ms | 是 | 1 |
| BasicFunction | 1-5 | Special Characters | ✓ 68ms | 是 | 1 |
| BasicFunction | 1-6 | Send with Enter Key | ✓ 120ms | 是 | 1 |
| MultiTab | 6-1 | Two Client Tabs | ✓ 11626ms | 否（仅 status） | 2 |
| MultiTab | 6-2 | Concurrent Multi-Client Sends | ✗ 79662ms | 是 | 2 |
| MultiTab | 6-3 | Close One Client | ✗ 79846ms | 是 | 2 |
| MultiTab | 6-4 | Refresh One Client | ✗ 79848ms | 是 | 2 |
| MultiTab | 6-5 | Multiple Server Instances | ✗ 81905ms | 是 | 2 server + 1 client |
| MultiTab | 6-6 | Shared SW Isolation | ✓ 9627ms | 否（仅 SW.scriptURL / 隔离） | 2 |

**直接读法**：

- `1 client + RPC` 全通。
- `2+ client + 不做 RPC` 可通。
- `2+ actor runtime + 做 RPC` 全挂。
- 6-5 另含 “1 SW 多 workload” 维度，所以它既可能共用 TD-006 根因，也可能叠加 `WORKLOAD` / `register_workload` 范畴的问题。

### 1.2 6-2 测试流程代码

`bindings/web/examples/echo/test-auto.js:855-903` 的结构是：

```text
browser.createBrowserContext()          // 全新 incognito
  -> open server
  -> open client1
  -> waitForEchoWorking(client1, 60s)   // 第一处可能挂
  -> open client2
  -> waitForEchoWorking(client2, 60s)   // 第二处可能挂
  -> sendEchoMessage(client1, "from-c1-first")
  -> verify client1 logs contain 📥
  -> sendEchoMessage(client2, "from-c2")
  -> verify client2 logs contain 📥
  -> sendEchoMessage(client1, "from-c1-again")
  -> verify client1 logs contain 📥
```

这里最关键的一句注释在 `test-auto.js:863-875`：

> `Sequential warm-up prevents WebRTC contention in the shared SW.`

也就是说测试已经显式把“并发建立连接”降成“顺序 warm-up”，但失败模式没有改善。TD-006 不是“只在并发 create_offer / ICE 时才会触发”的窄问题。

### 1.3 γ-validate 时间线

我能稳定复现的最小时间线是：

```text
T0  server ready
T1  client1 opened
T2  client1 registers in SW, actor_id=4@.../echo-client-app
T3  mock-actrix: client1 WS bound, serial=4
T4  client2 opened
T5  client2 registers, actor_id=5@.../echo-client-app
T6  mock-actrix: client2 WS bound, serial=5
T7  waitForEchoWorking(client1)
    -> DOM 显示 📤 Sending
    -> button ~60ms 后 re-enable
    -> 60s 内没有任何 📥
T8  timeout: Echo not working within 60000ms
```

期间 mock-actrix 常能看到：

```text
route candidates response count=1 target_type=acme.EchoService
```

这只说明 **discover 已成功**，不能说明后续 RPC 包已经穿过 WebRTC，也不能说明 server 端 handler 一定收到了请求。mock-actrix 看到的是信令流量，不是 DataChannel payload。

### 1.4 TD-004 诊断里的“次要发现”

TD-004 diagnostic agent 留下过一句非常重要的次要发现：

> client1 warmup 在 client2 还没出现前就已经失败（约 30s 无回复）

这句话不能被忽略，因为它意味着：

- TD-006 可能不是单一根因。
- 至少存在一种更底层的 warmup / WebRTC / headless 环境问题，会在“首个 client”阶段就把路径打坏。
- 但 default context 下的 `γ-validate` 又显示：在 client2 真正登记后，client1 也会稳定挂起。所以“纯 multi-client 隔离 bug”仍然存在高度嫌疑。

因此更准确的表述是：**当前观测像是“基础层 warmup 脆弱性”与“真 multi-client 隔离问题”可能叠加；TD-006 文档聚焦后者，但不能忘记前者。**

---

## §2 已排除的假说

### 2.1 `dispatch ctx` 单例覆盖

**结论**：已排除。

- 旧 bug 是 `guest_bridge` 只有单槽 `GUEST_CTX`；现在已改成 `DISPATCH_CTXS: HashMap<String, Rc<RuntimeContext>>`，按 `request_id` 查上下文。
- 代码位置：`bindings/web/crates/sw-host/src/guest_bridge.rs:107-150`。
- 伴随症状 `install_ctx called while another context is active` 已不再出现。
- 更关键的是：`BasicFunction` 1-3 `Rapid Consecutive Sends` 仍可通过，说明同一 client 内的并发/快连发已不再把 ctx 互相踩掉。

### 2.2 credential namespace 共享

**结论**：已排除。

- `SwRuntime::new` 现在按 `client_id` 分区 credential namespace。
- 代码位置：`bindings/web/crates/sw-host/src/runtime.rs:558-560`。
- 实测证据：mock-actrix 日志能看到两个 client 独立 serial（4 和 5），且 `WS actor rebound` WARN 不再 fire。
- 所以 TD-004 的根因链已经断开，不再是 TD-006 的解释。

### 2.3 `CLIENTS` / `ClientContext` 单例

**结论**：已排除。

- `CLIENTS` 是 `HashMap<client_id, Rc<ClientContext>>`，不是单槽。
- 代码位置：`bindings/web/crates/sw-host/src/runtime.rs:2487-2492`。
- `ClientContext` 自带独立的 `runtime` / `system` / `dispatcher` / `peer_gate` / `transport_manager` / `stream_handlers`。
- 代码位置：`bindings/web/crates/sw-host/src/runtime.rs:2354-2363`，初始化在 `2828-2868`。
- 这部分代码结构上已经是 multi-client aware。

### 2.4 DOM `MessagePort` 错配

**结论**：已排除。

- `actor.sw.js` 在 SW 全局维护的是 `clientPorts: Map<clientId, port>`。
- 代码位置：`bindings/web/packages/web-sdk/src/actor.sw.js:105-106`。
- `DOM_PORT_INIT` 时按 `clientId` 入表；之后 `control` / `webrtc_event` / `fast_path_data` / `register_datachannel_port` 都是通过闭包捕获的那个 `clientId` 回到 Rust。
- 代码位置：`bindings/web/packages/web-sdk/src/actor.sw.js:521-586`。
- 也就是说“两个 tab 把 DOM port 混了”这类错配，从代码结构上已经讲不通。

---

## §3 嫌疑空间

你给的 F→C→B 顺序在“先用最低成本排除最大面”意义上是成立的；但如果只看当前代码证据，我会把 **E 与 C** 提前，因为它们解释力更强，而且直接触到“同源 SW + 多 peer + 信令 / WebRTC 建联”这个交界面。下面仍按 A-F 展开。

### A. `WORKLOAD` / guest 实例共享竞争

**当前判断**：中等嫌疑，但应把重点从“`RefCell` 借用冲突”改成“全 SW 共享同一 guest dispatch 实例”。

**为什么仍可疑**：

- `WORKLOAD` 是 SW 全局 thread-local 单槽。
- 代码位置：`bindings/web/crates/sw-host/src/runtime.rs:2492`。
- 注册与读取都走同一个全局 workload。
- 代码位置：`bindings/web/crates/sw-host/src/runtime.rs:2513-2516`、`2633-2635`、`3028-3030`。
- `WasmWorkload` 只是包了一层 `Rc<dyn Fn(...) -> Future>`；`clone()` 复制的是共享 handler，不是独立实例。
- 代码位置：`bindings/web/crates/sw-host/src/workload.rs:32-38`、`61-80`。
- WBG 路径里 `dispatchFn` 也是在 SW 启动时注册一次，全局复用。
- 代码位置：`bindings/web/packages/web-sdk/src/actor.sw.js:406-435`。

**为什么又不是最高嫌疑**：

- 当前读点是 `WORKLOAD.with(|cell| cell.borrow().clone())`，借用本身不会跨 `.await`；所以“`RefCell` 借用跨 await 直接炸掉”这个旧推理并不准确。
- `BasicFunction` 1-3 的快速连发能过，说明 guest dispatch 至少不是“任何并发都会坏”的非重入实现。

**更真实的可疑点**：

- server、client1、client2 在同一 SW 内是否共享了同一套 guest JS/WASM 状态机。
- 如果 guest 内部还有隐藏全局态，`WORKLOAD` 只会把问题放大。

**验证方法**：

- 在 `runtime.rs:2633-2715` 与 `3001-3060` 周围打印 `client_id`、`actor_id`、`request_id`、`route_key`、dispatch enter/resolve。
- 如有需要，再在 `actor.sw.js:406-435` 的 `dispatchFn` 周围加 JS console，确认同一套 guest 是否交错处理多个 actor 的请求。

### B. inbound dispatcher / mailbox / scheduler 存在“单 peer”隐含假设

**当前判断**：中高嫌疑，但真正该查的不是 `InboundPacketDispatcher` 自身，而是它与 `handle_fast_path`、mailbox record、scheduler 的拼接处。

**为什么仍可疑**：

- `InboundPacketDispatcher` 本身很薄，只做 `from + message -> enqueue(mailbox)`。
- 代码位置：`bindings/web/crates/sw-host/src/inbound/dispatcher.rs:19-27`、`58-111`。
- 真正的关键路径是：
  - `handle_fast_path` 把新的 inbound RPC 判定为“不是 response”后，转入 dispatcher。
  - scheduler 再从 `record.from` 里恢复 `stream_id`，然后 `parse_peer_and_channel` 得到 `peer_id`。
  - 同时再从 envelope metadata 取 `sender_actr_id` 作为 `caller_id`。
- 代码位置：`bindings/web/crates/sw-host/src/runtime.rs:2037` 附近、`2644-2668`。

**如果这里有 bug，会怎么表现**：

- server 的 handler 确实被调用，但回包注册到了错误的 `peer_gate` 映射。
- 或者 `record.from` / `peer_id` 恢复错了，导致 response 被送回另一条 peer。
- 这正好会呈现为“discover 成功、button re-enable、DOM 永远收不到 📥”。

**反证**：

- `InboundPacketDispatcher` 结构体本身没有类似“current peer”单槽字段。
- 所以如果 B 成立，问题更可能在 dispatcher 周边，而不是 `dispatcher.rs` 这 100 来行本身。

**验证方法**：

- 在 `handle_fast_path` 入口、`dispatcher.dispatch`、scheduler 的 `2644-2668` 一路打印：
  - `client_id`
  - `stream_id`
  - `peer_id`
  - `request_id`
  - `sender_actr_id`
  - `caller_id`
- 预期关系：`stream_id -> peer_id` 与 `sender_actr_id` 应该在同一次请求里稳定一致；任何一处错位都优先锁定 B 或 D。

### C. WebRTC peer 连接隔离不足

**当前判断**：高嫌疑。

**为什么可疑**：

- SW 侧把 peer 身份压缩成单个 `peer_id = target.to_string_repr()`。
- 代码位置：`bindings/web/crates/sw-host/src/runtime.rs:1398-1409`。
- 发送到 DOM 的 `webrtc_command` 也只有 `peerId`，没有 `clientId`。
- 代码位置：`bindings/web/crates/sw-host/src/runtime.rs:1341-1355`；消息 schema 在 `bindings/web/packages/actr-dom/src/sw-bridge.ts:35-48`。
- DOM 侧 `WebRtcCoordinator` 用 `peers: Map<string, PeerConnectionInfo>` 纯按 `peerId` 建索引；重复 `create_peer` 只会告警 `Peer already exists` 并直接返回。
- 代码位置：`bindings/web/packages/actr-dom/src/webrtc-coordinator.ts:28`、`74-78`、`151-158`。

**为什么它解释力强**：

- 一旦某个 page 误收到了不属于自己的 `webrtc_command`，或某个旧 `peerId` 状态没清干净，DOM 侧不会显式报错，只会无声复用 / 拒绝重建。
- 这类问题最容易表现为：discover 正常、signaling 似乎也走了、但 DataChannel 永远开不起来。

**反证**：

- 每个 page 都会新建自己的 `ServiceWorkerBridge` 与 `WebRtcCoordinator`。
- 代码位置：`bindings/web/packages/actr-dom/src/index.ts:116-132`；示例页 `cli/assets/web-runtime/actr-host.html:645-651`。
- SW 侧 `handle_dom_webrtc_event` 也是按 `client_id` 回到对应 `SwRuntime`。
- 代码位置：`bindings/web/crates/sw-host/src/runtime.rs:3182-3192`。
- 所以 C 不是“显然存在的架构单例 bug”，而是“边界协议只带 `peerId` 是否足够”的怀疑。

**验证方法**：

- 只抓 DOM page console 就够：
  - 看是否出现无关 `create_peer`。
  - 看是否有 `Peer ... already exists`。
  - 看 `Command create_peer` 后是否真的跟上 `Peer connection created`、`create_offer/create_answer`、`DataChannel RPC_RELIABLE opened`。
- 如果命令链条在某个 page 中途断了，C 非常可疑。

### D. `caller_id` 追踪在并发 dispatch 下被污染

**当前判断**：低到中等嫌疑；比 TD-003 之前弱很多。

**为什么还没完全出局**：

- `RuntimeContext` 的 `caller_id` 仍是后续业务逻辑依赖的重要字段。
- 代码位置：`bindings/web/crates/sw-host/src/context.rs:21-41`、`45-61`。
- server 端 inbound 请求的 `caller_id` 是在 scheduler 阶段从 envelope metadata 动态解码出来，再灌进新的 `RuntimeContext`。
- 代码位置：`bindings/web/crates/sw-host/src/runtime.rs:2645-2649`、`2702-2710`。
- 同时还会把 `caller_id -> Dest::Peer(peer_id)` 注册进 `peer_gate`。
- 代码位置：`bindings/web/crates/sw-host/src/runtime.rs:2664-2668`。

**为什么它不是更高嫌疑**：

- 现在 `host_get_caller_id()` 已经不再依赖全局单槽，而是按 `request_id` 查上下文表。
- 代码位置：`bindings/web/crates/sw-host/src/guest_bridge.rs:107-150`，getter 在 `482` 附近。
- `call_raw()` 发出的 `sender_actr_id` 也来自 `self_id`，不是某个共享全局。
- 代码位置：`bindings/web/crates/sw-host/src/context.rs:93-131`。

**因此 D 更像什么**：

- 不是“getter 本身错了”，而是“上游传进来的 `sender_actr_id` / `peer_id` 已经错了”，随后被干净地传错下去。
- 这时 D 与 B 往往一起出现。

**验证方法**：

- 在 scheduler 构建 `RuntimeContext` 时把 `sender_actr_id`、`peer_id`、`caller_id` 全打出来。
- 再在 guest handler 里临时打印 `ctx.caller_id()`，看是否与入站 metadata 一致。

### E. `SignalingClient` 多 peer 信令冲突

**当前判断**：高嫌疑，而且这里多出一个很强的 mock/harness 放大器。

**为什么可疑**：

- 每个 `SwRuntime` 只有一条 `SignalingClient` websocket。
- 代码位置：`bindings/web/crates/sw-host/src/runtime.rs:98-108`、`487-499`、`579-583`。
- server runtime 必须在同一条 ws 上同时处理来自多个 client 的 `RoleNegotiation` / `SessionDescription` / `IceCandidate`。
- 相关状态虽然名义上是 per-peer map，但全部共存于同一个 runtime：`known_peers`、`open_channels`、`role_assignments`、`peer_connection_states` 等。
- 代码位置：`bindings/web/crates/sw-host/src/runtime.rs:510-521`、`1608-1737`、`1754-2011`。

**这里有一个额外的测试基架红旗**：

- `testing/mock-actrix/src/signaling.rs:557-611` 对 `RoleNegotiation` 的处理不是“发给 sender + target”，而是：
  - `envelope_for_from` 发给 sender。
  - `envelope_for_to` 发给所有其他连接。
- 代码位置：`testing/mock-actrix/src/signaling.rs:601-609`。
- 在拓扑 `server + client1 + client2` 下，这意味着 client2 可能收到本应只属于 `client1 <-> server` 那一对的 `RoleAssignment`，然后误执行 `create_peer(client1)`。
- sw-host 收到这类误投递时，确实会把它当真：
  - `handle_actr_relay` 在 `RoleAssignment` 分支里按 `remote_peer_id` 建立 peer 并可能 `create_offer`。
  - 代码位置：`bindings/web/crates/sw-host/src/runtime.rs:1709-1737`。

**但要保持一个边界**：

- 这个 mock/harness 问题 **不能解释** “client2 还没出现前，client1 warmup 已挂”的次要发现。
- 所以 E 很可能是“真 multi-client 场景下的额外污染源”，不是 TD-006 的唯一解释。

**验证方法**：

- 同时抓三页 console：server、client1、client2。
- 如果 client2 收到了与自己无关的 `role_assignment` / `create_peer` / `create_offer`，E 基本坐实。
- 一旦坐实，应该先修 mock-actrix 的 pairwise routing，再继续判断生产 runtime 是否还有独立问题。

### F. DOM 侧 `WebRtcCoordinator` 单实例（TD-001 遗留）

**当前判断**：后验概率最低，但验证成本最低，适合先一刀排掉。

**先说结论**：

- 字面上的“整个浏览器只一个 DOM `WebRtcCoordinator`”并不成立。
- `initActrDom()` / host page 都是“每个页面新建一个 coordinator”。
- 代码位置：`bindings/web/packages/actr-dom/src/index.ts:116-132`；`cli/assets/web-runtime/actr-host.html:645-651`。

**为什么仍值得保留为 F**：

- TD-001 暴露过一组 SW↔DOM dead bridge / zero-call setter，说明这一层的设计历史上确实有遗留。
- 当前 DOM 协调层又没有把实例暴露到 `window`；旧文档里提的 `window.__webrtcCoordinator` 实际上目前并不存在。
- 也就是说 F 更准确地说是：**DOM 侧观测与路由层是否还藏着“每页单例 + 只按 `peerId` 索引”的设计缺口**。

**为什么它大概率不是最终根因**：

- 多 tab 之间不共享 JS heap；跨 tab 的“DOM 单例”在浏览器模型上本就不成立。
- 如果 F 为真，多半是 C/E 先把消息错误地送进了某个 page，然后 DOM 侧再把这个错误静默吞掉。

**验证方法**：

- 非侵入式地抓 DOM console。
- 如果需要对象级检查，只做最小 patch：临时把 `coordinator` 挂到 `window`，不改 Rust，不改协议。
- 一旦看到每个 page 只收到自己的 peer/command，F 就可以快速降级。

---

## §4 建议诊断方法

### 4.1 非侵入：DOM 侧 console / 浏览器 API monkey-patch

这一档的目标是不重编 Rust、不碰 SW trace，先用 page console 判断“命令是否送到了正确页面、PeerConnection 是否真的创建、DataChannel 是否真的打开”。

**先抓现成 console**：

- `ServiceWorkerBridge` 已经会打印 `"[SW Bridge] -> SW"` / `"[SW Bridge] <- SW"`。
- 代码位置：`bindings/web/packages/actr-dom/src/sw-bridge.ts:193-205`、`241-248`。
- `WebRtcCoordinator` 已经会打印：
  - `Command ... for peer ...`
  - `Peer connection created: ...`
  - `Connection state changed: ...`
  - `DataChannel ... opened/closed`
- 代码位置：`bindings/web/packages/actr-dom/src/webrtc-coordinator.ts:101-158`、`445-503`。

**如果还不够，直接在 puppeteer 里 patch `RTCPeerConnection`**：

```js
await page.evaluateOnNewDocument(() => {
  const OrigPC = window.RTCPeerConnection;
  if (!OrigPC) return;

  window.RTCPeerConnection = class extends OrigPC {
    constructor(cfg) {
      super(cfg);
      const id = Math.random().toString(36).slice(2, 8);
      console.log('[TD006][pc:new]', id, JSON.stringify(cfg || {}));
      this.addEventListener('connectionstatechange', () => {
        console.log('[TD006][pc:state]', id, this.connectionState);
      });

      const origCreateDataChannel = this.createDataChannel.bind(this);
      this.createDataChannel = (label, opts) => {
        console.log('[TD006][dc:create]', id, label, JSON.stringify(opts || {}));
        const dc = origCreateDataChannel(label, opts);
        dc.addEventListener('open', () => console.log('[TD006][dc:open]', id, label));
        dc.addEventListener('close', () => console.log('[TD006][dc:close]', id, label));
        return dc;
      };
    }
  };
});
```

**这一档能直接回答的事**：

- 某个 page 是否收到了“不属于自己的 peer”的命令。
- `create_peer` 后有没有真的 `new RTCPeerConnection`。
- `DataChannel RPC_RELIABLE` 是否打开。
- 是否出现 `Peer already exists`、`Peer not found`、`DataChannel not open` 这类强信号。

**补一句现实问题**：

- 旧文档里提到的 `window.__webrtcCoordinator` 目前并未暴露；如果要对象级检查，需要在 `actr-host.html:645-651` 或 `initActrDom()` 附近临时挂一个 `window.__td006 = { bridge, coordinator }`。
- 这仍然算“低侵入”，因为不用重编 Rust，也不用改 SW↔DOM 协议。

### 4.2 侵入：sw-host trace

这一档的目标是把链路拆成 7 个检查点，确认请求到底死在哪一段。因为 `actor.sw.js` 已经把 SW console 广播到页面，所以这些 Rust `log::info!` 最终可以直接在 DOM page console 里看到，不必先搞 CDP attach SW target。

**推荐打点位置**：

1. `bindings/web/crates/sw-host/src/runtime.rs:3001-3060`  
   `handle_dom_control`  
   打：`client_id` / `route_key` / `request_id` / `actor_id`

2. `bindings/web/crates/sw-host/src/runtime.rs:3136-3154` 与 `1398-1409`  
   `discover_target_with_retry` / `ensure_peer_with_retry` / `ensure_peer`  
   打：`client_id` / `target_id` / `peer_id` / `known_peers`

3. `bindings/web/crates/sw-host/src/runtime.rs:1341-1355`  
   `send_webrtc_command`  
   打：`client_id` / `actor_id` / `action` / `peer_id`

4. `bindings/web/crates/sw-host/src/runtime.rs:3182-3256`  
   `handle_dom_webrtc_event` / `register_datachannel_port`  
   打：`client_id` / `event_type` / `peer_id` / `channel_id`

5. `bindings/web/crates/sw-host/src/runtime.rs:2037` 附近  
   `handle_fast_path`  
   打：`client_id` / `stream_id` / `request_id` / `is_response`

6. `bindings/web/crates/sw-host/src/inbound/dispatcher.rs:58-111` 与 `runtime.rs:2644-2715`  
   dispatcher + scheduler  
   打：`stream_id` / `peer_id` / `sender_actr_id` / `caller_id` / `route_key`

7. `bindings/web/crates/sw-host/src/runtime.rs:1520-1536`  
   `handle_rpc_response`  
   打：`client_id` / `request_id` / `PendingRpcTarget`

**推荐统一日志模板**：

```text
[TD006] dom_control client={client_id} req={request_id} route={route_key}
[TD006] ensure_peer client={client_id} target={target_id} peer={peer_id}
[TD006] webrtc_cmd client={client_id} action={action} peer={peer_id}
[TD006] webrtc_evt client={client_id} event={event_type} peer={peer_id}
[TD006] fast_path client={client_id} stream={stream_id} req={request_id} resp={is_response}
[TD006] scheduler client={client_id} stream={stream_id} peer={peer_id} sender={sender_actr_id} caller={caller_id}
[TD006] rpc_response client={client_id} req={request_id} target={pending_target}
```

**这一档能直接回答的事**：

- DOM-originated request 是否真的走到了 `ensure_peer`。
- DataChannel port 是否真的注册回到了正确 `client_id` 的 `PeerTransport`。
- server 端 inbound request 是否真的入了 mailbox / scheduler。
- response 是否被当成“unknown request_id”丢掉。

---

## §5 实际修复

这次最终落地的是组合修复：

- `testing/mock-actrix/src/signaling.rs`
  - `RoleNegotiation` 改成只投递给 `from/to` 两端，不再把 target 侧 `RoleAssignment` 广播给所有非 sender 连接。
  - 普通 relay 在 target 未绑定时改为 `warn + drop`，不再走“发给所有其他连接”的旧兼容分支。
  - `RouteCandidatesRequest` 改成只收集有活跃 WS 绑定的 actor，并优先最新 live registration。
- `bindings/web/crates/sw-host/src/runtime.rs`
  - stale peer / `datachannel_not_open` / `peer not found` 会真正触发 transport cleanup、peer reset、discovered target 失效与 pending RPC reject。
  - `disconnected` / `failed` / `closed` 现在都会同步关掉 transport，避免旧 handle 与旧 peer 状态残留。
- `bindings/web/crates/sw-host/src/transport/peer_transport.rs`
  - `get_or_create_transport()` 改成真正的 vacant-only 创建。
  - `inject_connection()` 改用 `reconnect()`，避免 DOM 侧新 port 注入后继续命中旧 ready handle。
- DOM / SW 侧
  - `bindings/web/packages/actr-dom/src/sw-bridge.ts`
  - `bindings/web/packages/actr-dom/src/webrtc-coordinator.ts`
  - `bindings/web/packages/web-sdk/src/actor.sw.js`
  - `bindings/web/packages/web-sdk/src/actor.sw.js`
  - `cli/assets/web-runtime/actor.sw.js`
  - `cli/assets/web-runtime/actor.sw.js`
  - `cli/assets/web-runtime/actr-host.html`
  - 这些地方补了 `CLIENT_UNREGISTER`、同 browser window remap 时清理旧 client、串行处理同一 `MessagePort`、lane-0 `bindRpcPort` 重绑、stale peer fast-fail 与 pending frame cleanup。
- echo 示例与 harness
  - `bindings/web/examples/echo/test-auto.js` 改成 `runBeforeUnload: true` 关闭页面，确保 unload 路径真的触发 unregister。
  - `bindings/web/examples/echo/server/src/main.ts` 与 `server-guest/src/lib.rs` 增加了更直接的观测日志，便于确认 server 确实收包与回包。

---

## §6 结案结果

结案验证：

- `SUITES='BasicFunction MultiTab' bash bindings/web/examples/echo/start-mock.sh`
  - 结果：`12/12` 全过。
  - 关键项：`1-0 Basic Echo Connectivity`、`6-4 Refresh One Client`、`6-5 Multiple Server Instances` 全部通过。
- `SUITES=BasicFunction bash bindings/web/examples/echo/start-mock.sh`
  - 结果：`6/6` 全过。

结论：

- TD-006 已关闭。
- 之前残留的 `1-0 Basic Echo Connectivity` 不再构成独立 blocker。
- 这次修复没有通过“跳过多 client / 多 workload 测试”收场，而是保留了 `MultiTab` 目标并让回归真实通过。

---

## §7 附录

### 7.1 commit 索引

- `b6b90f33`：登记 TD-001（SW↔DOM Rust `DataLane` zero-call setters）
- `829aec4c`：`CAPTURE_SW_CONSOLE` 支持，便于把 SW log 拉回 page
- `394cba1d`：登记 TD-003
- `b268b376`：`HashMap<RequestId, Ctx>`，修 dispatch ctx 单例
- `0550af5d`：WBG `actrHost*` 注入 `request_id`
- `61dbbfa7`：TD-004 根因文档（credential namespace）
- `eb034d94`：按 `client_id` 分区 credential namespace
- `b658d4f0`：mock-actrix 对 WS rebind 发 WARN
- `1ac5afd4`：同步 `cli/assets/web-runtime/` 产物
- `cb45a98d`：记录 TD-004 部分生效、TD-005/TD-006 继续追
- `eda786ed`：Phase 6 收尾与 TD-006 初始登记
- `6593fc6a`：TD-006 首版长分析文档
- `301c58d6`：multi-client RPC recovery 修复收敛；`BasicFunction + MultiTab` 12/12 全过

### 7.2 mock-actrix 日志证据

**TD-004 已经被修掉** 的直接证据：

```text
mock-actrix: registered actor (http) serial=3 name="EchoService"
mock-actrix: WS bound to HTTP-registered actor actor_id=3@.../EchoService
mock-actrix: registered actor (http) serial=4 name="echo-client-app"
mock-actrix: WS bound to HTTP-registered actor actor_id=4@.../echo-client-app
mock-actrix: registered actor (http) serial=5 name="echo-client-app"
mock-actrix: WS bound to HTTP-registered actor actor_id=5@.../echo-client-app
```

同时：

- **没有** `WS actor rebound` WARN。
- discover 阶段常能看到 `route candidates response count=1 target_type=acme.EchoService`。

这两点合起来说明：

- “第二个 client 抢走第一个 client 的 actor_id / ws 绑定”那条链已经不成立。
- 当前挂起点在 credential / discover 之后。

### 7.3 关键代码位置

| 文件 | 位置 | 说明 |
|------|------|------|
| `bindings/web/crates/sw-host/src/runtime.rs` | `487-521` | `SwRuntime` 字段；看 per-peer / per-runtime state |
| `bindings/web/crates/sw-host/src/runtime.rs` | `1398-1409` | `ensure_peer()`：`peer_id = target.to_string_repr()` |
| `bindings/web/crates/sw-host/src/runtime.rs` | `1608-1737` | `handle_actr_relay()`：session/ICE/role-assignment |
| `bindings/web/crates/sw-host/src/runtime.rs` | `1754-2011` | `handle_webrtc_event()`：DOM -> SW |
| `bindings/web/crates/sw-host/src/runtime.rs` | `2487-2516` | `CLIENTS` / `WORKLOAD` thread-local |
| `bindings/web/crates/sw-host/src/runtime.rs` | `2633-2715` | scheduler 构造 `caller_id` / `peer_id` / `RuntimeContext` |
| `bindings/web/crates/sw-host/src/runtime.rs` | `3001-3158` | DOM 控制请求 -> handler / remote call |
| `bindings/web/crates/sw-host/src/runtime.rs` | `3182-3256` | `handle_dom_webrtc_event()` / `register_datachannel_port()` |
| `bindings/web/crates/sw-host/src/inbound/dispatcher.rs` | `58-111` | inbound RPC 进入 mailbox |
| `bindings/web/crates/sw-host/src/context.rs` | `21-131` | `RuntimeContext` 与 `call_raw()` 的 `sender_actr_id` |
| `bindings/web/packages/actr-dom/src/sw-bridge.ts` | `35-48`、`75-89`、`219-223` | WebRTC command/event schema 只带 `peerId` |
| `bindings/web/packages/actr-dom/src/webrtc-coordinator.ts` | `28`、`74-78`、`151-158`、`445-503` | DOM peer map、create_peer、DataChannel open/close |
| `bindings/web/packages/web-sdk/src/actor.sw.js` | `105-106`、`521-586` | `clientPorts` + per-`clientId` 路由 |
| `bindings/web/examples/echo/test-auto.js` | `855-903` | 6-2 测试实现 |
| `testing/mock-actrix/src/signaling.rs` | `557-611` | `RoleNegotiation` 回包当前会对所有非 sender 连接扇出 |
| `testing/mock-actrix/src/signaling.rs` | `614-631` | 普通 relay 则是按 target 精确转发 |

### 7.4 复现环境

最常用的是：

```bash
cd bindings/web/examples/echo
NODE_PATH=/home/l/.local/n/lib/node_modules \
  SUITES="MultiTab" \
  CAPTURE_SW_CONSOLE=1 \
  bash start-mock.sh
```

补充说明：

- 如果要对照矩阵，用 `SUITES="BasicFunction MultiTab"`。
- 如果改过 `sw-host` Rust 或 `cli/assets/web-runtime/`，记得先处理 TD-002 提到的构建同步问题，否则观察到的不是最新 runtime。

### 7.5 历史教训

- `page.on('console')` 只能天然抓 page console，抓不到 SW console；不过当前 `actor.sw.js` 已经会把 SW log 转发回页面，所以新增 Rust `log::info!` 是能被现有测试看到的。
- 改 `sw-host` 代码后如果忘了同步 `cli/assets/web-runtime/` 并重建 `actr`，很容易拿旧 wasm 做新诊断。
- **不要过早把 mock 多 client 失败等同于生产 runtime 根因**：`mock-actrix` 的 `RoleNegotiation` 扇出语义本身就可能放大或制造 multi-client 异常。

---

## §8 下一步需要的输入

当前没有新增阻塞输入。

如果后续要继续收尾，优先级应该是：

1. 把本页前半部分标注为“修复前历史分析”的结构再做一次轻量整理。
2. 视需要把本页结案摘要同步回 `tech-debt.zh.md` / Phase 6 文档。
3. 如果后续再出现 multi-client 回归，优先先查 `mock-actrix` 信令日志和 SW stale-peer cleanup 路径。
