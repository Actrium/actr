# Echo Example - 手动测试计划

> 适用于 `actr-web/examples/echo` 项目（含 server + client 两个浏览器端应用）  
> 测试前确保 Actrix Signaling Server 已启动并可访问

---

## 环境准备

| # | 检查项 | 操作 | 预期 |
|---|--------|------|------|
| 0-1 | Actrix 运行 | 确认 signaling server 在 `wss://<host>:8081/signaling/ws` 可达 | WebSocket 握手成功 |
| 0-2 | Server 页面 | 浏览器打开 Echo Server 页面 | 状态显示"✅ 服务器运行中"，日志显示 echo_service_registered |
| 0-3 | Client 页面 | 浏览器打开 Echo Client 页面 | 状态显示"✅ 已连接"，5 秒后自动发送一条测试消息并收到回复 |
| 0-4 | DevTools | 打开 chrome://serviceworker-internals 或 Application → Service Workers | 确认 server 和 client 的 SW 均为 `activated and is running` |

---

## 一、基本功能测试

| # | 测试项 | 操作步骤 | 预期结果 | 通过 |
|---|--------|---------|---------|------|
| 1-1 | 手动发送消息 | 在 client 输入框输入文字，点击发送 | 日志显示 📤 发送 + 📥 回复，server 日志显示 📨 收到请求 + ✅ 响应 | ☐ |
| 1-2 | 空消息发送 | 清空输入框，直接点击发送 | 自动填充默认 `Hello! (时间戳)` 并正常收到回复 | ☐ |
| 1-3 | 快速连续发送 | 在 client 1 秒内连续点击发送按钮 5+ 次 | 所有请求均收到回复，无消息丢失或超时 | ☐ |
| 1-4 | 大消息发送 | 在 client 输入超长字符串（>10KB）发送 | 正常收到 Echo 回复，内容完整 | ☐ |
| 1-5 | 特殊字符 | 发送包含 emoji、中文、HTML 标签 `<script>` 等 | 正确回显，无 XSS 或渲染异常 | ☐ |
| 1-6 | Enter 键发送 | 在输入框按 Enter 键 | 等同点击发送按钮 | ☐ |

---

## 二、页面刷新测试

| # | 测试项 | 操作步骤 | 预期结果 | 通过 |
|---|--------|---------|---------|------|
| 2-1 | Client 立即刷新 | 正常通信后，F5 刷新 client 页面 | 重新连接成功，5s 后自动测试消息正常收到回复 | ☐ |
| 2-2 | Client 连续刷新 | 快速连续 F5 刷新 client 3-5 次 | 最终页面正常连接，SW 中旧 client 被清理（检查 console 日志 `Cleaning up stale client`） | ☐ |
| 2-3 | Server 立即刷新 | 正常通信后，F5 刷新 server 页面 | Server 重新注册成功，client 再次发送可收到回复（可能需重新发现目标） | ☐ |
| 2-4 | Server + Client 同时刷新 | 同时按 F5 刷新两端 | 双方均重新初始化，等待 5s 后 client 自动发送成功 | ☐ |
| 2-5 | 硬刷新（Ctrl+Shift+R） | 对 client 执行硬刷新（清缓存） | SW 重新 install → activate → claim，client 正常初始化 | ☐ |
| 2-6 | 硬刷新 server | 对 server 执行硬刷新 | SW 重新加载 WASM，EchoService 重新注册，可正常被发现 | ☐ |

---

## 三、Service Worker 生命周期测试

| # | 测试项 | 操作步骤 | 预期结果 | 通过 |
|---|--------|---------|---------|------|
| 3-1 | SW 空闲终止 | 双端正常后不操作，等待 30-60s（让浏览器杀死空闲 SW） | PING/PONG keep-alive（20s 间隔）应阻止 SW 被终止；确认 SW 仍 running | ☐ |
| 3-2 | 手动停止 SW | DevTools → Application → Service Workers → 点击 Stop | SW 立即终止。再次操作触发 SW 重启，但内存状态（clientPorts、wasmReady）丢失 | ☐ |
| 3-3 | 停止 SW 后发消息 | 停止 client SW，然后在 client 点击发送 | 发送失败（SW Bridge 无法送达）。页面日志显示错误，不崩溃 | ☐ |
| 3-4 | SW 更新 | 修改 actor.sw.js 内容，重新部署后刷新页面 | SW 进入 waiting → 页面刷新后激活新 SW，正常工作 | ☐ |
| 3-5 | SW Unregister | DevTools → Application → Service Workers → Unregister | SW 完全消失。刷新页面后应重新 register 新 SW 实例 | ☐ |
| 3-6 | 禁用 PING keep-alive 模拟 | 临时注释 `setInterval PING` 代码，等待 >30s | 浏览器可能终止 SW；heartbeat 失败后应触发 signaling 重连 | ☐ |

---

## 四、网络断开与恢复测试

| # | 测试项 | 操作步骤 | 预期结果 | 通过 |
|---|--------|---------|---------|------|
| 4-1 | Client 短暂断网 | 正常通信后，DevTools → Network → Offline 勾选，保持 5s，再取消 | Signaling WebSocket 断开 → heartbeat 失败 → 重连（最长 3×25s=75s 后触发）→ 重连后可正常发送 | ☐ |
| 4-2 | Client 长时间断网 | 断网 2 分钟再恢复 | heartbeat 连续失败 ≥3 次 → `reconnect_signaling()` 触发 → 指数退避重连（最多 10 次，最长至 60s 间隔）→ 网络恢复后重连成功 | ☐ |
| 4-3 | Server 短暂断网 | 对 server 页面断网 5s 再恢复 | Server signaling 重连成功，actor 重新注册，client 可重新发现 | ☐ |
| 4-4 | 双端同时断网 | 两端同时 Offline → 等待 10s → 同时恢复 | 双方各自重连 signaling，重新注册后 client 重新发现 server 并建立新 WebRTC 连接 | ☐ |
| 4-5 | 断网时发消息 | 断网状态下在 client 点击发送 | 发送失败，显示错误信息（如 RPC 超时 30s 或 WebSocket closed），不崩溃 | ☐ |
| 4-6 | 网络切换 (WiFi→有线) | 在真实设备上切换网络接口 | IP 变化导致所有连接重建，signaling 重连 + 新 WebRTC 连接建立 | ☐ |
| 4-7 | 弱网模拟 | DevTools → Network → Slow 3G | 消息可送达但延迟高，UI 不卡死。WebRTC STREAM_LATENCY_FIRST 通道可能丢包 | ☐ |
| 4-8 | Signaling 服务器重启 | 正常通信后重启 Actrix 进程 | 双端 WebSocket 断开 → 心跳失败 → 自动重连 → 重新注册 → client 重新发现 server | ☐ |

---

## 五、WebRTC 连接测试

| # | 测试项 | 操作步骤 | 预期结果 | 通过 |
|---|--------|---------|---------|------|
| 5-1 | DataChannel 4 通道 | 正常连接后，检查 `chrome://webrtc-internals` | 应看到 4 条 DataChannel：`RPC_RELIABLE`(id=0)、`RPC_SIGNAL`(id=1)、`STREAM_RELIABLE`(id=2)、`STREAM_LATENCY_FIRST`(id=3) | ☐ |
| 5-2 | ICE restart | 在 `chrome://webrtc-internals` 中观察到 ICE disconnected（或用防火墙规则短暂阻断 UDP） | ICE restart 自动触发（最多 5 次，5s-60s 退避），连接恢复后消息正常 | ☐ |
| 5-3 | ICE restart 失败 | 长期阻断 UDP（>5 分钟，超过 5 次 retry） | ICE restart 超限 → peer connection 被关闭 → 需要重新发现和建立连接 | ☐ |
| 5-4 | Peer 状态变化日志 | 观察 server 端日志 | 应显示 `connection_state_changed` 相关日志（new → connecting → connected） | ☐ |
| 5-5 | TURN fallback | 在 NAT 严格环境下测试（无法 STUN 直连） | 确认是否配置了 TURN server，否则连接将失败 | ☐ |

---

## 六、多标签页 / 多客户端测试

| # | 测试项 | 操作步骤 | 预期结果 | 通过 |
|---|--------|---------|---------|------|
| 6-1 | 两个 client 标签页 | 同一浏览器打开两个 client 标签页 | 每个标签页生成独立 clientId，各自独立注册和发现 server | ☐ |
| 6-2 | 多 client 同时发送 | 两个 client 同时向 server 发送消息 | Server 同时处理两个请求，各自收到正确回复 | ☐ |
| 6-3 | 关闭一个 client | 打开两个 client，关闭其中一个 | 剩余 client 不受影响，继续正常发送。关闭的 client 触发 `beforeunload → client.close()` | ☐ |
| 6-4 | 刷新一个 client | 两个 client 中刷新其中一个 | 被刷新的 client 重建，另一个 client 不受影响 | ☐ |
| 6-5 | 多 server 实例 | 打开两个 server 标签页 | 各自注册同一 `acme+EchoService` type，client 发现时按 `MaximumPowerReserve` ranking 选择 | ☐ |
| 6-6 | 共享 SW 隔离性 | 检查同源 client 标签页是否共享同一 SW 实例 | 共享同一个 SW，但每个 tab 有独立 `clientId`、独立 `SwRuntime`、独立 signaling WebSocket | ☐ |

---

	测试项	操作步骤	预期结果	通过
1-1	手动发送消息	在 client 输入框输入文字，点击发送	日志显示 📤 发送 + 📥 回复，server 日志显示 📨 收到请求 + ✅ 响应	
1-2	空消息发送	清空输入框，直接点击发送	自动填充默认 Hello! (时间戳) 并正常收到回复	☐
1-3	快速连续发送	在 client 1 秒内连续点击发送按钮 5+ 次	所有请求均收到回复，无消息丢失或超时	☐
1-4	大消息发送	在 client 输入超长字符串（>10KB）发送	正常收到 Echo 回复，内容完整	☐
1-5	特殊字符	发送包含 emoji、中文、HTML 标签 <script> 等	正确回显，无 XSS 或渲染异常	☐
1-6	Enter 键发送	在输入框按 Enter 键	等同点击发送按钮	☐



## 七、页面关闭与 beforeunload 测试

| # | 测试项 | 操作步骤 | 预期结果 | 通过 |
|---|--------|---------|---------|------|
| 7-1 | 正常关闭 client 标签页 | 点击标签页 ✕ 关闭 | `beforeunload` → `client.close()` 触发 → SW Bridge 关闭 → signaling WebSocket 关闭 | ☐ |
| 7-2 | 正常关闭 server 标签页 | 点击标签页 ✕ 关闭 | `beforeunload` → `server.stop()` → 清理资源 → signaling server 移除 actor 注册 | ☐ |
| 7-3 | 强杀浏览器 | Task Manager 杀死浏览器进程 | `beforeunload` 不触发。Signaling server 通过 heartbeat 超期（5 分钟）清理 actor 注册 | ☐ |
| 7-4 | 关闭 server 后 client 发消息 | 关闭 server 后，client 点击发送 | 发送失败（RPC 超时或 "No candidates"），显示错误信息，不崩溃 | ☐ |
| 7-5 | 关闭 server 后重开 server | 关闭 server → 等 5s → 重新打开 server | 新 server 注册成功。Client 旧路由失效，需重新 discover_target → 新 RPC 成功 | ☐ |

---

## 八、Signaling 连接边界测试

| # | 测试项 | 操作步骤 | 预期结果 | 通过 |
|---|--------|---------|---------|------|
| 8-1 | Signaling URL 不可达 | 修改 `RUNTIME_CONFIG.signaling_url` 为错误地址后启动 | 连接失败，指数退避重试 10 次（1s→2s→4s→...→60s），总共约 2 分钟，最终报错 | ☐ |
| 8-2 | Signaling 连接超时 | 使用防火墙 drop(非 reject) signaling server 端口 | WebSocket open 15s 超时 → 进入重试流程 | ☐ |
| 8-3 | Signaling WS 被 server 关闭 | Signaling server 主动关闭 WS 连接 | `onclose` 触发 → inbound channel 关闭 → relay loop 退出 → heartbeat 失败 → 重连 | ☐ |
| 8-4 | Realm 不匹配 | Client 和 server 配置不同的 `realm_id` | Client 的 `route_candidates` 无法找到跨 realm 的 server → "No candidates" | ☐ |
| 8-5 | ACL 不匹配 | Server `acl_allow_types` 不包含 client type | Client 的 `route_candidates` 被 ACL 拒绝 → 无法发现 server | ☐ |

---

## 九、Service Worker 空闲恢复测试（关键场景）

| # | 测试项 | 操作步骤 | 预期结果 | 通过 |
|---|--------|---------|---------|------|
| 9-1 | 短时空闲后操作 | 正常连接后，不操作 30s，然后发消息 | PING keep-alive 保持 SW 存活，直接发送成功 | ☐ |
| 9-2 | 中等空闲后操作 | 正常连接后，不操作 2 分钟，然后发消息 | SW 可能被终止 → WASM 状态丢失 → 需要看 signaling 重连是否工作。可能需重新建立 WebRTC 连接 | ☐ |
| 9-3 | 长时间空闲后操作 | 正常连接后不操作 5+ 分钟 | Signaling server heartbeat 过期（5 分钟），actor 注册被清理。发送消息失败 → 需要完全重新初始化 | ☐ |
| 9-4 | 空闲后刷新 client | 不操作 1 分钟后按 F5 | 刷新触发新 SW 注册流程，清理旧 stale client → 新 client 正常工作 | ☐ |
| 9-5 | 空闲后刷新 server | 不操作 1 分钟后刷新 server | Server 重新注册 actor → client 可重新发现 | ☐ |
| 9-6 | 笔记本合盖再打开 | 合上笔记本盖子 30s → 打开 | 系统休眠会关闭所有网络连接。恢复后所有连接需重建。检查 signaling 重连 + WebRTC ICE restart | ☐ |

---

## 十、浏览器兼容性与特殊行为

| # | 测试项 | 操作步骤 | 预期结果 | 通过 |
|---|--------|---------|---------|------|
| 10-1 | Chrome | 在 Chrome 中运行完整流程 | 全部功能正常 | ☐ |
| 10-2 | Firefox | 在 Firefox 中运行完整流程 | SW + WASM + WebRTC 均支持，功能正常 | ☐ |
| 10-3 | Safari | 在 Safari 中运行完整流程 | 注意：Safari SW 实现略有不同，可能存在差异 | ☐ |
| 10-4 | Edge | 在 Edge 中运行完整流程 | 与 Chrome 表现一致（Chromium 内核） | ☐ |
| 10-5 | 隐私/无痕模式 | 在无痕窗口中打开 | SW 在部分浏览器的无痕模式下可能被禁用或行为不同 | ☐ |
| 10-6 | 跨浏览器通信 | Server 在 Chrome，Client 在 Firefox | WebRTC 互通性测试，应正常工作 | ☐ |
| 10-7 | 浏览器标签页置于后台 | 将 client 标签页切到后台 1 分钟 | 浏览器可能限制后台 tab 的 timer/网络；PING keep-alive 可能延迟 | ☐ |
| 10-8 | 移动端浏览器 | 在 iOS Safari / Android Chrome 上测试 | SW + WASM 支持情况，移动端可能更积极终止后台 SW | ☐ |

---

## 十一、WASM 加载与初始化测试

| # | 测试项 | 操作步骤 | 预期结果 | 通过 |
|---|--------|---------|---------|------|
| 11-1 | WASM 文件缺失 | 删除/重命名 `actr_runtime_sw_bg.wasm` | SW 日志显示 `wasm_init_failed`，页面显示连接失败错误 | ☐ |
| 11-2 | JS glue 文件缺失 | 删除/重命名 `actr_runtime_sw.js` | SW 日志显示 fetch 失败，页面显示错误 | ☐ |
| 11-3 | WASM 文件损坏 | 替换 wasm 文件为空文件 | `wasm_bindgen()` 失败，页面显示初始化错误 | ☐ |
| 11-4 | WASM MIME type 不正确 | 服务器返回 wasm 文件但 MIME 不是 `application/wasm` | 部分浏览器可能拒绝编译，检查 `fetch` 使用 `no-store` 缓存策略 | ☐ |
| 11-5 | 慢网络加载 WASM | Slow 3G 下加载 | WASM 文件较大时加载时间长，页面应显示"连接中..."状态 | ☐ |

---

## 十二、并发与压力测试

| # | 测试项 | 操作步骤 | 预期结果 | 通过 |
|---|--------|---------|---------|------|
| 12-1 | 快速连续 Echo | 使用脚本或手动连续发送 100 条消息 | 所有消息都收到回复，无丢失。Server 统计数字正确 | ☐ |
| 12-2 | 多 client 并发 | 开 5 个 client 标签页同时发送 | Server 并发处理，各 client 收到各自的回复 | ☐ |
| 12-3 | 日志溢出 | 发送 200+ 条消息，观察 UI 日志区域 | 日志条目限制在 200 条（代码中有 `while > 200 removeChild`） | ☐ |
| 12-4 | 内存泄漏观察 | 持续运行 10 分钟，周期性发消息 | DevTools → Memory → 观察 heap 无持续增长 | ☐ |

---

## 十三、错误恢复与降级测试

| # | 测试项 | 操作步骤 | 预期结果 | 通过 |
|---|--------|---------|---------|------|
| 13-1 | Server 崩溃重建 | Server 发送过程中关闭 → 5s 后重新打开 | Client 旧 peer 连接无效 → RPC 超时或失败。新 server 启动后 client 需重新 discover → 新消息成功 | ☐ |
| 13-2 | RPC 超时 | Server 存在但 WASM 中 EchoService 处理卡死 | Client RPC 30s 超时后报错 | ☐ |
| 13-3 | DataChannel 断开 | `chrome://webrtc-internals` 手动关闭 datachannel | SW 端应检测到 `datachannel_close`，后续消息发送失败 | ☐ |
| 13-4 | P2P 连接重试 | 首次 WebRTC offer/answer 交换失败（比如 TURN 不可用） | 自动重试 3 次（3s→6s→12s 退避，max 15s） | ☐ |
| 13-5 | Signaling 重连后恢复 | 断网 → heartbeat 失败 3 次 → 恢复网络 | 自动重连 signaling → 重新注册 → 清空旧 peer 状态 → client 重新 discover → 新 RPC 成功 | ☐ |
| 13-6 | Signaling 重连 10 次都失败 | 断网不恢复，观察 console 日志 | 3 次 heartbeat 失败 → reconnect 用指数退避尝试 10 次 → 全部失败后 heartbeat loop 终止（致命），日志显示错误 | ☐ |

---

## 十四、跨端测试：Web Client + Rust Server

> **场景说明**：Client 运行在浏览器（actr-web），Server 运行为原生 Rust 进程（actr-examples/shell-actr-echo/server）。  
> 两端通过同一 Actrix Signaling Server 发现彼此，经由 WebRTC DataChannel 通信。  
>  
> **前置条件**：  
> - 确保 Web Client 与 Rust Server 使用**相同的 `realm_id`** 和**同一 signaling server URL**  
>   - Rust Server 默认 `realm_id=6`（[actr.toml](../../actr-examples/shell-actr-echo/server/actr.toml)），Web Client 默认 `realm_id=2368266035`（actor.sw.js）  
>   - **测试前需修改其中一端使两者一致**  
> - Rust Server 配置了 `force_relay = true`，若使用本地 TURN server 需确保其运行  
> - `actr_type` 已匹配：Server = `acme+EchoService`，Client = `acme+echo-client-app`  
> - ACL 双向已配置

### 14.0 环境准备

| # | 检查项 | 操作 | 预期 |
|---|--------|------|------|
| 14-0-1 | Actrix 运行 | 确认 signaling server 在双端可达的地址上运行 | WebSocket 握手成功 |
| 14-0-2 | realm_id 对齐 | 修改 Rust Server `actr.toml` 的 `realm_id` 或 Web Client `actor.sw.js` 的 `realm_id`，使两端一致 | 配置值相同 |
| 14-0-3 | signaling URL 对齐 | 确保两端指向同一 signaling server（注意 `ws://` vs `wss://`） | 双端能连到同一 server |
| 14-0-4 | TURN/STUN 环境 | 如 Rust Server `force_relay=true`，确保 TURN server 运行；或改为 `false` 并确保两端网络可达 | ICE 候选能匹配 |
| 14-0-5 | 启动 Rust Server | `cd actr-examples && cargo run -p echo-real-server` | 终端显示 "✅ Echo Server 已完全启动并注册"，打印 Server ID |
| 14-0-6 | 启动 Web Client | 浏览器打开 Echo Client 页面 | 状态显示"✅ 已连接"，5s 后自动发送测试消息 |

### 14.1 基本功能

| # | 测试项 | 操作步骤 | 预期结果 | 通过 |
|---|--------|---------|---------|------|
| 14-1-1 | 自动发送 | Web Client 初始化后等待 5s 自动发送 | Rust Server 终端打印 📨 收到请求 + 📤 发送响应，Web Client 显示 📥 回复 | ☐ |
| 14-1-2 | 手动发送 | 在 Web Client 输入框输入文字，点击发送 | Rust Server 正确回显，Web Client 显示回复 | ☐ |
| 14-1-3 | 快速连续发送 | Web Client 连续点击发送 5+ 次 | 所有请求在 Rust Server 依次处理并返回回复 | ☐ |
| 14-1-4 | 大消息 | 发送 >10KB 字符串 | Rust Server 正常处理，回复完整 | ☐ |
| 14-1-5 | 中文/Emoji | 发送"你好🌍" | Protobuf string 字段正确传输，Rust Server 回显内容一致 | ☐ |

### 14.2 WebRTC 互通性（浏览器 ↔ Native）

| # | 测试项 | 操作步骤 | 预期结果 | 通过 |
|---|--------|---------|---------|------|
| 14-2-1 | SDP 协商 | 观察 Rust Server 日志和 `chrome://webrtc-internals` | Offer/Answer SDP 格式双端兼容，ICE 协商完成 | ☐ |
| 14-2-2 | DataChannel 建立 | 检查 `chrome://webrtc-internals` | 4 条 DataChannel 正确建立（RPC_RELIABLE 等），与 Rust 端 lane 对应 | ☐ |
| 14-2-3 | STUN 直连 | 双端在同一局域网，Rust `force_relay=false` | ICE 候选为 `host` 或 `srflx`，不经 TURN | ☐ |
| 14-2-4 | TURN 中继 | Rust Server `force_relay=true`，TURN server 运行 | ICE 候选为 `relay`，数据经 TURN 转发 | ☐ |
| 14-2-5 | TURN 不可用 | Rust Server `force_relay=true`，但 TURN server 未运行 | ICE 协商失败 → WebRTC 连接建立不了 → RPC 超时 | ☐ |
| 14-2-6 | ICE restart 跨端 | 短暂阻断 UDP 流量触发 ICE disconnected | 浏览器侧（offerer）发起 ICE restart offer → Rust Server 回 answer → 连接恢复 | ☐ |

### 14.3 Rust Server 生命周期

| # | 测试项 | 操作步骤 | 预期结果 | 通过 |
|---|--------|---------|---------|------|
| 14-3-1 | Ctrl+C 停止 Server | 正常通信后，在 Rust Server 终端按 Ctrl+C | Server 优雅关闭，signaling 注销 actor。Web Client 后续发送失败（RPC 超时或 DataChannel closed） | ☐ |
| 14-3-2 | kill -9 Server | `kill -9 <pid>` 强杀 Rust Server 进程 | 无优雅关闭，signaling server 5 分钟后 heartbeat 过期清理。Web Client 立即感知 WebRTC disconnected → ICE restart 失败 | ☐ |
| 14-3-3 | 重启 Server | Ctrl+C 停止 → 等 3s → 重新 `cargo run` | 新 Server 注册新 actr_id。Web Client 旧路由失效，需重新 discover_target → 新 RPC 成功 | ☐ |
| 14-3-4 | Server 重启后 Client 自动恢复 | 停止 Server → 重启 Server → Web Client 发消息 | Client 旧 peer 失效 → ICE restart 失败 → close_peer → 下次 RPC 触发 ensure_peer_with_retry → 重新 discover → 新连接 → 成功 | ☐ |
| 14-3-5 | Server 长时间运行 | Rust Server 运行 10+ 分钟 | heartbeat 正常维持注册，Web Client 随时可发送 | ☐ |

### 14.4 Web Client 刷新/关闭（Server 为 Rust）

| # | 测试项 | 操作步骤 | 预期结果 | 通过 |
|---|--------|---------|---------|------|
| 14-4-1 | Client F5 刷新 | 正常通信后 F5 刷新 Web Client | 旧 SW runtime 被清理，新 Client 重新 register → discover Rust Server → 建立新 WebRTC → 发送成功 | ☐ |
| 14-4-2 | Client 连续刷新 | 快速 F5 3-5 次 | Rust Server 可能收到多个新 peer 的 role negotiation；旧连接自然超时。最终 Client 稳定连接 | ☐ |
| 14-4-3 | Client 关闭标签页 | 关闭 Web Client 标签页 | Rust Server 侧 WebRTC peer 状态变为 disconnected/failed → Server 端清理 peer。Server 继续正常运行 | ☐ |
| 14-4-4 | Client 关闭后重新打开 | 关闭 Client → 5s → 重新打开 Client 页面 | 新 Client 发现同一个运行中的 Rust Server → 新 WebRTC 连接 → 发送成功 | ☐ |
| 14-4-5 | 多个 Web Client → 1 Rust Server | 打开 2-3 个 Client 标签页，同时发送 | Rust Server 并发处理多个 peer 的 RPC，各 Client 收到各自回复 | ☐ |

### 14.5 网络异常（跨端）

| # | 测试项 | 操作步骤 | 预期结果 | 通过 |
|---|--------|---------|---------|------|
| 14-5-1 | Web Client 断网 | DevTools → Offline，保持 10s，再恢复 | Client signaling 重连 → 重新 register → discover → 新 WebRTC 连接到 Rust Server | ☐ |
| 14-5-2 | Rust Server 断网 | 拔网线或 `ifconfig down`，10s 后恢复 | Server signaling 重连（actr-runtime 自带），重新注册。Web Client 旧 peer failed → 重新 discover | ☐ |
| 14-5-3 | Signaling 重启 | 双端正常通信时重启 Actrix | 双端各自重连 signaling → 重新注册 → Client 重新 discover Server → 新 WebRTC 连接 | ☐ |
| 14-5-4 | Client 弱网 | Slow 3G 模拟 | DataChannel 数据可达但延迟高，Rust Server 正常处理 | ☐ |
| 14-5-5 | 跨网段通信 | Web Client 和 Rust Server 在不同子网 | 需要 TURN 中继或公网 STUN 穿透，检查 ICE 候选类型 | ☐ |

### 14.6 协议兼容性

| # | 测试项 | 操作步骤 | 预期结果 | 通过 |
|---|--------|---------|---------|------|
| 14-6-1 | Protobuf 兼容 | 对比 Web Client `echo.proto` 和 Rust Server `echo.proto` | 两端 proto 定义一致（EchoRequest/EchoResponse 字段匹配） | ☐ |
| 14-6-2 | RpcEnvelope 兼容 | 抓包或日志对比 DataChannel 帧格式 | Web Client (WASM) 编码的 `RpcEnvelope` 与 Rust Server 解码兼容 | ☐ |
| 14-6-3 | Signaling 协议兼容 | 对比两端的 `SignalingEnvelope` 序列化 | RegisterRequest、RouteCandidates、ActrRelay 格式一致 | ☐ |
| 14-6-4 | Role Negotiation 跨端 | 观察双端日志中的 role_assignment | 一端被分配为 offerer，另一端为 answerer，角色正确建立 | ☐ |
| 14-6-5 | ACL 跨端 | Web Client type = `acme+echo-client-app`，Rust Server ACL 允许该 type | route_candidates 返回 Rust Server 作为候选 | ☐ |

---

## 关键超时参数速查

| 参数 | 值 | 说明 |
|------|-----|------|
| WebSocket open | 15s | Signaling WS 连接超时 |
| Heartbeat 间隔 | 25s | 向 signaling server 发送 Ping |
| Heartbeat 失败阈值 | 3 次 | 连续失败后触发 signaling 重连 |
| Signaling 重连 | 最多 10 次 | 退避 1s→2s→4s→...→60s |
| DOM PING 间隔 | 20s | 保持 SW 活跃 |
| RPC 超时 | 30s | 默认 RPC 请求超时 |
| P2P 连接重试 | 最多 3 次 | 退避 3s→6s→12s (max 15s) |
| ICE restart | 最多 5 次 | 退避 5s→10s→20s→40s→60s |
| Signaling server 清理 | ~5 min | Heartbeat 过期后移除 actor 注册 |
| SW 浏览器终止 | ~30s 空闲 | 无活动时浏览器可能终止 SW |

---

## 测试结果汇总

| 区域 | 总项数 | 通过 | 失败 | 阻塞 | 备注 |
|------|--------|------|------|------|------|
| 一 基本功能 | 6 | | | | |
| 二 页面刷新 | 6 | | | | |
| 三 SW 生命周期 | 6 | | | | |
| 四 网络断开/恢复 | 8 | | | | |
| 五 WebRTC 连接 | 5 | | | | |
| 六 多标签页/多客户端 | 6 | | | | |
| 七 页面关闭 | 5 | | | | |
| 八 Signaling 边界 | 5 | | | | |
| 九 SW 空闲恢复 | 6 | | | | |
| 十 浏览器兼容性 | 8 | | | | |
| 十一 WASM 加载 | 5 | | | | |
| 十二 并发/压力 | 4 | | | | |
| 十三 错误恢复/降级 | 6 | | | | |
| 十四 跨端 Web C + Rust S | 28 | | | | |
| **合计** | **104** | | | | |
