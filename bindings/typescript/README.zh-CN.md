# actr-ts

中文 | [English](./README.md)
Language: 中文（简体）

actr-ts 是基于 napi-rs 的 ACTR（Actor-RTC）框架 TypeScript/Node.js 绑定。

## 概述

actr-ts 提供 ACTR 框架的原生 Node.js 绑定，使 TypeScript/JavaScript 开发者能够构建具备 WebRTC 能力的 actor 分布式系统。

## 特性

- 🚀 基于 Rust 与 napi-rs 的原生性能
- 📦 类型安全的 TypeScript API
- 🔄 Actor 并发模型
- 🌐 内置服务发现
- 📡 RPC 与流式支持
- 🔍 OpenTelemetry 可观测性

## 安装

```bash
npm install @actor-rtc/actr
```

## 快速开始

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

## 配置

创建 `Actr.toml` 配置文件：

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

Language: zh-CN.

示例客户端使用 `examples/**/generated` 下的生成文件，这些目录 **已经加入 .gitignore**。  
克隆仓库后，**需要先运行 codegen 脚本生成这些文件**，再运行示例（不需要 Actr CLI）。

前置条件：
- `npm install`（从 devDependencies 安装 `protobufjs` 与 `@iarna/toml`）

为 echo-client 生成：

```bash
npm run codegen -- --config examples/echo-client/Actr.toml
```

注意：
- 生成器优先读取 `Actr.lock.toml`；请确保包含你想生成的依赖。
- proto 默认来源为 `examples/echo-client/protos/remote`。

输出包括：
- `<package>.pb.ts` protobuf 编解码
- `<package>.client.ts` 路由辅助
- `local.actor.ts` 本地转发逻辑

## API 文档

### ActrSystem

创建 ACTR 系统的入口。

- `ActrSystem.fromConfig(configPath: string): Promise<ActrSystem>` - 从配置文件创建系统
- `system.attach(workload: Workload): ActrNode` - 绑定 workload

### ActrNode

启动前的 actor 节点。

- `node.start(): Promise<ActrRef>` - 启动节点并返回 actor 引用

### ActrRef

运行中的 actor 引用。

- `actorRef.actorId(): ActrId` - 获取 actor ID
- `actorRef.discover(targetType: ActrType, count: number): Promise<ActrId[]>` - 发现 actor
- `actorRef.call(routeKey, payloadType, payload, timeoutMs): Promise<Buffer>` - RPC 调用
- `actorRef.tell(routeKey, payloadType, payload): Promise<void>` - 仅发送不等待
- `actorRef.shutdown(): void` - 触发关闭
- `actorRef.waitForShutdown(): Promise<void>` - 等待关闭
- `actorRef.stop(): Promise<void>` - 关闭并等待

### Workload 接口

实现该接口以定义 actor 行为：

```typescript
interface Workload {
  onStart(ctx: Context): Promise<void>;
  onStop(ctx: Context): Promise<void>;
  dispatch(ctx: Context, envelope: RpcEnvelope): Promise<Buffer>;
}
```

## 从源码构建

### 前置条件

- Node.js >= 16
- Rust >= 1.88
- Cargo

### 构建步骤

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

## 示例

完整示例见 [examples](./examples) 目录：

- [echo-twice-server](./examples/echo-twice-server) - EchoTwice server
- [echo-client](./examples/echo-client) - Echo client with discovery

## 参考实现

该代码库与以下项目保持架构与 API 模式一致：

- **[libactr](../libactr)** – Rust FFI 层（UniFFI），封装 ACTR runtime。actr-ts 的 Rust 侧与其模块布局保持一致：`types`、`runtime`、`workload`、`context`、`error`、`logger`。
- **[actr-swift](../actr-swift)** – 基于 libactr 的 Swift SDK。TypeScript 层（如 `ActrSystem`、`ActrNode`、`ActrRef`、`fromConfig`、`callTyped`、`stop()`）与 actr-swift 的高层 API 保持一致。

当修改核心行为或新增 API 时，建议同步考虑 libactr 与 actr-swift 的一致性。

## 开发

本项目使用：

- [napi-rs](https://napi.rs/) 作为 Rust-Node.js 绑定
- [ACTR framework](https://github.com/actor-rtc/actr) 作为 actor runtime
- TypeScript 作为高层 API

## 许可证

Apache-2.0

## 贡献

欢迎贡献！请在 [GitHub repository](https://github.com/actor-rtc/actr-ts) 提交 issue 或 PR。

## 链接

- [ACTR Framework](https://github.com/actor-rtc/actr)
- [Documentation](https://docs.actor-rtc.org)
- [Examples](https://github.com/actor-rtc/actr-examples)
