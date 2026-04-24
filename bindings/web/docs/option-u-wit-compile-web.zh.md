# Option U：WIT → wasm-bindgen 编译器设计

**状态**：Phase 0-4 已完成，Phase 5/6 待办（2026-04-24）
**上下文文档**：[T18 分析](./t18-jco-async-lift-hang.zh.md) §7 选项空间；[TD-003 并发 dispatch context bug](./tech-debt.zh.md#td-003)；**[Phase 6 γ-unified 详细设计](./option-u-phase6-gamma-unified.zh.md)**

## 进度快照

| Phase | 状态 | 交付 |
|-------|------|------|
| **0 探查** | ✓ | `/tmp/wit-content-dump.md`；结论 GREEN LIGHT |
| **1 types.rs 生成** | ✓ | `tools/wit-compile-web/` + `actr-web-abi/src/types.rs`（10 record + 3 variant） |
| **2 guest.rs 生成** | ✓ | `actr-web-abi/src/guest.rs`（8 host imports + async wrappers） |
| **3 host.rs 生成** | ✓ | `actr-web-abi/src/host.rs`（`Workload` trait + 17 `#[wasm_bindgen]` exports） |
| **4 Echo 接入 + e2e** | ✓ | `server/client-guest-wbg` + `actor-wbg.sw.js` + `start-mock-wbg.sh`；**BasicFunction 6/6 PASS** |
| **4.5 MultiTab 压测** | ⚠️ 部分 | 1/6（只 6-1 过）；6-2~6-6 命中 TD-003 dispatch context 单例 bug |
| **5 CI drift + 文档收尾** | 待办 | CI 接 `cargo run -p actr-wit-compile-web -- --check`；T18 doc 标 resolved |
| **6 TD-003 修复 + 并发验证** | 待办 | 见下 §11 |

Option U 核心路径已跑通 —— jco / Component Model / JSPI 全部绕开，echo 简单 RPC ✓。暴露出的 TD-003 是 sw-host 原有并发模型问题，不是 WBG 特有。

---

## 0. 为什么存在这个方案

T18 分析把浏览器 Component Model + jco 路径定性为"有 bug、调度层复杂、收益低"之后，选项 R（放弃 CM 走 wasm-bindgen）被重新评估，发现：

- Rust `async fn` 在 wasm-bindgen 路径里**原生工作**（`wasm_bindgen_futures`），不需要 JSPI
- 浏览器侧我们**没用到** Component Model 的实际特性（无多语言 guest、无 dynamic composition）
- 真正值得保留的是 **WIT 契约本身**，而不是 Component Model 产物形态

Option U 的核心主张：

> **WIT 是唯一事实源，但浏览器产物形态从 Component Model 切到 core wasm + wasm-bindgen。自建一个薄 codegen 工具把 WIT 编译为浏览器产物。**

---

## 1. 整体架构

```
┌────────────────────────────────────────────────────────────┐
│ core/framework/wit/actr-workload.wit    （单一契约源）       │
└─────────────────────────┬──────────────────────────────────┘
                          │
             ┌────────────┴──────────┐
             │                       │
             ▼                       ▼
    ┌─────────────────┐    ┌──────────────────────────┐
    │ Native path     │    │ Web path                 │
    │                 │    │                          │
    │ wit-bindgen     │    │ tools/wit-compile-web    │
    │ (async: true)   │    │   (new)                  │
    │                 │    │                          │
    │ ↓               │    │ ↓                        │
    │ Component Model │    │ Rust 源码 (committed):   │
    │ .wasm           │    │ - actr-web-abi / guest   │
    │                 │    │ - actr-web-abi / host    │
    │ ↓               │    │                          │
    │ wasmtime        │    │ ↓                        │
    │                 │    │ cargo + wasm-pack        │
    │                 │    │   target=wasm32-unknown- │
    │                 │    │          unknown          │
    │                 │    │                          │
    │                 │    │ ↓                        │
    │                 │    │ core .wasm + .js glue    │
    │                 │    │ (wasm-bindgen 原生产物)   │
    │                 │    │                          │
    │                 │    │ ↓                        │
    │                 │    │ V8（无 JSPI 依赖）        │
    └─────────────────┘    └──────────────────────────┘
```

不变：native 侧、WIT 文件本身。
消失：`@bytecodealliance/jco`、`wasm-component-ld`、`wasm32-wasip2` target、Component Model adapter wasm、JSPI 依赖、async-lift 调度层。

---

## 2. 编译器职责

输入：`core/framework/wit/actr-workload.wit`

输出（全部以 **committed Rust 源码** 形式）：

1. **`bindings/web/crates/actr-web-abi/src/types.rs`**
   类型定义：每个 WIT record / variant / enum / flags 对应一个 Rust `#[derive(Serialize, Deserialize, Debug, Clone)]` 的 struct / enum

2. **`bindings/web/crates/actr-web-abi/src/guest.rs`**
   Guest 侧 import wrappers：每个 WIT import func 对应
   - 一个 `#[wasm_bindgen] extern "C"` 的 raw FFI 声明
   - 一个对用户友好的 async Rust 包装（`serde_wasm_bindgen::to_value` 入参 + `JsFuture::from` await + `from_value` 出参）

3. **`bindings/web/crates/actr-web-abi/src/host.rs`**
   Host 侧 export wrappers：每个 WIT export func 对应
   - 一个 `#[wasm_bindgen]` 导出的 entry point
   - 派发到用户实现的 trait 方法

4. **CI drift check**：compiler 可以以 `--check` 模式跑，重新生成后 `diff` committed 版本，不匹配就退 1。已有 `tools/wit-lint` 是类似模式。

---

## 3. 类型 marshaling 策略：`serde-wasm-bindgen`

WIT 类型 ↔ JsValue 映射由 `serde-wasm-bindgen` crate 提供，我们只管让生成的 Rust 类型 `derive(Serialize, Deserialize)`。

映射表（serde-wasm-bindgen 默认行为，符合 JS 直觉）：

| WIT | Rust | JsValue |
|-----|------|---------|
| bool | `bool` | boolean |
| u8..u32, s8..s32 | `u8..u32, i8..i32` | number |
| u64 / s64 | `u64 / i64` | bigint |
| f32 / f64 | `f32 / f64` | number |
| char | `char` | string (单字符) |
| string | `String` | string |
| list\<T\> | `Vec<T>` | Array |
| list\<u8\> | `Vec<u8>` | Uint8Array（零拷贝路径） |
| option\<T\> | `Option<T>` | T \| null |
| result\<T, E\> | `Result<T, E>` | `{Ok: T}` / `{Err: E}` |
| tuple\<T1, T2\> | `(T1, T2)` | `[T1, T2]` |
| record | `struct { ... }` | Object |
| variant | `enum { Variant(T), ... }` | `{Variant: T}` |
| enum | `enum { A, B, C }` | string |
| flags | `struct { a: bool, b: bool, ... }` | Object of bool |

**对调试友好性**：浏览器 DevTools 看到的是真实 JS Object，不是 opaque Uint8Array。

---

## 4. 生成物长啥样

给定 WIT 片段：

```wit
interface host {
    call-raw: func(target: actor-id, body: list<u8>) 
        -> result<list<u8>, actor-error>;
    discover: func(realm: u32, actor-type: string) -> actor-id;
    log: func(level: u32, msg: string);
}

record actor-id {
    realm: u32,
    serial-number: u64,
    actr-type: actor-type,
}
```

生成 `types.rs`：
```rust
// GENERATED — do not edit
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct ActorId {
    pub realm: u32,
    pub serial_number: u64,
    pub actr_type: ActorType,
}
```

生成 `guest.rs`：
```rust
// GENERATED — do not edit
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen(module = "/sw/actr-host.js")]
extern "C" {
    #[wasm_bindgen(js_name = actrHostCallRaw, catch)]
    fn __host_call_raw(target: JsValue, body: JsValue) 
        -> Result<js_sys::Promise, JsValue>;

    #[wasm_bindgen(js_name = actrHostDiscover, catch)]
    fn __host_discover(realm: u32, actor_type: &str) 
        -> Result<js_sys::Promise, JsValue>;

    #[wasm_bindgen(js_name = actrHostLog)]
    fn __host_log(level: u32, msg: &str);
}

pub async fn call_raw(
    target: &super::types::ActorId, 
    body: Vec<u8>,
) -> Result<Vec<u8>, super::types::ActorError> {
    let js_target = serde_wasm_bindgen::to_value(target)
        .map_err(serde_err)?;
    let js_body = serde_wasm_bindgen::to_value(&body)
        .map_err(serde_err)?;
    let js_promise = __host_call_raw(js_target, js_body)
        .map_err(js_err)?;
    let js_result = JsFuture::from(js_promise).await
        .map_err(js_err)?;
    serde_wasm_bindgen::from_value(js_result).map_err(serde_err)
}

pub async fn discover(realm: u32, actor_type: &str) -> ActorId {
    let js_promise = __host_discover(realm, actor_type).unwrap();
    let js_result = JsFuture::from(js_promise).await.unwrap();
    serde_wasm_bindgen::from_value(js_result).unwrap()
}

pub fn log(level: u32, msg: &str) {
    __host_log(level, msg);
}
```

用户 guest 代码（**不变，就像 native 那样写**）：
```rust
use actr_web_abi::guest as host;

async fn handle_echo(msg: &[u8]) -> Result<Vec<u8>, ActorError> {
    host::log(INFO, "echo handler");
    let peer = host::discover(0, "acme:EchoService:0.1.0").await;
    host::call_raw(&peer, msg.to_vec()).await
}
```

---

## 5. Phase 划分

| Phase | 内容 | 预计成本 | 产物可验证性 |
|-------|------|---------|-----------|
| **0** | 探查：用 `wit-parser` 读 `actr-workload.wit`，dump 全类型清单 + 函数清单；验证基础设施可行 | < 30 min | 一份 "WIT 内容清单" |
| **1** | 实现 types.rs 生成（record / variant / enum / flags） | 0.5 天 | 生成出的 Rust 文件 cargo check 通过 |
| **2** | 实现 guest.rs 生成（async func + serde-wasm-bindgen 管道） | 0.5 天 | 对单一 import func 生成代码 cargo check 通过 |
| **3** | 实现 host.rs 生成（export func + trait dispatch） | 0.5 天 | 同上 |
| **4** | Echo end-to-end：把新 `actr-web-abi` 接到 echo example，跑通 BasicFunction | 0.5 天 | 浏览器 e2e 1-0 PASS |
| **5** | CI drift check + 文档收尾 | 0.5 天 | CI 绿 |

**合计 ~2.5 天**（Phase 0 不计）。

---

## 6. Phase 0：探查步骤（最先跑）

1. 在 worktree 里写一个小 Rust 程序（或直接用 `tools/wit-lint` 现成的 wit-parser 调用）
2. 读 `core/framework/wit/actr-workload.wit`
3. dump 出：
   - 所有 record / variant / enum / flags 的名字和字段
   - 所有 import func 的签名（参数 / 返回 / async 与否）
   - 所有 export func 的签名
4. 产物：一份 Markdown 清单（`/tmp/wit-content-dump.md` 之类）
5. 评估：
   - 类型规模（目测是 10 个以内 record / 几个 variant）
   - 是否有 serde-wasm-bindgen 不支持的边缘特性（resource handles、stream、future 这类 async-task 相关）

**退出条件**：
- **Green light**：类型清单都是 serde 可处理的 primitive + record + variant/enum；进入 Phase 1
- **Yellow**：有少量 resource handle 或未知特性；评估是否可绕开
- **Red**：WIT 大量用 Component Model 专属特性（stream/future/resource）—— Option U 需要重新评估

---

## 7. 兼容性和回滚

**兼容性**：
- native 路径零影响
- 浏览器侧新方案和现有 CM 方案可以**并行共存**一段时间：
  - 保留 `bindings/web/scripts/transpile-component.sh` 作为历史路径
  - `actr-web-abi` 作为新路径
  - echo example 选其中一条跑

**回滚**：
- 新路径有问题时，把 `actr-web-abi` crate 全删，echo example 的 Cargo.toml 换回 `wasm32-wasip2`
- Commit 粒度细，每 Phase 独立，任何一段失败都可以停住

---

## 8. 对当前已有产物的影响

| 产物 | 影响 |
|------|------|
| `core/framework/wit/actr-workload.wit` | **不变**（依然是单一事实源） |
| `core/framework/src/guest/` Rust guest 适配层 | native 侧保留，浏览器侧改用 `actr-web-abi` |
| `bindings/web/crates/sw-host/` | 保留，但 `guest_bridge.rs` 里 jco bridge 相关代码可以大幅削减 |
| `bindings/web/crates/dom-bridge/` | 不变 |
| `bindings/web/scripts/transpile-component.sh` | Option U 落地后可 deprecate |
| `cli/assets/web-runtime/actr_sw_host*` | 继续用（只是内容变简单） |
| TD-001（5 个 zero-call setter） | 可能刚好有理由**真正用上** Rust 侧 DataLane |
| TD-002（cli/assets sync） | 仍然存在，但 jco 产物不再进这条路径 |
| jco fork + #1361 patch | 作为留档，不再主动维护 |

---

## 9. 未决问题（等 Phase 0 回答）

- [ ] `actr-workload.wit` 里有没有 resource / stream / future 这类 Component Model 专属特性？
- [ ] 类型清单的规模（影响 Phase 1-2 工时估算）
- [ ] 是否有 WIT 里 import / export 已经是 async（比如 `wit-bindgen async: true`）的特殊处理需要映射到 Promise-return？

---

## 10. Phase 0 命令行草稿

```bash
# 在 worktree 里：
cd tools/wit-lint
# 看它怎么调 wit-parser，复用
cargo run --example dump-wit-content -- core/framework/wit/actr-workload.wit
# 输出应覆盖 §6 步骤 3 的所有 dump 项
```

或直接写一个独立小 binary。

---

## 11. Phase 5 + Phase 6 TODO（下阶段执行清单）

Option U 核心路径已落地（Phase 0-4 完成，BasicFunction 6/6 PASS）。剩余工作拆两批：**Phase 5 是"收尾 + 保护"**，**Phase 6 是"并发修复"**。两者**逻辑上独立**，可以并行推，但 Phase 6 建议有架构决策后再动。

### Phase 5：Option U 收尾（~0.5 天，无架构决策）

| # | 项目 | 成本 | 产物 |
|---|------|------|------|
| 5.1 | CI drift check：给 GitHub Actions / 本仓 CI 配置加一步 `cargo run -p actr-wit-compile-web -- --check`。失败即拦截 PR | 15 min | `.github/workflows/*` 或 `scripts/ci-*.sh` 一个增量 |
| 5.2 | `wit-lint` 和 `wit-compile-web` 并存关系确认：是否合并 / 各自职责文档化 | 15 min | `tools/wit-lint/README.md` 加一段 / 或一份 TD |
| 5.3 | T18 doc (`t18-jco-async-lift-hang.zh.md`) 加 §11：标记"Option U 绕开"作为实际采用路径；#1361 patch / jco fork 在 /tmp 留痕状态 | 15 min | T18 doc 更新 |
| 5.4 | Option U doc 本身更新"进度快照"到 Phase 5-6 完成态 | 5 min | 本文档 |
| 5.5 | decision：CM 路径何时 deprecate？data-stream-peer-concurrent 是否也迁 WBG？ | 讨论 | 本文档新增决策记录 |

### Phase 6：TD-003 并发 dispatch context（依赖架构决策）

前置：看 [TD-003](./tech-debt.zh.md#td-003) 的修法选项 α/β/γ/δ/ε。

**第一步：拍板 α vs γ**

- **α（async mutex 串行化）**：改一处 `Mutex<()>` 加锁，快速止血。MultiTab 6-2/6-3/6-4 应全过。**不并发**。
- **γ（request_id 多 ctx）**：WIT 层加 request-id 参数；host imports 按 id 查 ctx；真并发。改动面更大。
- **建议先 α 再（如有需要）γ**。α 实际吞吐对 actor 场景可能够用 —— 每个 actor 一个 dispatch 排队不是大问题，除非有大量跨 actor 并发。

**第二步：α 实施（如选 α）**

| # | 项目 | 成本 | 产物 |
|---|------|------|------|
| 6.1 | `guest_bridge.rs::register_component_workload` 的 handler 外面包一个 `tokio::sync::Mutex<()>` 或 `futures::lock::Mutex` | 0.5 h | commit `fix(sw-host): serialize guest dispatches to prevent ctx overlap` |
| 6.2 | sync cli/assets（TD-002 流程）+ rebuild actr binary | 5 min | 同一 commit 或紧邻的 `chore(cli/assets): sync` |
| 6.3 | 跑 MultiTab 期待 6-1/6-2/6-3/6-4 PASS | 10 min | evidence log |
| 6.4 | 6-5 "Multiple Server Instances" 独立议：改 `set_workload` 为 `HashMap<ActrType, Box<dyn Workload>>` **或**把 6-5 分类成"非 SW 架构支持的测试" | 0.5 h | commit 或 test-auto.js 分类调整 |
| 6.5 | 6-6 "Shared SW Isolation" 独立调试 | 未知 | 可能另立 TD |

**第二步备选：γ 实施（长期路线，不在当前冲刺）**

| # | 项目 | 成本 | 产物 |
|---|------|------|------|
| 6.γ.1 | 给 WIT 所有 host import 加 `request-id` 参数 | 0.5 h | `core/framework/wit/actr-workload.wit` 改动 |
| 6.γ.2 | `tools/wit-compile-web` 跟着改，重生成 `actr-web-abi` | 0.5 h | 两个 commit（compiler + 生成物） |
| 6.γ.3 | `guest_bridge.rs` 改为 `HashMap<RequestId, Rc<RuntimeContext>>`，`host_*_async` 按传入 id 查表 | 1 h | sw-host commit |
| 6.γ.4 | 同步 cli/assets + 跑 MultiTab | 15 min | |
| 6.γ.5 | 评估对 native CM 路径的兼容性（wit-bindgen 改 WIT 后也要重生成 native 侧） | 0.5 h | 核实 native 跑的 tests 过 |

### 最终目标覆盖

| 测试 | 当前 | α 后 | γ 后 |
|------|------|------|------|
| BasicFunction 1-* | 6/6 ✓ | 6/6 | 6/6 |
| MultiTab 6-1 | ✓ | ✓ | ✓ |
| MultiTab 6-2 / 6-3 / 6-4 | ✗ | ✓ | ✓ |
| MultiTab 6-5 | ✗ | 需 set_workload 改 | 需 set_workload 改 |
| MultiTab 6-6 | ✗ | 独立修 | 独立修 |
| Webrtc 5-* | 未跑 | 待验证 | 待验证 |
| 全量 e2e | 未跑 | 待验证 | 待验证 |

### 决策依赖

Phase 6 推进前要你回答：

1. **α / γ / 先 α 后 γ**？
2. **MultiTab 6-5 Multiple Server Instances**：改 `set_workload` 支持多 workload，还是承认此场景不适用？
3. **CM 路径 deprecate 节奏**：立即删 / data-stream-peer-concurrent 也迁完后删 / 永久共存？
