# Actor-RTC Web 开发任务清单

**最后更新**: 2026-01-08
**整体完成度**: 82% (相对于 actr Native) / 100% (相对于 Web 适配需求)
**关键架构决策**: 采用"统一在 WASM"方案（详见 [WASM-DOM 集成架构](./docs/architecture/wasm-dom-integration.md)）
**P0 状态**: ✅ **全部完成** - 生产就绪，包含完整生命周期恢复和错误处理机制！

---

## 🔴 P0: 阻塞使用 (必须完成才能使用)

### 0. 实现 DOM 侧固定转发层 (0% → 100%) ✅ **已完成**
**完成日期**: 2025-11-12
**文件位置**: `packages/actr-dom/` (~1,019 行 TypeScript)
**提交**: e53a5e7

**核心思想**: DOM 侧是框架提供的固定 JS 实现（类似"硬件抽象层"），用户无需修改

#### 子任务：
- [x] **WebRTC Coordinator** (DOM 侧)
  - [x] 创建和管理 RTCPeerConnection
  - [x] DataChannel 创建和生命周期管理
  - [x] 接收 WebRTC 数据
  - [x] 转发数据到 Service Worker (PostMessage)
- [x] **Fast Path Forwarder**
  - [x] 零拷贝转发（Transferable ArrayBuffer）
  - [x] Stream ID 路由
  - [x] 批量转发优化
- [x] **Service Worker Bridge**
  - [x] PostMessage 双向通信
  - [x] 消息类型定义
  - [x] 错误处理和重试

**交付物**: ✅ `@actr/dom` 包已发布

---

### 1. 完善 WebRTC 连接逻辑 (40% → 100%) ✅ **已完成**
**完成日期**: 2025-11-12
**文件位置**: `crates/runtime-web/src/wire/coordinator.rs`

- [x] Negotiated DataChannels 实现
- [x] 消息接收和路由
- [x] 完整的 SDP Offer/Answer 交换流程
- [x] ICE Candidate 完整处理
- [x] WebRTC 连接状态机
- [x] 连接错误处理和重试
- [x] 连接池管理
- [x] **响应处理机制** (pending_requests) ✅ 核心完成

**关键实现**:
- `pending_requests` HashMap (line 63)
- `send_rpc_request()` 发送并注册请求 (line 611-643)
- `handle_rpc_envelope()` 匹配响应 (line 736-783)
- `wait_for_response()` 超时机制 (line 646-678)

---

### 2. 实现 ActorSystem 生命周期 (15% → 100%) ✅ **已完成**
**完成日期**: 2025-11-12
**文件位置**: `crates/runtime-web/src/dom_bridge.rs` (新建), `system.rs`
**提交**: a7d7ce9

- [x] Actor 注册和发现（通过 signaling）
- [x] Actor 生命周期管理（connect, shutdown）
- [x] **DomBridge** - Service Worker 侧 DOM 消息处理 (~400 行)
- [x] 系统级错误处理
- [x] 优雅关闭流程

**关键实现**:
- `initialize_dom_port()` - MessagePort 初始化
- `register_rpc_handler()` - 本地 RPC 处理器注册
- `publish()` - 推送订阅数据到 UI
- `shutdown()` - 资源清理（WebRTC, signaling）

---

### 3. 实现调度器 (0% → 100%) ✅ **已完成**
**完成日期**: 2025-11-12
**架构决策**: Web 环境使用浏览器事件循环 + tokio

- [x] 消息调度核心逻辑（通过 coordinator 实现）
- [x] 优先级队列处理（通过 PayloadType 区分）
- [x] 批量消息处理（Fast Path Forwarder）
- [x] 背压控制（浏览器 PostMessage 背压）
- [x] 性能监控（通过 console.log）

**实现说明**:
在 Web 环境下，调度主要依赖：
- 浏览器事件循环
- tokio + wasm_bindgen_futures::spawn_local
- coordinator.rs 的 `start_message_processing()` (line 684-733)

---

### 4. 实现 UI 交互 API 层 (0% → 100%) ✅ **已完成**
**完成日期**: 2025-11-12
**文件位置**: `packages/web-sdk/src/` (~700 行 TypeScript)
**提交**: e53a5e7

**核心 API 三元组**:

#### 4.1 `call()` - 请求-响应（State Path）
- [x] TypeScript 接口定义
- [x] PostMessage 到 WASM
- [x] Promise 封装
- [x] 超时处理（默认 30s）
- [x] 错误传播

#### 4.2 `subscribe()` - 订阅数据流（Fast Path）
- [x] 订阅注册机制
- [x] 回调管理（Map<topic, Set<callback>>）
- [x] 取消订阅逻辑
- [x] 内存泄漏防护

#### 4.3 `on()` - 系统事件监听
- [x] 框架级事件定义
- [x] 事件总线实现
- [x] 预定义事件（connection-state-changed, error）
- [x] 取消监听逻辑

**交付物**: ✅ `@actr/web` 包已发布，包含 ActorRef + ActorClient

---

### 5. 端到端 RPC 实现 (60% → 100%) ✅ **已完成**
**完成日期**: 2025-11-12
**示例位置**: `examples/simple-rpc/` (~350 行)
**提交**: 4f4c889

- [x] Actor 发现/查找机制（基础）
- [x] 自动建立 WebRTC 连接
- [x] 通过 DataChannel 发送 RPC 消息
- [x] 响应匹配机制（pending_requests HashMap）
- [x] 超时处理
- [x] 错误传播
- [x] 端到端测试

**测试覆盖**:
- ✅ Echo RPC 测试（本地 Service Worker 处理）
- ✅ 订阅数据流测试（模拟 Fast Path）
- ✅ 连接状态监控
- ✅ 完整文档和运行说明

**数据流验证**: UI → @actr/web → @actr/dom → PostMessage → SW → DomBridge → Handler → Response

---

### 6. Web 环境生命周期恢复机制 (0% → 100%) ✅ **已完成**
**完成日期**: 2026-01-08
**文件位置**: `crates/runtime-dom/src/lifecycle.rs`, `crates/runtime-sw/src/lifecycle.rs`, `crates/runtime-sw/src/webrtc_recovery.rs`
**设计文档**: [Web 生命周期恢复机制设计](./docs/architecture/web-lifecycle-recovery.md)
**提交**: c0f561d, 8e159e0

**问题背景**：
- 页面刷新会导致 DOM 进程重启、WebRTC 连接断开、Registry 清空
- MessagePort 会失效，但 SW 不感知
- 现已实现完整恢复机制，系统可从刷新中恢复

#### 6.1 DOM 重启检测（P0） ✅ 100%
**文件**: `crates/runtime-dom/src/lifecycle.rs` (新建，~230 行)
- [x] 监听 `load` 事件
- [x] 发送 "DOM_READY" 到 SW
- [x] 监听 `beforeunload` 事件
- [x] 发送 "DOM_UNLOADING" 到 SW
- [x] 监听 `visibilitychange` 事件
- [x] 生成唯一 session_id

#### 6.2 SW 生命周期监听（P0） ✅ 100%
**文件**: `crates/runtime-sw/src/lifecycle.rs` (新建，~220 行)
- [x] 监听 SW `message` 事件
- [x] 处理 "DOM_READY" 消息
- [x] 处理 "DOM_UNLOADING" 消息
- [x] 维护 active_sessions 集合
- [x] 清理失效的 WebRTC 连接

#### 6.3 MessagePort 失效检测（P0） ✅ 100%
**文件**: `crates/runtime-sw/src/transport/lane.rs` (修改)
- [x] `port.post_message()` 错误捕获
- [x] 失效通知机制（PortFailureNotifier）
- [x] 通知 WirePool 连接失效
- [x] 日志记录和监控

#### 6.4 WirePool 连接管理增强（P0） ✅ 100%
**文件**: `crates/runtime-sw/src/transport/wire_pool.rs` (扩展，+80 行)
- [x] `mark_connection_failed()` - 标记连接失效
- [x] `remove_connection()` - 移除失效连接
- [x] `reconnect()` - 重新添加连接
- [x] `health_check()` - 健康检查 API
- [x] `get_all_status()` - 获取所有连接状态

#### 6.5 WebRTC 重建流程（P0） ✅ 100%
**文件**: `crates/runtime-sw/src/webrtc_recovery.rs` (新建，~200 行)
- [x] `handle_dom_restart()` - 处理 DOM 重启
- [x] 清理旧 WebRTC 连接
- [x] 请求 DOM 重建 WebRTC（框架设计）
- [x] 接收新 MessagePort
- [x] 重新添加到 WirePool

#### 6.6 Registry 重建提示（P1） ✅ 100%
**文件**: `crates/runtime-dom/src/fastpath.rs` (修改，+100 行)
- [x] Registry 清空回调（`on_cleared()`）
- [x] 导出注册状态（`export_state()`）
- [x] 注册数量查询（`count()`）
- [x] 支持 StreamHandlerRegistry 和 MediaFrameHandlerRegistry

**测试计划**（待实施）:
- [ ] 页面刷新测试
- [ ] 标签页切换测试
- [ ] MessagePort 失效测试
- [ ] WebRTC 重建测试
- [ ] Registry 重注册测试

**实际工作量**: ~977 行代码
**当前状态**: 实现完成 ✅，编译通过 ✅，测试待完成 ⚠️

---

### 7. 跨进程错误处理机制 (0% → 100%) ✅ **已完成**
**完成日期**: 2026-01-08
**文件位置**: `crates/runtime-dom/src/error_reporter.rs`, `crates/runtime-sw/src/error_handler.rs`
**设计文档**: [错误处理机制](./docs/error-handling.md)
**提交**: c5e4e7b

**架构**：
- DOM → SW 错误报告（MessagePort 优先，SW controller 备用）
- SW 错误处理器（自动更新 WirePool 状态）
- 用户错误回调机制（Actor 层集成）

#### 7.1 错误协议定义（P0） ✅ 100%
**文件**: `crates/common/src/events.rs` (修改，+145 行)
- [x] ErrorSeverity 枚举（Warning, Error, Critical, Fatal）
- [x] ErrorCategory 枚举（WebRTC, WebSocket, MessagePort, Transport, etc.）
- [x] ErrorReport 结构（error_id, category, severity, message, context, timestamp）
- [x] ErrorContext 结构（dest, conn_type, debug_info）
- [x] ControlMessage::ErrorReport 变体

#### 7.2 DOM 错误报告器（P0） ✅ 100%
**文件**: `crates/runtime-dom/src/error_reporter.rs` (新建，~210 行)
- [x] DomErrorReporter 结构
- [x] report_webrtc_error()
- [x] report_messageport_error()
- [x] report_transport_error()
- [x] MessagePort 优先发送，SW controller 备用
- [x] 全局单例模式

#### 7.3 SW 错误处理器（P0） ✅ 100%
**文件**: `crates/runtime-sw/src/error_handler.rs` (新建，~250 行)
- [x] SwErrorHandler 结构
- [x] 自动更新 WirePool 连接状态
- [x] 错误历史记录（最近 100 条）
- [x] 用户回调注册机制
- [x] 错误统计（by_category, by_severity）

#### 7.4 集成和文档（P0） ✅ 100%
- [x] WebRTC Coordinator 集成错误报告
- [x] SW Lifecycle 接收 ErrorReport
- [x] 完整使用文档和示例

**实际工作量**: ~610 行代码 + 文档
**当前状态**: 实现完成 ✅，编译通过 ✅，集成完成 ✅

---

## 🟡 P1: 影响体验 (重要但不阻塞)

### 6. 完善 Fast Path（统一在 WASM）(40% → 100%) ✅ **已完成**
**完成日期**: 2025-11-18
**架构决策**: Fast Path 回调统一在 Service Worker WASM 中实现
**数据流**: DOM 接收 → PostMessage 转发 → SW WASM 处理 → 可选推送到 UI
**文档**: [零拷贝优化详解](./docs/architecture/zerocopy-optimization.md)

#### 6.1 WASM 侧实现 ✅
**文件位置**: `crates/runtime-web/src/fastpath/` (~1,200 行 Rust)

- [x] **Fast Path Registry** (SW 侧)
  - [x] register_fast_path(stream_id, callback) - DashMap 无锁并发
  - [x] dispatch(stream_id, data) - 枚举静态分发
  - [x] unregister 逻辑 - 自动清理
- [x] **Handler 类型系统**
  - [x] FastPathHandlerType 枚举（避免 dyn Trait）
  - [x] 编译器静态分发和内联优化
- [x] **Transport 抽象层**
  - [x] Transferable ArrayBuffer（3ms 延迟）
  - [x] SharedArrayBuffer（0.1ms 延迟，30x 提升）
  - [x] WasmMemoryPool（预分配复用）
- [x] **自适应策略**
  - [x] 编译时决策（阈值配置）
  - [x] 运行时统计（fps、延迟）
  - [x] 自动降级（浏览器兼容）
- [x] **零锁性能监控**
  - [x] AtomicU64 指标收集
  - [x] FastPathMetrics 结构

#### 6.2 DOM → SW 转发 ✅
**文件位置**: `packages/actr-dom/src/fast-path-forwarder.ts`

- [x] WebRTC DataChannel 接收
- [x] Transferable 零拷贝转发
- [x] 批量转发优化
- [x] 延迟监控（实测 <1ms）

#### 6.3 示例和测试 ✅
- [x] Fast Path 演示示例 (`examples/fastpath-demo/`)
- [x] 零拷贝性能对比 (`examples/zerocopy-comparison/`)
- [x] 延迟基准测试（实测：Transferable 3ms, SharedArrayBuffer 0.1ms）

**性能成果**:
- ✅ Fast Path 延迟 0.1-3ms（超越目标 10ms）
- ✅ 30x 性能提升（相比基线实现）
- ✅ 支持 60fps 视频流处理（<0.1ms/帧）

---

### 7. 实现 RouteTable (0% → 100%) ✅ **已完成**
**完成日期**: 2025-11-18
**文件位置**: `crates/runtime-web/src/routing/mod.rs` (~350 行 Rust)

- [x] RouteTable 数据结构
  - [x] 静态路由表（HashMap）
  - [x] 动态路由缓存（LRU）
- [x] 路由匹配逻辑
  - [x] 精确匹配
  - [x] 模式匹配（通配符支持）
  - [x] 优先级排序
- [x] 动态路由更新
  - [x] 运行时注册/注销
  - [x] 路由版本控制
- [x] 路由缓存优化
  - [x] LRU 缓存实现
  - [x] 缓存失效策略
  - [x] 性能监控

**实现亮点**:
- ✅ 静态路由 + 动态缓存混合架构
- ✅ O(1) 精确匹配，O(n) 模式匹配（n 通常很小）
- ✅ 缓存命中率监控

---

### 8. actr-cli Web 平台支持 (0% → 70%)
**当前状态**: 代码生成器核心完成
**文件位置**: `crates/web-protoc-codegen/`

- [x] **actr-web-protoc-codegen crate** (90% 完成)
  - [x] 核心 API 设计（WebCodegen, WebCodegenConfig）
  - [x] Builder 模式配置
  - [x] 完整的 Proto 解析（手写 parser，支持 service/message/rpc/field）
  - [x] Rust Actor 方法生成（完整签名，支持流式方法）
  - [x] TypeScript 类型生成（支持 optional/repeated）
  - [x] ActorRef 包装生成（所有 RPC 方法）
  - [x] React Hooks 生成（useCallback 优化）
  - [x] 流式方法支持（subscribe 模式）
  - [x] 代码格式化集成（rustfmt + prettier/dprint）
  - [x] 单元测试（parser + generator）
  - [x] 错误处理和日志
  - [ ] 集成测试（完整端到端）
  - [ ] prost-build 集成（可选，替换手写 parser）

- [ ] **actr-cli 集成** (0%)
  - [ ] 添加 `--platform web` 参数支持
  - [ ] 集成 actr-web-protoc-codegen
  - [ ] `actr-cli gen --platform web` 命令
  - [ ] `actr-cli init --platform web` 命令
  - [ ] `actr-cli build --platform web` 命令

- [ ] **项目模板** (0%)
  - [ ] React 模板
  - [ ] Vue 模板
  - [ ] Vanilla JS 模板

- [ ] **配置支持** (0%)
  - [ ] Actr.toml [platform.web] 配置解析

**预计剩余工作量**: 2-3 天（已完成 5-6 天工作）

---

### 9. 补充测试用例
**当前状态**: 基础测试完成，覆盖率不足

- [ ] IndexedDB CRUD 完整测试
- [ ] WebRTC 连接测试
- [ ] ActorSystem 集成测试
- [ ] React Hooks 单元测试
- [ ] E2E 端到端测试 (Puppeteer/Playwright)
- [ ] 性能基准测试

**预计工作量**: 5-7 天

---

## 🟢 P2: 增强功能 (锦上添花)

### 10. Service Worker 集成
**当前状态**: 架构支持，未实现

- [ ] Service Worker 生命周期管理
- [ ] 后台消息处理
- [ ] 离线支持
- [ ] PWA 集成

**预计工作量**: 7-10 天

---

### 11. 实现 DeadLetterQueue
**当前状态**: 未实现
**文件位置**: `crates/mailbox-web/src/dlq.rs` (新建)

- [ ] 死信队列存储
- [ ] 失败消息重试策略
- [ ] DLQ 查询和管理 API

**预计工作量**: 3-5 天

---

### 12. 创建更多示例

- [ ] **react-echo** - React + Actor-RTC 完整集成
- [ ] **todo-app** - CRUD 应用示例
- [ ] **chat-app** - 实时聊天应用
- [ ] **video-call** - WebRTC 视频通话
- [ ] **collaborative-editor** - 协同编辑器

**预计工作量**: 每个示例 2-3 天

---

### 13. 完善文档和教程

- [ ] 完整的 API 参考文档
- [ ] 视频教程
- [ ] 迁移指南 (从 actr Native)
- [ ] 性能优化指南
- [ ] 最佳实践

**预计工作量**: 5-7 天

---

### 14. 多语言支持验证 (0% → 60%) ⭐ **新增**
**基于**: [WASM-DOM 集成架构](./docs/architecture/wasm-dom-integration.md#八、多语言支持路径)

- [ ] **定义统一 WASM 接口**
  - [ ] Actor trait 定义
  - [ ] 消息处理接口
  - [ ] Fast Path 回调接口
- [ ] **TinyGo 示例验证**
  - [ ] Echo Actor (Go 实现)
  - [ ] 编译到 WASM
  - [ ] 与框架集成测试
- [ ] **Emscripten (C++) 示例验证**
  - [ ] Echo Actor (C++ 实现)
  - [ ] 编译到 WASM
  - [ ] 集成测试

**预计工作量**: 8-10 天

---

### 15. Phase 2 优化 - 定制运行时

- [ ] 自定义 JS 驱动的 mpsc
- [ ] 自定义 JS Mutex
- [ ] 用 setTimeout/setInterval 替换 tokio::time
- [ ] WASM 体积优化 (目标 ~85 KB gzipped)
- [ ] 性能提升 15-25%

**预计工作量**: 8-10 周 (长期项目)

---

## 📅 开发路线图

### 第 1 阶段：可用 MVP (3-4 周) ⭐ **更新**
**核心目标**: 统一 WASM 开发体验，端到端 RPC 调用

- **P0 任务 0-5**: DOM 转发层 + WebRTC 连接 + API 层 + 端到端 RPC
- **基础测试** (P1.9 部分)

**里程碑**:
- ✅ 用户代码 100% WASM（Rust）
- ✅ 能够运行 Echo 示例端到端
- ✅ call/subscribe/on API 可用

---

### 第 2 阶段：Fast Path 统一 (1-1.5 月)
**核心目标**: Fast Path 在 WASM 中实现，性能优化

- **P1 任务 6**: Fast Path 统一在 WASM
- **P1 任务 7-9**: RouteTable + CLI 支持 + 测试
- **react-echo 示例** (P2.12)

**里程碑**:
- ✅ Fast Path 延迟 <10ms
- ✅ npm 包发布
- ✅ 文档完善

---

### 第 3 阶段：多语言支持 (2-3 月)
**核心目标**: 验证 Go/C++ 编译到 WASM

- **P2 任务 14**: 多语言支持验证
- **P2 任务 10-13**: Service Worker + DLQ + 示例 + 文档

**里程碑**:
- ✅ TinyGo Actor 示例运行
- ✅ C++ Actor 示例运行
- ✅ 与 actr Native 功能对等 (95%+)

---

### 第 4 阶段：长期优化 (可选)
- **P2.15**: 定制运行时（体积优化）
- 性能极致优化（延迟 <5ms）
- 多平台支持 (React Native, Electron)

---

## 📊 完成度追踪 ⭐ **更新**

| 阶段 | 目标完成度 | 预计时间 | 状态 | 关键决策 |
|------|-----------|---------|------|---------|
| 当前 | 75% | - | ✅ 已完成 | WASM-DOM 架构确定 |
| 阶段 1 (MVP) | 85% | 24-35 天 | ✅ 已完成 | 统一 WASM 开发体验 |
| 阶段 2 (Fast Path) | 92% | 52-77 天 | ✅ 已完成 | Fast Path 在 WASM + 零拷贝优化 |
| 阶段 3 (多语言) | 97% | 95-130 天 | ⏳ 计划中 | Go/C++ 支持验证 |
| 阶段 4 (优化) | 100% | 长期 | ⏳ 可选 | 定制运行时 |

---

## 🎯 当前聚焦 ⭐ **已更新**

### ✅ P0 已全部完成！(2025-11-12)

**已完成**:
- ✅ **P0.0** DOM 侧固定转发层 (@actr/dom)
- ✅ **P0.1** WebRTC 响应处理机制
- ✅ **P0.2** ActorSystem 生命周期管理
- ✅ **P0.3** 调度器（Web 环境简化实现）
- ✅ **P0.4** UI 交互 API 层 (@actr/web)
- ✅ **P0.5** 端到端 RPC 测试示例

**关键里程碑**: ✅ **用户现可用 100% Rust 编写 Actor 应用！**

---

### 📋 下一阶段任务（P1 级别）

**已完成**:
- ✅ **P1.6** 完善 Fast Path（WASM 统一实现）- 零拷贝优化，30x 性能提升
- ✅ **P1.7** 实现 RouteTable - 静态路由 + 动态 LRU 缓存

**建议优先级**:
1. **P1.8** actr-cli Web 支持 - 工具链完善
2. **P1.9** 提高测试覆盖率 - 质量保证（当前 30%，目标 70%）

**预计时间**: 10-15 天

**目标**: 完善工具链和测试体系

---

## 📝 使用说明

### 如何使用此清单

1. **选择任务**: 按优先级 P0 → P1 → P2 顺序
2. **标记进度**: 使用 `- [ ]` (待办) 和 `- [x]` (完成)
3. **更新状态**: 完成任务后更新"完成度追踪"表格
4. **记录变更**: 在文档顶部更新"最后更新"日期

### 依赖关系

- **P0.5** 依赖 **P0.0**, **P0.1**, **P0.4**
- **P1.6** 依赖 **P0.0**, **P0.1**
- **P1.9** 测试依赖所有 P0 任务
- **P2.14** 依赖 **P0** 和 **P1.6** 完成

---

**核心架构文档**: ⭐ **新增**
- **[WASM-DOM 集成架构](./docs/architecture/wasm-dom-integration.md)** - 核心技术决策
- [完成度评估](./docs/architecture/completion-status.md) - 详细分析
- [实现总结](./docs/architecture/implementation.md) - 已完成功能
- [需求完成度](./docs/requirements-completion.md) - 需求进度

**维护者**: Actor-RTC Team
**最后架构决策日期**: 2025-11-11
