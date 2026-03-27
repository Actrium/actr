# actr-ts

[中文](./README.zh-CN.md) | English

TypeScript/Node.js bindings for ACTR. The binding is now `package-first`:
local source-defined workloads were removed. `actr-ts` currently supports
client-only nodes created from `manifest.toml`, then uses discovery plus explicit
remote calls.

## Overview

actr-ts provides native Node.js bindings for the ACTR framework through
napi-rs, with a TypeScript API centered on `ActrNode` and `ActrRef`.

## Features

- Native performance through Rust and napi-rs
- Type-safe TypeScript API
- Built-in service discovery
- Remote RPC and one-way messaging
- Package-first runtime model

## Installation

```bash
npm install @actrium/actr
```

## Quick Start

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
- `ActrRef.stop()` shuts down the actor and waits for completion.

## Current Scope

- Supported: client-only nodes, discovery, remote RPC, shutdown.
- Removed: source-defined local workloads, `ActrSystem`, `system.attach(...)`, `Workload`.
- For service hosting, build a verified `.actr` package and run it with Rust `Hyper.attach_package(...)`.

## Configuration

Create a `manifest.toml` configuration file:

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

## Generated Code (Examples)

The example clients use generated files under `examples/**/generated`, which
are git-ignored. After cloning the repository, run the codegen script before
running the examples.

Prerequisites:

- `npm install`

Generate for echo-client:

```bash
npm run codegen -- --config examples/echo-client/manifest.toml
```

Notes:

- The generator reads `manifest.lock.toml` first; ensure it includes the
  dependencies you want emitted.
- Proto sources default to `examples/echo-client/protos/remote`.
- Outputs include protobuf codecs, route helpers, and `local.actor.ts`.

## Build

```bash
npm install
npm run build
npm run compile:ts
```

## Publishing (Maintainers)

TypeScript package releases are managed from this monorepo through the manual
GitHub Actions workflow `Publish TypeScript Package`.

- Package name: `@actrium/actr`
- Workflow file: `.github/workflows/publish-typescript.yml`
- Authentication: npm trusted publishing via GitHub Actions OIDC
- Required workflow permission: `id-token: write`
- Initial release requirement: publish `@actrium/actr` once manually before
  adding the npm trusted publisher for this workflow
