Language: Simplified Chinese

# Package Echo Client Guest Refactor Plan

## Summary

将 `package-echo/client` 从纯客户端 host 改为 package-backed host，并在 `package-echo` 下新增 `client-guest` lib 作为 client workload 实现。

最终结构：

- `client` host 只负责加载本地 `.actr`、启动节点、读取 stdin、调用本地 guest
- `client-guest` 通过本地 dispatch 接收请求
- `client-guest` 内部负责 discover 远端 `EchoService`、发起 RPC、返回结果
- 全局移除 `Workload::None` 和 `attach_none`

## Implementation Changes

### Package Echo Client

- 将 `examples/rust/package-echo/client` 改为和 server 对齐的 package-backed host
- 删除 host 内现有业务逻辑：
  - `attach_none(config)`
  - `bootstrap_credential_from_config(...)`
  - host 侧 `discover_route_candidates(...)`
  - host 侧 `call_remote(...)`
- host 启动流程统一为：
  - 读取 `actr.toml`
  - 读取并验证 client `.actr`
  - `Hyper::attach_package(...)`
  - 基于 package manifest 向 AIS 注册
  - 启动后读取 stdin
  - 通过 `actr_ref.call(...)` 调本地 guest dispatch

### Client Guest Workload

- 在 `examples/rust/package-echo` 下新增 `client-guest` crate
- `client-guest` 作为可打包 workload 实现 client 业务
- 定义本地 client-facing RPC 接口，供 host 调用
- guest dispatch 收到本地请求后：
  - 解析输入
  - discover `actrium:EchoService:<version>`
  - 缓存 discovered `ActrId`
  - 远调 `EchoService.Echo`
  - 将结果映射成本地响应
- guest 内部提供一次失败恢复：
  - 若缓存目标调用失败，清空缓存并重新 discover 一次

### Framework Cleanup

- 从 `core/hyper/src/workload.rs` 删除 `Workload::None`
- 从 `core/hyper/src/lib.rs` 删除 `attach_none`
- 删除“no workload attached”相关错误分支和文档表述
- 清理 `bootstrap_credential_from_config(...)` 的纯客户端定位
- 统一运行时模型为“节点必须附带真实 workload/package”

### Affected Integrations

- 收口以下依赖 `attach_none` 的入口：
  - `bindings/ffi`
  - `bindings/python`
  - `bindings/typescript`
  - `cli` Rust 模板
- 原有纯客户端快捷入口不保留兼容分支
- 所有相关文档、示例说明、测试断言同步更新

## Test Plan

1. `package-echo/start.sh` 能同时构建并验证 server/client 两个 `.actr`
2. `package-echo-server` 通过 `attach_package(...)` 启动并注册成功
3. `package-echo-client` 通过 `attach_package(...)` 启动并注册成功
4. CLI 输入消息后，调用链为：
   host stdin -> local guest dispatch -> guest discover -> guest remote call -> reply
5. `client-guest` 首次请求 discover 成功并缓存目标
6. 后续请求复用缓存，不重复 discover
7. 缓存目标失效时可清缓存并重新 discover 一次
8. 仓库中不再存在 `attach_none` / `Workload::None` 调用点
9. `cargo fmt --all`
10. `cargo check`

## Assumptions

- 不考虑向后兼容，允许直接移除纯客户端运行模式
- `client-guest` 放在当前仓库 `examples/rust/package-echo` 下
- client host 保留 CLI/stdin 外壳，但不再承载 discover 或远端 RPC 业务
- host 对 guest 的调用必须进入 guest dispatch
- client package 与 server package 一样，走本地 `.actr` + manifest + AIS 注册链路
