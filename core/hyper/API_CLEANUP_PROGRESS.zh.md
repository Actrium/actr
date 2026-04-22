# core/hyper API 清理进展

更新日期：2026-04-22

## 背景

本轮工作承接 `/tmp/actr-session-summary-2026-04-22.md` 中的
`T5.5 C23 Batch 2 core/hyper` 收尾，目标是继续缩小 `actr-hyper`
的正式公开面，把只供 runtime 内部或 integration test 使用的低层类型移出正式 API。

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

### 第二批：当前待提交

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

## 验证

本轮改动已通过：

- `cargo fmt --all`
- `cargo check -p actr-hyper --all-features --quiet`
- `cargo check -p actr-hyper --tests --all-features --quiet`
- `git diff --check`

## 当前判断

到目前为止，`core/hyper` 里“低风险、纯机械、只影响内部实现或测试入口”的公开面收缩，已经基本做完。

剩余更可能需要设计判断而不是机械降级的部分，集中在：

- `signaling` / `transport` 相关高层类型
- 仍保留公开模块路径、但实际主要通过 crate 顶层 re-export 使用的模块入口
- `LinkedWorkloadHandle` / `WorkloadAdapter` 一类已经进入正式 attach API 的类型

## 下一步建议

1. 继续做 `core/hyper` Batch 2 的设计性收尾，重点筛 `signaling` / `transport` 高层公开类型。
2. 评估是否进一步压缩模块级公开入口，避免“类型已从顶层导出，但模块路径仍然公开”。
3. `core/hyper` 收尾后，再转去 `T5.5 Batch 5 bindings/web` 或 `T18`。
