# RFC-0419: 运行时时间语义与可选逻辑时钟

- Status: Proposed
- Date: 2026-07-21
- RFC PR: [#420](https://github.com/Actrium/actr/pull/420)
- Tracking issue: [#419](https://github.com/Actrium/actr/issues/419)
- Superseded by:
- Related: [Time, Clocks, and the Ordering of Events in a Distributed System](https://www.microsoft.com/en-us/research/wp-content/uploads/2016/12/Time-Clocks-and-the-Ordering-of-Events-in-a-Distributed-System.pdf), [Logical Physical Clocks and Consistent Snapshots in Globally Distributed Databases](https://cse.buffalo.edu/tech-reports/2014-04.pdf)

## Summary

actr 明确区分两种不可互换的时间语义：

1. runtime 使用单调时钟计算 timeout、deadline、退避和耗时；
2. 需要跨节点因果时间的上层协议可以使用可选 HLC 工具库。

本 RFC 不在 actr 内实现 UTC 校准服务，不给 `RpcEnvelope`、
`SignalingEnvelope` 或 `DataChunk` 增加 HLC 字段，也不要求应用使用逻辑时钟。
HLC 的物理分量来自可替换的 UTC source，默认使用平台系统时间；是否把 HLC 写入
payload、持久化、签名或用于排序，由上层协议决定。

## Motivation

actr 当前存在多种看起来像“时间”的值：

- `RpcEnvelope.timeout_ms` 表示调用时限；
- `SignalingEnvelope.timestamp` 记录发送方墙上时间；
- `DataChunk.timestamp_ms` 由数据生产者解释；
- mailbox `created_at` 是本地入队时间；
- tracing 和日志还会记录本地观测时间。

这些值回答的问题不同。把它们统一成一个核心时间戳，容易产生两个错误：

- 使用会因校时而跳变的 UTC 计算 timeout；
- 把 transport 时间误认为业务顺序、完整性或最终提交证明。

另一方面，HLC 对部分上层协议有通用价值，但并非每个 RPC、信令包或数据分块都需要
因果时间。强制把 HLC 放进核心信封会增加 wire 契约和所有平台的实现负担，却不能替代
上层仍需定义的消息 ID、父引用、设备序号或冲突规则。

因此，actr 应统一基础能力和术语，而不统一所有应用的时间语义。

## Detailed design

### 1. 时间边界

| 概念 | 回答的问题 | 所有者 | 是否进入 actr wire |
|---|---|---|---:|
| monotonic time | 过去了多久、是否到期 | runtime | 否 |
| wall time / UTC | 现实世界大约几点 | 平台与观测系统 | 保留既有字段，不新增 |
| HLC | 消息是否存在因果先后 | 选择使用它的上层协议 | 由上层 payload 决定 |
| business event time | 业务事件在其领域时基中的时间 | 上层协议 | 由 payload schema 决定 |

这些概念可以共享底层时间源，但不能互相替代：

- 单调时间没有日期，不能跨进程或跨设备比较；
- UTC 可以被校准、手工修改或在设备恢复后跳变；
- HLC 可以表达因果约束，但不能证明真实物理先后；
- 业务事件时间可能早于发送时间，也可能使用媒体、传感器或领域自己的时基。

### 2. Runtime 单调计时契约

runtime 内部所有相对时间必须来自单调时钟：

- RPC timeout 和 deadline；
- retry、reconnect 与 backoff；
- heartbeat、lease 和空闲检测；
- queue、transport 和 handler elapsed time；
- 测试中的虚拟时间推进。

禁止使用 `Utc::now()`、`SystemTime`、JavaScript `Date.now()` 或 protobuf timestamp
相减来判断上述时限。

实现提供平台内部的最小抽象：

```rust
pub(crate) trait MonotonicClock {
    type Instant: Copy + Ord;

    fn now(&self) -> Self::Instant;
    fn add(&self, instant: Self::Instant, duration: Duration) -> Self::Instant;
    fn elapsed(&self, earlier: Self::Instant, later: Self::Instant) -> Duration;
}
```

native runtime 可以使用 `std::time::Instant` 或 executor time；Web runtime 使用浏览器
单调计时源。该类型不能序列化，不能写入 wire，也不能在进程重启后恢复。

需要跨重启恢复的 timeout 应持久化“业务操作及其策略”，启动后根据策略重新计算本地
deadline；不得把旧进程的 monotonic instant 恢复到新进程。

### 3. 墙上时间的边界

actr 不实现 NTP、offset estimation、clock slewing、holdover 或可信授时协议，也不修改
操作系统时间。

日志和 telemetry 可以读取平台 UTC，但必须把它理解为本地观测值。需要更高精度的部署
可以向平台提供校准后的 UTC source，不改变 actr protocol。

既有字段语义固定如下：

- `SignalingEnvelope.timestamp` 是创建该信封的发送方墙上时间，不是服务端授时，不能
  用于因果排序、timeout 或授权；
- `DataChunk.timestamp_ms` 是生产者定义的数据时间，stream 顺序仍由 `sequence`
  决定；
- mailbox `created_at` 只用于本地诊断或本地队列策略，不表示分布式消息顺序；
- tracing 中的 send/receive timestamp 是对应节点的观测时间，不覆盖 payload 中的
  业务事件时间。

本 RFC 只要求修正文档和误导性注释，不改变这些字段的 wire schema。

### 4. 可选 HLC 工具库

actr 可以提供一个不依赖核心 protocol 的可选 HLC 库。它复用算法和测试，不自动给
任何信封盖章。

```rust
pub trait PhysicalClock {
    fn utc_now_ms(&self) -> i64;
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct HybridTimestamp {
    pub physical_ms: i64,
    pub logical: u32,
}

pub struct HybridClock<P: PhysicalClock> {
    physical: P,
    state: HybridTimestamp,
}

impl<P: PhysicalClock> HybridClock<P> {
    pub fn tick(&mut self) -> HybridTimestamp;
    pub fn observe(&mut self, remote: HybridTimestamp) -> HybridTimestamp;
    pub fn snapshot(&self) -> HybridTimestamp;
    pub fn restore(physical: P, state: HybridTimestamp) -> Self;
}
```

`PhysicalClock` 默认实现读取平台系统 UTC。校准时间、模拟时间或领域时间可以通过替换
实现接入，不改变 HLC 算法。

#### 本地事件

令当前状态为 `(p, l)`，当前 UTC 为 `now`：

```text
if now > p: (p, l) = (now, 0)
else:       (p, l) = (p, l + 1)
```

#### 接收远端事件

收到 `(rp, rl)` 时：

```text
next_p = max(now, p, rp)

if next_p == p and next_p == rp: next_l = max(l, rl) + 1
else if next_p == p:             next_l = l + 1
else if next_p == rp:            next_l = rl + 1
else:                             next_l = 0
```

比较使用 `(physical_ms, logical)` 字典序。若 A 因果先于 B，则 `A < B`；反过来不能
推出因果关系。完全相等也不代表同一事件。

逻辑计数器溢出时，physical component 增加一毫秒并清零；physical component 溢出
返回错误，不得回绕。

### 5. 上层集成规则

选择使用 HLC 的协议负责定义：

- HLC 位于哪个 payload 字段；
- 何时 `tick`，收到消息后何时 `observe`；
- retry 是否复用原消息时间；
- 如何持久化并恢复 `HybridClock`；
- 允许多大的远端时间偏差；
- 是否将 HLC 纳入签名或认证数据；
- 如何结合 sender ID、message ID、父引用或序号产生确定性顺序。

工具库不自动接受不可信远端值。它应提供范围和 future-skew validation helper，但具体
policy 由调用者给出。调用者必须先验证，再调用 `observe`。

HLC 不提供：

- 唯一事件 ID；
- 并发检测；
- 消息完整性或缺口检测；
- 全局全序或最终提交；
- 授权、重放防护或可信时间证明。

需要这些性质的协议必须定义额外结构，不能依赖 transport envelope 的时间字段。

### 6. 测试要求

runtime 必须覆盖：

- 系统 UTC 正常前进、暂停和倒退时，timeout 仍由单调时间正确触发；
- retry、heartbeat 和 reconnect 不受系统改钟影响；
- native、Web 和测试 runtime 对 deadline 具有一致语义；
- 虚拟单调时钟可以确定性推进，无需真实 sleep。

可选 HLC 库必须覆盖：

- UTC 前进、相等和倒退时，连续 `tick` 严格递增；
- `observe` 后产生的本地事件严格大于远端事件；
- snapshot/restore 测试；
- logical overflow、physical overflow 和非法远端值；
- 固定测试向量在受支持平台上产生相同结果。

## Drawbacks

- 核心信封不提供统一 HLC，跨应用的因果 tracing 不能默认依赖它；
- 需要因果时间的协议必须显式定义 payload 字段和持久化策略；
- 平台 UTC 的精度由操作系统和部署环境决定，actr 不负责改善；
- 可选工具库减少算法重复，但不能消除不同协议在安全和排序策略上的差异。

## Alternatives

### 强制所有核心信封携带 HLC

优点是每个 RPC 都有统一的可比较时间。缺点是增加永久 wire 契约、热路径开销和跨平台
状态管理，同时仍不能提供完整性、并发检测或最终排序。未使用该能力的应用也必须承担
成本，因此不采用。

### 在 runtime 内实现 UTC 校准

可以提高日志和 HLC physical component 的精度，但需要时间服务、认证、采样过滤、
holdover 和故障状态机。这不是消息 transport 的必要能力，部署环境也可能已经提供
NTP、移动网络或设备管理校时，因此不采用。

### 完全由各应用自行实现 HLC

核心最小，但算法、溢出、恢复和测试容易重复或分歧。提供不接触 wire 的可选工具库，
可以复用机制而不替应用决定协议语义。

### 使用 UTC 计算 timeout

实现简单，但系统校时和手工改钟会导致提前超时或永不超时，违反 runtime 基础语义，
因此禁止。

## Compatibility and phasing

本 RFC 不改变 protobuf wire format，不要求现有应用迁移，也不需要协同升级。

实施分为三步：

1. 审计 runtime 中 timeout、deadline、retry、heartbeat 和 elapsed-time 路径，移除
   对 wall time 的依赖；
2. 统一 native、Web 和测试 runtime 的 monotonic clock abstraction 与虚拟时间测试；
3. 增加独立、可选且不依赖 `core/protocol` 的 HLC 工具库。

验收标准：核心 wire diff 为空；现有 RPC 和数据流行为不变；runtime 时间测试覆盖系统
UTC 跳变；不使用 HLC 的应用不新增配置、状态或 wire 成本。

## Unresolved questions

无阻塞性设计问题。HLC 工具库的 crate 名称和目录、validation helper 的错误类型以及
平台虚拟时间适配方式可以在实现阶段按现有 workspace 结构确定。

## Future possibilities

- tracing 可以通过显式 opt-in middleware 采集上层提供的 HLC；若未来需要核心 wire
  扩展，应由独立 RFC 定义协商和安全边界；
- 平台或部署组件可以实现校准后的 `PhysicalClock`；
- 需要检测并发的协议可以在 HLC 之外使用父引用 DAG、version vector 或 vector clock；
- deadline persistence 可以按具体生命周期策略另行设计，不改变单调时钟契约。
