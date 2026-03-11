# Actor-RTC Web 文档

**Actor-RTC Web** 是 Actor-RTC 框架的浏览器端实现，基于 WebAssembly 和 Service Worker 技术，提供与原生版本一致的 Actor 模型编程体验。

---

## 📖 用户文档

面向使用 Actor-RTC Web 构建应用的开发者。

### 快速开始

- **[快速开始指南](./getting-started.md)** ⭐ 推荐先读
  - 客户端模式：调用远程 Actor 服务
  - Runtime 模式：浏览器内运行 Actor Runtime
  - React 集成和完整示例

### 故障排查

- **[故障排查指南](./troubleshooting.md)**
  - 常见问题和解决方案
  - 调试技巧
  - 性能优化建议

---

## 📋 需求与规划文档

项目需求、目标和规划文档：

- **[Web 适配需求](./requirements.md)** - 完整功能需求和架构设计

---

## 🏗️ 架构文档

面向框架贡献者和希望深入了解内部实现的开发者。

### 核心设计

进入 **[architecture/](./architecture/)** 目录查看完整架构文档：

1. **[架构总览](./architecture/overview.md)** - 双进程模型和核心组件
2. **[双层架构设计](./architecture/dual-layer.md)** - State Path vs Fast Path
3. **[API 层设计](./architecture/api-layer.md)** - Gate/Context/ActrRef
4. **[技术决策记录](./architecture/decisions.md)** - 9 个关键技术决策 (TDR)
5. **[完成度评估](./architecture/completion-status.md)** - 相对于 actr Native 的完成度 (78%)

---

## 🚀 快速预览

### 基础用法

```typescript
import { createActor } from '@actr/web';

// 创建 Actor
const actor = await createActor({
  signalingUrl: 'wss://signal.example.com',
  realm: 'demo',
});

// 调用远端 Actor
const response = await actor.call('echo-service', 'sendEcho', {
  message: 'Hello, Actor-RTC!',
});
```

### Runtime 模式（高级）

在浏览器中运行完整的 Actor Runtime（Service Worker + DOM 双进程架构）：

```rust
// Service Worker 侧
use actr_runtime_sw::*;

let manager = Arc::new(PeerTransport::new(...));
let mailbox = Arc::new(IndexedDbMailbox::new().await?);
let dispatcher = Arc::new(InboundPacketDispatcher::new(mailbox));
```

```rust
// DOM 侧
use actr_runtime_dom::*;

let registry = Arc::new(StreamHandlerRegistry::new());
let receiver = Arc::new(WebRtcDataChannelReceiver::new(registry));
```

---

## 📊 当前状态

| 维度 | 完成度 | 说明 |
|------|--------|------|
| 核心架构层 | 85% | Transport + Message + 完整传输栈 |
| 持久化与调度 | 95% | Mailbox 完成，Scheduler 已实现 |
| Fast Path 支持 | 50% | 框架完成，集成待完善 |
| 整体完成度 | **78%** | 接近 MVP |

详见 [完成度评估](./architecture/completion-status.md)。

---

## 🔗 相关资源

- **示例代码**: `../examples/`
- **Crate 源码**: `../crates/`
- **原型实现**: `/d/actor-rtc/actr/` (Native Rust)
- **GitHub**: https://github.com/actor-rtc/actor-rtc

---

**维护者**: Actor-RTC Team
**最后更新**: 2026-02-28
