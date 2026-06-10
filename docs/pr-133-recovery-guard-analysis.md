# PR #133 CI 失败分析：Recovery Guard 阻塞 Response 回包

## 问题现象

`test_mobile_inflight_large_message_interruptions` 测试在 CI 环境下失败：

```
server_data_echo failed to send echo response for mobile_answerer_inflight_short_background:
unavailable: Connection recovering: peer=ActrId { serial_number: 200 },
session_id=37, reason=ice restart started, elapsed_ms=39, timeout_ms=6000

short background request should complete: Unavailable("Request timeout: 30000ms")
```

本地无法稳定复现（13 次全部通过），CI 上必现。原因是时序敏感的竞态问题：本地 ICE restart ~1-2ms 完成，CI 上需要 ~200ms，echo 回包恰好撞上 guard 窗口。

## 失败时序还原

以 `inflight_short_background_survives_foreground_restore` (mobile_answerer) 为例：

```
T0  05:33:06.305  mobile 发送 204870 bytes 大消息 → server 收到
T1  05:33:06.778  测试注入 Restore 网络事件 → answerer 发 IceRestartRequest
T2  05:33:06.808  server (offerer) 收到 IceRestartRequest → 发起 ICE restart
                  → server 端 PeerGate 设 recovery guard (reason="ice restart started")
T3  05:33:06.848  server echo responder 回包 → guard 阻止，返回 Unavailable  ← 失败点
T4  05:33:07.014  ICE restart 完成，guard 清除（已太晚，echo 已丢弃）
T5  05:33:36.xxx  mobile 端 30s 请求超时
```

**核心矛盾**：消息已成功送达 server，但 server 回包时恰好撞上刚设的 recovery guard，echo 被直接拒绝且不重试。

## 为什么只影响 mobile_answerer

| 角色 | Restore 事件 → ICE restart 流程 | server 端设 guard？ | echo 受阻？ |
|------|-------------------------------|--------------------|-----------|
| mobile_offerer | mobile 自己直接发起 ICE restart | ❌ server 不会收到 IceRestartRequest | ❌ 不受阻 |
| mobile_answerer | mobile 发 IceRestartRequest → server 发起 | ✅ server 设 guard | ✅ **受阻** |

mobile_answerer 不是 offerer，无法自己发起 ICE restart，必须通知 server (offerer) 来发起。server 发起 ICE restart 时自身 PeerGate 设了 recovery guard，阻止了对 mobile_answerer 的回包。

## Recovery Guard 的必要性

Guard 不能简单去掉。其核心目的是**保护 SCTP 关联不被破坏**：

- ICE restart 期间如果网络已断，SCTP 发送失败会触发 association 重置
- 重置导致整条 DataChannel close，不可恢复
- Guard 阻止 send 就是为了避免 SCTP 在不确定的网络状态下发送

**Connected 状态 ≠ SCTP 安全**：Connected → Disconnected 有延迟，这个窗口内 SCTP 发送可能触发 DataChannel close。所以 ICE restart 一开始就必须设 guard。

## 根因

Guard 保护 SCTP 是对的，问题在于 **guard 期间消息直接丢弃不重试**：

```rust
// preflight_send — 所有 send 路径的公共检查
if !cleared_locally {
    return Err(Self::recovering_error(target, &status));  // ← 直接丢弃
}
```

ICE restart 通常 ~200ms 完成，但 echo 回包恰好撞在这 200ms 窗口内，一次失败就永久丢弃，没有重试机制。

## 修复方案

**在 response 发送点遇到 recovering 错误时，做有界等待重试，而不是直接丢弃。**

### 通用重试辅助函数

```rust
/// 发送 response 时遇到 recovering 错误，等待 recovery 解除后重试。
/// 仅针对 "ice/network recovery started" 场景，该场景 ICE restart 通常几百毫秒完成。
/// 兜底超时防止无限阻塞。
async fn send_response_with_recovery_retry(
    gate: &PeerGate,
    target: &ActrId,
    envelope: RpcEnvelope,
    max_wait: Duration,
) -> ActorResult<()> {
    match gate.send_message(target, envelope.clone()).await {
        Ok(()) => Ok(()),
        Err(e) if is_recovering_error(&e) => {
            let deadline = tokio::time::Instant::now() + max_wait;
            loop {
                if tokio::time::Instant::now() >= deadline {
                    return Err(e);
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
                match gate.send_message(target, envelope.clone()).await {
                    Ok(()) => return Ok(()),
                    Err(ref err) if is_recovering_error(err) => continue,
                    Err(err) => return Err(err),
                }
            }
        }
        Err(e) => Err(e),
    }
}

fn is_recovering_error(err: &ActrError) -> bool {
    matches!(err, ActrError::Unavailable(msg) if msg.contains("Connection recovering"))
}
```

### 需要修改的位置

| 文件 | 位置 | 说明 |
|------|------|------|
| 测试 echo responder | `webrtc_large_mobile_recovery.rs` | 当前 CI 失败的直接原因 |
| `node.rs:1794` | Guest → Shell 正常响应 | `response_tx.send_message(...)` |
| `node.rs:1837` | Guest → Shell 错误响应 | `response_tx.send_message(...)` |
| `context.rs:494` | RPC handler 回复 | `gate.send_message_with_type(...)` |

### 不改的位置

| 路径 | 原因 |
|------|------|
| `send_request` | 调用方有自己的 `timeout_ms`，框架不应吞掉超时控制权 |
| `send_message` 主动发消息 | 非响应场景，FailFast 行为不变 |
| `preflight_send` | 框架 guard 机制不变，保持 SCTP 保护 |

### 兜底超时

`max_wait = 2s`，理由：
- ICE restart 通常 ~200ms 完成
- 2s 已是 10 倍余量
- 超过 2s 还没恢复，说明真有问题，应直接报错
- 不会阻塞不同 peer 的消息（guard 是 per-peer 的，只等当前 target 恢复）

### 方案优势

- ✅ SCTP 保护不变，guard 照常设
- ✅ response 不再因短暂 guard 窗口丢弃
- ✅ 有兜底超时，不无限阻塞
- ✅ 不改 `preflight_send` / `send_message` 签名，框架行为不变
- ✅ 不影响 `send_request` 调用方的超时控制
- ✅ 不同 peer 的消息互不阻塞（per-peer 等待）

## 复现测试

`core/hyper/tests/recovery_guard_response_retry.rs` 包含两个测试：

### test 1: `test_response_blocked_by_ice_restart_recovery_guard`

验证 guard 阻塞行为，**当前应通过** ✅

1. 建立 mobile_answerer + server 连接
2. 通过 `send_event(IceRestartStarted)` 直接注入 recovery guard（无需依赖时序竞态）
3. 验证 `send_message` 被 guard 阻止，返回 `Unavailable("Connection recovering")`
4. 注入 `IceRestartCompleted` 清除 guard
5. 验证恢复后 `send_message` 重试成功
6. 验证端到端请求-响应恢复正常

### test 2: `test_response_retries_through_ice_restart_recovery_guard`

验证端到端请求-响应在 guard 窗口内的行为，**当前应失败** ❌（修复后应通过）

1. 建立 mobile_answerer + server 连接，启动 echo responder
2. 验证 baseline 请求-响应正常
3. 注入 `IceRestartStarted` 设置 guard
4. 发送请求 → server echo responder 尝试回包 → **被 guard 阻止**
5. 500ms 后注入 `IceRestartCompleted` 清除 guard
6. **期望**：echo responder 重试后回包成功，请求完成
7. **实际**（当前未修复）：echo 被丢弃，请求超时

本地运行结果：

```
test test_response_blocked_by_ice_restart_recovery_guard ... ok      ✅
test test_response_retries_through_ice_restart_recovery_guard ... FAILED  ❌

错误:
server_echo_retry: Failed to send response for request_during_guard:
  unavailable: Connection recovering: ..., reason=ice/network recovery started, elapsed_ms=105

request should succeed after recovery guard clears —
  response was likely dropped by guard without retry: Err(Unavailable("Request timeout: 5000ms"))
```

**关键**：通过 `send_event` 注入事件来可靠控制 guard 时序，不需要依赖 CI 环境的调度压力来触发竞态。
