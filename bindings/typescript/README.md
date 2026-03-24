# actr-ts

[Chinese](./README.zh.md) | English

TypeScript/Node.js bindings for ACTR. The binding is now `package-first`: local source-defined workloads were removed. `actr-ts` currently supports client-only nodes created from `actr.toml`, then uses discovery plus explicit remote calls.

## Quick Start

```typescript
import { ActrNode, ActrType, Dest, PayloadType } from '@actor-rtc/actr';

async function main() {
  const node = await ActrNode.fromConfig('./actr.toml');
  const actorRef = await node.start();

  const [serverId] = await actorRef.discover(
    { manufacturer: 'actrium', name: 'EchoService', version: '0.2.1-beta' },
    1,
  );

  if (!serverId) {
    throw new Error('No EchoService target discovered');
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

- `ActrNode.fromConfig(configPath)` creates a client-only node.
- `ActrRef.discover(targetType, count)` resolves remote actors.
- `ActrRef.call(target, routeKey, payloadType, payload, timeoutMs)` sends a remote RPC.
- `ActrRef.tell(target, routeKey, payloadType, payload)` sends a one-way remote message.

## Current Scope

- Supported: client-only nodes, discovery, remote RPC, shutdown.
- Removed: source-defined local workloads, `ActrSystem`, `system.attach(...)`, `Workload`.
- For service hosting, build a verified `.actr` package and run it with Rust `Hyper.attach_package(...)`.

## Build

```bash
npm install
npm run build
npm run compile:ts
```
