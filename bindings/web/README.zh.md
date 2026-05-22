# Actor-RTC Web

Actor-RTC Web 是 Actrium 的浏览器运行时与 SDK 入口。当前浏览器主路径是 Option U / wasm-bindgen guest 路径：

- 浏览器 workload 通过签名后的 `.actr` 包加载，同时需要同名旁车目录 `<package-stem>.wbg/`
- `.wbg/` 目录内包含 `guest.js` 和 `guest_bg.wasm`
- Service Worker 入口是 `actor.sw.js`
- Service Worker 加载 wasm-bindgen guest bundle，并调用 `register_guest_workload`

Component Model / jco 只作为旧浏览器桥接路径的历史背景保留；当前浏览器运行时不消费 jco 转译产物。

## 快速开始

安装 web workspace 依赖：

```bash
cd bindings/web
pnpm install
```

运行基于仓库内 mock actrix 的 Echo 示例：

```bash
cd bindings/web/examples/echo
bash start-mock.sh
```

运行浏览器托管服务的 data-stream peer 示例：

```bash
cd bindings/web/examples/data-stream-peer-concurrent
bash start.sh
```

重建并同步 `actr-cli` 内嵌的 Web runtime assets：

```bash
cd bindings/web
bash crates/sw-host/build.sh
bash scripts/sync-cli-assets.sh --build
```

`crates/sw-host/build.sh` 负责构建 Service Worker host 的 wasm-bindgen 产物。`scripts/sync-cli-assets.sh --build` 会重新构建并复制 canonical web assets 到 `cli/assets/web-runtime/`，让 `actr run --web` 使用当前运行时。

## 当前运行时结构

```
Browser tab
  DOM application
  @actrium/actr-web SDK
  @actrium/actr-dom bridge
      |
      | MessagePort / postMessage
      v
Shared Service Worker: actor.sw.js
  sw-host wasm-bindgen runtime
  register_guest_workload(dispatchFn)
      |
      v
<package-stem>.wbg/
  guest.js
  guest_bg.wasm
```

每个浏览器 tab 有自己的 DOM 侧 client identity。Service Worker 在同源下共享，并在内部维护按 client 隔离的运行时状态。

## 包结构

- `packages/actr-dom`：`@actrium/actr-dom`，DOM 侧桥接层，负责 Service Worker、WebRTC 和浏览器 API 协调。
- `packages/web-sdk`：`@actrium/actr-web`，浏览器 SDK 和 `actor.sw.js` 源文件。
- `packages/web-react`：`@actrium/actr-web-react`，React hooks。当前公开导出是 `useActorClient`、`useServiceCall`、`useSubscription`。
- `crates/sw-host`：通过 wasm-bindgen 构建的 Service Worker runtime。
- `crates/dom-bridge`：Rust 侧 DOM bridge 支持。
- `crates/mailbox-web`：IndexedDB mailbox 支持。

## 发布

Web npm 包通过 GitHub Actions 的 `Publish Web Packages` 手动 workflow 发布。本地等价验证命令是：

```bash
cd bindings/web
pnpm install --frozen-lockfile
bash scripts/publish.sh --dry-run --expected-version 0.1.0
```

脚本按依赖顺序发布：`@actrium/actr-dom`、`@actrium/actr-web`、`@actrium/actr-web-react`。

## 文档导航

优先阅读当前用户文档：

- [文档索引](./docs/README.md)
- [Getting started](./docs/getting-started.md)
- [Troubleshooting](./docs/troubleshooting.md)
- [Error handling](./docs/error-handling.md)
- [Requirements](./docs/requirements.md)

以下文档适合作为历史深挖材料，但不应当作为当前构建步骤使用：

- [Option U WIT compile web notes](./docs/option-u-wit-compile-web.zh.md)
- [2026-04 架构变更记录](./docs/architecture-changes-2026-04.zh.md)
- [jco async-lift 历史排障](./docs/t18-jco-async-lift-hang.zh.md)
- [架构笔记](./docs/architecture/README.zh.md)

## 开发提示

- 在 `bindings/web` 下使用 `pnpm install` 安装 workspace 依赖。
- 修改 `crates/sw-host` 后使用 `bash crates/sw-host/build.sh` 构建。
- 需要刷新 CLI 内嵌 web runtime assets 时，使用 `bash scripts/sync-cli-assets.sh --build`。
- 主 Echo smoke 路径使用 `bash examples/echo/start-mock.sh`。
- 浏览器托管 peer/service 检查使用 `bash examples/data-stream-peer-concurrent/start.sh`。

## License

本项目采用 Apache License 2.0。详见 [LICENSE](LICENSE)。
