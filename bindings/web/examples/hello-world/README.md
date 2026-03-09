# Hello World 示例

最简单的 Actor-RTC Web 示例，演示如何创建客户端并调用 Echo 服务。

## 运行示例

```bash
# 安装依赖
pnpm install

# 构建 Service Worker Runtime（生成 actr_runtime_sw.js/wasm）
./prepare-sw.sh

# 启动开发服务器
pnpm dev
```

在浏览器中打开 http://localhost:5173

## 项目结构

```
hello-world/
├── actr.toml              # Actor 配置文件
├── package.json
├── vite.config.ts
├── index.html
├── public/
│   ├── actor.sw.js        # Service Worker 入口（自动生成）
│   ├── actr_runtime_sw.js # WASM Runtime JS
│   └── actr_runtime_sw_bg.wasm  # WASM 文件
└── src/
    ├── main.ts            # 应用入口
    └── generated/         # 自动生成的代码
        ├── actr-config.ts    # 配置
        ├── echo-service.actorref.ts  # Echo 服务客户端
        ├── index.ts          # 导出入口
        └── remote/           # 依赖服务的类型定义
```

## 代码说明

### main.ts

```typescript
import { createActorClient } from '@actr/web';
import { actrConfig, EchoServiceActorRef } from './generated';

// 1. 使用生成的配置创建客户端
const client = await createActorClient(actrConfig);

// 2. 创建类型安全的服务客户端
const echoService = new EchoServiceActorRef(client);

// 3. 调用服务方法（完全类型安全）
const response = await echoService.echo({
  message: 'Hello, Actor-RTC!'
});

console.log('Reply:', response.reply);      // "Echo: Hello, Actor-RTC!"
console.log('Timestamp:', response.timestamp);
```

## 配置说明

### actr.toml

```toml
[package]
name = "echo-real-client-app"

[package.actr_type]
manufacturer = "acme"
name = "echo-client-app"

[dependencies]
echo-echo-server = { actr_type = "acme+EchoService" }

[system.signaling]
url = "wss://actrix1.develenv.com/signaling/ws"

[system.deployment]
realm_id = 2368266035
```

## 代码生成

使用 `actr gen` 命令生成 TypeScript 代码：

```bash
actr gen -l typescript
```

这将生成：
- `actr-config.ts` - 从 actr.toml 提取的配置
- `echo-service.actorref.ts` - 类型安全的 Echo 服务客户端
- `actor.sw.js` - Service Worker 入口（放入 public/ 目录）

## 架构说明

```
┌─────────────────────────────────────────────────────────────────┐
│                        Browser (UI 线程)                         │
│  ┌──────────┐                                                   │
│  │ main.ts  │ ──────────────────────────────────┐              │
│  │ (App)    │                                    ↓              │
│  └──────────┘                            ┌─────────────┐        │
│                                          │ @actr/web   │        │
│                                          │  (SDK)      │        │
│                                          └──────┬──────┘        │
│                                                 │               │
│  - - - - - - - - - - - - - - - - - PostMessage -│- - - - - - -  │
│                                                 ↓               │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                   Service Worker                          │   │
│  │  ┌─────────────┐    ┌──────────────────────────────┐    │   │
│  │  │ actor.sw.js │───→│ actr_runtime_sw (WASM)       │    │   │
│  │  │  (入口)     │    │  - Mailbox (IndexedDB)       │    │   │
│  │  └─────────────┘    │  - WebSocket (Signaling)     │    │   │
│  │                     │  - WebRTC DataChannel        │    │   │
│  │                     └──────────────────────────────┘    │   │
│  └──────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
                                    │
                                    │ WebRTC DataChannel
                                    ↓
                    ┌───────────────────────────────┐
                    │     Echo Server (Rust)        │
                    │  (actr-examples/shell-actr-   │
                    │      echo/server)             │
                    └───────────────────────────────┘
```

这个简单的示例展示了:
1. 如何创建 Actor 客户端
2. 如何调用远程服务方法
3. 如何处理响应

## 下一步

查看更复杂的示例:
- [react-echo](../react-echo/) - React 集成
- [todo-app](../todo-app/) - CRUD 应用
- [chat-app](../chat-app/) - 实时聊天
