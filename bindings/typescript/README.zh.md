# actr-ts

[English](./README.md) | 中文

这是 ACTR 的 TypeScript/Node.js 绑定。当前实现已经切到 `package-first`：本地源码形式的 workload 已被移除。`actr-ts` 现在只支持从 `actr.toml` 创建 client-only 节点，然后通过发现 + 显式远端调用访问服务。

## 快速开始

```typescript
import { ActrNode, ActrType, PayloadType } from '@actor-rtc/actr';

async function main() {
  const node = await ActrNode.fromConfig('./actr.toml');
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

- `ActrNode.fromConfig(configPath)`：创建 client-only 节点。
- `ActrRef.discover(targetType, count)`：发现远端 actor。
- `ActrRef.call(target, routeKey, payloadType, payload, timeoutMs)`：发起远端 RPC。
- `ActrRef.tell(target, routeKey, payloadType, payload)`：发送单向远端消息。

## 当前边界

- 支持：client-only 节点、服务发现、远端 RPC、关闭流程。
- 已移除：源码定义的本地 workload、`ActrSystem`、`system.attach(...)`、`Workload`。
- 如果要承载服务，请构建经过验证的 `.actr` 包，并通过 Rust `Hyper.attach_package(...)` 运行。

## 构建

```bash
npm install
npm run build
npm run compile:ts
```
