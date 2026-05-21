# Actor-RTC Web 路线图

> 状态说明：本文件已从旧的完成度清单重建为当前路线图。旧清单里的百分比完成度、已删除的需求完成度链接、已删除的 zero-copy optimization 链接、以及已经删除的旧模块路径不再作为当前事实维护。

## 当前主路径

浏览器主路径是 Option U / wasm-bindgen guest：

- workload 发布为签名 `.actr` 包，并带同名 `<package-stem>.wbg/` 旁车目录
- `.wbg/` 目录包含 `guest.js` 和 `guest_bg.wasm`
- Service Worker 入口是 `actor.sw.js`
- Service Worker 加载 wasm-bindgen guest bundle，并调用 `register_guest_workload`

Component Model / jco 是旧浏览器桥接路径的历史背景。当前用户路径不再要求 jco 转译产物。

## 已稳定维护的入口

- `packages/web-sdk`：浏览器 SDK 和 canonical `actor.sw.js`。
- `packages/actr-dom`：DOM 侧浏览器 API / Service Worker bridge。
- `packages/web-react`：React hooks，当前导出 `useActorClient`、`useServiceCall`、`useSubscription`。
- `crates/sw-host`：Service Worker 侧 wasm-bindgen runtime。
- `crates/dom-bridge`：DOM bridge 支撑代码。
- `crates/mailbox-web`：IndexedDB mailbox。
- `examples/echo/start-mock.sh`：当前 Echo smoke 入口。
- `examples/data-stream-peer-concurrent/start.sh`：浏览器托管服务 / peer 并发示例入口。
- `scripts/sync-cli-assets.sh`：同步 CLI 内嵌 web runtime assets。

## 近期路线图

### P0：保持当前主路径可验证

- [ ] 保持 `examples/echo/start-mock.sh` 覆盖 Option U / wasm-bindgen guest 端到端路径。
- [ ] 保持 `examples/data-stream-peer-concurrent/start.sh` 覆盖浏览器托管服务和多 peer 场景。
- [ ] 在修改 `crates/sw-host` 或 `packages/web-sdk/src/actor.sw.js` 后，确认 `bash scripts/sync-cli-assets.sh --build` 能刷新 CLI assets。
- [ ] 继续清理用户文档里的旧 CM/jco 构建说明，保留为历史上下文而非当前步骤。

### P1：文档和示例收敛

- [ ] 让用户文档统一指向 `bindings/web/docs/README.md`、`getting-started.md`、`troubleshooting.md`、`error-handling.md`、`requirements.md`。
- [ ] 将历史架构分析集中放在明确标注的 historical section。
- [ ] 对 Echo 和 data-stream 示例补齐最小故障排查索引。
- [ ] 对 React 使用说明只描述当前真实公开 hooks。

### P2：开发者体验

- [ ] 改进 `actr run --web` 与 `.wbg/` 旁车目录缺失时的错误提示。
- [ ] 补充 CLI asset drift 的诊断说明。
- [ ] 继续收敛浏览器端 service hosting 示例的文档命名和入口。

## 当前常用命令

```bash
cd bindings/web
pnpm install
```

```bash
cd bindings/web
bash crates/sw-host/build.sh
bash scripts/sync-cli-assets.sh --build
```

```bash
cd bindings/web/examples/echo
bash start-mock.sh
```

```bash
cd bindings/web/examples/data-stream-peer-concurrent
bash start.sh
```

## 历史阅读

- [Option U WIT compile web notes](./docs/option-u-wit-compile-web.zh.md)
- [2026-04 架构变更记录](./docs/architecture-changes-2026-04.zh.md)
- [jco async-lift 历史排障](./docs/t18-jco-async-lift-hang.zh.md)
- [架构笔记索引](./docs/architecture/README.zh.md)

这些文档可用于理解旧路径如何演进到当前 Option U / wasm-bindgen 路径，但其中的阶段性状态、完成度数字、旧脚本名和旧路径不应直接套用到当前用户流程。
