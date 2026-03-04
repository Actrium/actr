# 快速开始

本指南将帮助您在 10 分钟内开始使用 Actor-RTC Web。

## 前置要求

- Node.js 18+
- npm 9+
- 现代浏览器（Chrome/Firefox/Safari 最新 2 个版本）

---

## 两种使用模式

### 🎨 模式 1：客户端模式（推荐开始）

作为客户端调用远程 Actor 服务。适合大多数应用场景。

### ⚙️ 模式 2：Runtime 模式（高级）

在浏览器中运行完整的 Actor Runtime。适合需要在浏览器侧运行复杂逻辑的场景。

---

# 客户端模式

## 安装

### 1. 安装依赖

```bash
npm install @actr/web
```

如果您使用 React:

```bash
npm install @actr/web @actr/web-react
```

### 2. 配置构建工具

#### 使用 Vite (推荐)

```bash
npm install --save-dev vite-plugin-wasm vite-plugin-top-level-await
```

配置 `vite.config.ts`:

```typescript
import { defineConfig } from 'vite';
import wasm from 'vite-plugin-wasm';
import topLevelAwait from 'vite-plugin-top-level-await';

export default defineConfig({
  plugins: [wasm(), topLevelAwait()],
  server: {
    headers: {
      'Cross-Origin-Opener-Policy': 'same-origin',
      'Cross-Origin-Embedder-Policy': 'require-corp',
    },
  },
});
```

## 基础使用

### 创建 Actor

```typescript
import { createActor } from '@actr/web';

// 创建 Actor
const actor = await createActor({
  signalingUrl: 'wss://signal.example.com',
  realm: 'demo',
});

console.log('Connected!');
```

### 调用服务

```typescript
// 调用 Echo 服务
const response = await actor.call('echo-service', 'sendEcho', {
  message: 'Hello, Actor-RTC!',
});

console.log('Reply:', response.reply);
```

### 订阅数据流

```typescript
// 订阅实时数据
const unsubscribe = await actor.subscribe(
  'metrics-service',
  (data) => {
    console.log('CPU:', data.cpu);
  }
);

// 取消订阅
await unsubscribe();
```

## React 集成

### 使用 Hooks

```tsx
import { useActor, useServiceCall } from '@actr/web-react';

function App() {
  // 创建 Actor
  const { actor, loading, error } = useActor({
    signalingUrl: 'wss://signal.example.com',
    realm: 'demo',
  });

  // 调用服务
  const { call, data, loading: calling } = useServiceCall(
    actor,
    'echo-service',
    'sendEcho'
  );

  if (loading) return <div>连接中...</div>;
  if (error) return <div>错误: {error.message}</div>;

  return (
    <div>
      <button onClick={() => call({ message: 'Hello!' })} disabled={calling}>
        发送
      </button>
      {data && <div>回复: {data.reply}</div>}
    </div>
  );
}
```

## 完整示例

查看 `examples/` 目录获取完整的示例项目:

- **hello-world**: 最简单的 Echo 示例
- **echo**: 完整的 gRPC-Web Echo 示例（真实网络通信）
- **codegen-test**: 代码生成测试示例

---

# Runtime 模式（高级）

如果你需要在浏览器中运行完整的 Actor Runtime（Service Worker + DOM），请参考以下指南：

## Service Worker 侧初始化

```rust
use actr_runtime_sw::*;
use std::sync::Arc;

#[wasm_bindgen]
pub async fn init_sw() -> Result<(), JsValue> {
    // 初始化 tracing
    actr_runtime_sw::trace::init_tracing();

    // 1. Transport (发送)
    let wire_builder = Arc::new(WebWireBuilder::new());
    let manager = Arc::new(OutprocTransportManager::new(
        "sw-id",
        wire_builder.clone(),
    ));

    // 2. Mailbox (State Path)
    let mailbox = Arc::new(IndexedDbMailbox::new().await?);

    // 3. InboundPacketDispatcher (接收)
    let dispatcher = Arc::new(InboundPacketDispatcher::new(mailbox.clone()));

    // 4. MailboxProcessor (处理)
    let mut processor = MailboxProcessor::new(mailbox.clone(), 10);
    processor.set_handler(Arc::new(|msg| {
        Box::pin(async move {
            log::info!("Processing: {}", msg.id);
            Ok(())
        })
    }));
    processor.start();

    Ok(())
}
```

## DOM 侧初始化

```rust
use actr_runtime_dom::*;
use std::sync::Arc;

#[wasm_bindgen(start)]
pub async fn start() -> Result<(), JsValue> {
    init_dom_runtime();

    // 1. 注册器
    let stream_registry = Arc::new(StreamHandlerRegistry::new());
    let media_registry = Arc::new(MediaFrameHandlerRegistry::new());

    // 2. WebRTC 接收器
    let receiver = Arc::new(WebRtcDataChannelReceiver::new(
        stream_registry.clone(),
        media_registry.clone(),
    ));

    // 3. 注册 Stream 回调
    stream_registry.register("video-1", Arc::new(|data| {
        // 处理视频数据
    }));

    Ok(())
}
```

## 性能优化建议

### RPC 消息
- **推荐**: WebSocket (SW 侧)
- **原因**: 直接 Mailbox，无需转发

### Stream 数据
- **推荐**: WebRTC DataChannel (DOM 侧)
- **原因**: 本地处理，延迟最低（~1-2ms）

### Media 数据
- **推荐**: WebRTC MediaTrack
- **原因**: 原生支持，零拷贝（< 1ms）

---

## 下一步

- 查看 [架构文档](./architecture/)
- 浏览 [示例代码](../examples/)

## 故障排查

遇到问题? 查看 [故障排查指南](./troubleshooting.md)
