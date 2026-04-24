# T18: 浏览器 e2e 中 jco async-lift dispatch 永挂

**状态**：未决（2026-04-24）
**影响**：浏览器 Component Model 路径下的 e2e 跑不通。原生（native wasmtime）路径不受影响。
**关联 commits**：`548ad7d9`（Component Model bridge 完成）、`c6deaeb0`（core/hyper pub 面收缩）、`b6b90f33`（TD-001）、`10e830da` / `936cd555` / `829aec4c`（诊断设施落盘）
**关联文档**：[tech-debt.zh.md](./tech-debt.zh.md) TD-001 / TD-002

---

## 0. 这份文档在讲什么

本仓 `actr` 的浏览器 e2e（`bindings/web/examples/echo/test-auto.js`）在 Chrome M146/M147 上**跑不通**：简单 Echo RPC 发出去后 30 秒 timeout，没有任何远端响应。经过 4 轮诊断 spike 把根因收窄到 **jco 生成的 `guest.js` 里 async-lift 任务调度层**——guest component 的 dispatch 被触发了，但在调用任何 host import 之前就挂住不动。

这不是 "WebRTC peer 建不起来"，也不是 "signaling 转发失败"——这些都是表象；真正的断点比这些都要浅一层。

---

## 1. 为什么有 jco 这条链路

`actr` 的 workload 以 **WebAssembly Component Model** 形式分发，用 WIT 定义契约。三个执行底座：

| 底座 | 用途 | host 侧工具 |
|------|------|-----------|
| `wasmtime` | 原生 | 直接 Rust API，支持 Component Model |
| `wit-bindgen` C ABI (`dynclib`) | 原生动态库 | 自己的 ABI 打磨 |
| **`jco`** | **浏览器 Service Worker** | **必选**，无替代（见下） |

浏览器里 WebAssembly 的 host 强制是 JS（参见 `docs/architecture/wasm-dom-integration.zh.md`），而 Component Model 没有原生浏览器实现。`@bytecodealliance/jco` 是把 Component Model `.wasm` 转成 JS ES module 的**唯一**官方维护工具链。

guest 侧 `wit-bindgen` 用 `async: true`，生成的 core wasm 里出现 `context.get` 等 async-ABI 原语；对应 host 侧 `jco transpile --instantiation async`，这套 async-lift 机制**依赖 V8 的 JSPI**（JavaScript Promise Integration，Chrome 137+ 稳定）。

---

## 2. 浏览器 e2e 的整体结构

```
┌────────────────────────────────────────────────────────────────┐
│ Browser tab (DOM context)                                       │
│                                                                 │
│   ┌───────────────┐        ┌─────────────────────────────┐    │
│   │ app script    │  uses  │ @actr/web-sdk               │    │
│   │ (user code)   │───────▶│   actor.callRaw(…)          │    │
│   └───────────────┘        └─────────────┬───────────────┘    │
│                                          │ postMessage         │
│                                          ▼ {type:'control'}    │
│                          ┌───────────────────────────┐         │
│                          │ swPort (MessagePort)      │         │
│                          └─────────────┬─────────────┘         │
└────────────────────────────────────────┼───────────────────────┘
                                         │
┌────────────────────────────────────────┼───────────────────────┐
│ Service Worker (same origin)           ▼                        │
│                                                                 │
│   ┌───────────────────────┐  port.onmessage                    │
│   │ actor.sw.js           │───────────────┐                    │
│   │ (JS router, 548ad7d9) │               │                    │
│   └───────────────────────┘               ▼                    │
│                           wasm_bindgen.handle_dom_control(…)    │
│                                          │                      │
│                           ┌──────────────▼─────────────────┐   │
│                           │ sw-host (Rust → wasm-pack)      │   │
│                           │  runtime.rs / guest_bridge.rs   │   │
│                           │                                  │   │
│                           │  ┌─────────────────────────────┐│   │
│                           │  │ workload.dispatch(…)         ││   │
│                           │  │   │                          ││   │
│                           │  │   ▼                          ││   │
│                           │  │ jco bridge: invoke guest.js  ││   │
│                           │  └────────────┬────────────────┘│   │
│                           └───────────────┼─────────────────┘   │
│                                           │                      │
│                            ┌──────────────▼────────────────┐    │
│                            │ guest.js (jco transpile)       │    │
│                            │  + guest.core.wasm             │    │
│                            │  + adapter core1/core2 .wasm   │    │
│                            │                                │    │
│                            │  async-lift scheduling (V8 JSPI)│   │
│                            │  ──> user guest code runs ──   │    │
│                            │  ──> calls host_* imports  ──  │    │
│                            └────────────────┬──────────────┘    │
│                                             │ host imports       │
│                                             ▼                    │
│                           ┌──────────────────────────────┐      │
│                           │ sw-host guest_bridge.rs      │      │
│                           │  host_call_raw_async         │      │
│                           │  host_discover_async         │      │
│                           │  host_log_message  …         │      │
│                           └──────────┬───────────────────┘      │
└──────────────────────────────────────┼──────────────────────────┘
                                       │ WebSocket (WS)
                                       ▼
                        ┌─────────────────────────────┐
                        │ mock-actrix / real actrix    │
                        │ (signaling + AIS + MFR)      │
                        └─────────────────────────────┘
```

---

## 3. 预期的 dispatch 流程

DOM 发起一次 RPC，**按设计**会经过这些节点：

```
DOM: actor.callRaw(target, payload)
 │
 ▼  [1] swPort.postMessage({type:'control', action:'rpc_call', …})
 │
SW: port.onmessage
 │
 ▼  [2] wasm_bindgen.handle_dom_control(dom_client_id, payload)
 │
 ▼  [3] workload.dispatch(route, body, ctx)
 │
 ▼  [4] jco bridge → guest.js instantiate 后的 dispatch_fn(…)
 │
 ▼  [5] [ASYNC task] guest core.wasm 执行用户代码
 │
 ▼  [6] guest 调用 host import（e.g. host_discover_async → host_call_raw_async）
 │
 ▼  [7] host 通过 WebSocket → mock-actrix → peer → 返回
 │
 ▼  [8] host 把结果 resolve 给 guest 的 Promise
 │
 ▼  [9] guest 返回 Result
 │
 ▼  [10] dispatch Promise settle → SW 回复 DOM
```

---

## 4. 实际断点（H_Y 判定）

四轮 spike 逐层验证结果：

```
[1] DOM.postMessage          ✓  (DOM 日志可见 "📤 Sending")
[2] handle_dom_control       ✓  (BISECT probe 确认入口)
[3] workload.dispatch 分支   ✓  (BISECT probe: workload_path_selected=component)
[4] jco dispatch_fn invoked  ✓  (log: "dispatch_fn invoked, awaiting promise")
 ─────────────────────────────────
[5] guest core.wasm 执行     ✗  ← HANG HERE
[6] host import 被调用       ✗  ← 从未触达
[7] WebSocket → mock-actrix  ✗  (mock 只收到心跳，零 relay)
```

**H_Y 证据链**（第 4 轮 spike）：
- sw-host 在 `guest_bridge.rs` 8 处 host 导入函数入口加了 `log::info!("[SW][HX-PROBE] HOSTIMPORT {fn} called")` —— `host_call_raw_async` / `host_call_async` / `host_tell_async` / `host_discover_async` / `host_log_message` / `host_get_self_id` / `host_get_caller_id` / `host_get_request_id`
- 服务端实际返回的 wasm 里确实含 19 处 HX-PROBE 字符串（md5 确认）
- 同文件里其他 `log::info!`（如 `[SW] Component workload registered via jco bridge`）**能**流到 DOM console
- 90s window 里 `grep -c "HX-PROBE" test.log == 0`
- guest 甚至连 `host_log_message` / `host_get_self_id` 这类最基础的 host import 都**零调用**

**结论**：guest 从未执行到任何会调 host 的代码。断点在 `guest.js` 里的 async-lift 任务调度层，发生在用户 guest 代码被 scheduled execute 之前。

对应的 jco 源码区域：`crates/js-component-bindgen/src/intrinsics/p3/async_task.rs` 邻域（subtask_new / waitable_set / context.get 原语），属于 jco 对 WASI Preview3 async task 模型的 JS 实现。

---

## 5. 为什么之前的诊断反复走弯路

诊断时间线（每一轮打破了上一轮结论）：

```
Spike 1：e2e 跑不通 → 定位 "WebRTC 建不起来"
         ╳ 错判：WebRTC 是更下游的问题，根本没走到那
Spike 2：mock-actrix 无 SDP 转发 → 定位 "signaling 层 blocker"
         ╳ 错判：client 根本没发 SDP
Spike 3：handle_dom_control 日志缺席 → 定位 "DOM→SW port 坏了"
         ╳ 错判：日志在 SW console 里，只是 puppeteer 看不见
Spike 4：+ CDP SW console capture → 发现 guest dispatch Promise 挂住
         ✓ 正确：guest core.wasm 从未走到 host import
```

**两个反复踩的元问题**（已作 feedback memory + tech-debt 登记）：

- **盲区 1**：`puppeteer.page.on('console')` 只抓 page 的 console，**抓不到 Service Worker console**。Rust `log::info!` 经 wasm-logger 落到 SW console 完全隐形。已 commit env-var gated 的 CDP 抓取 (`CAPTURE_SW_CONSOLE=1`) 到 test-auto.js (`829aec4c`)；memory feedback `feedback_puppeteer_sw_console.md` 记录
- **盲区 2**：sw-host 编出的 wasm 放在 `bindings/web/dist/sw/`，但 `cli/src/web_assets.rs` 通过 `include_bytes!` 从 **`cli/assets/web-runtime/`** 嵌入。两者没有自动同步，改 sw-host 后忘记手动 cp 等于用旧 wasm 测新逻辑。已在 tech-debt.zh.md 登记为 **TD-002**

---

## 6. #1361 本地 patch 的去向

T18 早期定位到 bytecodealliance/jco 的 open issue **#1361**：`_lowerImport uses incorrect result pointer for async functions`（2026-04-03 报告，仍 open）。第 2 轮 spike 做了本地 patch：

```
crates/js-component-bindgen/src/intrinsics/p3/async_task.rs
  L2072:  resultPtr: params[0]   →   resultPtr: params.at(-1)
  L2298:  resultPtr: params[0]   →   resultPtr: params.at(-1)
```

产物确认：生成的 `guest.js` 里 `params.at(-1)` 出现 2 次（baseline 0）。Node ESM 冒烟通过。**但 H_Y 判定后**这个 patch 的优先级下降：

- #1361 修的是 "host import 调完后返回值内存地址算错"
- H_Y 说明 **guest 连 host import 都没调到**，#1361 路径根本触发不到
- 换句话说：patch 可能仍然正确（值得给上游 PR），但它不是我们的 blocker

**留痕**：patched jco 可以从 GitHub clone + checkout `jco-v1.18.1` + apply 上述两行 patch + `cargo xtask build debug` 快速重建。`transpile-component.sh` 已支持 `JCO_LOCAL=<path>` override (`10e830da`)。

---

## 7. 选项空间

从 "不改架构" 到 "战略让步" 依次列：

```
                    ┌────────────────────────────────────────┐
                    │ T18 root cause: jco async-lift task    │
                    │ scheduling never reaches guest body    │
                    └──────────────────┬─────────────────────┘
                                       │
          ┌────────────────────────────┼────────────────────────────┐
          │                            │                            │
          ▼                            ▼                            ▼
    ┌──────────┐              ┌─────────────────┐            ┌──────────────┐
    │ P: sync  │              │ Q: patch jco    │            │ R: 放弃 CM   │
    │ 路径     │              │ async_task.rs   │            │ 浏览器走     │
    │          │              │ 上游化          │            │ wasm-bindgen │
    └─────┬────┘              └────────┬────────┘            └──────┬───────┘
          │                            │                            │
    ┌─────▼────────┐             ┌─────▼────────┐             ┌─────▼──────┐
    │ 放弃 async:  │             │ 需要 jco /   │             │ Rust guest │
    │ guest 改     │             │ WASI-P3      │             │ 两套生成器 │
    │ wit-bindgen  │             │ async-task   │             │ (native vs │
    │ 非 async，   │             │ 协议深入理   │             │ web 不同   │
    │ host 用      │             │ 解；spike 成 │             │ 构)；战略  │
    │ --instantiation│           │ 本很高；不   │             │ 让步       │
    │ sync         │             │ 可控。       │             │            │
    │              │             │              │             │            │
    │ 绕开 JSPI    │             │ 可能顺带解   │             │ 浏览器 e2e │
    │ 和 async-task│             │ 决 #1361 上  │             │ 有成熟参   │
    │              │             │ 游问题       │             │ 考路径     │
    │              │             │              │             │            │
    │ 限制：guest  │             │              │             │ 限制：破   │
    │ 不能 await   │             │              │             │ 坏"原生    │
    │ 远端 RPC；   │             │              │             │ 浏览器同   │
    │ 需重新设计   │             │              │             │ 构"承诺    │
    │ 异步 I/O     │             │              │             │            │
    └──────────────┘             └──────────────┘             └────────────┘

                            ┌────────────────┐
                            │ S: 先冻结 T18  │
                            │ 不动代码，把本 │
                            │ 次成果全部落档 │
                            │ 等战略拍板     │
                            └────────────────┘
```

### P. `--instantiation sync` spike

- **代价**：guest 的 WIT import 从 async 变 sync，意味着 host import 必须**立即返回值**；涉及远端 I/O 的（例如 `host_call_raw_async`）必须换模型——可以返回一个 request_id，让 guest 用回调/轮询拿结果，或者拆成"立即返回 handle + 单独查询结果"
- **收益**：彻底绕开 JSPI 和 async-lift；浏览器 V8 不需要 stack switching
- **判断点**：actr 的 guest API 是否能接受 "sync host import + 自管异步"？目前 `Workload::dispatch` 返回 `ActorResult<Bytes>` 是同步签名，但内部可能 await 远端；如果可以在外层 sync、内层仍按当前模型调度，P 就可行
- **规模**：需要改 WIT + guest-side wit-bindgen 宏参数 + 整个 guest bridge 的 Promise 逻辑。中等规模

### Q. jco async_task.rs 上游化

- **代价**：需要 jco / WASI Preview3 async task 协议的深入知识；要能在 jco 仓库里跑 reproduce、搭测试用例并通过 code review
- **收益**：真修 upstream，带动整个生态
- **判断点**：我们手上没有这个级别的 jco 专家。派 agent 盲跑成本高、不可控
- **规模**：未知，可能一周以上

### R. 浏览器走 `wasm-bindgen`

- **代价**：浏览器端 Rust guest 不再是 Component Model，而是 core wasm + wasm-bindgen。需要维护两份 guest 代码（或抽象一层）
- **收益**：wasm-bindgen 成熟多年，Promise/async 用 JS 回调自然解决，无 JSPI 依赖
- **判断点**：这是对 "原生 Component Model、浏览器 Component Model 同构" 承诺的战略让步。愿不愿意让步？
- **规模**：大。要重做 bindings/web 的 guest 生成链路、重做 host side 的 wasm-bindgen import 桥

### S. 先冻结

- 不动代码。把本次所有成果（H_Y 判定、#1361 patch 留痕、诊断设施 commit、TD-001/TD-002）都已经落盘
- 原生（wasmtime）路径不受影响，该条 T18 只影响浏览器 e2e demo 和未来浏览器部署
- 等战略层拍板 P / Q / R
- **乐观叙述**：WASI Preview 3 把 async 做 first-class（async task / future / stream 类型）。jco 当前的 async-lift 实现处在 P2→P3 过渡期，很可能这就是坑的源头。P3 稳定后 jco 会重写 async task 调度，我们这个 bug 可能**自然消失**。2025 年上半年原计划发布，2026-04 仍未最终稳定，**节奏不可控**

---

## 7.1 社区层 jco 替代品调研（2026-04）

走了一圈社区，**Component Model + 浏览器 JS host 这个组合 2026-04 没有 jco 替代品**。

| 候选 | Component Model 支持 | 浏览器 JS host | 结论 |
|------|-------------------|----------------|------|
| **jco** (Bytecode Alliance) | ✓ | ✓ | 事实上的唯一选项 |
| **wasmer-js SDK** | ✗（走 WASIX 路线） | ✓ | 不同模型 |
| **Extism** (dylibso) | ✗（自家 PDK 协议） | ✓ | 不同模型 |
| **wasm-bindgen** | ✗（core wasm + JS 桥） | ✓ | 就是选项 R 的路径 |
| **wasmtime / WAMR** | ✓ | ✗（原生 only） | 不适用浏览器 |
| **Wasmer 原生** | 计划中（roadmap） | — | 未发布 |
| **`<wasm-compat>` custom element** | 概念提案（2025-09） | — | 未成熟 |

几乎所有 2026 CM 综述都默认 jco 是**唯一**的 JS host：

> "Browsers currently support raw .wasm modules, not full WASM components directly, which means that to use component-style bundles in the browser, you often need a transpilation step."
> 
> "Tools like the jco package on npm bridge the gap of component bundles not being directly supported in browsers..."

**含义**：
- "换一个 CM runtime 解决问题"**不成立**。要么 P / Q / R，要么等上游（S 的乐观叙述）
- 如果未来 Wasmer 的 CM 浏览器 runtime 发布，本节需要重新评估

---

## 8. 本次沉淀的产物索引

**已落盘 commit**：

| commit | 内容 |
|--------|------|
| `10e830da` | `transpile-component.sh` 加 `JCO_LOCAL` 逃生口 |
| `936cd555` | `sw-host/build.sh` + `dom-bridge/build.sh` 清 `RUSTFLAGS` 避免 mold 漏进 wasm32 |
| `829aec4c` | `test-auto.js` 加 `CAPTURE_SW_CONSOLE=1` 订阅 SW target console |
| `b6b90f33` | TD-001 登记（SW↔DOM DataLane 5 个 zero-call setter） |
| （本次）| TD-002 登记（sw-host wasm 未自动同步到 cli/assets） + 本文档 |

**未入库但可复活**：

| 位置 | 内容 | 复活方式 |
|------|------|---------|
| jco fork v1.18.1 + #1361 patch | `crates/js-component-bindgen/src/intrinsics/p3/async_task.rs` L2072 / L2298 两处 `params[0] → params.at(-1)` | clone + checkout tag + patch + `cargo xtask build debug`（注意 clear `RUSTFLAGS`） |
| HX-PROBE 诊断代码 | 8 处 `log::info!("[SW][HX-PROBE] HOSTIMPORT {fn} called …")` 加在 `bindings/web/crates/sw-host/src/guest_bridge.rs` host 导入函数入口 | grep git log / 本文档搜 `HX-PROBE` |

**memory**：
- `feedback_puppeteer_sw_console.md`：诊断浏览器 SW Rust 行为必须用 CDP SW target console

---

## 9. 下一位接手时的最小起步清单

1. 读本文档第 4 章（H_Y 判定）和第 7 章（选项）
2. 确定要走 P / Q / R 哪条
3. 启动前：
   - 如果要跑 e2e：记得 sw-host 改完后 `cp bindings/web/dist/sw/actr_sw_host_bg.wasm cli/assets/web-runtime/` + `cargo build -p actr-cli --bin actr`（TD-002）
   - 如果要抓 SW 日志：`CAPTURE_SW_CONSOLE=1 node test-auto.js ...`
   - 如果要用 patched jco：`export JCO_LOCAL=/path/to/jco/packages/jco/src/jco.js`
4. 重新做诊断之前读 memory `feedback_puppeteer_sw_console.md`，避免第三次踩 SW console 盲区

---

## 10. 未展开的遗留问题

- **favicon.ico 404**：`actr run --web` 的 static 路由没 serve `/favicon.ico`。多次 spike 里这条 404 总是混在失败日志里误导判断。修法小（给 `cli/src/commands/run.rs:728` 的路由表加一条 no-op 或空 favicon 响应），但非 T18 必修
- **rustup 软链异常**：`/home/l/.rustup` 是空目录，上轮 W2-F agent 临时 `sudo ln -s /mnt/sdb1/l_misc/.rustup /home/l/.rustup`。非本仓问题，不必在此解决
