# RFC-0419: 运行时时间语义与可选逻辑时钟

- Status: Proposed
- Date: 2026-07-22
- RFC PR: [#420](https://github.com/Actrium/actr/pull/420)
- Tracking issue: [#419](https://github.com/Actrium/actr/issues/419)
- Superseded by:
- Related: [Time, Clocks, and the Ordering of Events in a Distributed System](https://www.microsoft.com/en-us/research/wp-content/uploads/2016/12/Time-Clocks-and-the-Ordering-of-Events-in-a-Distributed-System.pdf)(DOI: 10.1145/359545.359563), [Logical Physical Clocks and Consistent Snapshots in Globally Distributed Databases](https://cse.buffalo.edu/tech-reports/2014-04.pdf)

## Summary

actr 明确区分三种不可互换的运行时时间语义：

1. runtime 使用进程内单调时钟计算 timeout、deadline、退避和耗时；
2. 跨越设备挂起期的真实流逝时长按可选平台能力处理：具备可靠 suspend-aware
   时钟源的平台测量并上报，缺失能力的平台执行规范化的保守回退；
3. 需要跨节点因果时间的上层协议可以使用可选 HLC 工具库。

本 RFC 不在 actr 内实现 UTC 校准服务，不给 `RpcEnvelope`、
`SignalingEnvelope` 或 `DataChunk` 增加 HLC 字段，也不要求应用使用逻辑时钟。
HLC 的物理分量来自可替换的物理时钟源，默认使用平台系统时间；是否把 HLC 写入
payload、持久化、签名或用于排序，由上层协议决定。

## Motivation

actr 当前存在多种看起来像“时间”的值：

- `RpcEnvelope.timeout_ms` 表示调用时限；
- `SignalingEnvelope.timestamp` 记录发送方墙上时间；
- `DataChunk.timestamp_ms` 由数据生产者解释；
- mailbox `created_at` 是本地入队时间；
- tracing 和日志还会记录本地观测时间。

这些值回答的问题不同。把它们统一成一个核心时间戳，容易产生两个错误：

- 使用会因校时而跳变的 UTC 计算 timeout 或耗时；
- 把 transport 时间误认为业务顺序、完整性或最终提交证明。

仓库现状同时提供了误用实例和不能一刀切的证据：

- `core/hyper/src/lifecycle/node.rs` 用墙钟差值计算队列耗时，属于应使用
  单调时钟的相对耗时；
- `core/hyper/src/lifecycle/compat_lock.rs`（兼容协商缓存，当前为预留能力）把
  绝对 UTC 期限持久化到磁盘做跨进程共享的缓存过期时间——这类用途墙钟是唯一
  可行时基，说明需要的是按用途分类，而不是一刀切的禁令；
- Kotlin 与 Swift 绑定（`bindings/kotlin/.../NetworkMonitor.kt`、
  `bindings/swift/.../ActrNode.swift`）用墙钟测量应用后台时长，交给
  `core/hyper/src/lifecycle/network_event.rs` 的长后台重连判定——因为进程内
  单调时钟在设备挂起期间冻结，测不出跨挂起的真实流逝。

另一方面，HLC 对部分上层协议有通用价值，但并非每个 RPC、信令包或数据分块都需要
因果时间。强制把 HLC 放进核心信封会增加 wire 契约和所有平台的实现负担，却不能替代
上层仍需定义的消息 ID、父引用、设备序号或冲突规则。

上述统一不需要触碰 wire format：需要修正的是文档、注释与实现内部的时钟选择，
protobuf schema 与既有契约保持不变，因此不存在协议层障碍（详见 Compatibility
and phasing）。

因此，actr 应统一基础能力和术语，而不统一所有应用的时间语义。

## Detailed design

### 论纲与导读

**本 RFC 的全部规范可归结为三条裁决：运行时计时行为仅信任本进程单调时钟；
wire 时间字段须满足三问与准入条件；业务与因果时间归应用，HLC 为其可选
工具库**。下面的章节是这三条裁决的展开。

| 你是谁 | 你关心什么 | 读哪里 |
|---|---|---|
| 写 runtime 或绑定层 | 超时、重连、心跳该用哪块表 | §1–§2 |
| 定协议、写 SDK | 哪些时间字段配上 wire | §3 |
| 写应用、要因果排序 | HLC 怎么用、边界在哪 | §4–§5 |
| 所有人 | 怎么证明没用错表 | §6 |

### 1. 时间边界——三类计时需求与其时基

| 概念 | 回答的问题 | 所有者 | 是否进入 actr wire |
|---|---|---|---|
| 单调时间（monotonic time） | 进程运行期间过去了多久、是否到期 | runtime | 否 |
| 跨挂起单调时间（suspend-aware monotonic） | 包含设备挂起在内真实流逝了多久 | runtime 与平台 | 否 |
| 墙上时间（wall time / UTC） | 现实世界大约几点 | 平台与观测系统 | 已在既有字段，不新增 |
| HLC | 消息是否存在因果先后 | 选择使用它的上层协议 | 由上层 payload 决定 |
| 业务事件时间（business event time） | 业务事件在其领域时基中的时间 | 上层协议 | 由上层 payload 决定 |

这些概念可以共享底层时间源，但不能互相替代：

- 单调时间没有日期，不能跨进程或跨设备比较，并且在设备挂起期间通常冻结
  （Linux `CLOCK_MONOTONIC` 与 Apple `mach_absolute_time` 均不计入挂起时长；
  Windows `QueryPerformanceCounter` 则计入睡眠——挂起行为跨平台并不一致）；
- 跨挂起单调时间（如 Linux `CLOCK_BOOTTIME`、Android `elapsedRealtime`）计入
  挂起时长，但各平台覆盖度与精度不同，Web 平台没有直接对应物；
- UTC 可以被校准、手工修改或在设备恢复后跳变；
- HLC 可以表达因果约束，但不能证明真实物理先后；
- 业务事件时间可能早于发送时间，也可能使用媒体、传感器或领域自己的时基。

### 2. Runtime 计时契约——运行时计时的规范契约

runtime 的相对时间用途分为两类，各自绑定不同的时钟。

**进程内相对时限**——挂起期间冻结是可接受的，甚至是期望的（设备睡眠时不应把
RPC 判为超时）：

- RPC timeout 和 deadline；
- retry、reconnect 与 backoff 的间隔计时；
- heartbeat 的发送节拍；
- queue、transport 和 handler 的耗时统计；
- 测试中的虚拟时间推进。

这类时限必须来自单调时钟。禁止使用 `Utc::now()`、`SystemTime`、JavaScript
`Date.now()` 或 protobuf timestamp 相减来判断这类时限。

**跨挂起真实流逝**——语义上关心现实世界流逝了多久，设备挂起必须计入：

- 长后台、长挂起后的重连与状态刷新判定；
- 与远端按真实时间维持的 lease、heartbeat 存活窗口的本地对齐；
- 空闲检测中以“用户或设备离开多久”为语义的部分。

这类时长的来源定义为可选的平台能力，回退策略必须显式规定：

1. 具备可靠 suspend-aware 时钟源的平台——Android `elapsedRealtime`、Darwin
   `CLOCK_MONOTONIC` / `mach_continuous_time`、Windows 无偏中断时间
   （`QueryUnbiasedInterruptTime`）、Linux `CLOCK_BOOTTIME`——用该源测量并由
   平台层上报（如绑定层报告的后台时长），runtime 对上报值做非负钳制与合理
   上界校验。绑定层现状用墙钟测量后台时长，语义上属于此类：实施审计时不得
   把它们机械替换为进程内单调时钟——那会使设备睡眠一小时后测得的后台时长
   约等于零、长后台重连判定永不触发；应替换为平台 suspend-aware 源并保留
   校验。
2. 能力缺失时的规范回退：进程内单调差值不得单独作为短/长挂起分类的输入
   ——它在挂起期间冻结，只能证明“至少流逝了这么久”（下界），永远无法
   证明“只流逝了这么久”。缺失该能力的平台必须采用下列策略之一：
   - (a) 回到前台一律按长挂起处理，保守地全量重建；
   - (b) 在保留既有连接之前执行同时覆盖信令与 WebRTC/ICE 数据路径的健康
     检查——仅信令可达不构成数据路径健康的证据（信令可通而 ICE/NAT/
     数据通道状态已陈旧）；
   - (c) 下界校验的合成估计：设进程内单调差值为 M（下界）、墙钟差值为
     W（估计）、分类阈值为 T、保护带 δ > 0，按固定顺序判定：
     1. M ≥ T ⇒ 长。下界已越线，无概率成分；
     2. W < M ⇒ 长。真实流逝不可能小于运行时长，W 与下界矛盾即证明
        墙钟在离开期间被回拨，估计作废；此事件必须记录；
     3. W ≥ T − δ ⇒ 长；
     4. 其余（M ≤ W < T − δ）⇒ 短。

     δ 的取值必须覆盖正常自动校时步进的量级（秒级）。此顺序下，误判短
     要求离开期间发生幅度大于 δ、且近似等于实际离开时长的回拨（幅度
     更大则触发第 2 条被证伪）——多重巧合。采用 (c) 的前提，三者缺一
     不可：M 与 W 锚点齐备（任一缺失回落 (a) 或 (b)）；误判短的残余
     风险能被既有恢复路径（心跳存活、发送失败分类）在有界时间内检出并
     自愈；(c) 的启用及第 1、2 条的触发均可观测。建议实现先以影子模式
     运行 (c)，并在能力在场时记录墙钟差值与观测值之差，以实测校时分布
     并校准 δ。
3. 未经下界校验的裸墙钟差值不得作为分类输入。墙钟差值单独使用时，仅可
   作为显式 opt-in 的启发式（例如提前预热重连），不得作为正确性回退。
4. 两个误判方向的代价不对称：误判为长挂起的代价是延迟（多做一次重建）；
   误判为短挂起的代价是正确性（带着陈旧连接状态继续运行）。回退策略必须
   偏向前者。策略 (c) 的判定顺序即该原则的机械化——所有不确定分支一律
   落向“长”。

墙上时间在计时路径上只有一类合法用途：**持久化或远端签发的绝对 UTC 有效期**
（凭据过期校验、跨进程共享的持久化缓存或租约的过期时间等）。单调时钟实例
不可持久化，此类期限的唯一
可行时基就是墙钟；实现必须容忍时钟跳变，例如对剩余时长做上界钳制，或在无法
信任本地时钟时保守地视为已到期。

deadline 的触发语义统一定义为：不早于 deadline 触发；允许迟到；挂起或平台节流
（如浏览器后台 tab）下迟到没有上界。到期判定由时钟语义保证，真实触发时刻受
调度与平台限制，两者是不同的承诺。

实现提供平台内部的最小抽象。它不进入公开 API surface；宿主 crate 与跨 runtime
复用方式——共享内部 crate，或各 runtime 以 cfg 分支实现同一契约——在实现阶段按
现有 workspace 结构确定：

```rust
trait MonotonicClock {
    type Instant: Copy + Ord;

    fn now(&self) -> Self::Instant;
    fn add(&self, instant: Self::Instant, duration: Duration) -> Self::Instant;
    fn elapsed(&self, earlier: Self::Instant, later: Self::Instant) -> Duration;
}
```

契约边界必须一致，避免各 runtime 实现分歧：

- `elapsed` 在 `later < earlier` 时饱和返回 `Duration::ZERO`（与现代
  `std::time::Instant::duration_since` 行为一致），不得 panic；
- `add` 溢出时返回一个不早于任何可达时刻的实现定义值（语义等价于永不触发的
  deadline），不得 panic（`std` 的 `Instant + Duration` 会 panic，实现内部应
  使用 `checked_add`）；
- `Instant` 关联类型不能序列化，不能写入 wire，不能在进程重启后恢复，也不得
  跨时钟实例或跨执行上下文（如 Web worker 与主线程之间）比较或运算；
- 不得假设亚毫秒精度（浏览器出于安全缓解对 `performance.now()` 做精度粗化）。

本抽象把“挂起期间不计入（冻结）”规定为进程内单调时钟的目标语义。std
`Instant` 的挂起行为跨平台未指定（Linux 与 Apple 平台冻结，Windows
`QueryPerformanceCounter` 计入睡眠）——这正是需要自建抽象的原因：各平台实现
必须选择满足冻结语义的时钟源（Windows 使用 `QueryUnbiasedInterruptTime`
一类明确排除睡眠的源），不得直接假定 `std::time::Instant` 满足。

native runtime 可以使用 `std::time::Instant`（在其挂起语义满足契约的平台上）
或异步运行时提供的时钟（如 `tokio::time`，便于测试中虚拟推进）；Web runtime
使用浏览器单调计时源（`performance.now()`），并注意其精度与后台节流限制。

需要跨重启恢复的 timeout 应持久化“业务操作及其策略”，启动后根据策略重新计算
本地 deadline；不得把旧进程的 monotonic instant 恢复到新进程。重算的时基是必须
显式选择的取舍：

- 纯延时类 timeout（如重试间隔）允许重启后重新起算，代价是崩溃重启会延长总时限
  （至少一次语义）；
- 具有互斥或租约含义的时限不得因重启延长，必须归入上文“持久化绝对 UTC 有效期”
  类别，用墙钟期限加跳变容忍重算，或保守地在重启后视为已到期。

### 3. 墙上时间的边界——wire 时间字段的准入与契约

actr 不实现 NTP、offset estimation、clock slewing、holdover 或可信授时协议，也不修改
操作系统时间。

日志和 telemetry 可以读取平台 UTC，但必须把它理解为本地观测值。需要更高精度的部署
可以向平台提供校准后的 UTC source，不改变 actr protocol。

wire 上的每个时间字段必须能回答三问：谁的钟（参考系）、谁消费（合法读方）、
缺失语义（字段不存在或为零时的行为）。三问回答不全的时间字段不应进入 wire
schema。时间预算类字段还须同时满足两条准入条件：其一，时限只有发送方知道
（调用方定义的任意工作，协议无从内定）；其二，下游持有它能做有用之事（如
受理前 fail-fast、驻留递减）。协议自定节奏的交互（信令、心跳）不满足其一，
其时限归入审计定时器清单；陈旧性判定归代际与会话门；队列防护归容量上界——
三者都是免时钟机制，不需要新的 wire 时间字段。

既有字段按上述纪律逐条固定语义如下：

- `SignalingEnvelope.timestamp` 是创建该信封的发送方墙上时间。
  `core/protocol/proto/signaling.proto` 中该字段现有注释
  “enqueue time (server clock)” 与实际盖章方不符：全部 flow 的信封（含
  envelope_error）都由创建方（客户端或服务端自身）用本地墙钟盖章，本 RFC 将该
  注释更正为发送方墙上时间。
  该字段不是服务端授时，仅限诊断消费，不能用于因果排序、timeout 或授权；
  proto2 required 使其不存在缺失态，退役路径见 Future possibilities；
- `RpcEnvelope.timeout_ms` 是发送方声明的本地相对预算，必须区分两个参考系
  解读：发送方帧——发送方以单调时钟自发送时刻起算，调用方的等待窗口包含
  网络在途与对端排队，它们消耗的是调用方的预算；接收方帧——调用链上每一跳
  独立解释该值，从各自的到达时刻重新起算（到达时刻加 `timeout_ms` 作为本地
  处理时限的提示），因此接收方推导出的时限是调用方真实剩余预算的上界，在途
  与排队时间并未从接收方视角扣除；按既有 proto 契约，该值大于 0 当且仅当
  direction 为 REQUEST，TELL 与 RESPONSE 填 0 且接收方必须忽略；它不构成
  跨节点 deadline，端到端时限收敛（deadline 传播）不在本 RFC 范围内；
- `DataChunk.timestamp_ms` 是生产者定义的数据时间，仅由理解该生产者时基的
  消费方解读，runtime 不消费该值；字段为 optional，缺失表示生产者未提供
  数据时间；stream 顺序仍由 `sequence` 决定，该值不参与排序、不作新鲜度
  判据，接收方不得将其与本地时钟比较。它是应用语义标注被提升为专字段的
  唯一特例，特权仅来自一点：时间是流数据压倒性最常见的标注，专字段让全部
  SDK 共享同名同类型的标准插槽，避免退化为逐应用的字符串约定，并在高频流
  上省去键名与包装开销。此特例不得援引为先例——其它应用语义标注一律放入
  `metadata`；
- mailbox `created_at` 只用于本地诊断或本地队列策略，不表示分布式消息顺序；
- tracing 中的 send/receive timestamp 是对应节点的观测时间，不覆盖 payload 中的
  业务事件时间。

本 RFC 只要求修正文档和误导性注释（见 Compatibility and phasing 第 2 步），
不改变这些字段的 wire schema。

### 4. 可选 HLC 工具库——不进 wire 的因果时钟

actr 可以提供一个不依赖核心 protocol 的可选 HLC 库。它复用算法和测试，不自动给
任何信封盖章。

若本 RFC 接受后 actr-hlc 走向对外发布，本节的 API 细则将迁出为库自己的规格
文档；本 RFC 保留的是边界裁决——HLC 是库、不进 wire、职责切在哪。

```rust
pub trait PhysicalClock {
    fn now_ms(&self) -> i64;
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct HybridTimestamp {
    /// 逻辑物理分量；observe 远端时间戳或 logical 溢出进位后可能领先当前 UTC。
    pub physical_ms: i64,
    pub logical: u32,
}

/// 时钟内部可持久化状态。字段会随版本演进（如增加高水位），因此保持
/// 不可穷举；`new` 对未来新增字段填充保守默认值，且该默认值必须在字段
/// 缺失时仍满足下文的恢复安全前置条件。
#[non_exhaustive]
pub struct HlcState {
    pub last: HybridTimestamp,
}

impl HlcState {
    pub fn new(last: HybridTimestamp) -> Self;
}

pub struct HybridClock<P: PhysicalClock> {
    physical: P,
    state: HlcState,
}

impl<P: PhysicalClock> HybridClock<P> {
    pub fn new(physical: P) -> Self;
    pub fn local_event(&mut self) -> Result<HybridTimestamp, HlcError>;
    pub fn observe(&mut self, remote: HybridTimestamp) -> Result<HybridTimestamp, HlcError>;
    pub fn export_state(&self) -> HlcState;
    pub fn from_state(physical: P, state: HlcState) -> Self;
}
```

`PhysicalClock` 的默认实现读取平台系统 UTC（Unix 纪元毫秒）。所有交换 HLC
时间戳的参与方必须共享同一时基，物理分量的跨节点可比性完全取决于这一点；校准
时间、模拟时间或领域时间可以通过替换实现接入，不改变 HLC 算法。

物理分量取毫秒而非纳秒是有意选择：毫秒 Unix 时间在 2^53 以内，可被
JavaScript 与 JSON 安全表示（Web 绑定需要）；毫秒内的并发事件由 u32 logical
吸收，容量充足。

`local_event` 在每个本地事件或发送事件发生时调用一次，`observe` 在每个接收事件
投递给处理逻辑之前调用一次；两者都是事件驱动的操作，不是周期性推进。`observe`
对任意输入只执行算法，除 physical 分量溢出外不做策略校验——远端值的合法性
校验完全由第 5 节的 validation helper 在调用前承担。返回 `Err` 时时钟状态保持
不变。`new` 构造的时钟初始状态为 `(physical_ms: 0, logical: 0)`，首次
`local_event` 或 `observe` 即收敛到当前时钟读数。`HlcError` 的具体形态在实现
阶段确定，至少须区分 physical 分量溢出。

类型与方法的最终命名在实现阶段按 crate 与模块布局确定：类型置于专用 `hlc`
模块内时使用裸名（如 `hlc::Timestamp`）；需要平铺再导出（如 FFI 绑定）时使用
`Hlc` 前缀消歧。本文草图中的 `Hybrid*` 名称仅为示意。

伪代码中 `p`、`l` 即当前状态的 `physical_ms` 与 `logical`，`r` 前缀表示远端
时间戳的对应分量，`now` 为 `PhysicalClock::now_ms()` 的当前读数。两段规则
算出的 `(next_p, next_l)` 既写回时钟状态，也作为返回的时间戳。

#### 本地事件

```text
next_p = max(now, p)

if next_p == p: next_l = l + 1
else:           next_l = 0
```

#### 接收远端事件

收到 `(rp, rl)` 时：

```text
next_p = max(now, p, rp)

if next_p == p and next_p == rp: next_l = max(l, rl) + 1
else if next_p == p:             next_l = l + 1
else if next_p == rp:            next_l = rl + 1
else:                            next_l = 0
```

比较使用 `(physical_ms, logical)` 字典序。若 A 因果先于 B，则 `A < B`；该保证
仅在所有参与方对每个本地或发送事件调用 `local_event`、对每个接收事件在处理前
调用 `observe`、且时钟状态未经不安全恢复（见下）的前提下成立。反过来 `A < B`
不能推出因果关系，完全相等也不代表同一事件。

逻辑计数器溢出时，physical 分量增加一毫秒并清零 logical；physical 分量溢出
返回错误，不得回绕，时钟状态不变。future-skew 校验越宽松，logical 越可能被
远端推着持续累加——远端偏差校验与溢出处理是同一防线的两层。

#### 状态导出与恢复

`export_state` 与 `from_state` 单独不保证跨重启单调：若在导出状态之后、进程
终止之前又签发过时间戳，用旧状态恢复的时钟会重新签发已用过、甚至更小的时间戳，
破坏上文的因果保证。调用方必须保证恢复所用状态不小于该时钟已签发的最大时间戳，
例如：先持久化后签发（persist-before-issue）；周期性持久化一个未来上界并保证
签发不越过该上界，恢复后等待或前跳到上界（把写盘成本从每次签发摊销为每周期
一次）；或在恢复后把 physical 分量前跳不小于部署环境最大时钟误差的量。库不
隐式提供这一保证，因为其代价——签发路径上的持久化、上界租约的管理，或恢复时
的额外前跳——必须由上层协议按自身一致性需求选择。

### 5. 上层集成规则——采用 HLC 的协议须定义的事项

选择使用 HLC 的协议负责定义：

- HLC 位于哪个 payload 字段；
- 把协议中的哪些点位映射为本地、发送与接收事件（须遵守第 4 节的调用纪律）；
- retry 是否复用原消息时间；
- 如何持久化并恢复时钟状态，以及如何满足第 4 节的恢复安全前置条件；
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

- 系统 UTC 正常前进、暂停和倒退时，进程内相对时限仍由单调时间正确触发；
- retry、heartbeat 和 reconnect 不受系统改钟影响；
- 挂起、恢复场景：进程内相对时限在挂起期间不误触发；跨挂起真实流逝类判定
  （如长后台重连）在恢复后按真实时长正确触发；
- 采用第 2 节回退策略 (c) 的实现须为四条判定各提供确定性测试，含回拨与
  前跳的墙钟模拟；
- deadline 触发语义（不早于到期触发）在 native、Web 和测试 runtime 一致；
- `elapsed` 的饱和语义与 `add` 的溢出语义符合第 2 节契约；
- 虚拟单调时钟可以确定性推进，无需真实 sleep。

可选 HLC 库必须覆盖：

- UTC 前进、相等和倒退时，连续 `local_event` 严格递增；
- `observe` 后产生的本地事件严格大于远端事件；
- 状态导出、恢复往返：恢复最新导出状态后，下一个输出严格大于该状态；
- 崩溃点在状态导出之后的场景：验证按恢复安全前置条件处理后不重发已签发的
  时间戳；
- logical overflow、physical overflow（含出错后状态不变），以及 validation
  helper 对非法远端值（负值、超出 future-skew 阈值）的拒绝；
- 固定测试向量在受支持平台上产生相同结果。

## Drawbacks

- 核心信封不提供统一 HLC，跨应用的因果 tracing 不能默认依赖它；
- 需要因果时间的协议必须显式定义 payload 字段和持久化策略；
- 运行时时间被划分为进程内单调、跨挂起、墙钟三类，审计与实现需要逐用途分类，
  一次性成本高于一刀切的“全部换成单调时钟”；
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

### 在 actr 内提供带校准状态机的 UTC 时钟库

[评审讨论](https://github.com/Actrium/actr/pull/420#issuecomment-5043106735) 提出过
一套 `actr-clock` 建模：`UtcClock` trait 返回带不确定度的
`UtcReading { unix_millis, uncertainty_millis }`，内置
`BuiltinUtcClock::{System, Disciplined}` 与 `UtcDisciplineState`
（Acquiring/Tracking/Holdover/Fault）状态机，HLC 消费任意 `UtcClock`。不采纳的
理由：默认 System 实现只能虚报 `uncertainty = 0`，而 HLC 算法本身不消费不确定度
（消费它的是 TrueTime 式 commit-wait，超出本 RFC 范围）；只有一个真实实现时，
内置 enum 层是过早抽象；Disciplined 时钟需要 offset 与 round-trip 样本的注入接口
和信任模型设计，这正是上一备选被拒绝的维护成本，放进可选 crate 并不会消失。
该讨论有两点被本 RFC 吸收：时钟持久化状态使用可演化的独立类型（`HlcState`，
`#[non_exhaustive]`），以及构造采用 `from_state` 命名。校准时钟库整体留作未来
独立 RFC（见 Future possibilities）。

### 完全由各应用自行实现 HLC

核心最小，但算法、溢出、恢复和测试容易重复或分歧。提供不接触 wire 的可选工具库，
可以复用机制而不替应用决定协议语义。

### 使用 UTC 计算 timeout

实现简单，但系统校时和手工改钟会导致提前超时或永不超时，违反 runtime 基础语义，
因此禁止。

## Compatibility and phasing

本 RFC 不改变 protobuf wire format，新增内容均为可选库与内部抽象，不动用 0.x
breaking-change window，不要求现有应用迁移，也不需要协同升级。

实施分为四步：

1. 审计 runtime 与绑定层的计时路径，把每个用点归入四类之一并记录：进程内相对
   时限（换单调时钟）、跨挂起真实流逝（换 suspend-aware 源或平台上报加校验，
   能力缺失的平台按第 2 节回退策略处理）、持久化或远端签发的绝对 UTC 有效期
   （保留墙钟，补跳变容忍）、观测墙钟（保留）。不得不加分类地机械替换；审计
   清单在 tracking issue 维护；
2. 修正误导性注释与文档，包括 `core/protocol/proto/signaling.proto` 中 timestamp
   字段的 “enqueue time (server clock)” 注释（注释修正不改 wire schema），并确认
   仓库外的 signaling server 实现没有依赖“服务端授时”语义；
3. 统一 native、Web 和测试 runtime 的单调时钟抽象与虚拟时间测试；
   Web 侧现存以 `Date.now()` 差值实现的超时判断是主要工作量；
4. 增加独立、可选且不依赖 `core/protocol` 的 HLC 工具库。

验收标准：核心 wire diff 为空（proto 注释修正不改变 schema）；现有 RPC 和数据流
行为不变；长后台重连等跨挂起判定行为不回归；runtime 时间测试覆盖系统 UTC 跳变
与挂起、恢复；不使用 HLC 的应用不新增配置、状态或 wire 成本。

## Unresolved questions

- 评审期需确认：Alternatives 中对 `actr-clock` 建模讨论的裁决——部分吸收、
  校准时钟库推迟至独立 RFC；
- 实现阶段确定：HLC 工具库的 crate 名称和目录、`HlcError` 的具体形态、类型命名
  的模块布局策略、validation helper 的错误类型、平台虚拟时间适配方式，以及各
  平台 suspend-aware 时钟源选型的最终确认（候选源见第 2 节；Web 平台无
  suspend-aware 源，走能力缺失回退，需在第 2 节三种回退策略中选定——
  `performance.now()` 的节流暂停只影响下界的紧致度、不破坏其下界性质，
  `Date.now()` 提供墙钟差值，预期路径即策略 (c)，具体选定留实现阶段）。

## Future possibilities

按论纲与导读中的读者侧分组。

**运行时侧**

- 校准 UTC 时钟源（disciplined clock）可作为独立 opt-in crate 由后续 RFC 定义，
  需包含样本注入接口与信任模型；其校准状态机可沿用评审讨论中的
  Acquiring/Tracking/Holdover/Fault 命名——其中 Tracking 一词刻意不宣称已与真实
  UTC 一致；
- deadline persistence 可以按具体生命周期策略另行设计，不改变本 RFC 的计时契约。

**wire 侧**

- 跨节点 deadline 传播（`timeout_ms` 的多跳收敛）可由独立 RFC 定义。既定
  候选模型是转发跳递减重盖（gRPC `grpc-timeout` 的成熟模型）：转发、中继与
  信箱在本地单调时钟上量出自身驻留时长，交付时从 `timeout_ms` 扣减后重盖
  同一字段；误差项仅为未计量的跳间在途时间，方向宽松（不会误杀），且 wire
  兼容（同一字段重盖新值）。actr 的信箱驻留可达分钟级，比典型 RPC 代理链
  更能从该模型受益。届时需配套级联取消语义：按关联 id 的显式控制消息，
  尽力而为；
- 受理前预算 fail-fast：对经信箱长驻留后才投递的 REQUEST，受理点可以判定
  预算已明显耗尽而不进入 handler（须与 dedup 的完成语义协调），避免白跑
  handler 产生注定被按方向路由丢弃的孤儿响应；
- `SignalingEnvelope.timestamp` 的退役路径：该字段为 required，但语义上禁止
  任何逻辑消费（仅诊断），观测时间的正确归宿是 tracing 通道（信封已携带
  traceparent/tracestate）。可借 `envelope_version` 分阶段退役：先钉死“仅
  诊断、禁止消费”的语义（本 RFC 的注释修正即此步）；下次信封版本升级时改为
  optional，写方为旧读者继续盖章；最低支持版本越过该版本后停止盖章，保留
  字段号。

**应用侧**

- tracing 可以通过显式 opt-in middleware 采集上层提供的 HLC；若未来需要核心 wire
  扩展，应由独立 RFC 定义协商和安全边界；
- 需要检测并发的协议可以在 HLC 之外使用父引用 DAG、version vector 或 vector
  clock。
