# actr-ts

[中文](./README.zh-CN.md) | English

TypeScript/Node.js bindings for the ACTR (Actor-RTC) framework using napi-rs.

## Overview

actr-ts provides native Node.js bindings for the ACTR framework, enabling TypeScript/JavaScript developers to build actor-based distributed systems with WebRTC capabilities.

## Features

- 🚀 Native performance through Rust and napi-rs
- 📦 Type-safe TypeScript API
- 🔄 Actor-based concurrency model
- 🌐 Built-in service discovery
- 📡 RPC and streaming support
- 🔍 OpenTelemetry observability

## Installation

```bash
npm install @actor-rtc/actr
```

## Quick Start

### EchoTwice Server

```typescript
import { ActrSystem, Workload, Context, RpcEnvelope } from '@actor-rtc/actr';

class EchoTwiceServerWorkload implements Workload {
  async onStart(ctx: Context): Promise<void> {
    console.log('EchoTwice server started');
  }

  async onStop(ctx: Context): Promise<void> {
    console.log('EchoTwice server stopped');
  }

  async dispatch(ctx: Context, envelope: RpcEnvelope): Promise<Buffer> {
    if (envelope.routeKey === 'echo_twice.EchoTwiceService.EchoTwice') {
      return envelope.payload; // EchoTwice response is omitted for brevity
    }
    throw new Error(`Unknown route: ${envelope.routeKey}`);
  }
}

async function main() {
  const system = await ActrSystem.fromConfig('./Actr.toml');
  const node = system.attach(new EchoTwiceServerWorkload());
  const actorRef = await node.start();

  console.log('Server started:', actorRef.actorId());
  await actorRef.waitForShutdown();
}

main().catch(console.error);
```

### Echo Client

```typescript
import { ActrSystem, Workload, PayloadType } from '@actor-rtc/actr';

// ... implement EchoClientWorkload ...

async function main() {
  const system = await ActrSystem.fromConfig('./Actr.toml');
  const node = system.attach(new EchoClientWorkload());
  const actorRef = await node.start();

  // Discover server
  const servers = await actorRef.discover(
    { manufacturer: 'acme', name: 'EchoTwiceService' },
    1
  );

  // Call RPC
  const request = Buffer.from('Hello, ACTR!');
  const response = await actorRef.call(
    'echo_twice.EchoTwiceService.EchoTwice',
    PayloadType.RpcReliable,
    request,
    5000
  );

  console.log('Response:', response.toString());
  await actorRef.stop();
}

main().catch(console.error);
```

## Configuration

Create an `Actr.toml` configuration file:

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

The example clients use generated files under `examples/**/generated`, which are **git-ignored**.  
After cloning the repository, you **must run the codegen script** before running the examples.

Prerequisites:
- `npm install` (installs `protobufjs` and `@iarna/toml` from devDependencies)

Generate for echo-client:

```bash
npm run codegen -- --config examples/echo-client/Actr.toml
```

Notes:
- The generator reads `Actr.lock.toml` first; ensure it includes all dependencies you want emitted.
- Proto sources default to `examples/echo-client/protos/remote`.
Outputs include:
- `<package>.pb.ts` protobuf codecs
- `<package>.client.ts` route helpers
- `local.actor.ts` local forwarding glue

## API Documentation

### ActrSystem

Entry point for creating an ACTR system.

- `ActrSystem.fromConfig(configPath: string): Promise<ActrSystem>` - Create system from config file
- `system.attach(workload: Workload): ActrNode` - Attach a workload

### ActrNode

Represents an actor node before it's started.

- `node.start(): Promise<ActrRef>` - Start the node and get an actor reference

### ActrRef

Reference to a running actor.

- `actorRef.actorId(): ActrId` - Get the actor's ID
- `actorRef.discover(targetType: ActrType, count: number): Promise<ActrId[]>` - Discover actors
- `actorRef.call(routeKey, payloadType, payload, timeoutMs): Promise<Buffer>` - RPC call
- `actorRef.tell(routeKey, payloadType, payload): Promise<void>` - Fire-and-forget message
- `actorRef.shutdown(): void` - Trigger shutdown
- `actorRef.waitForShutdown(): Promise<void>` - Wait for shutdown
- `actorRef.stop(): Promise<void>` - Shutdown and wait

### Workload Interface

Implement this interface to define actor behavior:

```typescript
interface Workload {
  onStart(ctx: Context): Promise<void>;
  onStop(ctx: Context): Promise<void>;
  dispatch(ctx: Context, envelope: RpcEnvelope): Promise<Buffer>;
}
```

## Building from Source

### Prerequisites

- Node.js >= 16
- Rust >= 1.88
- Cargo

### Build Steps

```bash
# Install dependencies
npm install

# Build native module (debug)
npm run build:debug

# Build native + TypeScript layer (release); use this before running examples
npm run build

# Compile TypeScript only (if native already built)
npm run compile:ts

# Run tests
npm test

# Run examples (from repo root; run `npm run build` first)
node --import tsx examples/echo-twice-server/index.ts
node --import tsx examples/echo-client/index.ts
```

## Examples

See the [examples](./examples) directory for complete examples:

- [echo-twice-server](./examples/echo-twice-server) - EchoTwice server
- [echo-client](./examples/echo-client) - Echo client with discovery

## Reference implementations

This codebase follows the same architecture and API patterns as:

- **[libactr](../libactr)** – Rust FFI layer (UniFFI) that wraps the ACTR runtime. The actr-ts Rust side mirrors its module layout: `types`, `runtime`, `workload`, `context`, `error`, `logger`.
- **[actr-swift](../actr-swift)** – Swift SDK that consumes libactr bindings. The TypeScript layer (e.g. `ActrSystem`, `ActrNode`, `ActrRef`, `fromConfig`, `callTyped`, `stop()`) is aligned with actr-swift's high-level API in `Sources/Actr/`.

When changing core behavior or adding APIs, consider keeping parity with libactr and actr-swift where applicable.

## Development

This project uses:

- [napi-rs](https://napi.rs/) for Rust-Node.js bindings
- [ACTR framework](https://github.com/actor-rtc/actr) for actor runtime
- TypeScript for high-level API

## License

Apache-2.0

## Contributing

Contributions are welcome! Please open an issue or pull request on the [GitHub repository](https://github.com/actor-rtc/actr-ts).

## Links

- [ACTR Framework](https://github.com/actor-rtc/actr)
- [Documentation](https://docs.actor-rtc.org)
- [Examples](https://github.com/actor-rtc/actr-examples)
