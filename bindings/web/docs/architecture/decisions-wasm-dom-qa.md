# WASM + DOM 架构设计讨论记录

**日期**: 2025-11-11
**参与者**: 架构团队
**类型**: 技术决策讨论

---

## 讨论背景

在 actr-web 项目开发中，我们面临一个核心架构问题：

**如何在 WASM 无法直接访问 DOM/WebRTC API 的约束下，实现用户代码的统一性（避免 Rust + JS 割裂）？**

---

## 问题陈述

### 原始诉求

1. **用户代码全部 WASM 化**
   - 业务逻辑（包括 Fast Path 处理）统一用 Rust/Go/C++ 编写
   - 编译为 WASM，在浏览器中运行
   - 支持多语言生态

2. **避免体验割裂**
   - 不希望用户同时写 Rust（Service Worker）和 JS（DOM）
   - 提供统一的开发范式

3. **保持性能**
   - Fast Path 需要低延迟（目标 <15ms）
   - State Path ~30-40ms 可接受

### 核心矛盾

```
用户需求：所有代码在 WASM（包括 Fast Path）
        ↓
技术约束：WASM 无法访问 WebRTC API
        ↓
现实限制：WebRTC 只能在 DOM 上下文创建
        ↓
关键问题：Fast Path 数据如何处理？
```

---

## 方案探讨过程

### 讨论的方案

#### 方案 A：分层实现（割裂）
- Service Worker: WASM (State Path)
- DOM: 手工 JS (Fast Path)
- **结论**: 体验割裂，不可接受

#### 方案 B：WASM + JS 绑定层
- 用 Rust 封装 WebRTC API
- **结论**: 技术复杂度极高，"生命周期地狱"，不可行

#### 方案 C：统一在 WASM ✅ **最终采纳**
- DOM 侧固定转发层（框架提供）
- 所有用户代码在 Service Worker WASM
- Fast Path 数据通过 PostMessage 转发
- **结论**: 平衡统一性和性能，可接受

---

## 关键讨论要点

### Q1: 谁触发谁？UI 和 WASM 的交互模式？

**回答**：

- **启动流程**: UI 启动 → 初始化 WASM Runtime → UI 获得 runtime 引用
- **调用方向**:
  - UI → WASM: 通过 `actorRef.call()` (命令式)
  - WASM → UI: 通过 `eventEmitter.publish()` (事件驱动)

**心智模型**：
> WASM 是"运行在浏览器里的后端服务"，UI 是"前端"

### Q2: ActorRef API 设计？

**回答**：设计三个核心 API

| API | 方向 | 用途 | 延迟 |
|-----|------|------|------|
| `call()` | UI → WASM → UI | RPC 调用 | 30-40ms |
| `subscribe()` | WASM → UI | 数据流订阅 | 6-13ms |
| `on()` | WASM → UI | 系统事件 | <5ms |

**使用示例**：
```typescript
// 启动通话（call - RPC）
await actorRef.call('video-service', 'startCall', { peerId });

// 订阅视频流（subscribe - Fast Path）
const unsub = await actorRef.subscribe('video-stream', (frame) => {
    renderToCanvas(frame);
});

// 监听连接状态（on - 系统事件）
actorRef.on('connection-state-changed', (state) => {
    setStatus(state);
});
```

### Q3: Fast Path 数据要不要给 UI？

**回答**：按需通知，分场景

- **场景 A：视频通话**（需要渲染）
  - WASM 解码 → emit 给 UI → Canvas 渲染

- **场景 B：文件下载**（UI 只需进度）
  - WASM 内部处理 → 只 emit 进度百分比

- **场景 C：AI 推理**（完全内部）
  - WASM 内部处理 → 只在异常时 emit 告警

**原则**：
- Fast Path 数据默认不发给 UI（避免性能开销）
- 按需通知：只发送 UI 真正需要的内容
- 批量/采样：如果必须发送，使用采样（每 10 帧发 1 帧）

### Q4: 延迟从 1-3ms 增加到 6-13ms 可接受吗？

**回答**：视场景而定，但整体可接受

**延迟对比**：
```
原生 DOM 处理:     1-3ms   (未采纳)
DOM → SW 转发:     6-13ms  (采纳方案) ✅
State Path:       30-40ms (对比参考)
```

**关键洞察**：
- 6-13ms 仍然是 "Fast Path"（比 State Path 快 3-5 倍）
- 大多数应用场景（视频通话、文件传输）可以接受
- 换取的是**用户代码 100% 统一**和**多语言支持**

### Q5: 如何支持多语言（Rust/Go/C++）？

**回答**：定义统一的 WASM 接口

```
用户代码（Go）      用户代码（Rust）    用户代码（C++）
    ↓                  ↓                   ↓
TinyGo → WASM      rustc → WASM      Emscripten → WASM
    ↓                  ↓                   ↓
        统一的 WASM Interface
              ↓
      Service Worker Runtime
```

**关键**：类似 WASI，定义统一的入口点和回调接口

---

## 最终架构决策

### 核心设计原则

1. **职责分离**：
   - UI 层：视图渲染、用户交互
   - WASM 层：所有业务逻辑（State Path + Fast Path）
   - DOM 固定层：WebRTC 管理、数据转发（框架提供）

2. **谁驱动谁**：
   - UI 是启动者（初始化 WASM）
   - WASM 是服务层（提供 API）
   - DOM 是 HAL（硬件抽象层）

3. **数据流向**：
   - UI → WASM: 命令式调用
   - WASM → UI: 事件驱动推送
   - DOM → WASM: 数据转发（Fast Path）

### 关键心智模型

> **把 DOM 侧当作"网卡驱动"，Service Worker 侧当作"应用代码"**
>
> 没人要求驱动程序也用 Rust 写 —— **抽象的边界就是价值所在**

### 性能权衡

**得到**：
- ✅ 用户代码 100% 统一（全 WASM）
- ✅ 支持多语言（Rust/Go/C++）
- ✅ 架构清晰简洁
- ✅ DOM 侧用户无需写代码

**付出**：
- ⚠️ Fast Path 延迟增加 5-10ms（1-3ms → 6-13ms）
- ⚠️ PostMessage 序列化开销（通过 Transferable 缓解）

**结论**：权衡合理，可接受

---

## 实施要点

### DOM 侧固定实现（关键）

**设计原则**：
- DOM 侧是框架提供的固定 JS 代码
- 用户只需引入 `<script src="actr-dom.min.js"></script>`
- 无需修改或扩展

**核心组件**：
```javascript
// packages/actr-dom/src/
├── webrtc_coordinator.ts    // WebRTC 管理
├── fast_path_forwarder.ts   // 数据转发（零拷贝）
└── sw_bridge.ts              // PostMessage 通信
```

### 性能优化策略

1. **使用 Transferable**：零拷贝转移 ArrayBuffer
2. **批量转发**：减少 PostMessage 次数
3. **采样推送**：不是每帧都推送给 UI
4. **按需订阅**：UI 只订阅真正需要的数据流

### 多语言支持路径

**Phase 1**：Rust 优先（立即）
**Phase 2**：定义 WASM 接口规范（3 个月）
**Phase 3**：验证 TinyGo 和 Emscripten（6 个月）

---

## 后续行动

### 立即执行（本周）

1. ✅ 编写架构设计文档（本文档）
2. ⏳ 实现 DOM 侧固定 JS 层
   - WebRTC Coordinator
   - Fast Path Forwarder
   - PostMessage Bridge

### 近期计划（本月）

3. ⏳ 实现 Service Worker 侧 Fast Path Registry
4. ⏳ 实现 `call/subscribe/on` 三个 API
5. ⏳ 端到端测试（验证延迟和正确性）

### 长期跟踪（3-6 个月）

6. ⏳ 性能优化（批量、采样、压缩）
7. ⏳ 多语言支持验证
8. ⏳ 编写用户文档和示例

---

## 经验总结

### 架构设计经验

1. **不要追求完美的统一性**
   - 方案 B（WASM + JS 绑定层）理论上完美，但实践中不可行
   - 方案 C 在抽象边界处妥协，反而获得更好的平衡

2. **性能要有具体指标**
   - 不是"越快越好"，而是"满足场景需求"
   - 6-13ms vs 1-3ms 的差异，在大多数场景下不重要
   - 但 6-13ms vs 30-40ms 的差异，对 Fast Path 至关重要

3. **心智模型很重要**
   - "WASM 是后端服务"的类比，让架构决策变得清晰
   - "DOM 是网卡驱动"的类比，合理化了抽象边界

### 技术决策经验

1. **先问场景，再问技术**
   - 用户主要场景是什么？（客户端模式 90%）
   - Fast Path 真的需要 <5ms 吗？（大多数不需要）
   - 多语言支持是核心诉求吗？（重要但非必须）

2. **用数据说话**
   - 延迟对比：1-3ms vs 6-13ms vs 30-40ms
   - 代码复用率：方案 A 50% vs 方案 C 100%
   - 开发成本：方案 B 20 周 vs 方案 C 5 周

3. **保持灵活性**
   - 提供渐进式能力暴露（Level 1/2/3）
   - 保留"逃生舱"（可选的极致性能模式）
   - 架构支持未来演进（多语言接口预留）

---

## 相关文档

- **[WASM-DOM 集成架构](./wasm-dom-integration.md)** - 完整技术设计
- **[技术决策记录](./decisions.md)** - 其他 TDR
- **[双层架构设计](./dual-layer.md)** - State Path vs Fast Path
- **[架构总览](./overview.md)** - 整体架构

---

**文档维护**: 本文档记录了架构设计过程中的关键讨论和决策依据，供后续回顾和新成员理解设计意图。
