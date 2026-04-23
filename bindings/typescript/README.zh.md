# actr-ts

[English](./README.md) | 中文

这是 ACTR 的 TypeScript/Node.js 绑定。当前实现已经切到 `package-first`：本地源码形式的 workload 已被移除。`actr-ts` 现在会从 `manifest.toml` 启动一个 `ActrNode` 包装层，自动加载同目录的 `actr.toml`，然后通过发现 + 显式远端调用访问服务。

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

- `ActrNode.fromConfig(configPath)`：从 `manifest.toml` 启动 `ActrNode` 包装层。
- `ActrRef.discover(targetType, count)`：发现远端 actor。
- `ActrRef.call(target, routeKey, payloadType, payload, timeoutMs)`：发起远端 RPC。
- `ActrRef.tell(target, routeKey, payloadType, payload)`：发送单向远端消息。

## 与 Rust Node Typestate 的关系

Rust 宿主暴露 typestate 链 `Node<Init> → Node<Attached> → Node<Registered> → ActrRef`（`from_config_file` → `attach_*` → `register` → `start`），便于系统层观察并自定义每次状态迁移。TypeScript 绑定有意把这条流水线压扁为一步 `ActrNode.fromConfig(path).start()`：应用开发者几乎不需要中间态，扁平接口更契合当前 TypeScript 绑定表面。需要精细控制（自定义 `TrustProvider`、复用 `Hyper`、工作负载托管等）时，请下沉到原生 Rust 的 `actr_hyper::{Hyper, Node}` 接口。

## 当前边界

- 当前支持：manifest 启动、服务发现、远端 RPC、关闭流程。
- 已移除：源码定义的本地 workload、`ActrSystem`、`system.attach(...)`、`Workload`。
- 如果要承载服务，请构建经过验证的 `.actr` 包，并通过 Rust `Hyper.attach_package(...)` 运行。

## 构建

```bash
npm install
npm run build
npm run compile:ts
```
