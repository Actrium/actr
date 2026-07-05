# RFC-0001：显式回复（Explicit Reply）——受理与回复解耦

- 状态：提案（待评审）
- 日期：2026-07-05
- 关联：并发模型 P0（见 `docs/api-surface-review.zh.md` §二）、issue #257（超时选项）、Direction+TELL=3 协议演进

## 摘要

把被调方（callee）的"回复时刻"从"handler 返回时刻"上解开：新增一种按方法选择的回复模式，handler 通过一个可移动的 `Reply<T>` 句柄显式发送回复，消息泵在 handler 返回后立即受理下一条消息，而不等待回复产生。**wire 层零改动**——请求与响应本来就是两条以 `request_id` 关联的单向 envelope，本 RFC 只改变被调方 dispatch 层的 API 形态。

设计哲学（已确认）：**消息泵严格串行（受理有序），业务并发由应用按场景自选**。现状的缺口在于：call 的响应被焊死为 handler 的返回值，导致入站 call 的并发度恒为 1，应用无从选择。本 RFC 补上这个选择权，且不向简单场景征税。

## 背景：现状的三点焊接

当前接收循环（`core/hyper/src/lifecycle/node.rs`，inproc 与 mailbox 两条循环相同）：

```
pop envelope → await handle_incoming() ─┬→ 返回值即 response bytes
                                        ├→ 循环构造 RESPONSE envelope 发回
                                        └→ ack 邮箱 → pop 下一条
```

**handler 完成、response 产生、ack 三个时刻焊死在同一点**。后果：

1. 入站 call 队头阻塞：10 个互不相关的调用方同时 call 一个 handler 均耗时 100ms 的节点，第 10 个白等 900ms；
2. 分布式死锁：A 的 handler 内同步 call B，B 的 handler 内同步 call A，双方泵都被占住，互等到超时；
3. 无法表达"受理即返回、稍后回复"（长轮询、等外部事件、批量聚合）。

而 wire 层从不要求这种焊接：响应按 `request_id` 关联、乱序合法、迟到的孤儿响应被方向路由丢弃（#268/#263）。焊接纯粹是 server 侧 API 的选择。

## 设计

### 1. 回复模式（proto method option）

在 `core/protocol/proto/actr/options.proto` 增加 method 级选项：

```proto
enum ReplyMode {
    REPLY_MODE_RETURN = 0;    // 默认：回复来自 handler 返回值（现状，零迁移）
    REPLY_MODE_EXPLICIT = 1;  // 显式：handler 通过 Reply<T> 句柄发送回复
}

extend google.protobuf.MethodOptions {
    ReplyMode reply_mode = <ext-tag>;
}
```

用法：

```proto
service MediaService {
    rpc Probe(ProbeRequest) returns (ProbeResponse);   // 默认 RETURN
    rpc Transcode(TranscodeRequest) returns (TranscodeResponse) {
        option (actr.reply_mode) = REPLY_MODE_EXPLICIT;
    }
}
```

命名说明：两个值命名于**机制**而非时机——`RETURN`（回复取自返回值）/ `EXPLICIT`（回复经句柄显式发送）。不叫 `DEFERRED`，因为显式模式下也允许在 handler 内立即回复；不叫 `CONCURRENT`，因为并发与否是应用的选择而非该选项的承诺。

### 2. 生成的 handler 签名

```rust
// RETURN（不变）
async fn probe<C: Context>(&self, req: ProbeRequest, ctx: &C)
    -> ActorResult<ProbeResponse>;

// EXPLICIT（新）
async fn transcode<C: Context>(&self, req: TranscodeRequest, ctx: &C,
    reply: Reply<TranscodeResponse>) -> ActorResult<()>;
```

EXPLICIT 的返回值 `ActorResult<()>` 表示**受理结果**：返回 `Err` 时，若 `reply` 尚未消费，框架用该错误发送错误回复（`?` 运算符在校验前缀里可直接使用）；若已消费则仅记日志。

### 3. `Reply<T>` 句柄

```rust
pub struct Reply<T> { /* request_id、回程路由、trace 上下文、deadline、dedup 完成器 */ }

impl<T: prost::Message> Reply<T> {
    /// 发送回复并消费句柄。同步入队（内部交给写出任务做 IO），不是 async。
    pub fn send(self, result: ActorResult<T>);

    /// 调用方的截止时刻（到达时刻 + 请求的 timeout_ms）。
    /// 慢任务可据此提前放弃；超过 deadline 后 send 仍然合法（回复会在
    /// 调用方侧作为孤儿响应被丢弃，无害）。
    pub fn deadline(&self) -> Option<Instant>;

    pub fn request_id(&self) -> &str;
}
```

**类型系统承担契约**：

- `send(self)` 消费句柄 → **二次回复在编译期不可能**；
- `Reply<T>: Send + 'static` → 可 move 进 `tokio::spawn`、可存入状态 map（长轮询/订阅式应答的自然表达）；
- **`Drop` 兜底**：句柄未经 `send` 被析构（包括 spawn 任务 panic 展开）时，自动发送 `ActrError::Internal("reply dropped without response")` 并记 warn——调用方永远不会因为被调方忘记回复而白等到超时。`send` 必须是同步入队正是为此：`Drop` 不能 await。

**句柄携带的上下文**：`request_id`、回程路由（inproc lane 或 peer gate）、`traceparent/tracestate`（延迟回复仍接续原调用链）、deadline、dedup 完成器（见 §5）。

### 4. 消息泵契约（随本 RFC 一并成文，独立于新特性生效）

| 时刻 | 语义 | 承诺 |
|---|---|---|
| **受理（accept）** | handler 返回 | 严格串行、按到达顺序；ack 于此刻发出 |
| **回复（reply）** | RESPONSE envelope 发出 | RETURN 模式 = 受理时刻；EXPLICIT 模式 ≥ 受理时刻，跨请求无序 |

- **串行范围**：handler body 之间互相串行（包括 EXPLICIT 模式——泵仍 await handler 返回，框架**不隐式 spawn**）；handler 内 spawn 出去的延续与后续 handler body 并发。由此得到一个有用的模式：**串行校验前缀 + 并发执行体**——在 handler 内串行地检查/预占状态，然后 spawn 慢活并立即返回。
- **崩溃窗口**：ack 前崩溃 → 持久邮箱重投（at-least-once）；ack 后、回复前崩溃 → 调用方超时，RpcReliable 重试由 dedup 兜住（见 §5）。
- **死锁规则**（文档义务）：RETURN 模式 handler 内不得同步 call 可能回调自己的对端；需要该拓扑时使用 EXPLICIT 模式（A 受理后 spawn 中 call B，A 的泵得以继续服务 B 的入站请求——死锁解除）。

### 5. dedup 交互（关键细节）

dedup 条目的完成时刻从"handler 返回"移到"**回复发出**"（`Reply::send` / `Drop` 内完成，RETURN 模式两个时刻重合，行为不变）：

- 回复未发出期间到达的重复 call → 命中 InFlight，等待真正的回复（受调用方超时上界约束）；
- 回复已发出后的重复 → 命中 Done 缓存，立即返回缓存回复；
- spawn 任务 panic → `Reply` 在展开中 Drop → 错误回复 + dedup 以 Err 完成，重复请求得到一致的错误而非悬挂。

### 6. tell 与 EXPLICIT 的交叉

`tell` 到达 EXPLICIT 方法时，dispatcher 注入一个**空回程**的 `Reply`：`send` 成为静默 no-op（debug 日志），`Drop` 不发送任何东西。契约：fire-and-forget 的语义由调用方决定，被调方代码无需感知。

### 7. 配套护栏（可选，phase 2）

- 未决回复计数 gauge（`outstanding_replies`）与受理→回复延迟 histogram；
- 可选配置 `max_pending_replies: Option<usize>`（默认 None）：未决显式回复超过阈值时暂停泵的 pop，形成天然背压，与 `on_mailbox_backpressure` hook 联动。

## 典型场景

```rust
// 1. 简单方法：完全不变（RETURN 模式，零迁移成本）
async fn probe<C: Context>(&self, req: ProbeRequest, _ctx: &C) -> ActorResult<ProbeResponse> {
    Ok(ProbeResponse { alive: true })
}

// 2. 慢方法：串行校验 + 并发执行
async fn transcode<C: Context>(&self, req: TranscodeRequest, ctx: &C,
    reply: Reply<TranscodeResponse>) -> ActorResult<()> {
    let job = self.validate_and_reserve(&req)?;   // 串行前缀：状态检查/预占，Err 自动变错误回复
    let ctx = ctx.clone();
    tokio::spawn(async move {                     // 并发执行体：泵已继续受理下一条
        let result = run_transcode(job, &ctx).await;
        reply.send(result);                        // 迟到无害；panic 由 Drop 兜底
    });
    Ok(())
}

// 3. 长轮询/事件驱动应答：把 Reply 存进状态，外部事件到来时回复
async fn wait_for_update<C: Context>(&self, req: WaitRequest, _ctx: &C,
    reply: Reply<UpdateEvent>) -> ActorResult<()> {
    self.waiters.lock().insert(req.session_id, reply);   // Reply: Send + 'static
    Ok(())
}
// 别处：if let Some(reply) = waiters.remove(&id) { reply.send(Ok(event)); }
```

## 已考虑并否决的替代方案

| 方案 | 否决理由 |
|---|---|
| A. 维持现状 | 入站 call 并发度恒为 1，与"应用自选并发"哲学矛盾；死锁与队头阻塞无解 |
| B. 全量显式（所有方法都拿 Reply） | 向多数简单方法征税；echo 从 6 行变 10 行且引入无谓概念 |
| D. 泵对每条消息隐式 spawn | 并发选择权回到框架手里；状态竞争成为默认；破坏"串行受理"承诺与顺序保证 |
| E. handler 返回 future、泵不 await（隐式分离） | 隐藏并发，签名看不出语义差异，品味差于显式句柄 |

C（本案：按方法双签名 + 显式句柄）在"简单场景零成本"与"复杂场景全表达力"之间不设中间税。

## 兼容性与分期

- **wire 零改动**；与 Direction+TELL=3 正交（迟到回复作为孤儿响应被方向路由丢弃的行为，两者共同保证）。
- **additive**：默认签名与行为完全不变，proto option 是新增项——不占 0.5 破坏窗口，随时可落。
- 分期：
  - Phase 1（本 RFC 主体）：options.proto 扩展、`actr-framework::Reply`、hyper 泵/dedup 调整、Rust codegen 双签名、泵契约文档与测试；
  - Phase 2：FFI（uniffi 暴露 Reply 对象，Kotlin/Swift 得到同等能力）+ 护栏指标；
  - Phase 3：WIT guest ABI 扩展（现 `dispatch` 同步返回响应字节，需新增 `send-reply` host import 并让 dispatch 返回 `replied(bytes) | deferred` 变体）——在此之前 WASM guest 仅支持 RETURN 模式，codegen 对 wasm 目标遇 EXPLICIT 选项时报错而非静默降级。

## 待定项

1. `reply_mode` 的 proto extension tag 号（从仓库既有扩展号段分配）；
2. `max_pending_replies` 默认值与是否进 Phase 1；
3. FFI 侧 `Reply` 对象的生命周期约定（uniffi 对象跨语言 Drop 语义需验证）。
