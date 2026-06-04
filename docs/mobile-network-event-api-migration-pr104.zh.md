# PR #104 移动端网络事件 API 调整说明

适用范围：`Actrium/actr#104`，相对于 `main` 分支的移动端网络事件 API 调整。

PR head：`fix/mobile-network-event-supervisor` / `6519dfc`。  
PR 说明里提到 Android/Swift 生成绑定和 SDK helper 这次没有同步更新，需要后续重新生成并适配。

## 1. 背景

`main` 分支的移动端网络事件 API 比较粗：

```text
handle_network_available()
handle_network_lost()
handle_network_type_changed(is_wifi, is_cellular)
cleanup_connections()
```

这套 API 的问题是：

- 网络状态只有 available/lost/typeChanged，无法表达 VPN、以太网、昂贵网络、受限网络等移动端真实状态。
- `cleanup_connections()` 在旧版本里实际是“清理 + 重新连接”的兼容语义，容易和 App 后台/前台、网络恢复事件混在一起。
- 多个系统回调短时间内同时到达时，只能按事件顺序处理，容易重复 reconnect 或处理过期状态。

PR #104 把移动端事件统一成“完整网络快照 + App 生命周期 + 显式清理/重连命令”。

## 2. API 变化概览

### 2.1 旧 API

```text
NetworkEvent.Available
NetworkEvent.Lost
NetworkEvent.TypeChanged { is_wifi, is_cellular }
NetworkEvent.CleanupConnections

handle_network_available()
handle_network_lost()
handle_network_type_changed(is_wifi, is_cellular)
cleanup_connections()
```

### 2.2 新 API

新增类型：

```text
NetworkAvailability:
  Unknown
  Available
  Unavailable

NetworkTransportFlags:
  wifi
  cellular
  ethernet
  vpn
  other

NetworkSnapshot:
  sequence
  availability
  transport
  is_expensive
  is_constrained

AppLifecycleState:
  Background
  Foreground { background_duration_ms }

CleanupReason:
  AppTerminating
  UserLogout
  StaleConnectionSuspected
  ManualReset

ReconnectReason:
  NetworkPathChanged
  LongBackground
  ProbeFailed
  ManualReconnect
  StaleConnectionSuspected
```

新的 `NetworkEventHandle` 方法：

```text
handle_network_path_changed(snapshot)
handle_app_lifecycle_changed(state)
cleanup_connections(reason)
force_reconnect(reason)
```

新增 API 说明：

| API | 参数 | Android / Swift 调用时机 | Rust 用它判断什么 | 替代 main 旧 API |
| --- | --- | --- | --- | --- |
| `handle_network_path_changed(snapshot)` | `NetworkSnapshot` | 系统网络路径变化时调用，例如断网、恢复、WiFi/蜂窝切换、VPN/以太网变化 | 根据 `availability` 判断 offline / restore / probe；根据 `sequence` 选择最新快照 | `handle_network_available()`、`handle_network_lost()`、`handle_network_type_changed(is_wifi, is_cellular)` |
| `handle_app_lifecycle_changed(state)` | `AppLifecycleState` | App 进入后台、回到前台时调用 | 根据 `Foreground.background_duration_ms` 判断短后台探测还是长后台强制重连 | 旧版本没有独立生命周期 API；部分场景替代旧的前台 `cleanup_connections()` 用法 |
| `cleanup_connections(reason)` | `CleanupReason` | 需要只释放连接资源、不希望立即恢复时调用，例如退出、登出、手动重置 | API 本身表示 cleanup only；`reason` 当前主要用于语义区分和预留策略 | `cleanup_connections()` 中“只清理”的使用场景 |
| `force_reconnect(reason)` | `ReconnectReason` | 需要清理旧连接后立即重建时调用，例如手动重连、长后台恢复、怀疑连接陈旧 | API 本身表示 cleanup + reconnect；`reason` 当前主要用于语义区分和预留策略 | 旧 `cleanup_connections()` 中依赖“清理后恢复”的使用场景 |

## 3. 旧调用如何迁移

| main 旧调用 | PR #104 新调用 | 说明 |
| --- | --- | --- |
| `handle_network_lost()` | `handle_network_path_changed(snapshot)`，`availability = Unavailable` | 表示当前无可用网络路径。 |
| `handle_network_available()` | `handle_network_path_changed(snapshot)`，`availability = Available` | 同时带上当前 transport flags。 |
| `handle_network_type_changed(is_wifi, is_cellular)` | `handle_network_path_changed(snapshot)`，`availability = Available`，`transport.wifi/cellular = ...` | 不再单独上报“网络类型变化”，而是上报完整快照。 |
| `cleanup_connections()` 用于退出/登出/手动清理 | `cleanup_connections(reason)` | 新语义是只清理，不自动重连。 |
| `cleanup_connections()` 用于“清掉旧连接再恢复” | `force_reconnect(reason)` | 如果旧代码依赖 cleanup 后马上恢复，需要改成 force reconnect。 |
| App 回前台后旧代码调用 `cleanup_connections()` | `handle_app_lifecycle_changed(Foreground { background_duration_ms })`，再上报当前 `NetworkSnapshot` | 普通回前台不建议再用 cleanup 表示恢复。 |

## 4. 移动端接入建议

### 4.1 网络变化统一上报 `NetworkSnapshot`

每次平台网络状态变化时，构造一个新的 `NetworkSnapshot`：

```text
NetworkSnapshot {
  sequence: next_sequence,
  availability: Available / Unavailable / Unknown,
  transport: NetworkTransportFlags {
    wifi,
    cellular,
    ethernet,
    vpn,
    other,
  },
  is_expensive,
  is_constrained,
}
```

字段要求：

| 字段 | Android | Swift | Rust 当前用途 | 可省略/默认 |
| --- | --- | --- | --- | --- |
| `sequence` | 必须传真实递增值 | 必须传真实递增值 | 同一批网络事件里选择最新快照，避免过期状态覆盖新状态 | 不能省略 |
| `availability` | 必须传 | 必须传 | 判断执行 offline / restore / probe | 不能省略 |
| `transport.wifi` | 建议传 | 建议传 | 当前不参与动作选择，预留给后续策略 | SDK helper 可省略，默认 `false` |
| `transport.cellular` | 建议传 | 建议传 | 当前不参与动作选择，预留给后续策略 | SDK helper 可省略，默认 `false` |
| `transport.ethernet` | 可选 | 可选 | 当前不参与动作选择，预留给后续策略 | SDK helper 可省略，默认 `false` |
| `transport.vpn` | 可选 | 可选 | 当前不参与动作选择，预留给后续策略 | SDK helper 可省略，默认 `false` |
| `transport.other` | 可选 | 可选 | 当前不参与动作选择，预留给后续策略 | SDK helper 可省略，默认 `false` |
| `is_expensive` | 可选 | 可选 | 当前不参与动作选择，预留给流量敏感策略 | SDK helper 可省略，默认 `false` |
| `is_constrained` | 可选 | 可选 | 当前不参与动作选择，预留给低数据/受限网络策略 | SDK helper 可省略，默认 `false` |

说明：

- `sequence` 由移动端维护，必须单调递增。
- `availability` 是 Rust 当前决策最关键的信息：`Unavailable` 表示断网，`Available` 表示可恢复，`Unknown` 表示先探测。
- 底层 UniFFI 结构体字段不是 optional，直接调用底层绑定时仍然要构造完整 `NetworkSnapshot`；这里的“可省略”是指 Swift/Kotlin SDK helper 可以不要求业务层传入，并由 helper 填默认值。
- iOS 可从 `NWPath` 映射 interface、`isExpensive`、`isConstrained`。
- Android 可从 `ConnectivityManager` / `NetworkCapabilities` 映射 transport、metered/constrained 状态。

### 4.2 App 生命周期单独上报

App 进入后台：

```text
handle_app_lifecycle_changed(Background)
```

App 回到前台：

```text
handle_app_lifecycle_changed(Foreground {
  background_duration_ms
})
```

字段要求：

| 字段/事件 | Android | Swift | Rust 当前用途 | 可省略/默认 |
| --- | --- | --- | --- | --- |
| `Background` | 必须按 App 后台事件上报 | 必须按 App 后台事件上报 | 记录生命周期事件；当前不等于清理连接 | 不能省略事件本身 |
| `Foreground.background_duration_ms` | 必须传真实后台时长 | 必须传真实后台时长 | 判断短后台探测还是长后台强制重连，阈值是 `30000ms` | 不建议省略；不知道时可默认 `0` |

说明：

- `Background` 本身不会自动清理连接。
- `Foreground` 如果后台时长小于 30s，runtime 默认做一次连接探测。
- `Foreground` 如果后台时长大于等于 30s，runtime 会走强制重连逻辑。
- 回前台后建议再上报一次当前 `NetworkSnapshot`，让 runtime 拿到最新网络路径。

### 4.3 清理和重连要分开

新版本里：

```text
cleanup_connections(reason)
```

只做清理，不自动重连。

字段要求：

| 方法 | Android | Swift | Rust 当前用途 | 可省略/默认 |
| --- | --- | --- | --- | --- |
| `cleanup_connections(reason)` | 需要主动释放连接时才调用 | 需要主动释放连接时才调用 | 方法本身决定执行 cleanup only；`reason` 当前不改变动作，预留给策略/日志 | SDK helper 可默认 `ManualReset` |
| `force_reconnect(reason)` | 需要清理后立即恢复时调用 | 需要清理后立即恢复时调用 | 方法本身决定执行 cleanup + reconnect；`reason` 当前不改变动作，预留给策略/日志 | SDK helper 可默认 `ManualReconnect` |

`cleanup_connections(reason)` 适合：

- App 即将退出：`AppTerminating`
- 用户登出：`UserLogout`
- 手动重置但不希望立即恢复：`ManualReset`
- 怀疑连接陈旧且只想先释放资源：`StaleConnectionSuspected`

如果需要“清理后立即恢复”，使用：

```text
force_reconnect(reason)
```

适合：

- 手动要求重建连接：`ManualReconnect`
- 长时间后台回来：`LongBackground`
- 网络探测失败后主动恢复：`ProbeFailed`
- 怀疑连接陈旧且希望立即重建：`StaleConnectionSuspected`

## 5. 推荐迁移顺序

1. 先更新生成绑定，确认旧方法已经从生成文件中消失，新类型和新方法可用。
2. 在移动端 SDK helper 内新增 `NetworkSnapshot` 构造函数，统一封装 iOS/Android 的系统网络状态映射。
3. 把网络 available/lost/typeChanged 三类旧回调全部改成 `handle_network_path_changed(snapshot)`。
4. 把 App 前后台回调改成 `handle_app_lifecycle_changed(...)`。
5. 检查所有旧 `cleanup_connections()` 调用点：需要只清理的改成 `cleanup_connections(reason)`，需要重建的改成 `force_reconnect(reason)`。
6. 针对 WiFi/蜂窝切换、断网恢复、短后台、长后台、退出/登出各跑一遍移动端集成验证。
