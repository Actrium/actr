# core/hyper API 清理进展

更新日期：2026-04-23

## 背景

本轮工作承接 `/tmp/actr-session-summary-2026-04-22.md` 中的
`T5.5 C23 Batch 2 core/hyper` 收尾，目标是继续缩小 `actr-hyper`
的正式公开面，把只供 runtime 内部或 integration test 使用的低层类型移出正式 API，
同时把 ACTR 的公开术语收敛到当前已经拍板的口径：

- 节点按 `peer` 叙述，不再混入 `client/server`
- workload 接入动词只保留：
  - `attach(...)`：`wasm / dyn lib`
  - `link(...)`：`static lib`

## 已完成

### 第一批：已提交

提交：`a69cb003 refactor(hyper): hide internal workload loader helpers`

主要内容：

- 将 `LoadedWorkload`、`Hyper::load_workload_package()` 收回到 crate 内部。
- 新增 `test_support::inspect_workload_package()`，让测试改走摘要视图，而不是依赖内部工作负载类型。
- 收回一批配置与验证相关的内部 helper：
  - `config` 中的默认常量和 `[hyper]` 解析辅助类型
  - `verify_ed25519_manifest`
  - `HostState`
  - `INITIAL_CONNECTION_TIMEOUT`
- 更新依赖这些入口的测试与 example 测试。

### 第二批：已提交

提交：`9faa0753 refactor(hyper): hide low-level engine instantiation APIs`

主要内容：

- 收回 WASM / dynclib 低层实例化入口：
  - `WasmHost::instantiate()` → `pub(crate)`
  - `WasmWorkload` 及其 `init/call_on_start/handle` → `pub(crate)`
  - `DynclibHost::instantiate()` → `pub(crate)`
  - `DynclibInstance` → `pub(crate)`
  - `DynClibWorkload` → `pub(crate)`
- 新增 test-only 包装层，避免 integration test 继续直接依赖内部运行时类型：
  - `test_support::TestWasmWorkload`
  - `test_support::instantiate_wasm_workload()`
  - `test_support::TestDynclibWorkload`
  - `test_support::instantiate_dynclib_workload()`
- 将这些测试切换到新的 `test_support` 入口：
  - `core/hyper/tests/component_model_dispatch.rs`
  - `core/hyper/tests/dynclib_host.rs`
  - `core/hyper/tests/dynclib_actor_e2e.rs`
- 顺手收回 runtime 内部枚举：
  - `workload::Workload` → `pub(crate)`
  - 从 crate 顶层 re-export 中移除 `Workload`

### 第三批：已提交

提交：`8477ff06 refactor(hyper): collapse redundant module paths`

主要内容：

- 收敛冗余公开子模块路径，保留现有 re-export，不改变对外类型名：
  - `transport::connection_event` → 私有模块，改由 `transport::{ConnectionEvent, ConnectionState}` 暴露
  - `transport::error` → 私有模块，改由 `transport::{NetworkError, NetworkResult}` 暴露
  - `wire::webrtc::{signaling, coordinator}` → 私有模块，改由 `wire::webrtc::*` 边界暴露
  - `storage::db` → 私有模块，保留 `storage::ActorStore`
  - `verify::{trust, cert_cache}` → 私有模块，保留 `verify::*`
  - `wasm::{host, error}` → 私有模块，保留 `wasm::{WasmHost, WasmError}`
- 清理 crate 内部和 integration tests 对这些深路径模块的直接依赖，统一改走模块边界 re-export。

### 第四批：已提交

提交：`44595b95 refactor(hyper): remove attach_none and fix peer terminology`

主要内容：

- 彻底删除 `Node::attach_none()` 和 `Workload::None`。
- `Node<Attached>::register*()` 不再把非 package 路径当成异常分支；`link(...)` 路径可以正常注册。
- Python / TypeScript 绑定不再走“无 workload”路径，而是改成挂一个最小 `link(...)` workload。
- 清理 `attach_none`、`Workload::None`、`client-only` 等相关描述，统一回到 peer 模型。

### 第五批：当前工作区

主要内容：

- 正式公开动词收敛为：
  - `Node::attach(...)`：`wasm / dyn lib`
  - `Node::link(...)`：`static lib`
- 原 `attach_linked(...)` 改名为 `link(...)`。
- 低层 object-safe 桥接退回内部：
  - `link_handle(...)` → `pub(crate)`
  - `LinkedWorkloadHandle` → `pub(crate)`
  - `WorkloadAdapter` → `pub(crate)`
  - 从 crate 顶层 re-export 移除
- `signaling / transport` 继续按“契约公开、实现内收”收敛：
  - 默认公开：`SignalingConfig`、`ReconnectConfig`、`AuthConfig`、`AuthType`、`DisconnectReason`、`SignalingEvent`、`SignalingStats` 等契约/值对象
  - `PeerTransport`、`WireBuilder`、`DefaultWireBuilder`、`WebRtcCoordinator`、`WebSocketSignalingClient` 只在 `test-utils` 下对外
- `outbound` / `inbound` / 低层 transport 实现进一步从默认公开面退出：
  - `outbound` 模块默认改为 crate 内部，`HostGate` / `PeerGate` 只在 `test-utils` 下公开
  - `inbound` 模块默认改为 crate 内部，`MediaFrameRegistry` 不再作为默认公开 API
  - `HostTransport`、`DataLane`、`WireHandle`、`ConnType` 改为仅在 `test-utils` 下公开
- 清理这批内部 warning：
  - `Hyper::load_workload_package()` 仅在 `test-utils` 下编译
  - `WebSocketSignalingClient::connect_to()` 改为 test-only
  - 删除未使用的 `WorkloadAdapter::from_arc()`
  - 清理 `outbound` / transport 内部 test-only helper 引起的默认构建 `dead_code` warning
  - 修正 `WebRtcConnection` / `WebSocketConnection` 的 `is_connected()` 低层实现，避免递归歧义
- 扩展术语清理，把 `WebSocket C/S`、`client-side`、`server only` 一类描述从相关框架 / FFI / 绑定生成代码注释中一并扫掉。

## 验证

已提交批次已通过：

- `cargo fmt --all`
- `cargo check -p actr-hyper --all-features --quiet`
- `cargo check -p actr-hyper --tests --all-features --quiet`
- `git diff --check`

第五批当前工作区已通过：

- `cargo fmt --all`
- `cargo fmt --all` in `bindings/python`
- `cargo fmt --all` in `bindings/typescript`
- `cargo check -p actr-hyper --quiet`
- `cargo check -p actr-hyper --tests --all-features --quiet`
- `cargo check -p actr-framework --quiet`
- `cargo check --quiet` in `bindings/python`
- `cargo check --quiet` in `bindings/typescript`
- `git diff --check`

## 当前判断

到目前为止，`core/hyper` 里“低风险、纯机械、只影响内部实现或测试入口”的公开面收缩，已经基本做完。

已经不再属于真实剩余项的旧 TODO：

- “第三批当前工作区”这一条已经提交，不再是未完成事项
- `LinkedWorkloadHandle` / `WorkloadAdapter` 已经收回内部，不再是公开 API 待清理项
- `attach_none` / `Workload::None` 路径已经完全删除

剩余更可能需要设计判断而不是机械降级的部分，集中在：

- `signaling / transport` 的高层正式契约边界
  - 例如 `ConnectionEvent`
  - signaling 侧 `ConnectionState`
  - transport 侧 `ConnectionState`
- `HostGate` / `PeerGate` 是否继续作为正式公开高层入口长期保留
- `core/hyper` 收尾后，转入 `T5.5 Batch 5 bindings/web` 与 `T18`

## 下一步建议

1. 先提交第五批，把 `attach/link` 命名收敛和 `signaling / transport` 契约收敛落盘。
2. 再做 `core/hyper` Batch 2 的最后一轮设计性收尾，专门审高层契约类型是否继续保留为正式 API。
3. `core/hyper` 收尾后，再转去 `T5.5 Batch 5 bindings/web` 或 `T18`。
