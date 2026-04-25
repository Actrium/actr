# Option U Phase 6：γ-unified 详细设计

**状态**：设计阶段（2026-04-24）
**上下文**：[Option U 总览](./option-u-wit-compile-web.zh.md) §11 / [TD-003 并发 dispatch context bug](./tech-debt.zh.md#td-003)
**决策**：Phase 6 = γ-unified（用户 workload 代码写一份，编译 / runtime 机制自动跨 target；并发隔离作为副产品一起解决）

---

## 1. 目标（单句）

**让 workload 开发者写一份 Rust handler 代码，无需 cfg 分叉，在 native (wasm32-wasip2 via Component Model) 和 web (wasm32-unknown-unknown via wasm-bindgen) 两个 target 都能正确编译、运行，并发隔离由框架内部自动处理。**

---

## 2. 目标代码形态（用户视角）

```rust
// 用户的 workload crate —— 一份代码，跨 target 编译
use actr_framework::prelude::*;
use bindings::acme::echo::{EchoRequest, EchoResponse};  // 由 protoc-gen 产出

#[derive(Default)]
struct EchoService;

// 业务 handler trait 由 protoc-gen 生成，方法签名含 ctx
impl EchoServiceHandler for EchoService {
    async fn echo<C: Context>(
        &self,
        ctx: &C,
        req: EchoRequest,
    ) -> ActorResult<EchoResponse> {
        // ctx.self_id() / ctx.request_id() / ctx.call(target, req) / ctx.tell(target, msg)
        ctx.log_info(&format!("handling echo from {:?}", ctx.caller_id()));
        Ok(EchoResponse { reply: format!("Echo: {}", req.message) })
    }
}

// 一个宏，两个 target 下各自正确展开
actr_framework::entry!(EchoService);
```

构建：

- **Native**：`cargo build --target wasm32-wasip2` + `wasm-component-ld`  
  `entry!` 展开成 wit-bindgen 生成的 exports + Workload struct
- **Web**：`cargo build --target wasm32-unknown-unknown` + `wasm-pack`  
  `entry!` 展开成 `actr_web_abi::host::set_workload(...)` + 一个适配器

用户**完全不需要知道** WIT、Component Model、JSPI、wasm-bindgen、request_id 在底层如何流转。

---

## 3. API 合约（并行 agent 必须遵守）

### 3.1 `Context` trait（共享，两个 target 通用）

位置：`core/framework/src/context.rs`（现有，最小扩展）

```rust
#[async_trait(?Send)]  // ← ?Send 加进去（web 要）；native 仍支持 Send auto trait
pub trait Context: Clone + 'static {
    // ── 数据 ──
    fn self_id(&self) -> &ActrId;
    fn caller_id(&self) -> Option<&ActrId>;
    fn request_id(&self) -> &str;

    // ── 通信 ──
    async fn call<Req, Resp>(
        &self,
        target: &Dest,
        request: Req,
    ) -> ActorResult<Resp>
    where
        Req: ProtoEncode,
        Resp: ProtoDecode;

    async fn call_raw(
        &self,
        target: &Dest,
        route_key: &str,
        body: &[u8],
    ) -> ActorResult<Vec<u8>>;

    async fn tell<Msg>(&self, target: &Dest, message: Msg) -> ActorResult<()>
    where
        Msg: ProtoEncode;

    async fn discover(&self, realm: u32, actor_type: &str) -> ActorResult<ActrId>;

    // ── 观测 ──（可选，若 native 已有则沿用）
    fn log(&self, level: LogLevel, msg: &str);
}
```

**现有 native Context trait 的改动**：当前是 `Send + Sync + Clone`，需要改成 `Clone + 'static`（去掉 `Send + Sync`），否则 web 实现不上。对 native 侧如果依赖 `Send`，要用 `#[cfg_attr]` 条件处理。

### 3.2 `Workload` trait（统一后的形态）

位置：`core/framework/src/workload.rs`（现有，扩展以适配 web）

```rust
#[async_trait(?Send)]
pub trait Workload: 'static {
    type Dispatcher: MessageDispatcher<Workload = Self>;

    // Lifecycle hooks 都带 <C: Context>，与现有 native 一致
    async fn on_start<C: Context>(&self, _ctx: &C) -> ActorResult<()>;
    async fn on_ready<C: Context>(&self, _ctx: &C) -> ActorResult<()>;
    async fn on_stop<C: Context>(&self, _ctx: &C) -> ActorResult<()>;
    // ... 其余 13 个 hook 同
}
```

**关键**：Workload 本身**不带 dispatch 方法**。`dispatch` 走 `Dispatcher` 关联类型（native 已是这个模型）。web 侧要适配这个模型，不要引入 web 专属的 `dispatch` 方法。

### 3.3 web 侧 `actr_framework::web::WebContext`

位置：**新增** `core/framework/src/web/context.rs`（`#[cfg(target_arch = "wasm32")]` + feature gate）

```rust
#[derive(Clone)]
pub struct WebContext {
    inner: Rc<WebContextInner>,
}

struct WebContextInner {
    self_id: ActrId,
    caller_id: Option<ActrId>,
    request_id: String,   // 每次 dispatch 创建新 instance，携带真 request_id
}

impl Context for WebContext {
    fn self_id(&self) -> &ActrId { &self.inner.self_id }
    fn caller_id(&self) -> Option<&ActrId> { self.inner.caller_id.as_ref() }
    fn request_id(&self) -> &str { &self.inner.request_id }

    async fn call_raw(&self, target: &Dest, route_key: &str, body: &[u8]) 
        -> ActorResult<Vec<u8>> 
    {
        // 内部调用 actr_web_abi::guest::__host_call_raw(
        //     self.inner.request_id.clone(),  ← 从 Context 本身取
        //     target_js,
        //     route_key,
        //     body_js,
        // )
        actr_web_abi::guest::call_raw_with_request_id(
            &self.inner.request_id,
            target,
            route_key,
            body,
        ).await
    }
    // 其他方法类似
}
```

**关键不变量**：`WebContext` 是不可变的，`request_id` 在创建时就绑定。**不再有 install_ctx / current_ctx 的 thread_local 概念**。

### 3.4 `actr-web-abi::guest::*` 新签名

位置：`bindings/web/crates/actr-web-abi/src/guest.rs`（由 `wit-compile-web` 重生成）

所有 host import wrapper 接受 `request_id: &str` 作为第一参数：

```rust
// 旧（当前）
pub async fn call_raw(target: &ActrId, route: &str, body: Vec<u8>) 
    -> Result<Vec<u8>, ActrError> { ... }

// 新
pub async fn call_raw_with_request_id(
    request_id: &str,
    target: &ActrId, 
    route: &str, 
    body: Vec<u8>,
) -> Result<Vec<u8>, ActrError> {
    // 内部把 request_id 透传给 __host_call_raw
    let js_promise = __host_call_raw(
        js_sys::JsString::from(request_id).into(),
        serde_wasm_bindgen::to_value(target)?,
        serde_wasm_bindgen::to_value(route)?,
        serde_wasm_bindgen::to_value(&body)?,
    )?;
    // ...
}
```

**是否保留不带 request_id 的版本**：**不保留**。用户代码永远通过 `WebContext` 间接调用，不直接用 `actr_web_abi::guest::*`。保留会埋 footgun。

`#[wasm_bindgen] extern fn __host_*` 的原始声明也加 `request_id` 参数：

```rust
#[wasm_bindgen(module = "/sw/actr-host.js")]
extern "C" {
    #[wasm_bindgen(js_name = actrHostCallRaw, catch)]
    fn __host_call_raw(
        request_id: JsValue,   // ← 新增
        target: JsValue, 
        route: JsValue, 
        body: JsValue,
    ) -> Result<js_sys::Promise, JsValue>;
    // ...
}
```

### 3.5 `actr-web-abi::host::*` 新签名

位置：`bindings/web/crates/actr-web-abi/src/host.rs`（由 `wit-compile-web` 重生成）

原 `Workload` trait 的 17 方法签名**删除**（不直接使用它）。改为内部适配器：

```rust
// 不再对用户暴露 pub trait Workload —— 由 framework::entry! 展开调用
pub(crate) struct WorkloadAdapter<W: Workload + Clone> {
    inner: W,
    // WebContext 在 dispatch 时 per-request 创建
}

impl<W: Workload + Clone> WorkloadAdapter<W> {
    pub(crate) fn new(w: W) -> Self { Self { inner: w } }

    // 17 个 #[wasm_bindgen] 入口点由 `actr_framework::entry!` 展开时生成
    // 或者由 actr-web-abi 提供一个 pub fn register(w: W) 接管所有注册逻辑
}

pub fn register_workload<W: Workload + Clone>(w: W) {
    let adapter = WorkloadAdapter::new(w);
    // 注册到内部全局 + 配置 #[wasm_bindgen] dispatcher
    INSTANCE.set(adapter).map_err(|_| "workload already registered")?;
}
```

**`entry!` 宏展开到**：`actr_web_abi::host::register_workload(MyWorkload::default())` 加上 `#[wasm_bindgen(start)]` 的 bootstrap。

### 3.6 sw-host `guest_bridge.rs` HashMap 协议

位置：`bindings/web/crates/sw-host/src/guest_bridge.rs`

```rust
thread_local! {
    static DISPATCH_CTXS: RefCell<HashMap<String, Rc<RuntimeContext>>> 
        = const { RefCell::new(HashMap::new()) };
}

// 不再有 install_ctx / current_ctx。新 API：
fn ctx_insert(request_id: String, ctx: Rc<RuntimeContext>) {
    DISPATCH_CTXS.with(|c| c.borrow_mut().insert(request_id, ctx));
}

fn ctx_get(request_id: &str) -> Result<Rc<RuntimeContext>, JsValue> {
    DISPATCH_CTXS.with(|c| 
        c.borrow().get(request_id).cloned()
         .ok_or_else(|| JsValue::from_str(&format!("no ctx for request_id={}", request_id)))
    )
}

fn ctx_remove(request_id: &str) {
    DISPATCH_CTXS.with(|c| c.borrow_mut().remove(request_id));
}
```

`register_component_workload` 的 handler：
```rust
async move {
    let request_id = ctx.request_id().to_string();
    ctx_insert(request_id.clone(), ctx.clone());
    
    let result = dispatch_fn.call1(...).await;
    
    ctx_remove(&request_id);   // 无论成功失败都清
    result
}
```

所有 `host_*_async` 新签名：
```rust
#[wasm_bindgen]
pub async fn host_call_raw_async(
    request_id: String,   // ← 新增第一参数
    target: JsValue, 
    route_key: String, 
    payload: js_sys::Uint8Array,
) -> Result<js_sys::Uint8Array, JsValue> {
    let ctx = ctx_get(&request_id)?;   // ← 按 request_id 查
    // ... 其余逻辑不变
}
```

**并发安全**：JS 单线程，HashMap 的 insert/get/remove 都在同一线程，`RefCell::borrow_mut` 不会 panic（insert 和 remove 是瞬时原子操作，host import 查询也是瞬时）。

---

## 4. 改动清单（并行可拆分到独立 agent）

### 4.1 `actr-framework` 跨 target 编译 & WebContext（独立 agent 可做）

**Agent P6-F**：

- [F1] `core/framework/Cargo.toml` 加 `[features] web = [...]` 并处理依赖
- [F2] `core/framework/src/context.rs` 去掉 `Send + Sync` bound 或用 cfg_attr
- [F3] `core/framework/src/workload.rs` 同上（考虑保留 native 的 Send auto trait）
- [F4] 新增 `core/framework/src/web/` 模块（含 `context.rs` 实现 `WebContext`）
- [F5] `cargo check --target wasm32-unknown-unknown --features web` 通过

**前置依赖**：无
**下游依赖**：P6-I 整合
**验收**：workspace 三 target 都 `cargo check` 通过

### 4.2 `sw-host::guest_bridge.rs` HashMap 重构（独立 agent 可做）

**Agent P6-S**：

- [S1] `DISPATCH_CTXS` HashMap 替换 `GUEST_CTX`
- [S2] `ctx_insert` / `ctx_get` / `ctx_remove` 函数
- [S3] `register_component_workload` 的 handler 改用 insert/remove 包装 dispatch
- [S4] 所有 8 个 `host_*_async` 加 `request_id: String` 第一参数 + 内部改用 `ctx_get`
- [S5] cli/assets sync（TD-002 流程）
- [S6] `cargo check -p actr-sw-host --target wasm32-unknown-unknown` 通过

**前置依赖**：无（API 合约在 §3.4/3.6 锁定）
**下游依赖**：P6-I 整合
**验收**：sw-host 独立编译通过；单元测试（如有）绿

### 4.3 `wit-compile-web` codegen 更新 + actr-web-abi 重生成（独立 agent 可做）

**Agent P6-C**：

- [C1] `tools/wit-compile-web/src/lib.rs`：模板加 request_id 参数
- [C2] 生成的 `guest.rs` 函数名从 `call_raw` 改为 `call_raw_with_request_id`（对齐 §3.4）
- [C3] 生成的 `host.rs`：删除对外暴露的 `Workload` trait（改为 pub(crate) + `register_workload` 公开函数）
- [C4] `cargo run -p actr-wit-compile-web` 重生成 `actr-web-abi/src/{types,guest,host}.rs`
- [C5] `cargo check -p actr-web-abi` + `--target wasm32-unknown-unknown` 通过
- [C6] `cargo run -p actr-wit-compile-web -- --check` 绿（regeneration 是幂等）

**前置依赖**：无（API 合约锁定）
**下游依赖**：P6-I 整合
**验收**：wit-compile-web build + 产物编译通过

### 4.4 整合 + 临时 echo 手迁移（串行，依赖 F/S/C）

**Agent P6-I**：

- [I1] 等 P6-F / P6-S / P6-C 三个 commit 都 cherry-pick 到 main
- [I2] 更新 `server-guest-wbg/src/lib.rs` + `client-guest-wbg/src/lib.rs` 用新的 `register_workload + WebContext` API（暂不用 entry! 宏，手动调）
- [I3] `actor.sw.js`（Phase 8 后即原 wbg 版本）：`dispatchFn` 把 envelope.requestId 透传给 guest bootstrap 代码（检查是否已经走通）
- [I4] `bash start-mock.sh`：BasicFunction + MultiTab 6-1~6-4 全跑
- [I5] 验收：BasicFunction 6/6 ✓，MultiTab 6-1 ✓，6-2/6-3/6-4 ✓ (γ 并发隔离证明)

**前置依赖**：P6-F + P6-S + P6-C
**下游依赖**：P6b 统一 entry! 宏
**验收**：e2e 证明 γ 真并发 work

### 4.5 Phase 6b：entry! 宏 + protoc 跨 target（串行，依赖 P6-I）

**Agent P6b**：

- [b1] `core/framework/src/entry.rs`：`entry!` 宏按 `target_arch` cfg 展开两套
- [b2] `tools/protoc-gen/rust` 输出更新：Handler trait 方法签名改为 `fn xxx<C: Context>(&self, ctx: &C, req) -> ActorResult<Resp>`
- [b3] 如果 `protoc-gen/rust` 已经这样（现有 native 样子就是），只需要 web 路径也消费同一套 trait
- [b4] 验收：一份 echo handler 源码，两 target 都编过 + 跑过

**前置依赖**：P6-I
**下游依赖**：P6c

### 4.6 Phase 6c：echo 一份化 + 全套件验证（串行）

**Agent P6c**：

- [c1] 删 `server-guest-wbg/` 和 `client-guest-wbg/`，改回用 `server-guest/` + `client-guest/`（cfg_attr 两 target）
- [c2] echo 业务代码只剩一份
- [c3] 跑 BasicFunction + MultiTab + Webrtc 全套；native 侧 `data-stream-peer-concurrent` 也要重跑确认没回归
- [c4] 更新 `option-u-wit-compile-web.zh.md` 标 Phase 6 完成

**前置依赖**：P6b
**下游依赖**：Phase 7（data-stream 迁移）/ Phase 8（CM 删除）

---

## 5. 并行调度策略

```
t0: 发布设计文档 + 同时启动三个 agent
     ├── P6-F（framework）  ─────────┐
     ├── P6-S（sw-host）    ─────────┤
     └── P6-C（wit-compile-web）──── ┤
                                     ▼
t1: 三个 commit 落地（需 ~1 天） ──► P6-I（整合 + γ 并发验证）
                                     │
                                     ▼  ~0.5 天
t2: P6-I PASS ──► P6b（entry! 宏 + protoc）
                                     │
                                     ▼  ~1 天
t3: P6b done ──► P6c（迁移 + 全套件）
                                     │
                                     ▼  ~0.5 天
t4: Phase 6 完成
```

**三并行的前提**：§3 API 合约**严格锁定**。任何 agent 在自己的 commit 里都不能偏离合约；整合时签名对得上。

---

## 6. 回滚策略

- P6-F / P6-S / P6-C 任一失败 → 另外两个继续做完；整合时发现合约不对齐，回查根因
- P6-I 失败 → 证明合约设计有漏洞，必须开会拍板修
- P6b 失败（entry! 宏问题）→ echo 维持两份 guest crate，继续用 P6-I 的手工 API
- P6c 失败（迁移后 e2e 回归）→ 保留 `*-guest-wbg` 作为并行产物；cfg 分两套 guest
- **任何时候**都可以 `git revert` 回到 Phase 4 的稳定点

---

## 7. 预期时间线

- **Day 1**：P6-F + P6-S + P6-C 并行（一个工作日内 3 个 commit cherry-pick 到 main）
- **Day 2 上午**：P6-I 整合 + 并发验证（MultiTab 6-2/6-3/6-4 PASS）
- **Day 2 下午**：P6b 实施 entry! 宏 + protoc-gen 跨 target
- **Day 3**：P6c echo 一份化 + 全套件验证 + 文档收尾

合计 **~2.5 工作日**，与原 γ-unified 预估持平。

---

## 8. 未决但 non-blocking 的问题

- MultiTab 6-5（"多 server 实例"）：Phase 6 内顺手改成 `skipTest`，业务确认不支持多 workload
- MultiTab 6-6（Shared SW Isolation）：γ 是否顺带修？不确定。γ 落地后单独跑 6-6，fail 就另立 TD
- Webrtc 5-1 / 5-4：BasicFunction 已经涉及 WebRTC DataChannel，应该能通；γ 不直接影响这里

---

## 9. 与下阶段的衔接

Phase 6 完成 → 排 Phase 7（data-stream-peer-concurrent 迁 WBG 统一 API）→ 排 Phase 8（CM 路径整体删除）。详见 [Option U 总览](./option-u-wit-compile-web.zh.md) §11。
