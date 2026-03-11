# Echo Server

Actor-RTC 浏览器端服务器示例，演示如何使用 `@actr/web` 统一 Actor API 创建运行在浏览器中的服务。

## 架构

```
┌─────────────────────────────────────────────────────────────────┐
│                    浏览器端 Echo Server                          │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │                         DOM                                │  │
│  │  ┌──────────────────┐   ┌──────────────────────────────┐ │  │
│  │  │  main.ts         │   │  @actr/dom                   │ │  │
│  │  │  - createActor   │   │  - WebRTC 管理               │ │  │
│  │  │  - UI 更新       │   │  - DataChannel               │ │  │
│  │  └──────────────────┘   └──────────────────────────────┘ │  │
│  └──────────────────────────────────────────────────────────┘  │
│                              ↑ PostMessage                      │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │                   Service Worker                          │  │
│  │  ┌──────────────────────────────────────────────────────┐ │  │
│  │  │  WASM Runtime                                        │ │  │
│  │  │  - RPC 解码/编码                                     │ │  │
│  │  │  - 消息路由                                          │ │  │
│  │  │  - Signaling 通信                                    │ │  │
│  │  └──────────────────────────────────────────────────────┘ │  │
│  └──────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
                              ↑ WebRTC DataChannel
┌─────────────────────────────────────────────────────────────────┐
│                    Echo Client (另一个浏览器)                    │
└─────────────────────────────────────────────────────────────────┘
```

## RPC 消息接收流程 (State Path)

```
1. WebRTC DataChannel 接收二进制数据
2. 数据通过 PostMessage 转发到 Service Worker
3. WASM Runtime 解码 RpcEnvelope
4. 提取 service, method, payload
5. 路由到 WASM 中注册的 service handler
6. Rust EchoService Workload 处理业务逻辑
7. 响应编码为 RpcEnvelope
8. 通过 DataChannel 返回给调用方
```

## 快速开始

```bash
# 安装依赖
pnpm install

# 启动开发服务器
pnpm dev

# 构建生产版本
pnpm build
```

## Actor 创建方式

```typescript
import { createActor } from '@actr/web';

const actor = await createActor({
  signalingUrl: 'wss://signal.example.com',
  realm: 12345,
  serviceWorkerPath: '/actor.sw.js',
});

console.log('Echo Server started successfully');
```

实际 RPC 处理由 WASM Service Worker 中的 Rust EchoService Workload 完成，
TypeScript 侧只需创建 Actor 并初始化 SW Bridge + WebRTC 连接。

## 文件结构

```
server/
├── index.html           # HTML 入口
├── package.json         # NPM 配置
├── vite.config.ts       # Vite 配置
├── tsconfig.json        # TypeScript 配置
├── actr.toml            # Actor 配置
├── proto/
│   └── echo.proto       # Protobuf 定义
└── src/
    ├── main.ts          # 主入口
    └── generated/
        ├── index.ts
        └── actr-config.ts   # 生成的配置
```

## 与 hello-world 客户端配合

1. 启动此 Echo Server: `pnpm dev` (端口 5174)
2. 在另一个终端启动 hello-world 客户端: `cd ../../../hello-world && pnpm dev` (端口 5173)
3. 打开客户端页面，发送 Echo 请求
4. 观察服务器页面的日志和统计信息
