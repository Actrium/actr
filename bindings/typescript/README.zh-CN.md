# actr-ts

中文 | [English](./README.md)
Language: 中文（简体）

这是 ACTR 的 TypeScript/Node.js 绑定。当前实现已经切到
`package-first`：本地源码形式的 workload 已被移除。`actr-ts` 现在会从
`manifest.toml` 启动一个 `ActrNode` 包装层，自动加载同目录的
`actr.toml`，然后通过发现 + 显式远端调用访问服务。

## 概述

actr-ts 通过 napi-rs 提供 ACTR 的原生 Node.js 绑定，TypeScript API 的核心
是 `ActrNode` 和 `ActrRef`。

## 特性

- 基于 Rust 与 napi-rs 的原生性能
- 类型安全的 TypeScript API
- 内置服务发现
- 远端 RPC 与单向消息
- package-first 运行模型

## 安装

```bash
npm install @actrium/actr
```

## 快速开始

```typescript
import { ActrNode, PayloadType } from '@actrium/actr';

async function main() {
  const node = await ActrNode.fromConfig('./manifest.toml');
  const actorRef = await node.start();

  const [serverId] = await actorRef.discover(
    { manufacturer: 'actrium', name: 'EchoService', version: '0.2.1-beta' },
    1,
  );

  if (!serverId) {
    throw new Error('没有发现 EchoService');
  }

  const response = await actorRef.call(
    serverId,
    'echo.EchoService.Echo',
    PayloadType.RpcReliable,
    Buffer.from('hello'),
    5000,
  );

  console.log(response.toString());
  await actorRef.stop();
}

main().catch(console.error);
```

## API

- `ActrNode.fromConfig(configPath)`：从 `manifest.toml` 启动
  `ActrNode` 包装层。
- `ActrRef.discover(targetType, count)`：发现远端 actor。
- `ActrRef.call(target, routeKey, payloadType, payload, timeoutMs)`：发起远端 RPC。
- `ActrRef.tell(target, routeKey, payloadType, payload)`：发送单向远端消息。
- `ActrRef.stop()`：关闭 actor 并等待完成。

## 与 Rust Node Typestate 的关系

Rust 宿主暴露的是 typestate 链
`Node<Init> → Node<Attached> → Node<Registered> → ActrRef`
（`from_config_file` → `attach_*` → `register` → `start`），方便系统层代码观察并自定义每一次状态迁移。TypeScript 绑定有意把这条流水线压扁成一步 `ActrNode.fromConfig(path).start()`：应用开发者几乎用不到中间态，扁平接口更适合当前 TypeScript 绑定表面。需要精细控制（自定义 `TrustProvider`、复用已有 `Hyper`、工作负载托管等）时，请下沉到原生 Rust 的 `actr_hyper::{Hyper, Node}` 接口。

## 当前边界

- 当前支持：manifest 启动、服务发现、远端 RPC、关闭流程。
- 已移除：源码定义的本地 workload、`ActrSystem`、`system.attach(...)`、`Workload`。
- 如果要承载服务，请构建经过验证的 `.actr` 包，并通过 Rust `Node::attach(...)`（`wasm` / `dyn lib`）运行。

## 配置

创建 `manifest.toml` 配置文件：

```toml
edition = 1
exports = []

[package]
name = "my-actor"
description = "My Actor"

[package.actor]
manufacturer = "actr"
name = "my-actor"

[network]
bind_address = "0.0.0.0:0"

[network.discovery]
multicast_address = "239.255.42.99:4242"
interface = "0.0.0.0"

[observability]
filter_level = "info"
tracing_enabled = false
```

## 生成代码（示例）

示例客户端使用 `examples/**/generated` 下的生成文件，这些目录已经加入
`.gitignore`。克隆仓库后，需要先运行 codegen 脚本，再运行示例。

前置条件：

- `npm install`

为 echo-client 生成：

```bash
npm run codegen -- --config examples/echo-client/manifest.toml
```

注意：

- 生成器优先读取 `manifest.lock.toml`；请确保包含你想生成的依赖。
- proto 默认来源为 `examples/echo-client/protos/remote`。
- 输出包括 protobuf 编解码、路由辅助，以及 `local.actor.ts`。

## 构建

```bash
npm install
npm run build
npm run compile:ts
```

## 发布（维护者）

TypeScript 包发布通过仓库里的手动 GitHub Actions workflow
`Publish TypeScript Package` 管理。

- 包名：`@actrium/actr`
- Workflow 文件：`.github/workflows/publish-typescript.yml`
- 认证方式：GitHub Actions OIDC + npm trusted publishing
- 必需权限：`id-token: write`
- 首次发布要求：先手动发布一次 `@actrium/actr`，再为该 workflow 配置 npm trusted publisher
