# @actr/dom

**Actor-RTC DOM-side Fixed Forwarding Layer**

这是 Actor-RTC 框架提供的固定 JavaScript 层（Hardware Abstraction Layer），负责 DOM 侧的 WebRTC 管理和数据转发。

## 设计理念

> **DOM 侧 = "网卡驱动"，Service Worker 侧 = "应用代码"**

用户的所有业务逻辑都在 Service Worker（WASM）中实现，DOM 侧只是框架提供的固定实现，用户无需修改。

## 核心职责

1. **WebRTC 连接管理** - 创建和管理 RTCPeerConnection（只有 DOM 上下文才能访问 WebRTC API）
2. **Fast Path 数据转发** - 将 WebRTC DataChannel 接收的数据零拷贝转发到 Service Worker
3. **PostMessage 通信桥梁** - 提供 DOM 与 Service Worker 的双向通信

## 安装

```bash
npm install @actr/dom
```

## 使用方法

### 基础用法

```typescript
import { initActrDom } from '@actr/dom';

// 初始化 DOM 运行时
const runtime = await initActrDom({
  serviceWorkerUrl: '/my-actor.sw.js',  // Service Worker 文件路径
  webrtcConfig: {
    iceServers: [{ urls: 'stun:stun.l.google.com:19302' }],
  },
});

console.log('Actor-RTC DOM runtime initialized');

// 运行时会自动处理：
// 1. 注册 Service Worker
// 2. 建立 PostMessage 通信
// 3. 监听来自 SW 的 WebRTC 命令
// 4. 转发 Fast Path 数据到 SW
```

### HTML 引入

```html
<!DOCTYPE html>
<html>
<head>
  <title>Actor-RTC App</title>
</head>
<body>
  <div id="app"></div>

  <!-- 引入 DOM 运行时 -->
  <script type="module">
    import { initActrDom } from 'https://cdn.example.com/@actr/dom/dist/index.js';

    const runtime = await initActrDom({
      serviceWorkerUrl: '/worker.js',
    });

    // 您的 UI 代码...
  </script>
</body>
</html>
```

## API 参考

### `initActrDom(config)`

初始化 Actor-RTC DOM 运行时。

**参数**：
- `config.serviceWorkerUrl` (string) - Service Worker 文件路径
- `config.webrtcConfig` (object, 可选) - WebRTC 配置
  - `iceServers` (RTCIceServer[]) - ICE 服务器列表
  - `iceTransportPolicy` (RTCIceTransportPolicy) - ICE 传输策略

**返回**：`Promise<ActrDomRuntime>`

### `ActrDomRuntime`

DOM 运行时实例。

**方法**：
- `getSWBridge()` - 获取 Service Worker 桥接
- `getForwarder()` - 获取 Fast Path 转发器
- `getCoordinator()` - 获取 WebRTC 协调器
- `dispose()` - 清理所有资源

## 架构设计

详见：[WASM-DOM 集成架构](../../docs/architecture/wasm-dom-integration.md)

### 数据流

```
WebRTC 数据到达 DOM
  ↓
WebRtcCoordinator 接收
  ↓
FastPathForwarder 零拷贝转发（Transferable ArrayBuffer）
  ↓
PostMessage → Service Worker WASM
  ↓
Fast Path Registry.dispatch()
  ↓
用户回调（Rust）
```

### 性能特性

- **零拷贝传输**：使用 Transferable ArrayBuffer
- **批量转发**：可配置批量参数减少 PostMessage 次数
- **目标延迟**：~6-13ms（vs State Path 30-40ms）

## 组件说明

### ServiceWorkerBridge

负责 DOM 与 Service Worker 的 PostMessage 通信。

### FastPathForwarder

负责将 WebRTC DataChannel 数据转发到 Service Worker。

支持两种模式：
- `forward()` - 立即转发单条数据
- `forwardBatch()` - 批量转发（高吞吐场景）

### WebRtcCoordinator

负责管理 WebRTC 连接和 DataChannels。

**核心功能**：
- 创建 RTCPeerConnection
- 创建 4 个 negotiated DataChannels（对应 4 种 PayloadType）
- 处理 SDP Offer/Answer 交换
- 处理 ICE Candidate
- 自动转发接收的数据

## 开发

```bash
# 安装依赖
npm install

# 编译
npm run build

# 监听模式
npm run watch

# 清理
npm run clean
```

## 相关文档

- [架构总览](../../docs/architecture/overview.md)
- [WASM-DOM 集成架构](../../docs/architecture/wasm-dom-integration.md)（核心）
- [双层架构设计](../../docs/architecture/dual-layer.md)

## License

Apache-2.0

---

**维护者**: Actor-RTC Team
**版本**: 0.1.0
**最后更新**: 2025-11-11
