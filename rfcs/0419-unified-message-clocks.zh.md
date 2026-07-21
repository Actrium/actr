# RFC-0419: 统一时间模型与消息时钟

- Status: Proposed
- Date: 2026-07-21
- RFC PR:
- Tracking issue: [#419](https://github.com/Actrium/actr/issues/419)
- Superseded by:
- Related: [Time, Clocks, and the Ordering of Events in a Distributed System](https://www.microsoft.com/en-us/research/wp-content/uploads/2016/12/Time-Clocks-and-the-Ordering-of-Events-in-a-Distributed-System.pdf), [Logical Physical Clocks and Consistent Snapshots in Globally Distributed Databases](https://cse.buffalo.edu/tech-reports/2014-04.pdf), [RFC 5905: Network Time Protocol Version 4](https://www.rfc-editor.org/rfc/rfc5905)

## Summary

actr 引入 runtime 所有的 `ActrClock`，统一提供三种不可混用的时间能力：

1. 单调时钟计算 timeout、deadline、退避和耗时；
2. 经校准的 UTC 估计用于展示、日志和外部时间映射；
3. 混合逻辑时钟（Hybrid Logical Clock，HLC）标记消息因果关系。

`RpcEnvelope` 和 `SignalingEnvelope` 必须携带 HLC。一个逻辑消息只生成一次 HLC，
重试和透明中继不得改写。`DataChunk` 只使用 stream sequence，RTP 继续使用自身的
sequence 和 timestamp。现有语义模糊的 signaling `timestamp` 与 DataChunk
`timestamp_ms` 被移除，不提供旧 wire 兼容层。

HLC 不是精确 UTC、唯一 ID、全序、并发检测、完整性证明或共识。需要严格完整顺序的
应用仍应使用消息 ID、父引用、设备序号和提交规则。

## Motivation

actr 当前把不同概念放在相似的时间字段里：

- `core/protocol/proto/signaling.proto` 将 `SignalingEnvelope.timestamp` 注释为
  server clock，但 native、Web 客户端和 signaling server 都在各自创建出站信封时
  使用本机 UTC 填写；
- `core/protocol/proto/package.proto` 的 `RpcEnvelope` 没有因果时间；
- `DataChunk.timestamp_ms` 没有统一时基，而 `sequence` 已经定义 stream 内顺序；
- `core/runtime-mailbox/src/sqlite.rs` 使用本地 `created_at` 排队；
- timeout 与重试若依赖 UTC，会受到校时、休眠和手工改钟影响。

单个物理时间戳不能同时解决这些问题。设备 UTC 可能漂移或倒退；中心授时会增加依赖，
但仍不能表达离线消息的因果关系；单个逻辑计数器又不利于跨节点日志和大致时间展示。

目标是给 runtime 一个统一且可测试的时间底座，同时保持边界清楚：framework 负责
计时、UTC 估计和消息因果；应用负责业务事件时间、会话完整性和最终排序。

## Detailed design

### 1. 不变量

实现必须满足以下不变量：

- 时长只由单调时钟计算，绝不由 UTC 或 HLC 相减得到；
- UTC 是带不确定度的估计，系统校时不得让 runtime 的 UTC 估计倒退；
- 若事件 A 因果先于 B，则 `hlc(A) < hlc(B)`；反命题不成立；
- HLC 相等不代表同一事件，HLC 可比较也不代表两个事件存在真实先后；
- 一个逻辑消息的 HLC 不因重试、存储、转发或 transport 切换而变化；
- 本地入队、接收和发送尝试时间属于观测数据，不得覆盖消息创建时间；
- 时间不参与授权、重放防护、证书有效期或最终提交，除非上层协议另行签名并验证。

### 2. `ActrClock`

每个可独立发送消息的 Actr 身份拥有一个 clock domain。runtime 为该 domain 创建
`ActrClock`：

```rust
pub struct UtcEstimate {
    pub unix_ms: i64,
    pub uncertainty_ms: u64,
    pub state: ClockState,
}

pub enum ClockState {
    Bootstrapping,
    Synchronizing,
    Synchronized,
    Holdover,
    Faulted,
}

pub struct HybridTimestamp {
    pub physical_ms: i64,
    pub logical: u32,
}

pub trait ClockSource {
    fn monotonic_now(&self) -> MonotonicInstant;
    fn system_utc_now_ms(&self) -> i64;
}

impl ActrClock {
    pub fn utc_now(&self) -> UtcEstimate;
    pub fn deadline_after(&self, duration: Duration) -> Deadline;
    fn stamp(&self) -> Result<HybridTimestamp, ClockError>;
    fn observe(&self, remote: HybridTimestamp) -> Result<(), ClockError>;
}
```

`MonotonicInstant` 和 `Deadline` 是当前进程内的 opaque value，不能序列化、跨进程比较
或写入 wire。`ClockSource` 可替换，以便用虚拟时间做确定性测试。

应用只能读取 UTC 估计和当前入站消息的 HLC；只有 runtime 可以 `stamp` 和
`observe`，防止应用破坏 clock domain 状态。

### 3. UTC 校准状态机

`ActrClock` 不修改操作系统时间。它维护一个“单调时刻到 UTC”的映射：

```text
estimated_utc = anchor_utc + elapsed(anchor_monotonic, now_monotonic) × rate
```

启动时将持久化的 UTC 下界与当前系统 UTC 取较大值，再锚定到新进程的单调时刻，进入
`Bootstrapping`；旧进程的单调时刻绝不跨重启复用。有可用时间源时，通过经过认证的
四时间戳交换采样 offset、round-trip delay 和 uncertainty；时间源可以是 actrix、
企业时间服务或其他已认证端点，不是消息排序者。

状态转换如下：

```text
Bootstrapping -> Synchronizing : 获得可用时间源
Synchronizing -> Synchronized : 样本数量、延迟和不确定度满足 realm policy
Synchronized  -> Holdover      : 时间源暂时不可用
Holdover      -> Synchronized : 获得新的合格样本
*             -> Faulted       : anchor 损坏、回退超限或样本持续违反 policy
Faulted       -> Synchronizing : 管理员修复或重新建立可信 anchor
```

采样与校准遵循以下规则：

- 本地经过时间使用单调时钟，避免采样期间系统 UTC 跳变；
- 优先使用低往返样本，拒绝负 delay、超限 offset 和离群值；
- 多时间源可用时以交集/中位区间降低单一错误源影响；
- 小偏差通过调整 rate 平滑收敛，大偏差提高 uncertainty，估计值绝不向后 step；
- 进入 `Holdover` 后继续由单调时钟推进 UTC，同时按平台漂移上界扩大 uncertainty；
- 同步状态和 uncertainty 只用于诊断与 realm admission policy，不改变 HLC 的因果
  算法。

离线不等于停钟。只要持久化 anchor 有效，runtime 可以在 `Holdover` 中创建、存储和
转发消息；恢复联网后再平滑校准。`Faulted` 则拒绝生成新的消息 HLC 和加入 realm，
直到可信 anchor 与 durable clock state 被修复。

### 4. HLC 算法

每个 clock domain 维护 `(p, l)`。`p` 是 Unix epoch 毫秒附近的逻辑物理分量，`l`
是同一 `p` 下的逻辑计数器。

本地创建消息时，令 `now = utc_now().unix_ms`：

```text
if now > p: (p, l) = (now, 0)
else:       (p, l) = (p, l + 1)
```

接收合法远端 `(rp, rl)` 时：

```text
next_p = max(utc_now().unix_ms, p, rp)

if next_p == p  == rp: next_l = max(l, rl) + 1
else if next_p == p:   next_l = l + 1
else if next_p == rp:  next_l = rl + 1
else:                   next_l = 0

(p, l) = (next_p, next_l)
```

比较使用 `(physical_ms, logical)` 字典序。`logical` 溢出时，先将 `physical_ms`
增加 1 毫秒再清零；`physical_ms` 溢出是不可恢复的 runtime fault，不得回绕。

runtime 必须在 actor 看见消息之前 `observe`，并在构造新的 request、response 或 tell
时 `stamp`。因此 request、处理结果和由结果触发的后续消息保留因果顺序。

### 5. 持久化与崩溃恢复

稳定 Actr 身份必须配置原子、持久的 `ClockStore`。runtime 不得在 clock state 丢失后
静默复用同一稳定身份；否则无法保证重启前后的本地事件顺序。临时身份可以不持久化，
但每次启动必须使用新身份。

为避免每条消息同步写盘，`ActrClock` 使用 physical high-watermark reservation：

1. 持久化并 flush 一个严格高于当前 `p` 的 `reserved_until_ms`；
2. 只生成 `physical_ms < reserved_until_ms` 的 HLC；
3. 本地生成或 observe 远端值即将使 `p` 到达边界时，先持久化新的 reservation，再
   更新内存状态；
4. 崩溃重启后从旧 `reserved_until_ms` 开始，而不是从最后可能未落盘的值开始。

默认 reservation window 为 60 秒，可按写入介质配置。崩溃后 HLC 最多向未来跳过一个
window，但绝不倒退。clock state、UTC anchor 和身份密钥必须使用同一持久化生命周期。

### 6. Wire schema

在 `core/protocol/proto/actr.proto` 定义唯一的消息时钟类型：

```proto
message HybridTimestamp {
  required int64 physical_ms = 1;
  required uint32 logical = 2;
}
```

`0 <= physical_ms <= 253402300799999`。HLC 是因果元数据，不是唯一 ID 或可信 UTC。

`RpcEnvelope` 必须携带逻辑消息创建时间：

```proto
message RpcEnvelope {
  // Other fields omitted.
  required HybridTimestamp event_time = 105;
}
```

`SignalingEnvelope.timestamp` 直接替换为：

```proto
message SignalingEnvelope {
  // Other fields omitted.
  required actr.HybridTimestamp event_time = 4;
}
```

`DataChunk.timestamp_ms` 删除。DataChunk 的规范顺序只由
`(stream_id, stream_epoch, sequence)` 表示；`stream_epoch` 在 stream open 控制消息中
建立，分块本身只携带 `stream_id` 和 `sequence`。业务采样时间、媒体 presentation
time 等必须由对应 payload schema 明确单位、时基和不确定度。RTP 保持自身时间模型。

需要稳定地排列无因果关系的消息时，使用：

```text
(event_time.physical_ms, event_time.logical,
 authenticated_sender_or_session_id, envelope_or_request_id)
```

这是确定性 tie-break，不是会话提交顺序。严格会话顺序必须由应用协议决定。

### 7. 消息生命周期

消息时间状态机如下：

```text
Draft --runtime stamp--> Created --persist/send--> InFlight --verify/observe--> Delivered
                              |                         |
                              +---- retry -------------+
                                   event_time 不变
```

- `event_time` 在逻辑消息从 `Draft` 进入 `Created` 时生成一次；
- transport retry 复用原信封、message/request ID 和 `event_time`；
- mailbox 存储和恢复完整信封，不重新盖章；
- 透明中继保留内层逻辑消息的 `event_time`；若中继创建新的外层 hop envelope，外层使用
  中继自身的新 HLC；
- response 是新的逻辑消息，接收方 observe request 后为 response 生成新 HLC；
- 用户明确重发或编辑后发送属于新逻辑消息，必须使用新 ID 和 HLC。

### 8. 异常远端时钟

realm policy 定义 `max_future_skew`，standalone 默认值为 5 分钟。接收方只接受：

```text
0 <= remote.physical_ms <= 253402300799999
remote.physical_ms <= local_utc_upper_bound + max_future_skew
```

其中 `local_utc_upper_bound = utc_now.unix_ms + utc_now.uncertainty_ms`。

缺失、越界或超前的 HLC 是协议错误：runtime 不 observe、不投递给 actor，并返回
`InvalidMessageClock`。不能一边投递一边忽略远端 HLC，否则会破坏 RFC 承诺的因果
不变量。异常必须产生带 peer、偏差和 clock state 的 metric/tracing event。

时间源与 peer 必须经过现有身份认证；即便如此，HLC 仍不得用于授权、新鲜度或重放
判断。安全协议应使用 nonce、签名设备序号、父消息引用和明确有效期。

### 9. Runtime 与 framework 使用规则

| 场景 | 唯一规范机制 |
|---|---|
| timeout、deadline、retry backoff | 单调时钟 |
| UTC 展示、日志时间、外部系统映射 | `UtcEstimate`，同时展示 uncertainty/state |
| RPC 与 signaling 因果关系 | 必填 `HybridTimestamp` |
| mailbox 同优先级 FIFO | 持久化 `enqueue_sequence`，不用 `created_at` 排序 |
| DataChunk 顺序 | stream epoch + sequence |
| RTP 顺序与播放时间 | RTP sequence/timestamp |
| 业务事件时间 | payload 自己的 typed field |
| 会话严格完整顺序 | 应用层 parent/device sequence/commit protocol |

`Context` 增加只读接口：

```rust
pub trait Context {
    fn utc_now(&self) -> UtcEstimate;
    fn incoming_event_time(&self) -> Option<HybridTimestamp>;
}
```

生命周期 hook 没有入站消息时返回 `None`；正常 RPC handler 必须为 `Some`。发送 API
不接受调用者传入 HLC，runtime 在信封创建边界统一生成。

### 10. 验收测试

实现至少覆盖：

- 虚拟 UTC 正常前进、暂停和倒退时，HLC 始终严格递增；
- request `<` response `<` follow-up 的跨 runtime 因果链；
- 重试、mailbox 恢复、transport 切换和透明中继不改写 `event_time`；
- high-watermark 在进程崩溃点穷举测试中不产生倒退；
- UTC 校准只 slew、不会 step backward，Holdover uncertainty 单调扩大；
- 缺失、越界和 future-skew HLC 在 actor 投递前被拒绝；
- mailbox 的 FIFO 不受系统 UTC 跳变影响；
- Rust、Web、FFI/WIT 对固定测试向量产生相同编码和比较结果；
- 高并发 clock domain 的吞吐和锁竞争满足 runtime 性能基线。

## Drawbacks

- 这是一次 protocol、runtime、mailbox、Web 和 FFI/WIT 的协同破坏性变更。
- 稳定身份必须有可靠 `ClockStore`，嵌入式和浏览器平台需要明确持久化实现。
- 强制 HLC 增加少量 wire 字节和每个逻辑消息一次串行状态更新。
- 严格拒绝异常时钟可能暂时阻断错误设备通信，但比静默破坏因果不变量更可诊断。
- HLC 很容易被误用为全序；API、文档和 tracing 必须持续暴露其边界。

## Alternatives

### 只修正现有 timestamp

墙上时间仍会倒退，也无法表达因果；timeout、mailbox 和数据流的误用不会消失。

### 只使用 Lamport clock

能表达因果且更小，但日志和离线消息还需第二套 UTC 字段。HLC 用一个值保留因果并
接近经校准 UTC，更适合通用消息信封。

### 使用 vector clock

可以检测并发，但大小随参与者增长，需要成员和压缩协议，不适合所有 runtime 信封。
需要并发检测的应用应使用父引用 DAG 或 version vector。

### 中心时间/序号服务

中心序号能提供特定域内全序，却增加往返、形成可用性依赖并阻断离线创建。时间服务只
校准 UTC，不参与消息排序；最终提交服务应由需要它的应用单独定义。

### HLC 可选或由应用自行添加

可选会让 request/response 因果保证取决于部署组合；应用实现则会在 native、Web、
重试和中继上产生不同语义。终局协议因此选择 runtime 统一、控制消息必填。

## Compatibility and phasing

本 RFC 明确不兼容旧 wire，不提供 optional 字段、dual write、字段回退或旧中继保留
规则。升级使用 0.x breaking-change 窗口，所有协议参与方按同一 protocol version
完成协同切换；版本不匹配在握手阶段立即失败，不进入消息投递。

实现分三步开发，但只在全部完成后发布目标协议：

1. `ActrClock`、虚拟时间测试、UTC discipline 和 durable reservation；
2. protobuf、runtime、mailbox、signaling、Web 与 FFI/WIT 全量迁移；
3. 跨平台互操作、崩溃注入、future-skew 和性能验收。

切换完成后删除旧字段和所有兼容分支，不保留永久 feature flag。

## Unresolved questions

无阻塞性设计问题。`max_future_skew`、reservation window、校时采样频率和允许的
uncertainty 由 realm/runtime policy 配置；它们不改变 wire 语义和因果不变量。

## Future possibilities

- tracing collector 可以用 HLC 合并跨节点事件，但必须把无因果关系显示为不确定；
- 对高吞吐单身份可实现分段 reservation 或无锁 HLC，前提是不改变生成结果；
- 应用层可在 HLC 之外定义父引用 DAG、version vector 或会话提交证明；
- 可信硬件时间证明可以作为独立签名字段加入业务协议，不改变 `ActrClock` 的信任边界。
