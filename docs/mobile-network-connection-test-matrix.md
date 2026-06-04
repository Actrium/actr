# Actr Mobile Network Event Model and Recovery Test Matrix

整理时间：2026-06-03

原始代码版本：`3b1c886`

API 描述更新：2026-06-04，按 `Actrium/actr#104` / `fix/mobile-network-event-supervisor` / `6519dfc` 的移动端网络事件 API 调整。

本文档梳理移动端通过 Swift/Kotlin binding 使用 Rust `actr` 时，网络恢复相关的事件模型、需要补充的事件、详细测试用例和竞态场景。最终目标是：**在所有可恢复场景下，连接最终能恢复或重新建立，消息最终能成功发送；在不可恢复场景下，请求必须有明确、有限的失败结果，不能挂死、不能污染后续连接。**

## 成功口径

| 场景 | 期望结果 |
|---|---|
| 网络可用 | 能建立 signaling/WebRTC/transport，RPC/DataStream 能成功发送 |
| 短暂断网/切网 | 连接能恢复；in-flight 请求要么完成，要么 bounded failure 后 retry 成功 |
| 长时间断网/长后台 | 旧连接被清理；恢复后新连接能建立，消息能重新发送 |
| 网络不可用 | 请求不能永久挂起，应快速失败或超时失败，错误可解释 |
| 事件重复/乱序/晚到 | Rust 幂等处理，不重复建连，不把旧 session 事件应用到新 session |
| 移动端生命周期复杂 | 前后台、息屏亮屏、页面重建、shutdown 后 callback 都不应造成 hang、泄漏或旧 handle 调用失控 |

## 验收目标覆盖表

G1/G2/G3/G4 是最终验收目标，不新增独立测试分类；具体用例落到现有 `L1/L2/L4/L5/RC` 中。

| 验收目标 | 覆盖位置 | 需要显式验证什么 |
|---|---|---|
| G1 可恢复场景最终恢复/重建并成功发送 | L2、L3、L4、L5、RC | 断网、切网、cleanup、长后台后，连接最终恢复或重建，恢复后 RPC/DataStream 能成功 |
| G2 不可恢复场景必须明确失败 | L2、L4、L5 | 无网络、signaling 不可达、peer 离线、DataChannel 长时间不 ready 时，请求返回明确错误或 deadline timeout |
| G3 不能挂死 | L1、L4、RC | 所有 event/RPC/DataStream/cleanup/connect await 都必须有 deadline 验证，超时后 pending 能清理 |
| G4 不能污染后续连接 | L1、L2、L3、RC | 旧 node、旧 handle、旧 transport、旧 session、旧 response 晚到时，不能影响新连接或新请求 |

## 当前技术架构

| 层级 | 组件 | 当前职责 | 风险点 |
|---|---|---|---|
| Android SDK | `NetworkMonitor` | 监听 `ConnectivityManager`，把网络变化映射成 `NetworkSnapshot` 后调用 `handle_network_path_changed(snapshot)` | 需要维护单调递增 `sequence`；callback 可能并发、乱序、晚到 |
| Swift SDK | `NetworkEventMonitor` | 监听 `NWPathMonitor`，把 path 变化映射成 `NetworkSnapshot` 后调用 `handle_network_path_changed(snapshot)` | `ActrNode` 创建时 monitor 即启动，可能早于 Rust `start()` |
| Swift/Android SDK | `AppLifecycleMonitor` | 监听前后台，调用 `handle_app_lifecycle_changed(Background/Foreground)` | `Foreground.background_duration_ms` 是 Rust 判断短后台探测/长后台重连的关键字段 |
| FFI | `ActrNode` | 持有 Rust node，缓存 `NetworkEventHandle` | `start()` one-shot；shutdown 后复用旧 node 应明确失败 |
| FFI | `NetworkEventHandleWrapper` | Android/Swift 调用 async 网络事件方法 | pre-start/post-shutdown/multi-handle await 需要 bounded result |
| Rust lifecycle | `NetworkEventHandle` + reconciler | 接收 network path、app lifecycle、cleanup、force reconnect 事件，settle window 合并后选择恢复动作 | 事件成功不等于 WebRTC/DataChannel ready |
| Rust recovery | `DefaultNetworkEventProcessor` | probe signaling、restore、offline、cleanup | cleanup 与 request/建连并发时要保证幂等 |
| Rust transport | `PeerGate` / `PeerTransport` / `DestTransport` / `WebRtcCoordinator` | 发送前置检查、创建连接、session guard、stale 防护 | 首次并发发送、旧 session 事件晚到、重复恢复是重点 |

## 移动端事件覆盖表

结论：PR #104 后，Rust FFI 提供四类移动端入口：网络路径快照、App 生命周期、只清理连接、强制清理后重连。网络路径变化统一走 `handle_network_path_changed(snapshot)`；`cleanup_connections(reason)` 是 cleanup only，不再表示“清理后恢复”；需要重建连接时必须显式调用 `force_reconnect(reason)`。

新增 API 概览：

| API | 参数 | Android/Swift 必须传的信息 | Rust 当前决策字段 | 预留/可默认字段 |
|---|---|---|---|---|
| `handle_network_path_changed(snapshot)` | `NetworkSnapshot` | `sequence`、`availability` | `sequence` 用来选择最新快照；`availability` 决定 offline/restore/probe | `transport.wifi/cellular/ethernet/vpn/other`、`is_expensive`、`is_constrained` 可由 SDK helper 默认 |
| `handle_app_lifecycle_changed(state)` | `AppLifecycleState` | `Background` 事件；`Foreground.background_duration_ms` | `background_duration_ms >= 30000` 时走长后台强制重连，否则先 probe | 无 |
| `cleanup_connections(reason)` | `CleanupReason` | 调用意图本身；reason 建议传真实原因 | API 本身决定 cleanup only | `reason` 当前主要用于语义/日志，helper 可默认 `ManualReset` |
| `force_reconnect(reason)` | `ReconnectReason` | 调用意图本身；reason 建议传真实原因 | API 本身决定 cleanup + reconnect | `reason` 当前主要用于语义/日志，helper 可默认 `ManualReconnect` |

| 移动端事件/场景 | 是否网络事件 | 是否已提供 | API | 当前是否足够 | 建议 |
|---|---|---|---|---|---|
| 当前网络从不可用变为可用 | 是 | 是 | `handle_network_path_changed(snapshot)`，`availability=Available` | 足够 | 必须传递递增 `sequence`；transport 建议带上但当前不参与 Rust 动作选择 |
| 当前网络确认不可用 | 是 | 是 | `handle_network_path_changed(snapshot)`，`availability=Unavailable` | 足够 | Android 需要确认 lost 的是 active/default network，避免旧 network 晚到误判 offline |
| Wi-Fi/蜂窝切换 | 是 | 是 | `handle_network_path_changed(snapshot)`，`availability=Available`，`transport.wifi/cellular` | 足够 | Wi-Fi/蜂窝字段当前是预留策略字段，建议真实传，helper 可默认 false |
| VPN 开启/关闭或当前网络经 VPN | 是 | 是 | `handle_network_path_changed(snapshot)`，`transport.vpn` | 部分足够 | `vpn` 当前不改变 Rust 动作，只作为预留/日志字段 |
| Wi-Fi 已连接但不可达互联网/captive portal/DNS 异常 | 是 | 是 | `handle_network_path_changed(snapshot)`，可用性不确定时 `availability=Unknown` | 部分足够 | Rust 会先 probe；端上应记录原始 validated/DNS 状态用于排查 |
| 网络事件重复、乱序、晚到 | 是 | 是 | `handle_network_path_changed(snapshot)` | 足够 | `sequence` 必须单调递增；必要时移动端仍要过滤非 active/default network 的旧 callback |
| 任意网络路径变化统一上报 | 是 | 是 | `handle_network_path_changed(snapshot)` | 足够 | Rust 当前必须用 `sequence` 和 `availability`；其余字段可预留 |
| 网络恢复异常、怀疑 stale connection | 否，是恢复命令 | 是 | `force_reconnect(StaleConnectionSuspected)` 或 `cleanup_connections(StaleConnectionSuspected)` | 足够 | 想立即重建用 force reconnect；只想释放资源用 cleanup |
| App 进入后台 | 否 | 是 | `handle_app_lifecycle_changed(Background)` | 足够 | 不等于 cleanup；不应默认关闭连接 |
| App 回前台 | 否 | 是 | `handle_app_lifecycle_changed(Foreground(background_duration_ms))` | 足够 | `background_duration_ms` 必须真实传；回前台后建议再补发当前 `NetworkSnapshot` |
| inactive/active | 否 | 否 | 未提供 | 不覆盖 | 只适合端上日志或轻量诊断，不应映射成网络事件 |
| App terminating/进程退出 | 否 | 部分 | `cleanup_connections(AppTerminating)` 或正常 shutdown | 部分足够 | 如果 runtime 仍可调用，cleanup 是只清理不重连；进程退出优先走 shutdown/释放资源流程 |
| 息屏/亮屏/锁屏 | 否 | 否 | 未提供 | 不覆盖 | 不应直接触发网络恢复；作为端上测试日志字段即可 |
| App 被杀后重启 | 否 | 否 | 未提供 | 需要端上流程保证 | 新进程按冷启动处理，重新创建 handle/start，并上报当前网络状态 |

## 移动端事件与连接恢复测试

### 分层

| 层级 | 测试类型 | 执行方式 | 谁写 | 覆盖目标 |
|---|---|---|---|---|
| L0 | 事件纯逻辑 | Rust 自动化测试 | Rust | action 选择、batch、debounce、snapshot 映射 |
| L1 | FFI/handle 生命周期 | Rust 自动化测试 | Rust | pre-start、start 中、shutdown 后、多 handle |
| L2 | Transport/连接恢复 | Rust 自动化测试或专项回归 | Rust | signaling/WebRTC/PeerTransport/DestTransport 恢复 |
| L3 | 移动端事件回放 | Rust replay 测试 | Rust + 移动端日志 | Android/iOS 真实 snapshot/lifecycle/command 序列 replay |
| L4 | 发送中网络波动 | Rust 慢速/专项测试 | Rust | RPC/DataStream 发送中断网、切网、retry、bounded failure |
| L5 | 真机/模拟器端上 | 移动端专项测试 | Android/Kotlin、Swift | OS callback、前后台、息屏亮屏、VPN、弱网 |

分类说明：

- L0 是最内层逻辑测试，只验证 Rust 内部对事件的解析、去重、状态转换、快照合并和 action 选择，不启动真实节点、不建立真实连接。
- L1 验证移动端通过 binding 调 Rust API 时的入口安全性，重点是 start 前、start 中、shutdown 后、重复 handle、旧 handle 回调等生命周期边界。
- L2 验证连接恢复主流程，重点是断网、恢复网络、切 Wi-Fi/蜂窝/VPN、cleanup 后，signaling/WebRTC/transport 能否恢复或重建，并继续成功发送消息。
- L3 是移动端真实事件序列的 Rust replay，把 Android/iOS 端上采集到的 callback 顺序回放到 Rust，验证 Rust 能承受真实系统事件的乱序、重复和抖动。
- L4 验证发送过程中遇到断网、切网、cleanup、后台恢复等网络波动时，RPC/DataStream 是否能明确成功、明确失败或恢复后可重试，不挂死、不重复响应。
- L5 是必须在 Android/iOS 真机或模拟器上验证的端上场景，覆盖 OS callback、前后台、息屏亮屏、VPN、弱网、App 被杀、权限变化等 Rust 无法完整模拟的行为。
- RC 是竞态测试矩阵，专门覆盖两个或多个动作同时发生的临界情况，例如发送中 cleanup、建连中断网、旧连接 late ready、双端同时首发、shutdown 和请求重叠。

### 覆盖池统计

| 覆盖池 | 数量 | 说明 |
|---|---:|---|
| Android 网络事件序列 | 24 | 冷启动、Wi-Fi/蜂窝、飞行模式、VPN、后台、进程重启 |
| iOS 网络事件序列 | 21 | 冷启动、Wi-Fi/蜂窝、飞行模式、VPN/热点、低数据、后台、multi scene |
| 网络事件处理/debounce | 19 | path snapshot、app lifecycle、cleanup/reconnect、result feedback、batch、debounce、probe |
| 移动端 full disconnect | 2 | 15s ICE restart、65s rebuild |
| 大包/DataStream 中断 | 7 | baseline、type switch、short/long offline、short/long background、delivery uncertain |
| WebRTC/PeerGate 恢复 | 约 15 | answerer/offerer recovery、coalesce、cleanup rebuild、latency、early RPC |
| stale session/旧资源防护 | 5 | old failed/closed/ready、old response late、old handle 不污染新连接 |
| cleanup/request/deadline overlap | 6 | request during cleanup、deadline vs cleanup/connect、pending 清理 |
| 不可恢复明确失败 | 5 | signaling 不可达、peer 离线、DataChannel 不 ready、RPC/DataStream 不可恢复 |
| deadline/不能挂死 | 5 | FFI event、RPC、DataStream、cleanup/connect/send 都要 bounded |
| Android 端上基础链路 | 3 | Echo RPC、DataStream、Unified workload |
| Swift 包级链路 | 3 | 类型转换、linked workload API、本地 dispatch |

## L0 事件纯逻辑测试

事件短记法：

- `Path(Available,wifi)` 表示 `handle_network_path_changed(NetworkSnapshot { availability=Available, transport.wifi=true, ... })`
- `Path(Unavailable)` 表示 `handle_network_path_changed(NetworkSnapshot { availability=Unavailable, ... })`
- `Path(Unknown)` 表示 `handle_network_path_changed(NetworkSnapshot { availability=Unknown, ... })`
- `Lifecycle(Foreground(45000))` 表示 `handle_app_lifecycle_changed(Foreground { background_duration_ms=45000 })`
- `Cleanup(ManualReset)` 表示 `cleanup_connections(ManualReset)`
- `ForceReconnect(ManualReconnect)` 表示 `force_reconnect(ManualReconnect)`

| Case ID | 优先级 | 输入 | 期望 | 测试实现方 |
|---|---|---|---|---|
| L0-01 | P0 | 空事件 | `Noop` | Rust |
| L0-02 | P0 | `Path(Available)` | `Restore` | Rust |
| L0-03 | P0 | `Path(Unavailable)` | `Offline` | Rust |
| L0-04 | P0 | `Path(Available,wifi)` | `Restore` | Rust |
| L0-05 | P0 | `Path(Available,cellular)` | `Restore` | Rust |
| L0-06 | P0 | `Path(Available,other)` | `Restore` | Rust |
| L0-07 | P0 | `Cleanup(ManualReset)` | `CleanupOnly` | Rust |
| L0-08 | P0 | `Path(Unavailable),Path(Available,wifi)` | `Restore` | Rust |
| L0-09 | P0 | `Path(Available),Path(Unavailable)` | `Offline` | Rust |
| L0-10 | P0 | `Cleanup(ManualReset),Path(Available,wifi)` | cleanup 优先 | Rust |
| L0-11 | P0 | `Path(Available),Cleanup(ManualReset),Path(Available,cellular)` | cleanup 优先 | Rust |
| L0-12 | P0 | `Path(Available,wifi),Cleanup(ManualReset)` | cleanup 优先 | Rust |
| L0-16 | P0 | SDK helper 把 legacy available callback 转成 `Path(Available)` | 新 API 行为不变 | Rust |
| L0-17 | P0 | `Path(Available,wifi)` 且 sequence 最新 | `Restore` | Rust |
| L0-13 | P1 | 重复 `Path(Available)` 10 次 | restore 受控，不重复 probe 风暴 | Rust |
| L0-14 | P1 | 重复 `Path(Unavailable)` 10 次 | offline 幂等 | Rust |
| L0-15 | P1 | `Path(Unavailable),Path(Available),Path(Unavailable),Path(Available)` | 以最新 sequence 的有效快照决策 | Rust |
| L0-18 | P1 | 端上 validated 原始状态变化触发 `Path(Unknown)` | 不 panic；策略明确，通常 probe/restore 后失败可解释 | Rust |
| L0-19 | P1 | `Path(Available,vpn)` | `Restore` 或 probe | Rust |
| L0-20 | P1 | 端上 cost/constrained 原始状态变化触发 `Path(Available)` | 不强制 cleanup，策略稳定 | Rust |

## L1 FFI 和节点生命周期测试

| Case ID | 优先级 | 场景 | 操作 | 期望 | 测试实现方 |
|---|---|---|---|---|---|
| L1-01 | P0 | event before start | 创建 handle，node 未 start，调用 network event | bounded result，不永久 await | Rust |
| L1-02 | P0 | event during start | start 正在进行，event 已排队 | reconciler 启动后 drain，调用返回 | Rust |
| L1-03 | P0 | event after shutdown | shutdown/drop 后调用 handle | 快速失败或可控返回 | Rust |
| L1-04 | P0 | handle create vs start | 并发 `createNetworkEventHandle()` 和 `start()` | 无 panic/无挂死 | Rust |
| L1-05 | P0 | repeated handle creation | 多次 `createNetworkEventHandle()` | 复用缓存，不创建多套 channel | Rust |
| L1-06 | P0 | multi-handle concurrent await | 多 cloned handle 并发发事件 | 不串结果、不死锁 | Rust |
| L1-07 | P0 | start one-shot | 同一 node start 两次 | 第二次明确失败 | Rust |
| L1-08 | P0 | reuse node after shutdown | shutdown 后复用旧 node 重连 | 明确失败；必须新建 node | Rust |
| L1-11 | P0 | FFI deadline contract | network event/cleanup 调用统一包 timeout | 超时内返回或明确失败，不永久 await | Rust |
| L1-09 | P1 | handle dropped while event pending | 调用中 drop handle | Rust 不泄漏任务 | Rust |
| L1-10 | P1 | app callback after node close | 模拟移动端旧 monitor 调旧 handle | 错误可解释，不挂 | Rust + 移动端 |

## L2 连接恢复测试

| Case ID | 优先级 | 场景 | 事件/动作 | 期望 | 测试实现方 |
|---|---|---|---|---|---|
| L2-01 | P0 | 基础建连 | 两 peer 首次 RPC | signaling/WebRTC/transport 建立，消息成功 | Rust |
| L2-03 | P0 | 短断网恢复 | 已连接后 `Path(Unavailable) -> Path(Available,wifi)` | WebRTC 恢复或重建，后续 RPC 成功 | Rust |
| L2-04 | P0 | 长断网恢复 | `Path(Unavailable)` 后等待，再 `Path(Available)` | 旧连接清理，新连接成功 | Rust |
| L2-05 | P0 | Wi-Fi -> 蜂窝 | `Path(Unavailable),Path(Available,cellular)` | 最终可 RPC/DataStream | Rust + Android/iOS replay |
| L2-06 | P0 | 蜂窝 -> Wi-Fi | `Path(Available,cellular),Path(Available,wifi)`，旧 lost 晚到另测 | 不误判 offline，最终可 RPC | Rust + Android/iOS replay |
| L2-07 | P0 | 飞行模式开 | `Path(Unavailable)` | 进入 offline，请求 bounded failure | Rust |
| L2-08 | P0 | 飞行模式关 | `Path(Available,cellular)` | 恢复后 retry 成功 | Rust |
| L2-09 | P0 | cleanup + restore | `ForceReconnect(ManualReconnect)` 或 `Cleanup(ManualReset),Path(Available)` | cleanup only 不自动恢复；需要恢复时必须 force reconnect 或后续 path restore | Rust |
| L2-10 | P0 | cleanup 后立即 RPC | cleanup 返回后马上发送 | 可短暂 `Connection recovering`，retry 成功 | Rust |
| L2-14 | P0 | request in-flight lost | RPC 发送中 `Path(Unavailable)` | 不挂死；恢复后 retry 成功 | Rust |
| L2-15 | P0 | signaling 不可达 | 阻断 signaling 后发送/恢复 | 明确失败或 deadline timeout；恢复 signaling 后 retry 成功 | Rust |
| L2-16 | P0 | peer 离线/不存在 | 向不可达 peer 发送 RPC | bounded failure，pending 清零，不污染后续 peer | Rust |
| L2-17 | P0 | DataChannel 长时间不 ready | WebRTC 建连卡在未 ready 状态 | bounded failure；后续重建后可发送 | Rust |

## L3 移动端事件序列回放

Android/Swift 端上提供原始日志，Rust 把 `network_snapshot`、`lifecycle_event`、`cleanup_command`、`reconnect_command` 序列转成 replay test。

| Case ID | 平台 | 优先级 | 场景 | 回放输入 | 期望 |
|---|---|---|---|---|---|
| L3-A01 | Android | P0 | 冷启动在线 | `Path(Available,wifi)` | start 后可 RPC |
| L3-A02 | Android | P0 | 冷启动离线 | `Path(Unavailable)` | offline 明确，不挂 |
| L3-A03 | Android | P0 | Wi-Fi 打开 | `Path(Available,wifi)` | restore |
| L3-A04 | Android | P0 | Wi-Fi 断且无蜂窝 | `Path(Unavailable)` | offline |
| L3-A05 | Android | P0 | Wi-Fi -> 蜂窝 | `Path(Unavailable),Path(Available,cellular)` | restore |
| L3-A06 | Android | P0 | 蜂窝 -> Wi-Fi，旧 lost 晚到 | `Path(Available,cellular),Path(Available,wifi),old Path(Unavailable)` | 以最新 sequence 决策，不误 offline |
| L3-A07 | Android | P0 | 短网络抖动 | `Path(Unavailable),Path(Available)` | restore，连接数受控 |
| L3-A08 | Android | P0 | 飞行模式开 | `Path(Unavailable)` | offline，bounded failure |
| L3-A09 | Android | P0 | 飞行模式关 | `Path(Available,cellular)` | retry 成功 |
| L3-A14 | Android | P0 | 后台不 cleanup | background 后无事件 | 不误 cleanup |
| L3-A15 | Android | P0 | 短后台回前台 | foreground duration < 阈值 | 不强制 cleanup，必要时 probe |
| L3-A16 | Android | P0 | 长后台回前台 | foreground duration > 阈值 | cleanup/rebuild 后可 RPC |
| L3-A17 | Android | P0 | 后台期间切网，前台补发 online | `Lifecycle(Foreground(...)) + Path(Available)` | 最终可 RPC |
| L3-A18 | Android | P0 | 后台期间断网，前台补发 offline | `Lifecycle(Foreground(...)) + Path(Unavailable)` | offline 明确 |
| L3-A23 | Android | P0 | Activity 重建重复 monitor | duplicate `Path(Available)` | 幂等，不重复建连 |
| L3-A24 | Android | P0 | shutdown 后 callback | old monitor event | 不挂，不污染新 node |
| L3-I01 | iOS | P0 | 冷启动在线 | 初始 path + `Path(Available,wifi)` | 初始 suppress 后 start 可 RPC |
| L3-I02 | iOS | P0 | 冷启动离线 | `Path(Unavailable)` | offline 明确 |
| L3-I03 | iOS | P0 | Wi-Fi -> 蜂窝，unsatisfied gap | `Path(Unavailable),Path(Available,cellular)` | restore |
| L3-I04 | iOS | P0 | 蜂窝 -> Wi-Fi | `Path(Available,wifi)` | restore |
| L3-I05 | iOS | P0 | Wi-Fi 断且无蜂窝 | `Path(Unavailable)` | offline |
| L3-I06 | iOS | P0 | 飞行模式开 | `Path(Unavailable)` | offline |
| L3-I07 | iOS | P0 | 飞行模式关 | `Path(Available,cellular)` | retry 成功 |
| L3-I12 | iOS | P0 | 短后台回前台 | foreground duration < 阈值 | 不误 cleanup |
| L3-I13 | iOS | P0 | 长后台回前台 | foreground duration > 阈值 | cleanup/rebuild 后可 RPC |
| L3-I14 | iOS | P0 | suspend 后 online 恢复 | `Lifecycle(Foreground(...)) + Path(Available)` | 最终可 RPC |
| L3-I15 | iOS | P0 | suspend 后 offline 恢复 | `Lifecycle(Foreground(...)) + Path(Unavailable)` | offline 明确 |
| L3-I16 | iOS | P0 | multi scene duplicate events | duplicate `Path(Available)` | 幂等 |
| L3-I20 | iOS | P0 | shutdown 后 path callback | old handle event | 不挂，不污染新 node |
| L3-I21 | iOS | P0 | deinit 与 Task await 并发 | pending event + deinit | 不泄漏/不挂 UI |
| L3-A10 | Android | P1 | VPN 开关 | `Path(Available,vpn=true/false)` | probe/restore，不能重复建连 |
| L3-A11 | Android | P1 | captive portal / validated 变化 | 端上记录 validated 原始状态，Rust 至少收到 path changed | 错误可解释，validated 后恢复 |
| L3-A12 | Android | P1 | DNS/link properties 变化 | `Path(Unknown)` 或 `Path(Available)` | probe/restore |
| L3-A13 | Android | P1 | metered 变化 | 端上记录 metered 原始状态，Rust 可不新增核心字段 | 不强制 cleanup |
| L3-A19 | Android | P1 | Doze 延迟 callback | 延迟 `Path(Available)` | 不挂，最终恢复 |
| L3-A20 | Android | P1 | 进程重启 online | 新 node + `Path(Available)` | 新连接成功 |
| L3-A21 | Android | P1 | 进程重启 offline | 新 node + `Path(Unavailable)` | offline 明确 |
| L3-A22 | Android | P1 | websocket remote close | 无 network event | 不误触发 network recovery |
| L3-I08 | iOS | P1 | VPN/热点变化 | `Path(Available,other/vpn)` | probe/restore |
| L3-I09 | iOS | P1 | Low Data Mode | 端上记录 constrained 原始状态，Rust 可不新增核心字段 | 不破坏连接 |
| L3-I10 | iOS | P1 | expensive network | 端上记录 expensive 原始状态，Rust 可不新增核心字段 | 不强制 cleanup |
| L3-I11 | iOS | P1 | route/DNS 变化 | `Path(Unknown)` 或 `Path(Available)` | probe/restore |
| L3-I17 | iOS | P1 | app killed restart online | 新 node + `Path(Available)` | 新连接成功 |
| L3-I18 | iOS | P1 | app killed restart offline | 新 node + `Path(Unavailable)` | offline 明确 |
| L3-I19 | iOS | P1 | websocket remote close | 无 network event | 不误触发 network recovery |

## L4 发送中网络波动测试

| Case ID | 优先级 | 场景 | 期望 | 测试实现方 |
|---|---|---|---|---|
| L4-02 | P0 | baseline DataStream | stream 发送成功，顺序正确 | Rust |
| L4-12 | P0 | RPC 发送中不可恢复 | RPC in-flight 后进入长期无网络/signaling 不可达 | 请求 deadline 内失败，失败结果可解释 | Rust |
| L4-13 | P0 | DataStream 发送中不可恢复 | DataStream in-flight 后 channel 长时间不可用 | stream 明确失败或 delivery uncertain，不永久挂起 | Rust |
| L4-03 | P1 | 大包 RPC baseline | payload hash 一致 | Rust |
| L4-04 | P1 | 大包发送中 type switch | 原请求完成或 bounded retry 成功 | Rust |
| L4-05 | P1 | 大包发送中短断网 | 原请求恢复完成 | Rust |
| L4-06 | P1 | 大包发送中长断网 | 原请求 bounded failure，retry 成功 | Rust |
| L4-07 | P1 | 大包发送中短后台 | 原请求完成 | Rust |
| L4-08 | P1 | 大包发送中长后台 cleanup | 完成或 bounded failure，retry 成功 | Rust |
| L4-09 | P1 | DataStream channel close | delivery uncertain hook 发出 | Rust |
| L4-10 | P1 | event storm + 连续发送 | 无 pending 泄漏，最终发送成功 | Rust |
| L4-11 | P2 | 长时间 30min 稳定性 | 连接数/任务数/内存不持续增长 | Rust + 移动端 |

## 竞态测试矩阵

| Case ID | 优先级 | 竞态 | 触发方式 | 期望 | 测试实现方 |
|---|---|---|---|---|---|
| RC-03 | P0 | shutdown vs event | shutdown/drop 同时 network callback | bounded result，不污染新 node | Rust + 移动端 |
| RC-04 | P0 | multi monitor duplicate event | 两个 monitor 同时发 `Path(Available)` | 幂等，不重复建连 | Rust + 移动端 |
| RC-05 | P0 | first concurrent send same dest | N 个首次 RPC 同时发 | 一个 creator，一个有效 session | Rust |
| RC-06 | P0 | send vs cleanup | RPC in-flight，同时 cleanup | bounded result，pending 清零 | Rust |
| RC-07 | P0 | create transport vs cleanup | 建连未完成时 cleanup | cancel 生效，无 stale peer | Rust |
| RC-08 | P0 | create transport vs shutdown | 建连未完成时 shutdown | task 清理 | Rust |
| RC-09 | P0 | old failed late | 新 session 已 ready，旧 failed 晚到 | 不 reblock | Rust |
| RC-10 | P0 | old closed late | 新 session 已 ready，旧 closed 晚到 | 不关闭新 session | Rust |
| RC-11 | P0 | old ready late | 新 session 已 ready，旧 ready 晚到 | 不切回旧 transport | Rust |
| RC-16 | P0 | Android available vs capabilities | `onAvailable` 和 `onCapabilitiesChanged` 同时生成 snapshot | sequence 顺序可解释，Rust 幂等 | Android + Rust replay |
| RC-17 | P0 | Android old lost late | 新 `Path(Available)` 后旧 network lost callback 晚到 | 旧 callback 不应生成更新 sequence 的 offline 快照；不误 offline | Android + Rust replay |
| RC-18 | P0 | Android stopMonitoring vs callback | Activity destroy 同时切网 | 不调用旧 node 或错误可控 | Android |
| RC-19 | P0 | Android reconnect button vs callback | 用户断开/重连同时切网 | 不复用旧 node | Android |
| RC-20 | P0 | iOS path vs start | `NWPathMonitor` callback 早于 `start()` | 不 pre-start hang | Swift + Rust |
| RC-21 | P0 | iOS foreground lifecycle vs path update | 长后台回前台同时 path update | `Lifecycle(Foreground(...))` 和 `Path(...)` 合并后最终可 RPC | Swift + Rust replay |
| RC-22 | P0 | iOS deinit vs Task await | 页面释放时 Task 等 Rust result | 不泄漏、不挂 UI | Swift |
| RC-23 | P0 | iOS multi scene duplicate foreground | 多 Scene 同时 foreground | 幂等，不重复 cleanup storm | Swift |
| RC-25 | P0 | request timeout vs late response | 请求 timeout 后旧 response 晚到，同时新请求已发出 | 旧 response 丢弃，不完成新 request | Rust |
| RC-26 | P0 | deadline vs cleanup/connect | cleanup/connect/send 触发 deadline 同时恢复继续执行 | pending 清理，下一次请求不受影响 | Rust |
| RC-27 | P0 | old handle vs new node | 新 node 已启动，旧 handle 又收到 callback | 旧事件不影响新 node，不污染新连接 | Rust + 移动端 |
| RC-12 | P1 | cleanup vs ICE restart | ICE restart 中触发 cleanup | cleanup 优先，不双恢复 | Rust |
| RC-13 | P1 | signaling reconnect vs cleanup | WS reconnect 与 WebRTC cleanup 重叠 | 旧 transport 不复活 | Rust |
| RC-14 | P1 | 双端同时首发 | 两端同一时间 RPC | 不重复 offer/answer，不死锁 | Rust |
| RC-15 | P1 | 双端同时切网恢复 | 两端同时 `Path(Available)` | 不产生 offer storm | Rust + E2E |
| RC-24 | P1 | lifecycleScope/Swift Task cancel | FFI await 中协程/Task 被取消 | 不泄漏、不锁 result receiver | Android/Swift + Rust |

## L5 移动端端上测试

| Case ID | 平台 | 优先级 | 场景 | 操作 | 期望 |
|---|---|---|---|---|---|
| E2E-A01 | Android | P0 | 冷启动在线/离线 | 在线、飞行模式分别启动 | 在线可 RPC，离线不挂 |
| E2E-A02 | Android | P0 | Wi-Fi -> 蜂窝 | RPC 后关闭 Wi-Fi | 最终可继续或 retry 成功 |
| E2E-A03 | Android | P0 | 蜂窝 -> Wi-Fi | 蜂窝在线后打开 Wi-Fi | 不误 offline |
| E2E-A04 | Android | P0 | 飞行模式开关 | RPC 中开关飞行模式 | offline bounded failure，恢复后成功 |
| E2E-A05 | Android | P0 | 前后台短/长恢复 | 后台 5s/60s 再前台 | 短后台不误 cleanup，长后台可 rebuild |
| E2E-A06 | Android | P0 | Activity/Compose 重建 | 旋转/重建页面 | 不重复 monitor，不重复建连 |
| E2E-A07 | Android | P0 | shutdown 后 callback | disconnect 后切网 | 不挂，不调用旧 node 造成问题 |
| E2E-I01 | iOS | P0 | 冷启动在线/离线 | 在线、飞行模式分别启动 | 在线可 RPC，离线不挂 |
| E2E-I02 | iOS | P0 | Wi-Fi -> 蜂窝 | RPC 后关闭 Wi-Fi | unsatisfied gap 后恢复 |
| E2E-I03 | iOS | P0 | 蜂窝 -> Wi-Fi | 打开 Wi-Fi 切回 | 最终可 RPC |
| E2E-I04 | iOS | P0 | 前后台短/长恢复 | 后台 5s/60s 再前台 | 短后台不误 cleanup，长后台可 rebuild |
| E2E-I05 | iOS | P0 | 多 Scene/ViewModel | 多窗口/页面重复创建 | 不重复 observer/monitor storm |
| E2E-I06 | iOS | P0 | shutdown 后 path callback | stop 后切网 | 不挂，不污染新 node |
| E2E-B01 | 双端 | P0 | 双端同时切网 | 手机和桌面同时断/恢复 | 不 offer storm，最终可 RPC |
| E2E-A08 | Android | P1 | 息屏亮屏 | RPC 后息屏/亮屏 | 不直接 cleanup；回前台后按 lifecycle 恢复 |
| E2E-A09 | Android | P1 | VPN/captive portal | 开关 VPN/连接无互联网 Wi-Fi | 错误可解释，恢复后成功 |
| E2E-A10 | Android | P1 | Doze/锁屏 | 锁屏进入省电后切网 | 解锁/前台后最终恢复 |
| E2E-I07 | iOS | P1 | 锁屏亮屏 | RPC 后锁屏/亮屏 | 不把亮屏误报 foreground recovery |
| E2E-I08 | iOS | P1 | VPN/热点/Low Data Mode | 开关 VPN/热点/低数据 | 不破坏连接，必要时恢复 |
| E2E-B02 | 双端 | P1 | 长时间稳定性 | 30min 多轮切网/前后台/发送 | 连接数/任务数/内存不持续增长 |

## 测试分工

| 范围 | 谁写 | 原因 | 产物 |
|---|---|---|---|
| 事件模型、恢复决策 | Rust | 不依赖 OS，状态机由 Rust 定义 | L0 tests |
| FFI handle 生命周期 | Rust | 属于 Rust binding contract | L1 tests |
| WebRTC/signaling/transport 恢复 | Rust | TestHarness/VNet 可稳定模拟 | L2/L4 tests |
| Android callback 映射 | Android/Kotlin | 真实 callback 顺序由系统决定 | L5 Android + JSONL |
| Android 前后台/息屏亮屏 | Android/Kotlin | Activity/ProcessLifecycle/Doze 由系统决定 | L5 Android |
| Swift path/lifecycle 映射 | Swift | `NWPathMonitor`/Scene/Task 行为由系统决定 | L5 iOS + JSONL |
| iOS 前后台/锁屏亮屏 | Swift | UIApplication/Scene/suspend 由系统决定 | L5 iOS |
| 端上序列回放 | Rust + 移动端 | 移动端给日志，Rust 固化回归 | L3 replay tests |

## 移动端日志格式

| 字段 | 示例 | 用途 |
|---|---|---|
| `case_id` | `E2E-A02` | 对齐测试矩阵 |
| `t_ms` | `123456789` | 排序和对齐 |
| `platform` | `android` / `ios` | 平台 |
| `device` | `Pixel 8` / `iPhone 15` | 设备 |
| `os_version` | `Android 15` / `iOS 18` | 系统 |
| `app_state` | `foreground/background/suspended` | 生命周期 |
| `screen_state` | `on/off/locked/unlocked` | 设备状态 |
| `object_id` | `activity=1 monitor=2 node=3 handle=3` | 排查重复对象 |
| `raw_callback` | `onAvailable` / `NWPath.satisfied` | 平台原始 callback |
| `network_snapshot` | `{sequence:12,availability:"Available",transport:{wifi:true,cellular:false,vpn:false},is_expensive:false,is_constrained:false}` | 新网络快照；`sequence` 和 `availability` 是 Rust 当前决策字段 |
| `lifecycle_event` | `{state:"Foreground",background_duration_ms:45000}` | 新生命周期事件；`background_duration_ms` 是 Rust 当前决策字段 |
| `cleanup_command` | `{reason:"UserLogout"}` | 只清理连接，不自动重连 |
| `reconnect_command` | `{reason:"ManualReconnect"}` | 清理后立即恢复连接 |
| `background_duration_ms` | `45000` | Rust 恢复决策 |
| `actr_state` | `node_created/starting/started/shutdown` | Rust 生命周期 |
| `connection_state` | `signaling_connected/webrtc_ready/recovering/offline` | 连接状态 |
| `user_action` | `call_rpc/disconnect/reconnect/background` | 用户并发动作 |
| `request_id` | `prepare-stream-1` | RPC/DataStream 关联 |
| `result` | `ok/Connection recovering/timeout/error` | 结果 |

## 落地顺序建议

这张表的意思是：先把最容易导致“连接恢复不了、消息发不出去、请求挂死”的问题做成自动化测试，再逐步补充端上专项场景。

| 顺序 | 目标 | Rust 侧工作 | 移动端配合 | 完成标准 |
|---|---|---|---|---|
| 1 | 先稳住新网络事件入口 | 覆盖 L0、L1：事件 action 选择、debounce、start 前/后、shutdown 后、重复 handle | 确认 Android/iOS 当前调用 `handle_network_path_changed`、`handle_app_lifecycle_changed`、`cleanup_connections`、`force_reconnect` 的时机 | Rust 事件入口不挂死、不重复建 handle，新 API 行为明确 |
| 2 | 覆盖核心连接恢复 | 覆盖 L2 P0、RC P0：断网、恢复、切网、cleanup/reconnect、并发发送 | 提供最小端上事件序列：冷启动在线/离线、Wi-Fi/蜂窝切换、飞行模式，必须包含 `sequence` 和 `availability` | 连接最终能恢复或明确失败，恢复后 RPC 能成功 |
| 3 | 把真实移动端事件变成 replay | 固化 L3 P0 replay，用 JSONL 回放 Android/iOS snapshot/lifecycle/command 顺序 | Android 输出 A01-A07 JSONL；iOS 输出 I01-I06 JSONL | Rust 能稳定回放真实事件序列，重复/乱序/晚到不导致错误恢复 |
| 4 | 覆盖发送中网络波动 | 覆盖 L4：RPC/DataStream 发送中断网、切网、长后台、event storm | 端上补充大包、DataStream、后台恢复过程日志 | 请求不永久挂死；恢复后可继续发送；失败结果可解释 |
| 5 | 补端上复杂网络 | 补充 P1 测试：VPN、captive portal、弱网、低数据模式 | Android 测 VPN/captive portal/Doze；iOS 测 VPN/热点/Low Data Mode/suspend | 能定位 Rust 能模拟和必须端上验证的边界 |
| 6 | 补低频设备状态 | 只做日志和专项验证，不默认映射成网络事件 | Android/iOS 测息屏、亮屏、锁屏、进程被杀重启 | 不误触发网络恢复；不会污染新 node/handle |
