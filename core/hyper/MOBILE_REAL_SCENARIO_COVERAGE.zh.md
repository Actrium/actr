# Mobile Real Scenario Test Coverage

基线：`origin/main` 最新提交 `ccc20e6 fix(hyper): retry response sends blocked by ICE restart recovery guard (#139)`

工作分支：`test/mobile-real-scenario-coverage`

日期：2026-06-11

## 最终目标

这些测试的目标不是单纯提高用例数量，而是把移动端真实使用场景固化成回归保护：

- 服务端正在向移动端发送 RPC/DataStream 时，移动端切后台、长时间息屏、网络断开、app 被杀、进程重启、恢复在线，都不能导致发送路径永久挂住。
- 任一方向的请求超时或路由失败后，`PeerGate` 的 pending request 必须清理干净，旧 `DestTransport` 不能被无限复用。
- 移动端作为 WebRTC offerer 和 answerer 两种角色时，恢复语义都一致可验证。
- mobile -> server 和 server -> mobile 两个方向都必须覆盖，因为 server -> mobile 更容易暴露 stale DataChannel / stale transport 问题。

## 本次提交

| 提交 | 类型 | 内容 |
| --- | --- | --- |
| `375acb0` | fix + test | request 已发送但等待 response 超时后，后台关闭 stale `DestTransport`；扩展大消息移动中断测试到双向、双角色。 |
| `0ec26bb` | test | 半开 WebSocket/WebRTC 恢复测试覆盖 mobile offerer/answerer、mobile -> server/server -> mobile。 |
| `d5d4588` | test | app terminating cleanup 后，server -> mobile 在移动端 killed 期间有界失败；app 在线重启后双向恢复。 |
| `1c021e1` | test | 真实 mobile network event storm 测试改为双向，并覆盖两个方向的 DataStream bounded send。 |
| `954f700` | test | app killed 后离线重启时，server -> mobile 有界失败；后续 online restore 后双向恢复。 |

## 覆盖矩阵

| 测试场景 | 覆盖用例 | 角色/方向 | 期望结果 |
| --- | --- | --- | --- |
| 短时网络中断后恢复 | `test_mobile_inflight_large_message_interruptions` | offerer/answerer；双向 | 原始请求可恢复或重试成功；pending 清零。 |
| 网络类型切换 WiFi/Cellular | `test_mobile_inflight_large_message_interruptions` | offerer/answerer；双向 | `Restore` 后双向大消息完整回包，payload hash 一致。 |
| 长时间离线/长时间息屏后恢复 | `test_mobile_inflight_large_message_interruptions` | offerer/answerer；双向 | in-flight 请求有界结束；`ForceReconnect(StaleConnectionSuspected)` 后重试成功。 |
| 短后台返回前台 | `test_mobile_inflight_large_message_interruptions` | offerer/answerer；双向 | `Restore` 后请求恢复，不泄漏 pending。 |
| 长后台返回前台 | `test_mobile_inflight_large_message_interruptions` | offerer/answerer；双向 | `ForceReconnect(LongBackground)` 后重试成功。 |
| DataStream 发送期间移动端中断 | `test_mobile_inflight_large_message_interruptions` | offerer/answerer；双向 | 发送不挂死；恢复后 RPC 验证链路可用。 |
| 15s half-open 恢复窗口 | `test_mobile_half_open_15s_semantics_recovers_with_ice_restart` | offerer/answerer；双向 | 通过 ICE restart 恢复，保留原 WebRTC session。 |
| 65s half-open/stale 窗口 | `test_mobile_half_open_65s_semantics_rebuilds_webrtc` | offerer/answerer；双向 | offerer 走 ICE restart；answerer 关闭 stale session 并重建；双向请求成功。 |
| app killed 后在线重启 | `test_mobile_app_kill_cleanup_then_restart_online_recovers_bidirectional_server_send` | offerer/answerer；双向，重点 server -> mobile | killed 期间 server -> mobile 有界失败且 pending 清零；online restore 后双向请求成功。 |
| app killed 后离线重启 | `test_mobile_app_kill_restart_offline_bounds_server_send_until_online_restore` | offerer/answerer；双向，重点 server -> mobile | offline 阶段不重连，server -> mobile 有界失败；网络恢复后双向请求成功。 |
| 复杂网络事件 storm + 真实 outage | `test_complex_mobile_event_storms_with_real_network_outage` | offerer/answerer；双向 | offline/online/duplicate event 批处理结果正确，最终双向请求成功。 |
| NetworkEventHandle 并发 storm + RPC/DataStream | `test_mobile_network_event_handle_storm_then_call_and_data_stream_are_bounded` | offerer/answerer；双向 RPC + 双向 DataStream | 所有 event result 成功；RPC/DataStream 不挂死；两端 pending 清零。 |
| Android/iOS 文档化网络事件 | `test_android_documented_network_scenarios` / `test_ios_documented_network_scenarios` | 动作归约 | documented SDK event sequence 归约到预期 `Noop/Offline/Probe/Restore/ForceReconnect/CleanupOnly`。 |
| 真实日志形状 JSONL 回放 | `test_mobile_jsonl_replay_maps_real_log_shape_to_recovery_actions` | 动作归约 | Android/iOS/cleanup log shape 归约结果符合预期。 |

## 问题时间线和修复方案

### 1. server -> mobile stale DestTransport 复用

时间线：

1. 移动端经历长时间后台、长时间离线或 app 重启，移动端本地 WebRTC/DataChannel 已经关闭或重建。
2. 服务端仍缓存旧 `DestTransport`，向移动端发送请求时，底层 stale DataChannel 的 send 可能返回 `Ok(())`。
3. 因为请求实际到不了移动端，`PeerGate` 只会等 response 超时。
4. 旧 pending request 被移除，但旧 `DestTransport` 没有被关闭，下一次重试仍复用同一个 stale transport。
5. 结果是 server -> mobile 在移动端恢复后仍可能持续无响应。

修复：

- 在 `core/hyper/src/outbound/peer_gate.rs` 的 request send path 中，如果 payload 已发送但等待 response 超时，则移除 pending request 后异步调用 `transport_manager.close_transport(&stale_dest)`。
- 关闭 stale dest 后，后续 retry 会重新走 transport build，避免无限复用旧 DataChannel。

验证：

- `cargo test -p actr-hyper --test webrtc_large_mobile_recovery --features test-utils`
- `cargo test -p actr-hyper --test retry_core_mechanics --features test-utils`
- `cargo test -p actr-hyper --test retry_behavior --features test-utils`
- `cargo test -p actr-hyper --test retry_dedup --features test-utils`

### 2. 双向测试中的 receive loop 竞争

时间线：

1. 早期 harness 的 `connect(from, to)` 会在目标 peer 启动 echo responder，在源 peer 启动 response receiver。
2. 如果为了测双向再对反方向调用 `connect(to, from)`，同一个 coordinator 上会出现多个 receive loop。
3. 多个 loop 竞争 `receive_message()`，可能导致 request 或 response 被错误 loop 消费，测试表现为偶发 timeout。

修复：

- 在双向移动场景测试里使用单一 `spawn_rpc_router`。
- 每个 peer 只启动一个 receive loop，同时处理：
  - `route_key == "response"`：转给 `gate.handle_response()`。
  - 普通 request：echo payload 回 response。

验证：

- `cargo test -p actr-hyper --test mobile_full_disconnect_recovery --features test-utils`
- `cargo test -p actr-hyper --test mobile_network_event_scenarios --features test-utils`

### 3. 长时间后台/离线的恢复语义

时间线：

1. 短后台或短网络切换可以用 `Restore`/ICE restart 恢复。
2. 长后台、长时间息屏、长时间离线后，移动端和服务端对旧 DataChannel 是否仍可用的认知可能不一致。
3. 如果仍按普通 `Restore` 验证，测试会混淆“短恢复窗口”和“stale connection suspected”两种语义。

修复/约定：

- 短时中断、网络类型切换：继续断言 `Restore`。
- 长后台、长时间离线、stale suspected：断言显式 `ForceReconnect`，先清理再恢复。
- 测试中使用 `ReconnectReason::LongBackground` 或 `ReconnectReason::StaleConnectionSuspected` 固化这个边界。

验证：

- `test_mobile_inflight_large_message_interruptions`
- `test_mobile_half_open_15s_semantics_recovers_with_ice_restart`
- `test_mobile_half_open_65s_semantics_rebuilds_webrtc`

### 4. app killed 期间 server -> mobile 的错误类型

时间线：

1. 新增 app killed 在线重启测试后，mobile answerer 场景中 server -> mobile 在 killed 阶段返回 `No route: all transport candidates exhausted for RpcReliable`。
2. 这不是 hang，也不是 pending 泄漏；它表示移动端 cleanup 后没有可用 transport，是合理的有界失败。
3. 原始断言只接受 timeout/closed/recovering 类错误，导致测试失败。

修复：

- 将 `not found`、`no route`、`all transport candidates exhausted` 纳入 bounded send error 白名单。
- 同时保留 pending 清零断言，避免把真正泄漏误判为成功。

验证：

- `test_mobile_app_kill_cleanup_then_restart_online_recovers_bidirectional_server_send`
- `test_mobile_app_kill_restart_offline_bounds_server_send_until_online_restore`

## 最终验证

本轮最终执行并通过：

```bash
cargo fmt
cargo test -p actr-hyper --test webrtc_large_mobile_recovery --features test-utils
cargo test -p actr-hyper --test mobile_full_disconnect_recovery --features test-utils
cargo test -p actr-hyper --test mobile_network_event_scenarios --features test-utils
```

结果：

- `webrtc_large_mobile_recovery`: 2 passed
- `mobile_full_disconnect_recovery`: 2 passed
- `mobile_network_event_scenarios`: 8 passed
