# RFC-0323: 显式回复（Explicit Reply）——受理与回复解耦

- 状态（Status）：提案（Proposed）
- 日期（Date）：2026-07-08
- RFC PR：[Actrium/actr#289](https://github.com/Actrium/actr/pull/289)
- 跟踪议题（Tracking issue）：[Actrium/actr#323](https://github.com/Actrium/actr/issues/323)
- 替代 RFC（Superseded by）：无
- 关联（Related）：[Actrium/actr#257](https://github.com/Actrium/actr/issues/257)、[Actrium/actr#263](https://github.com/Actrium/actr/pull/263)、[Actrium/actr#268](https://github.com/Actrium/actr/pull/268)、[Actrium/actr#280](https://github.com/Actrium/actr/pull/280)

## 摘要（Summary）

把被调方（callee）的"回复时刻"从"handler 返回时刻"上解开：新增一种按方法选择的回复模式，handler 通过一个可移动的 `Reply<T>` 句柄显式发送回复，消息泵在 handler 返回后立即受理下一条消息，而不等待回复产生。**wire 层零改动**——请求与响应本来就是两条以 `request_id` 关联的单向 envelope，本 RFC 只改变被调方 dispatch 层的 API 形态。

设计哲学（已确认）：**消息泵严格串行（受理有序），业务并发由应用按场景自选**。现状的缺口在于：call 的响应被焊死为 handler 的返回值，导致入站 call 的并发度恒为 1，应用无从选择。本 RFC 补上这个选择权，且不向简单场景征税。

## 动机（Motivation）

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

## 详细设计（Detailed design）

### 1. 回复模式（proto method option）

在 `core/protocol/proto/actr/options.proto` 增加 method 级选项：

```proto
enum ReplyMode {
    REPLY_MODE_RETURN = 0;    // Reply through the handler return value (default)
    REPLY_MODE_EXPLICIT = 1;  // Reply explicitly through a Reply<T> handle
}

extend google.protobuf.MethodOptions {
    ReplyMode reply_mode = <ext-tag>;
}
```

用法：

```proto
service MediaService {
    rpc Probe(ProbeRequest) returns (ProbeResponse);   // Defaults to RETURN
    rpc Transcode(TranscodeRequest) returns (TranscodeResponse) {
        option (actr.reply_mode) = REPLY_MODE_EXPLICIT;
    }
}
```

命名说明：两个值命名于**机制**而非时机——`RETURN`（回复取自返回值）/ `EXPLICIT`（回复经句柄显式发送）。不叫 `DEFERRED`，因为显式模式下也允许在 handler 内立即回复；不叫 `CONCURRENT`，因为并发与否是应用的选择而非该选项的承诺。

### 2. 生成的 handler 签名

```rust
// RETURN (unchanged)
async fn probe<C: Context>(&self, req: ProbeRequest, ctx: &C)
    -> ActorResult<ProbeResponse>;

// EXPLICIT (new)
async fn transcode<C: Context>(&self, req: TranscodeRequest, ctx: &C,
    reply: Reply<TranscodeResponse>) -> ActorResult<()>;
```

EXPLICIT 的返回值 `ActorResult<()>` 表示**受理结果**：返回 `Err` 时，若 `reply` 尚未消费，框架用该错误发送错误回复（`?` 运算符在校验前缀里可直接使用）；若已消费则仅记日志。

### 3. `Reply<T>` 句柄

```rust
pub struct Reply<T> { /* request_id, return route, trace context, deadline, dedup completer */ }

impl<T: prost::Message> Reply<T> {
    /// Enqueues a reply synchronously and consumes the handle.
    pub fn send(self, result: ActorResult<T>);

    /// Returns the caller's deadline: arrival time plus timeout_ms.
    /// Sending after the deadline remains valid; the caller discards the
    /// resulting orphan response.
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

### 8. 典型场景

```rust
// 1. Simple method: unchanged RETURN mode with no migration cost
async fn probe<C: Context>(&self, req: ProbeRequest, _ctx: &C) -> ActorResult<ProbeResponse> {
    Ok(ProbeResponse { alive: true })
}

// 2. Slow method: serial validation followed by concurrent execution
async fn transcode<C: Context>(&self, req: TranscodeRequest, ctx: &C,
    reply: Reply<TranscodeResponse>) -> ActorResult<()> {
    let job = self.validate_and_reserve(&req)?;   // Serial prefix: validate and reserve state
    let ctx = ctx.clone();
    tokio::spawn(async move {                     // Concurrent body: the pump can accept the next message
        let result = run_transcode(job, &ctx).await;
        reply.send(result);                        // Late replies are harmless; Drop handles panics
    });
    Ok(())
}

// 3. Long polling or event-driven response: retain Reply until the event arrives
async fn wait_for_update<C: Context>(&self, req: WaitRequest, _ctx: &C,
    reply: Reply<UpdateEvent>) -> ActorResult<()> {
    self.waiters.lock().insert(req.session_id, reply);   // Reply: Send + 'static
    Ok(())
}
// Elsewhere: if let Some(reply) = waiters.remove(&id) { reply.send(Ok(event)); }
```

## 缺点（Drawbacks）

- 双 handler 签名增加了 codegen、dispatch 与 FFI 的实现和维护成本，使用者也需要理解 RETURN 与 EXPLICIT 两套契约。
- `Reply<T>` 跨任务存活后，trace 上下文、deadline、dedup 完成状态与回程路由必须随句柄一起保存，运行时资源占用高于同步返回。
- 在 Phase 2 的背压护栏落地前，应用可以持续积累未决回复；错误使用可能造成内存与邮箱压力。
- `Drop` 只能覆盖正常析构与 panic 展开。进程 abort 或崩溃仍可能发生 ack 后、回复前的窗口，只能由调用方超时与可靠调用重试处理。

## 替代方案（Alternatives）

| 方案 | 否决理由 |
|---|---|
| A. 维持现状 | 入站 call 并发度恒为 1，与"应用自选并发"哲学矛盾；死锁与队头阻塞无解 |
| B. 全量显式（所有方法都拿 Reply） | 向多数简单方法征税；echo 从 6 行变 10 行且引入无谓概念 |
| D. 泵对每条消息隐式 spawn | 并发选择权回到框架手里；状态竞争成为默认；破坏"串行受理"承诺与顺序保证 |
| E. handler 返回 future、泵不 await（隐式分离） | 隐藏并发，签名看不出语义差异，品味差于显式句柄 |

C（本案：按方法双签名 + 显式句柄）在"简单场景零成本"与"复杂场景全表达力"之间不设中间税。

## 兼容性与分期（Compatibility and phasing）

- **wire 零改动**；与 Direction+TELL=3 正交（迟到回复作为孤儿响应被方向路由丢弃的行为，两者共同保证）。
- **additive**：默认签名与行为完全不变，proto option 是新增项——不占 0.5 破坏窗口，随时可落。
- 分期：
  - Phase 1（本 RFC 主体）：options.proto 扩展、`actr-framework::Reply`、hyper 泵/dedup 调整、Rust codegen 双签名、泵契约文档与测试；
  - Phase 2：FFI（uniffi 暴露 Reply 对象，Kotlin/Swift 得到同等能力）+ 护栏指标；
  - Phase 3：WIT guest ABI 扩展（现 `dispatch` 同步返回响应字节，需新增 `send-reply` host import 并让 dispatch 返回 `replied(bytes) | deferred` 变体）——在此之前 WASM guest 仅支持 RETURN 模式，codegen 对 wasm 目标遇 EXPLICIT 选项时报错而非静默降级。

## 未决问题（Unresolved questions）

1. `reply_mode` 的 proto extension tag 号（从仓库既有扩展号段分配）；
2. `max_pending_replies` 默认值与是否进 Phase 1；
3. FFI 侧 `Reply` 对象的生命周期约定（uniffi 对象跨语言 Drop 语义需验证）。

## 未来可能性（Future possibilities）

- 在 Phase 2 的指标与背压机制稳定后，可基于未决回复数量、deadline 与取消信号提供更细粒度的资源治理策略。
- FFI 与 WIT 获得同等的显式回复能力后，可评估将 `Reply<T>` 扩展为进度通知或流式回复抽象；这些语义需要独立 RFC，不属于本提案。
- 显式回复建立的“串行受理、应用控制并发”契约可作为后续 actor 调度与公平性设计的共同基础。
