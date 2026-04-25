# 系统架构变更报告 · 2026-04 review batch

**评审分支**：`review/main-pending-2026-04` (HEAD = `67736e2d`，相对 `origin/main` 领先 169 commits)
**生成日期**：2026-04-26
**关联文档**：
- [Option U 总体设计](./option-u-wit-compile-web.zh.md)
- [Option U Phase 6 γ-unified 详细设计](./option-u-phase6-gamma-unified.zh.md)
- [TD-006 multi-client RPC 分析](./td-006-multi-client-rpc-analysis.zh.md)
- [T18 jco async-lift hang（已绕开）](./t18-jco-async-lift-hang.zh.md)
- [tech-debt 登记](./tech-debt.zh.md)

---

## 0. 执行摘要 (TL;DR)

本批次 169 个 commit 同时推进了 **5 条独立主线**，最终落点是：

1. **浏览器路径整体替换** —— CM (Component Model) + jco transpile 改为 Option U（WIT → wasm-bindgen 直产物）。jco / Component Model 在浏览器侧**完全删除**。原生 wasmtime 端 CM 路径**不变**。
2. **原生宿主重构** —— `Hyper<State>` 引入 typestate，旧 `Hyper::attach` API 拆为 `Hyper` + `Node<State>` 两阶段；`PlatformProvider` 收敛为 3 个领域问题；`TrustProvider` 可插拔。
3. **API 公共面收窄** —— `core/hyper` / `core/framework` 系统性地把内部实现降级为 `pub(crate)`，仅保留稳定的对外 API。
4. **测试基建升级** —— `mock-actrix` 取代外部 actrix 依赖；浏览器 e2e（BasicFunction + MultiTab）`12/12` 稳过；CI 加 WIT codegen + cli/assets 双 drift gate。
5. **技术债批量结案** —— TD-001 / TD-002 / TD-003 / TD-004 / TD-005 / TD-006 全部关闭。

**变更体量**：

| 维度 | 数 |
|------|----|
| commit | 169 |
| 改动文件 | 531 |
| 行净增 | +64,527 / -23,150 (净 +41,377，主要是新增 `actr-web-abi` 生成产物 + WIT 契约 + 新 codegen 工具) |
| 删除整模块 | 5 (`webrtc_recovery`, `dom-bridge::WebRtcCoordinator`, CM browser bridge, `transpile-component.sh`, TS echo-workload polyglot demo) |
| 新增整模块 | 4 (`tools/wit-compile-web`, `actr-web-abi`, `core/framework/web/`, `mock-actrix`) |

---

## 1. 架构全景图

### 1.1 三分图：Native / Browser / Mobile 三个底座

```
                    ┌──────────────────────────────────────┐
                    │  core/framework/wit/actr-workload.wit │
                    │       （单一契约真源）                 │
                    └──────┬───────────────┬───────────┬───┘
                           │               │           │
        ┌──────────────────┘               │           └──────────────────┐
        │                                  │                              │
        ▼                                  ▼                              ▼
  ┌──────────┐                       ┌─────────┐                    ┌─────────┐
  │  Native  │                       │ Browser │                    │ Mobile  │
  │ (server) │                       │  (SW)   │                    │(static) │
  └────┬─────┘                       └────┬────┘                    └────┬────┘
       │                                  │                              │
   wit-bindgen                  tools/wit-compile-web              wit-bindgen c
       │                                  │                              │
   wasm-component-ld              wasm-pack --target                hand-written
       │                          no-modules                        C ABI
       ▼                                  ▼                              ▼
  ┌──────────┐                       ┌─────────┐                    ┌─────────┐
  │wasmtime  │                       │ wasm-   │                    │   FFI   │
  │ +CM async│                       │ bindgen │                    │  dlopen │
  │ +JSPI ❌ │                       │ +async  │                    │         │
  │not needed│                       │原生     │                    │         │
  └──────────┘                       └─────────┘                    └─────────┘
   core/hyper                bindings/web                       bindings/{ffi,
                                                                kotlin,swift}
```

**关键不变量**：

- WIT 契约是**唯一事实源**。三个底座都从这一份 `actr-workload.wit` 派生绑定。
- 三种产物形态**互不兼容**（CM canonical ABI ≠ wasm-bindgen JS ABI ≠ C ABI），但**语义等价**。
- 浏览器底座**不再依赖 CM**（Phase 8 删除）。这是本批次最深的架构变更。

### 1.2 浏览器子系统（Before / After 对比）

#### Before：CM + jco（已删除）

```
   .actr 包                                          浏览器
   ┌─────────────┐                              ┌──────────────┐
   │ Component   │   actr build → jco transpile │ Service Worker│
   │ Model wasm  │ ─────────────────────────────►│              │
   └─────────────┘                              │ ┌──────────┐ │
   ┌─────────────┐                              │ │ jco-gen  │ │
   │ <stem>.jco/ │   guest.js + core wasm       │ │ ES module│ │
   │  guest.js   │ ─────────────────────────────►│ └────┬─────┘ │
   │  core.wasm  │                              │      │       │
   └─────────────┘                              │      │ host  │
                                                │      │imports│
                                                │ ┌────▼─────┐ │
                                                │ │ sw-host  │ │
                                                │ │   wasm   │ │
                                                │ └──────────┘ │
                                                └──────────────┘

   依赖：jco 1.18.1 (npm) · wit-bindgen async: true ·
         Chrome JSPI (M137+, M146 仍是过渡态)
   断点：T18 jco async-lift dispatch promise 永挂
```

#### After：Option U（当前唯一路径）

```
   WIT 契约
   core/framework/wit/actr-workload.wit
        │
        │ tools/wit-compile-web (本批次新增)
        ▼
   bindings/web/crates/actr-web-abi/src/
        ├── types.rs    （10 record + 3 variant 镜像）
        ├── guest.rs    （8 host imports + async wrappers）
        └── host.rs     （Workload trait + 17 #[wasm_bindgen] exports）
        │
        │ wasm-pack --target no-modules
        ▼
   .actr 包                                          浏览器
   ┌─────────────┐                              ┌──────────────┐
   │ wasm32-     │   actr build (sign only)     │ Service Worker│
   │ unknown-    │ ─────────────────────────────►│              │
   │ unknown     │                              │ ┌──────────┐ │
   └─────────────┘                              │ │ guest    │ │
   ┌─────────────┐                              │ │ wasm     │ │
   │ <stem>.wbg/ │   guest.js + guest_bg.wasm   │ │ (wbg)    │ │
   │  guest.js   │ ─────────────────────────────►│ └────┬─────┘ │
   │  guest_bg.wasm                              │      │       │
   └─────────────┘                              │      │ actrHost│
                                                │      │ globals│
                                                │ ┌────▼─────┐ │
                                                │ │ sw-host  │ │
                                                │ │   wasm   │ │
                                                │ └──────────┘ │
                                                └──────────────┘

   依赖：wasm-bindgen + wasm_bindgen_futures (Rust 原生 async)
   优点：1. 不依赖 JSPI / 浏览器版本特性
         2. 不依赖 jco 工具链
         3. Rust async fn 原生工作
   代价：~600 行自维护 codegen (tools/wit-compile-web)，但可控且 CI 守
```

#### 关键决策时间线（从 T18 → Option U）

```
T18 诊断 (4 轮 spike)
  │
  ├─ 假说 1: WebRTC peer 建不起来          → 否
  ├─ 假说 2: signaling 转发失败             → 否
  ├─ 假说 3: DOM→SW MessagePort 协议         → 否
  └─ 假说 4: jco async-lift 调度永挂         → ✓ 真因
              │
              ├─ 选项 R: 放弃 jco / 放弃 CM
              ├─ 选项 P: patch jco upstream (#1361)
              ├─ 选项 L: 我们 fork jco
              └─ 选项 U: 自建 WIT→wasm-bindgen ←─ 选定
                          │
                          ├─ Phase 0  探查 (GREEN LIGHT)
                          ├─ Phase 1  types.rs 生成
                          ├─ Phase 2  guest.rs 生成
                          ├─ Phase 3  host.rs 生成
                          ├─ Phase 4  Echo e2e BasicFunction 6/6
                          ├─ Phase 5  CI drift gate + 文档
                          ├─ Phase 6  γ-unified 整合 (entry! 宏)
                          │           ├─ TD-003 GUEST_CTX 单例 → HashMap<RequestId>
                          │           ├─ TD-004 cred namespace 按 client_id
                          │           └─ TD-006 multi-client RPC 综合修
                          ├─ Phase 7  data-stream 迁 WBG (跳过, 不依赖 CM)
                          └─ Phase 8  CM 路径整体删除
```

---

## 2. 浏览器路径详解

### 2.1 模块依赖（Browser 端）

```
   ┌──────────────────────────────────────────────────────┐
   │ Application (echo client / server / data-stream...) │
   └────────────┬───────────────────────────┬─────────────┘
                │                           │
                │ actr_framework::entry!{}  │
                │ (跨 target unified macro) │
                ▼                           ▼
   ┌─────────────────────────┐   ┌──────────────────────┐
   │ core/framework          │   │ actr-web-abi         │
   │  ├── web/               │   │ (生成自 WIT)         │
   │  │   ├── adapter.rs     │   │  ├── types.rs        │
   │  │   ├── context.rs     │◄──┤  ├── guest.rs        │
   │  │   └── mod.rs         │   │  └── host.rs         │
   │  ├── service_handler.rs │   │                      │
   │  └── workload.rs        │   │  Workload trait      │
   └────────┬────────────────┘   │  (#[doc(hidden)] pub)│
            │                    └──────────┬───────────┘
            │                               │
            │ WebWorkloadAdapter<W>         │ register_workload
            │   wraps Workload → web_host::Workload (WIT)
            ▼                               ▼
   ┌──────────────────────────────────────────────────────┐
   │ Service Worker JS (actor.sw.js)                      │
   │  ├── installActrHostGlobals()  ←── 8 host import shim │
   │  ├── loadWithGuestBridge()                            │
   │  └── wasm_bindgen.register_guest_workload(dispatchFn)│
   └────────────┬─────────────────────────────────────────┘
                │
                ▼
   ┌──────────────────────────────────────────────────────┐
   │ actr-sw-host (wasm-bindgen)                          │
   │  ├── guest_bridge.rs   ── host imports + ctx routing │
   │  │   └── DISPATCH_CTXS: HashMap<RequestId, Ctx>      │
   │  ├── runtime.rs        ── client lifecycle           │
   │  ├── transport/        ── PeerTransport / SwTransport│
   │  ├── inbound/          ── packet dispatch            │
   │  └── outbound/         ── HostGate / PeerGate        │
   └──────────────────────────────────────────────────────┘
```

### 2.2 一次 echo RPC 的完整数据流（Option U / Phase 6 γ-unified）

```
[DOM page]                 [Service Worker]              [Remote peer]
     │                            │                            │
     │ user click "Send"          │                            │
     │ DOM control → SW           │                            │
     ├───────────────────────────►│                            │
     │   {request_id, route_key}  │                            │
     │                            │ handle_dom_control()       │
     │                            │                            │
     │                            │ ctx = WebContext::new(     │
     │                            │   self_id, caller_id,      │
     │                            │   request_id)              │
     │                            │                            │
     │                            │ workload.dispatch(envelope, ctx)
     │                            │                  │         │
     │                            │                  ▼         │
     │                            │ JS ─► wasm_bindgen.dispatch(envelope)
     │                            │     (guest module via wasm-pack)
     │                            │                  │         │
     │                            │  guest 代码 await actrHostCallRaw(
     │                            │     request_id, target, route, payload)
     │                            │                  │         │
     │                            │                  ▼         │
     │                            │  sw-host host_call_raw_async:
     │                            │     ctx_get(request_id)    │
     │                            │     → RuntimeContext       │
     │                            │     → call_raw via PeerTransport
     │                            │                  │         │
     │                            │                  ├───────► │
     │                            │                  │ WebRTC DC│
     │                            │                  │         │
     │                            │                  │ ◄───────┤ reply
     │                            │                  │         │
     │                            │ ◄────────────────┘         │
     │                            │ guest 收 reply, return    │
     │                            │ from dispatch              │
     │                            │                            │
     │ ◄──────────────────────────┤ control_response           │
     │   {request_id, result}     │                            │
     │                            │                            │
   render reply                                                │
```

**关键不变量**：
- `request_id` **贯穿全链**——DOM 生成 → SW 注入 ctx → guest 透传给每个 host import → sw-host 用 id 查 ctx
- `DISPATCH_CTXS` 是 `HashMap<RequestId, Rc<RuntimeContext>>`（γ-unified §3.6）
- 多 client 并发时不会串味（这是 TD-003 的修复）

### 2.3 Service Worker 路由表

```
┌────────────────────────────────────────────────────┐
│ actr CLI ── actr run --web -c <config>.actr.toml   │
└────────────────────────────────────────────────────┘
       │
       │ axum Router (cli/src/commands/run.rs)
       ▼
   Routes:
   ┌──────────────────────────────────────────────┐
   │ GET  /actr-runtime-config.json               │ ─► 注入 trust / package_url / signaling_url
   │ GET  /actor.sw.js                            │ ─► 嵌入式 SW (Phase 8 后单一 entry)
   │ GET  /packages/actr_sw_host_bg.wasm          │ ─► sw-host wasm-pack 产物
   │ GET  /packages/actr_sw_host.js               │ ─► sw-host JS glue
   │ GET  /packages/<name>.actr                   │ ─► 签名后的 .actr 包
   │ GET  /packages/<name>.wbg/*                  │ ─► wbg 伴生 (guest.js + guest_bg.wasm)
   │ GET  /                                       │ ─► actr-host.html (内嵌 actr-dom)
   └──────────────────────────────────────────────┘

   Phase 8 之前还有：
       GET /packages/<name>.jco/*   (CM jco bundle, 已删)
       env ACTR_WEB_GUEST_MODE=cm/wbg 切两套 SW (已删, 永远 wbg)
```

---

## 3. 原生路径变更

### 3.1 Hyper<S> typestate 引入

**Before**：单一 `Hyper` struct，`.attach()` / `.run()` 等方法在所有阶段都可用，编译器无法保证调用顺序。

**After**：分两层 typestate

```
   Hyper<Init>  ──with_runtime_config────────► Hyper<Configured>
                                                    │
                                                    │ build_node()
                                                    ▼
                                               Node<Init>
                                                    │
                                ┌───────────────────┼─────────────────────┐
                                │                   │                     │
                       attach_workload()      load_package()         from_config_file()
                                │                   │                     │
                                ▼                   ▼                     ▼
                          Node<Attached>      Node<Loaded>           Node<Init>
                                                    │
                                                    │ run()
                                                    ▼
                                              Node<Running>
                                                    │
                                                    │ stop().await
                                                    ▼
                                              Node<Stopped>
```

**收益**：
- 编译期阻止"未配置就 run"、"未 attach 就 dispatch"等误用
- IDE 自动补全只显示当前阶段合法的方法

**关联 commit**：
- `48ba2f42 refactor(hyper): introduce Hyper<S> typestate host pipeline`
- `e8df80ec refactor(hyper): split Hyper<State> typestate into Hyper + Node<State>`
- `eae01427 feat(hyper): introduce Node<Init> state and Node entry methods`
- `fa1a0f45 refactor(hyper): remove Hyper::attach, migrate all callers to Node`

### 3.2 PlatformProvider 三问

**Before**：`PlatformProvider` trait 暴露 ~10 个方法，每个新平台要实现一堆，且语义重叠。

**After**：收敛为 3 个领域问题（commit `3b222abc`）：

```
    pub trait PlatformProvider {
        fn storage_root(&self) -> &Path;     // "我数据存哪？"
        fn networking(&self) -> &Networking; // "我能联网吗？怎么连？"
        fn lifecycle(&self) -> &Lifecycle;   // "我什么时候被叫起 / 暂停？"
    }
```

每个子 trait（`Networking`、`Lifecycle`）独立演进，`PlatformProvider` 只负责持有它们。

### 3.3 TrustProvider 可插拔

`Node::from_config_file` 现在**强制**显式传入 `TrustProvider`（commit `c56c36d3`）。Trust 验证逻辑也下沉到 wasm 侧（commit `0da57b54 refactor(hyper,web): pluggable TrustProvider + wasm-side package verify`）。

**影响**：
- Node 启动时不会"默认信任所有"——如果忘传 trust，编译报错
- web 端 SW 内部用 `verify_and_extract_actr_package(buffer, trustJson)` 验证 .actr 包

### 3.4 命名规范化

| 旧 | 新 | commit | 理由 |
|----|----|--------|------|
| `ActrSystem` | `ActrNode` | `498c20dc` | "system" 太宽泛，`Node` 表达"集群里一个节点" |
| `PackageExecutionBackend` | `BinaryKind` | `0304a086` | 名字直接说明它就是 binary 类型枚举 |
| `PackageExecutionBackend::Cdylib` | `BinaryKind::DynClib` | `a0fdce60` | 与底座命名（DynClib / Linked / Wasm）对齐 |
| `inject_credential` | `pre_registered_credential` | `01deb819` | "inject" 含安全暗示，实际是预注册 |
| `Hyper::attach` | 删除，迁 `Node<Attached>` | `fa1a0f45` | typestate 重构后无意义 |
| `is_server` 标志 | per-peer connection role negotiation | `f4a66517` `956976fc` | 角色不该是全局常量 |

### 3.5 API 公共面收敛

15+ 个 `refactor(hyper): narrow ... public surface` commit 把内部实现降级为 `pub(crate)`。背景：T5.5 跨 binding 公开面扫描（W1-A...W2-F 任务系列）。

收窄前后**对外 trait 数量基本不变**，但**结构体 / 函数公开面缩了 ~40%**。这是为了：
- 锁定稳定的 SDK 表面，方便后续内部实现重写不破坏外部
- 降低 IDE 自动补全噪声

具体扫描清单见 `core/hyper/API_CLEANUP_PROGRESS.zh.md`。

---

## 4. 测试基础设施

### 4.1 mock-actrix：自包含信令 / AIS / MFR

**Before**：浏览器 e2e 必须依赖外部 `actrix` 二进制（peer 仓库 `../../actrix`）+ `sqlite3` 种数据。CI 需要预装这俩。

**After**：`testing/mock-actrix` crate（commit `40309a0e testing: expand mock-signaling into full mock-actrix`），单一进程提供：

```
   ┌─────────────────────────────────────────┐
   │ mock-actrix (cargo run -p actr-mock-actrix)
   │                                         │
   │  HTTP   : :8081/admin/* (seed realm/MFR/pkg)
   │  HTTP   : :8081/ais     (Authority Info Service)
   │  HTTP   : :8081/mfr     (Manufacturer Registry)
   │  WS     : :8081/signaling/ws (signaling)
   └─────────────────────────────────────────┘
   零 sqlite，零外部依赖，e2e 自包含
```

**配套**：
- `bindings/web/examples/echo/start-mock.sh` ── 一行起来 mock + actr run --web + 跑 puppeteer
- `bindings/web/examples/echo/register-mock.sh` ── HTTP 写 /admin/* 种数据

### 4.2 浏览器 e2e 测试矩阵

| Suite | # | 名称 | 结果 |
|-------|---|------|------|
| BasicFunction | 1-1 | Manual Send | ✓ |
| BasicFunction | 1-2 | Empty Message Send | ✓ |
| BasicFunction | 1-3 | Rapid Consecutive Sends | ✓ |
| BasicFunction | 1-4 | Large Message Send | ✓ |
| BasicFunction | 1-5 | Special Characters | ✓ |
| BasicFunction | 1-6 | Send with Enter Key | ✓ |
| MultiTab | 6-1 | Two Client Tabs | ✓ |
| MultiTab | 6-2 | Concurrent Multi-Client Sends | ✓ |
| MultiTab | 6-3 | Close One Client | ✓ |
| MultiTab | 6-4 | Refresh One Client | ✓ |
| MultiTab | 6-5 | Multiple Server Instances | ✓ |
| MultiTab | 6-6 | Shared SW Isolation | ✓ |

**`12/12 PASS`** —— `bash bindings/web/examples/echo/start-mock.sh` (默认 BasicFunction，传 `SUITES='BasicFunction MultiTab'` 跑全套)

### 4.3 CI Drift Gates（Phase 5 新增）

`.github/workflows/ci-web.yml` 在原 `cargo check --workspace --target wasm32-unknown-unknown` 之后追加两步：

```yaml
- name: WIT codegen drift gate (Option U)
  run: cargo run -p actr-wit-compile-web -- --check

- name: cli/assets sync drift gate
  run: bash bindings/web/scripts/sync-cli-assets.sh --check
```

捕获两类漂移：

1. WIT 改了但忘记重新 generate `actr-web-abi/src/{types,guest,host}.rs`
2. sw-host wasm 或 actor.sw.js 改了但忘记 sync 到 `cli/assets/web-runtime/`（这是 TD-002 的核心痛点）

---

## 5. 技术债结案

| TD | 问题 | 修复 | commit |
|----|------|------|--------|
| **TD-001** | dom-bridge / sw-host 5 个 zero-call DataLane setter（548ad7d9 后绕开未删） | 删除整个 setter cascade（-703 LOC）+ 后续 `WebRtcCoordinator` 整模块 + `webrtc_recovery` 整模块 | `ec6ffafd` `15fa2aa1` `67736e2d` |
| **TD-002** | sw-host wasm 产物到 cli/assets 无自动同步 | `bindings/web/scripts/sync-cli-assets.sh` + sw-host build.sh 末尾自动调 + CI `--check` | `cd5971da` `f4986dd0` |
| **TD-003** | `GUEST_CTX: thread_local Option<Ctx>` 单例覆盖（多 client 并发踩） | γ-unified：改 `DISPATCH_CTXS: HashMap<RequestId, Ctx>`；WIT 所有 host import 加 request_id 参数 | Phase 6 系列 |
| **TD-004** | 同源 SW 下第二个 client 复用第一个 client 的 IndexedDB 凭据 | cred_kv_namespace 改为 `actr_credentials_{actr_type}_{client_id}`；mock-actrix 加 rebind WARN | `eb034d94` `b658d4f0` |
| **TD-005** | "WebWorkloadAdapter Reflect.get called on non-object" | 误报：stale `target/debug/actr` binary。重 build 即过 | （无 commit，文档结案） |
| **TD-006** | 2 client 并发 RPC 仍挂（TD-003/004 修后剩余 4 个 MultiTab 失败） | 综合修：mock-actrix relay 精确投递 + sw-host stale peer cleanup + DOM/SW client lifecycle | `301c58d6` |

**全部 6 个 TD 已 close**，`tech-debt.zh.md` 全部条目状态 = "已解决"。

---

## 6. 删除清单（废弃方向产物清算）

### 6.1 浏览器侧 CM/jco 路径（Phase 8 + 后续清扫）

| 删除项 | 类型 | 说明 |
|--------|------|------|
| `bindings/web/packages/web-sdk/src/actor.sw.js` (CM 旧版) | 文件 | `loadWithComponentBridge` + jco import 表 |
| `bindings/web/scripts/transpile-component.sh` | 文件 | 调 `jco transpile` 的 wrapper |
| `bindings/web/examples/echo/start.sh` | 文件 | CM-only launcher (`jco transpile` + `ACTR_WEB_GUEST_MODE=cm`) |
| `bindings/web/examples/echo/server/wasm/` | 目录 | pre-Phase-6 的 `echo-server-web` 独立 crate |
| `bindings/web/examples/echo/{server,client}-guest-wbg/` | 目录 | Phase 6c 删 source 但 disk 留下 target/pkg |
| `bindings/web/examples/echo/release/*.jco` | 目录 | jco 转译产物 |
| `cli/src/commands/run.rs::serve_actor_sw_js` 的 env 选择器 | 代码 | `ACTR_WEB_GUEST_MODE` 不再有意义 |
| `cli/src/commands/run.rs` 的 `jco_dir` mount 路由 | 代码 | `<package_url>.jco/*` SW 不再请求 |
| `cli/src/web_assets.rs::ACTOR_WBG_SW_JS` const | 代码 | 与 `ACTOR_SW_JS` 合并 |
| `bindings/web/package.json` 的 `@bytecodealliance/jco@1.18.1` | 依赖 | npm devDep |
| `bindings/web/package.json` 的 `transpile-component` script | 配置 | 引用已删脚本 |

### 6.2 死代码模块

| 模块 | 删除原因 | commit |
|------|---------|--------|
| `bindings/web/crates/sw-host/src/webrtc_recovery.rs` | `WebRtcRecoveryManager` 零外部消费，`request_webrtc_rebuild` 自带 `#[allow(dead_code)]` | `67736e2d` |
| `bindings/web/crates/dom-bridge/src/webrtc/coordinator.rs` (整模块) | TD-001 cascade 收尾；`WebRtcCoordinator` 在 dom-bridge 这一份零调用 | `15fa2aa1` |
| `bindings/web/examples/{fastpath-demo,zerocopy-comparison}/` | README-only（无源码），移到 `bindings/web/docs/` | `15fa2aa1` |
| `examples/typescript/echo-workload/` | jco componentize polyglot demo，"experimental / not in CI"，零代码引用 | `67736e2d` |

### 6.3 重命名（向后兼容打破点）

| 旧名 | 新名 | 范围 | commit |
|------|------|------|--------|
| `actor.sw.js` (CM) | 已删 | web-sdk + cli/assets | `855bb658` |
| `actor-wbg.sw.js` | `actor.sw.js` | web-sdk + cli/assets | `855bb658` |
| `start-mock-wbg.sh` | `start-mock.sh` | echo example | `855bb658` |
| `register_component_workload` | `register_guest_workload` | sw-host 公开面 + JS 调用 | `855bb658` |

---

## 7. API 兼容性影响

### 7.1 Rust crate 公开面

**新增**：

```rust
// core/framework
pub mod web;                          // WebContext / WebWorkloadAdapter
pub trait ServiceHandler;             // 6b 关联类型 trait
pub use entry;                        // entry! macro 加 web target arm

// bindings/web/crates/actr-web-abi
pub mod types;  pub mod guest;  pub mod host;  // 自 WIT 生成

// tools/wit-compile-web
// 不是 lib，只有 bin
```

**删除 / 收窄**：

```rust
// core/hyper 大量 pub → pub(crate)
// 详细清单见 core/hyper/API_CLEANUP_PROGRESS.zh.md
// 对外仍提供：Node / Hyper / WorkloadPackage / Context / RpcRequest 等核心类型

// bindings/web/crates/sw-host
- pub fn register_component_workload  →  + pub fn register_guest_workload
- pub use webrtc_recovery::{...}       →  整模块删除
- pub use webrtc::WebRtcCoordinator    →  整模块删除 (dom-bridge)
```

### 7.2 WIT 契约 (`actr-workload.wit`)

WIT 本身**结构稳定**（17 个方法 + 10 record + 3 variant）。**重要**变化：

- 所有 `host` interface 方法**首参数**加 `request-id: string`（Phase 6 γ-unified §3.4）
- 这是为了让 sw-host 端 `DISPATCH_CTXS` HashMap 能根据 id 查 ctx
- 原生 wasmtime 端透明（wit-bindgen 重新生成自动适应）

### 7.3 JS / TS 公开面

- `web-sdk/src/actor.sw.js` ── 现在指向 wbg-style SW（原 actor-wbg.sw.js 内容），调用方式不变（仍然 `navigator.serviceWorker.register('/actor.sw.js')`）
- `actrHost*` JS globals ── 8 个 wasm 导入的 JS 实现，由 SW JS 在 `installActrHostGlobals()` 中安装，签名加 `requestId` 首参
- `register_component_workload` → `register_guest_workload` ── 这是 wasm-bindgen 导出，JS 调用要改名

---

## 8. 升级 / 迁移指南

### 8.1 给浏览器侧 SDK 消费者

| 改动 | 你需要做什么 |
|------|------------|
| `actor.sw.js` 行为变 | 无需改 ── URL 不变，HTML script tag 不动 |
| `actr run --web` 不再读 `ACTR_WEB_GUEST_MODE` | 删掉环境变量设置 |
| 自定义 SW 调 `register_component_workload` | 改成 `register_guest_workload` |
| 用 `<package>.jco/` 作 SW 旁车 | 改用 `<package>.wbg/`；产物由 `wasm-pack --target no-modules` 而不是 `jco transpile` 出 |

### 8.2 给原生 host 消费者

| 改动 | 你需要做什么 |
|------|------------|
| `Hyper::attach(...)` API | 改用 `Hyper.build_node().attach_workload(...)` |
| `ActrSystem` 类型名 | 全部 → `ActrNode` |
| `PackageExecutionBackend::Cdylib` | → `BinaryKind::DynClib` |
| `Node::from_config_file` 不传 trust | 编译错；显式传 `TrustProvider` impl |
| `inject_credential` API | → `pre_registered_credential` |

### 8.3 给 Web 示例 / e2e 维护者

```bash
# 旧
cd bindings/web/examples/echo
./start.sh             # CM-only, 已删
./start-mock-wbg.sh    # 已重命名

# 新
cd bindings/web/examples/echo
./start-mock.sh                                   # default BasicFunction
SUITES='BasicFunction MultiTab' ./start-mock.sh   # 全套 12/12
```

### 8.4 给 protoc-gen 用户

`bindings/web/examples/codegen-test` 是 codegen 的 smoke target。本批次 `tools/protoc-gen/web/src/codegen.rs` 增了 `ServiceHandler` impl 自动 emit（commit `cd372f94`），无需改 `.proto` 即可获得新 trait 实现。

---

## 9. 风险与后续

### 9.1 已知风险（已评估接受）

| 风险 | 缓解 |
|------|------|
| `tools/wit-compile-web` 是自建 codegen，要跟随 wit-parser / wasm-bindgen 升级 | 钉死 `wit-parser=0.247.0` 与 `wit-lint` 同版本；CI drift gate 守 |
| 浏览器侧失去 Component Model "可组合性" | 当前没用到这个特性；如果未来要用，得重新评估 |
| 169 commits 一次性合入 main 的审查负担 | 拆 review 分支已开（`review/main-pending-2026-04`），可按 commit 拆细 PR |
| 隔壁同学 `bindings/python/examples/` WIP 误放位置 | 已清理（不属于 git tree） |

### 9.2 后续工作（非本批次）

- **WebContext 5 个 NotImplemented**（`register_stream` / `send_data_stream` / `register_media_track` / `send_media_sample` / `add_media_track` / `remove_media_track`）：浏览器侧 stream / media fast path 是设计上的 permanent NotImplemented；如果未来要支持，需要扩 WIT 契约 + actr-web-abi 重生成
- **sw-host 几个保留 TODO**：lifecycle PONG / error_handler 自动恢复 / system local actor (Phase 2) ── 都已改为说明性注释，描述真实的未来架构工作
- **dom-bridge crate**：浏览器实际用 TS @actr/dom，dom-bridge 是 Rust 影子 crate 没有运行时消费者。删除是更彻底的清理（本批次未做）
- **Mobile 静态链接路径**（Linked variant）：当前完全靠 hand-written C ABI；`wit-lint` 是漂移守护，但仍是手工维护

### 9.3 数据流死角检查

本批次未触及但仍带 TODO 的领域：
- `core/hyper/src/wasm/host.rs::call_on_start` ── 已 gate 在 `feature = "test-utils"`，生产路径未拉起 lifecycle
- `bindings/web/crates/dom-bridge/src/lifecycle.rs::DOM_PING` ── 发送侧（dom-bridge crate）在 production 未真正跑（用 TS @actr/dom）

---

## 10. 提交时间线（按主线分组）

```
2026-04 review batch 主线时间线（不严格按时间，按主线分组）
=========================================================

[早期：原生重构铺垫]
1c59175b  refactor(cli): reorganize commands by audience
48ba2f42  refactor(hyper): introduce Hyper<S> typestate
3b222abc  refactor(platform): collapse PlatformProvider to 3 questions
e8df80ec  split Hyper<State> into Hyper + Node<State>
0da57b54  refactor(hyper,web): pluggable TrustProvider

[CM 服务端确立]
84767deb  feat(framework): define actr workload WIT contract
1d0d6ca6  feat(hyper): rewrite wasm backend on Component Model
aef0ba9e  feat(framework): switch wasm guest runtime to wit-bindgen
15c274c7  feat(pack): bump .actr format for Component binaries

[Web 第一波 (Phase 1-3 component bridge)]
548ad7d9  ... (CM browser bridge, 后被 Option U 替代)
... (诊断 spike 系列, T18 分析)

[Option U Phase 0-4]
9a6ba209  feat(web/examples): echo guests on wasm-bindgen ABI
8f0a138e  feat(web/sw): actor-wbg.sw.js bridges wasm-bindgen guests
fa2d13e0  feat(cli/run): ACTR_WEB_GUEST_MODE=wbg env var
c7fd6f81  feat(web/examples/echo): start-mock-wbg.sh runs WBG e2e

[Phase 6 γ-unified]
351b61a6  feat(framework): add ServiceHandler associator trait
5a783fed  feat(framework/web): add WebWorkloadAdapter
a7204998  feat(framework): add web branch to entry! macro
cd372f94  feat(protoc-gen): emit ServiceHandler impl
b46dee70  docs(web): spec Option U Phase 6 γ-unified detailed design
0afb4697  feat(framework/web): add WebContext implementation

[TD-003/004 修复]
fd3bc05c  feat(web/examples/echo): adapt wbg guests to request_id imports
eb034d94  fix(sw-host): namespace stored credentials by client_id
b658d4f0  chore(mock-actrix): warn on WS actor rebind
1ac5afd4  chore(cli/assets): sync sw-host wasm

[Phase 6c echo unification]
87b3401d  feat(web/examples/echo): unify guest crates via entry! macro
a4ec581b  chore: delete server/client-guest-wbg duplicates
cff7f71a  chore: build unified guests in start-mock-wbg.sh

[TD-006 multi-client RPC fix (隔壁同学)]
301c58d6  fix(web): stabilize browser multi-client rpc recovery
3ce41dc8  docs(web): close out TD-006 analysis

[本 session]
cd5971da  chore(web): add sync-cli-assets script (TD-002)
f4986dd0  ci(web): add WIT codegen + cli/assets drift gates (Phase 5)
ec6ffafd  chore(web): drop orphan DataLane setter cascade (TD-001)
0db0c424  chore(cli/assets): sync sw-host wasm after TD-001
855bb658  refactor(web): drop Component Model + jco path (Phase 8)
5561ffe1  chore: ignore .claude/ and smoke crate's Cargo.lock
15fa2aa1  chore: post-Phase-8 cleanup
cae0d55b  chore(web): purge remaining CM/jco artefacts
d1fad7f5  docs(examples/ts): clarify jco componentize vs transpile
67736e2d  chore(web): close out 4 sw-host inline TODOs + dead bits
```

---

## 11. 评审建议

按这个顺序看效率最高：

1. **Option U 主线**（核心架构变更）
   - 先读 [option-u-wit-compile-web.zh.md](./option-u-wit-compile-web.zh.md) §0-§4
   - 再读 [option-u-phase6-gamma-unified.zh.md](./option-u-phase6-gamma-unified.zh.md) §3 / §6
   - 看代码：`tools/wit-compile-web/src/lib.rs` 主入口
   - 看代码：`bindings/web/crates/actr-web-abi/src/{types,guest,host}.rs` 生成产物

2. **TD-006 修复**（最复杂 bug）
   - 读 [td-006-multi-client-rpc-analysis.zh.md](./td-006-multi-client-rpc-analysis.zh.md) §0-§5
   - 看 commit `301c58d6` 的 diff（横跨 mock-actrix / sw-host runtime / DOM 三层）

3. **原生 typestate 重构**
   - 看 commit `48ba2f42` + `e8df80ec` + `eae01427` + `fa1a0f45` 的串
   - 重点是 `core/hyper/src/lib.rs` + `core/hyper/src/lifecycle/node.rs`

4. **API 收敛批量**
   - 不必逐 commit 看；查 `core/hyper/API_CLEANUP_PROGRESS.zh.md` 知道整理思路

5. **Phase 8 删除清单 + 后续清理**
   - commit `855bb658` + `cae0d55b` + `15fa2aa1` + `67736e2d` 一组
   - 验证：`grep -rn "jco\|Component Model" bindings/web/ --include='*.rs'` 不应命中浏览器路径，只命中 wasmtime native 路径

如果时间紧，最关键的 3 个文件读：

```
bindings/web/docs/option-u-wit-compile-web.zh.md   # 整体设计
bindings/web/docs/td-006-multi-client-rpc-analysis.zh.md   # 最难的 bug
core/hyper/API_CLEANUP_PROGRESS.zh.md   # 公开面收敛思路
```

---

**报告完。** 如有具体问题或需要某段更深入的图，告诉我。
