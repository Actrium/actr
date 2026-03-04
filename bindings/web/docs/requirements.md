# Actor-RTC Web 适配需求文档

## 文档信息

- **版本**: v0.1.0-draft
- **创建日期**: 2025-01-10
- **最后更新**: 2025-01-10
- **状态**: 需求分析阶段

---

## 一、项目概述

### 1.1 背景

Actor-RTC (actr) 是一个基于 Actor 模型的分布式实时通信框架，使用 Rust 实现，以 WebRTC 作为通信底座。当前版本主要面向原生平台（Linux, macOS, Windows），本文档描述将框架适配到 Web 平台（浏览器环境）的完整方案。

### 1.2 目标

将 Actor-RTC 核心能力移植到 Web 环境，通过 WebAssembly (WASM) 技术复用现有代码库，为 JavaScript/TypeScript 开发者提供类型安全、易用的 API。

### 1.3 核心价值

- **高代码复用率**: 85-90% 的核心逻辑可直接复用
- **类型安全**: Rust + TypeScript 双重类型保证
- **性能优势**: WASM 接近原生性能
- **统一体验**: 与原生 SDK 保持一致的开发范式

---

## 二、架构设计

### 2.1 整体架构图

```
┌─────────────────────────────────────────────────────────┐
│              Web Browser Environment                     │
│                                                          │
│  ┌────────────────────────────────────────────────────┐ │
│  │  Main Thread (用户界面)                            │ │
│  │  - React/Vue 组件                                  │ │
│  │  - 与 Worker 通信 (postMessage)                    │ │
│  └──────────────┬────────────────────────────────────┘ │
│                 │                                        │
│  ┌──────────────▼────────────────────────────────────┐ │
│  │  Service Worker / Web Worker                      │ │
│  │                                                    │ │
│  │  ┌──────────────────────────────────────────────┐ │ │
│  │  │  WASM Runtime                             │ │ │
│  │  │  (runtime-sw.wasm + runtime-dom.wasm)     │ │ │
│  │  │  ┌────────────────────────────────────────┐ │ │ │
│  │  │  │  ActorSystem (Rust)                    │ │ │ │
│  │  │  │  - Scheduler (复用)                    │ │ │ │
│  │  │  │  - Mailbox (IndexedDB 适配)            │ │ │ │
│  │  │  │  - Network (浏览器 API 绑定)           │ │ │ │
│  │  │  │  - Fast Path Registry (复用)           │ │ │ │
│  │  │  └────────────────────────────────────────┘ │ │ │
│  │  │                                              │ │ │
│  │  │  ┌────────────────────────────────────────┐ │ │ │
│  │  │  │  Workload (业务逻辑)                   │ │ │ │
│  │  │  │  - 完全复用现有代码                    │ │ │ │
│  │  │  └────────────────────────────────────────┘ │ │ │
│  │  └──────────────────────────────────────────────┘ │ │
│  │                                                    │ │
│  │  ┌──────────────────────────────────────────────┐ │ │
│  │  │  JS Glue Layer (wasm-bindgen 生成)          │ │ │
│  │  └──────────────────────────────────────────────┘ │ │
│  └────────────────────────────────────────────────────┘ │
│                                                          │
│  ┌────────────────────────────────────────────────────┐ │
│  │  Browser APIs                                      │ │
│  │  - WebRTC API, WebSocket, IndexedDB, Worker       │ │
│  └────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
```

### 2.2 层次结构

```
应用层 (JS/TS)
    ↓
高级 SDK (@actr/web)
    ↓
类型定义层 (自动生成)
    ↓
WASM 核心层 (runtime-sw.wasm + runtime-dom.wasm)
    ↓
浏览器 API
```

### 2.3 角色定位

Web Actor 支持两种角色模式：

- **客户端模式** (90% 场景): 连接后端 Actor 服务，调用远程服务
- **服务端模式** (10% 场景): 通过信令服务器注册服务，接受其他 Actor 连接，实现真正的 P2P 应用

**设计原则**: 初期专注客户端模式，架构上保留服务端能力的扩展性。

---

## 三、关键技术决策

### 3.1 异步运行时选择

#### 决策: 采用渐进式混合方案

**Phase 1: 最小 tokio (立即实施)**

```toml
[dependencies]
tokio = {
    version = "1",
    features = ["sync", "macros", "time"],
    default-features = false
}
```

**预期指标**:
- WASM 大小: ~350KB (gzip: ~120KB)
- 开发时间: 2 周
- 代码修改: <5%
- 风险: 极低

**Phase 2: 定制运行时 (长期优化)**

仅替换实际使用的 tokio 组件：
- `tokio::sync::mpsc` → 自定义 JS 驱动的 mpsc
- `tokio::sync::Mutex` → 自定义 JS Mutex
- `tokio::time` → `setTimeout`/`setInterval` 包装

**预期收益**:
- WASM 大小: ~250KB (gzip: ~85KB)
- 性能提升: 15-25%
- 内存减少: -2MB

#### 技术对比

| 方案 | WASM 大小 | 开发时间 | 代码复用率 | 维护成本 | 推荐度 |
|------|----------|---------|-----------|---------|--------|
| 最小 tokio | 350KB | 2 周 | 90% | 1x | ⭐⭐⭐⭐⭐ |
| 混合方案 | 250KB | 10 周 | 75% | 1.3x | ⭐⭐⭐⭐ |
| 纯 JS 重写 | 180KB | 20 周 | 50% | 2x | ⭐ |

### 3.2 持久化存储适配

#### SQLite → IndexedDB

**接口保持不变**: 复用现有 `Mailbox` trait

```rust
#[async_trait]
pub trait Mailbox: Send + Sync {
    async fn enqueue(&self, from: Vec<u8>, payload: Vec<u8>, priority: MessagePriority) -> StorageResult<Uuid>;
    async fn dequeue(&self) -> StorageResult<Vec<MessageRecord>>;
    async fn ack(&self, message_id: Uuid) -> StorageResult<()>;
    async fn status(&self) -> StorageResult<MailboxStats>;
}
```

**实现替换**:

```rust
// 原生平台
use actr_mailbox::SqliteMailbox;

// Web 平台
use actr_mailbox_web::IndexedDbMailbox;
```

**IndexedDB 特性**:
- ✅ 支持事务 (ACID 语义)
- ✅ 支持复合索引 (优先级调度)
- ✅ 异步 API (与 tokio 配合)
- ✅ 浏览器原生支持

### 3.3 网络层适配

#### Rust WebRTC → 浏览器 WebRTC API

通过 `web-sys` crate 绑定浏览器原生 API：

```rust
use web_sys::{RtcPeerConnection, RtcDataChannel};

pub struct WebRtcConnection {
    peer_connection: RtcPeerConnection,
    data_channels: DashMap<String, RtcDataChannel>,
}
```

**绑定层**:
- `wasm-bindgen`: Rust ↔ JavaScript 互操作
- `web-sys`: 浏览器 Web API 类型绑定
- `js-sys`: JavaScript 标准对象绑定

### 3.4 Worker 选择

| 特性 | Service Worker | Web Worker |
|------|---------------|------------|
| 生命周期 | 独立于页面 | 随页面销毁 |
| 后台运行 | 支持 | 不支持 |
| 拦截网络 | 支持 | 不支持 |
| 调试复杂度 | 较高 | 较低 |

**决策**:
- 生产环境: 优先 Service Worker (持久化、后台能力)
- 开发环境: 可选 Web Worker (调试方便)
- 提供配置选项，让开发者根据场景选择

---

## 四、代码复用策略

### 4.1 完全复用 (90%)

```rust
✅ actr-protocol     // Protobuf 定义
✅ actr-framework    // 核心 trait
✅ 业务逻辑 (Workload)
✅ 消息路由
✅ 双路径架构
✅ Fast Path Registry
✅ Scheduler 核心逻辑
```

### 4.2 接口复用，实现替换 (5%)

```rust
⚙️ Mailbox trait       // 接口不变，实现为 IndexedDB
⚙️ Network trait       // 接口不变，实现为浏览器 API
⚙️ 异步运行时抽象      // 统一接口，底层可选 tokio 或 JS
```

### 4.3 平台特定实现 (5%)

```rust
🆕 IndexedDB Mailbox
🆕 浏览器 WebRTC 绑定
🆕 Worker 管理
🆕 WASM 导出 API
```

---

## 五、API 设计

### 5.1 WASM 核心 API

#### Rust 端导出 (wasm-bindgen)

```rust
#[wasm_bindgen]
pub struct ActorSystem {
    inner: Arc<ActrSystemInner>,
}

#[wasm_bindgen]
impl ActorSystem {
    #[wasm_bindgen(constructor)]
    pub async fn new(config: ActorSystemConfig) -> Result<ActorSystem, JsValue>;

    #[wasm_bindgen]
    pub async fn call(&self, service: String, method: String, request: Vec<u8>)
        -> Result<Vec<u8>, JsValue>;

    #[wasm_bindgen]
    pub async fn tell(&self, service: String, method: String, message: Vec<u8>)
        -> Result<(), JsValue>;

    #[wasm_bindgen]
    pub async fn subscribe(&self, service: String, topic: String)
        -> Result<StreamHandle, JsValue>;

    #[wasm_bindgen]
    pub fn connection_state(&self) -> String;

    #[wasm_bindgen]
    pub async fn shutdown(&self) -> Result<(), JsValue>;
}
```

### 5.2 TypeScript 高级 SDK

#### 类型安全封装

```typescript
/**
 * 统一 Actor 主类
 */
export class Actor {
    /**
     * 调用原始 RPC（已编码 payload）
     */
    async callRaw(routeKey: string, payload: Uint8Array, timeout?: number): Promise<Uint8Array>;

    /**
     * 类型安全的 RPC 调用
     */
    async call<TRequest, TResponse>(
        service: string,
        method: string,
        request: TRequest,
        options?: RpcOptions
    ): Promise<TResponse>;

    /**
     * 订阅数据流
     */
    async subscribe<T>(
        topic: string,
        callback: (data: T) => void
    ): Promise<() => void>;

    /**
     * 关闭 Actor
     */
    async close(): Promise<void>;
}
```

#### 使用示例

```typescript
// 1. 创建 Actor
const actor = await createActor({
    signalingUrl: 'wss://signal.example.com',
    realm: 'demo',
});

// 2. 调用服务 (完全类型安全)
const response = await actor.call('echo-service', 'sendEcho', {
    message: 'Hello, World!',
});

console.log(response.reply); // TypeScript 自动补全

// 3. 订阅流
const unsubscribe = await actor.subscribe(
    'cpu-usage',
    (data: { cpu: number }) => {
        console.log(`CPU: ${data.cpu}%`);
    }
);
```

### 5.3 React Hooks 集成

```typescript
/**
 * 使用 Actor
 */
export function useActor(config: ActorConfig): {
    actor: Actor | null;
    loading: boolean;
    error: Error | null;
}

/**
 * 调用服务方法
 */
export function useServiceCall<TRequest, TResponse>(
    actor: Actor | null,
    service: string,
    method: string
): {
    call: (request: TRequest) => Promise<TResponse>;
    data: TResponse | null;
    loading: boolean;
    error: Error | null;
}

/**
 * 订阅数据流
 */
export function useSubscription<T>(
    actor: Actor | null,
    topic: string,
    enabled?: boolean
): {
    data: T[];
    error: Error | null;
}
```

---

## 六、类型生成流程

### 6.1 自动生成管道

```
.proto 文件
    ↓
protobuf.js 生成 TypeScript 类型
    ↓
wasm-bindgen 生成 WASM 绑定类型
    ↓
actr-cli 生成高级 SDK 封装
    ↓
完整 TypeScript 类型系统
```

### 6.2 生成文件结构

```
src/generated/
├── types/
│   ├── echo.v1.d.ts            # Protobuf 消息类型
│   ├── user.v1.d.ts
│   ├── actr-bindings.d.ts      # WASM 绑定类型
│   └── services.d.ts           # 服务客户端接口
├── actr_runtime_web.js         # WASM 绑定代码
└── actr_runtime_web_bg.wasm    # WASM 二进制
```

### 6.3 类型安全保证

- **编译时检查**: TypeScript 编译器验证类型
- **运行时验证**: Protobuf 编解码校验
- **端到端类型**: Rust → Protobuf → TypeScript 全链路类型传递

---

## 七、开发工具链

### 7.1 actr-cli 扩展

#### 新增 Web 平台支持

```bash
# 创建 Web 项目
actr-cli init my-web-app --platform web --framework react

# 生成类型和 WASM
actr-cli gen --platform web

# 构建 WASM
actr-cli build --platform web --profile release
```

#### 项目模板

```
my-web-app/
├── Actr.toml                   # Actor 配置
├── package.json                # npm 配置
├── tsconfig.json               # TypeScript 配置
├── vite.config.ts              # Vite 配置
├── proto/                      # Protobuf 定义
│   └── echo.v1.proto
├── src/
│   ├── generated/              # 自动生成 (不要编辑)
│   ├── App.tsx                 # 应用入口
│   └── main.ts
└── public/
```

### 7.2 配置扩展

#### Actr.toml 新增字段

```toml
[package]
name = "my-web-app"
manufacturer = "acme"

# Web 平台特定配置
[platform.web]
generate_types = true                    # 生成 TypeScript 类型
types_output = "./src/generated/types"   # 类型输出目录
generate_react_hooks = true              # 生成 React Hooks
worker_type = "service-worker"           # Worker 类型

[dependencies]
echo_service = { actr_type = "acme:echo-service" }
```

### 7.3 构建工具集成

#### Vite 配置模板

```typescript
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import wasm from 'vite-plugin-wasm';
import topLevelAwait from 'vite-plugin-top-level-await';

export default defineConfig({
    plugins: [react(), wasm(), topLevelAwait()],
    optimizeDeps: {
        exclude: ['@actr/web'],
    },
    server: {
        headers: {
            'Cross-Origin-Opener-Policy': 'same-origin',
            'Cross-Origin-Embedder-Policy': 'require-corp',
        },
    },
});
```

---

## 八、实施计划

### 8.1 分阶段路线图

```
Phase 1: MVP - 基础适配 (2 个月)
├─ Week 1-2: 环境搭建与最小 tokio 配置
├─ Week 3-4: IndexedDB Mailbox 实现
├─ Week 5-6: 浏览器 WebRTC 绑定
├─ Week 7-8: 基础 WASM API 导出与测试

Phase 2: SDK 与工具链 (1.5 个月)
├─ Week 9-10: TypeScript SDK 实现
├─ Week 11-12: actr-cli Web 支持
├─ Week 13-14: React Hooks 与示例项目

Phase 3: 优化与完善 (1.5 个月)
├─ Week 15-16: 性能优化 (批量、缓存)
├─ Week 17-18: Service Worker 集成
├─ Week 19-20: 文档与浏览器兼容性测试

Phase 4: 长期优化 (可选)
├─ 定制运行时实现 (8-10 周)
├─ 离线支持
├─ PWA 集成
```

### 8.2 里程碑

| 里程碑 | 目标 | 完成标志 |
|-------|------|---------|
| M1: 技术验证 | 验证 WASM + IndexedDB 可行性 | 简单 Echo Demo 运行 |
| M2: 核心完成 | 完整 ActorSystem 实现 | 通过核心功能测试 |
| M3: SDK 发布 | TypeScript SDK 可用 | npm 包发布 |
| M4: 生产就绪 | 完整开发者体验 | 示例项目运行 |

### 8.3 交付物清单

#### npm 包

```
1. @actr/web-runtime (核心)
   - actr_runtime_web.wasm
   - actr-bindings.d.ts
   - 大小: ~120KB (gzip)

2. @actr/web (高级 SDK)
   - Actor 类 + createActor
   - 类型安全封装
   - 大小: ~30KB (gzip)

3. @actr/web-react (React 集成)
   - useActor Hook
   - useServiceCall Hook
   - useSubscription Hook
   - 大小: ~10KB (gzip)

4. @actr/web-vue (Vue 集成, 可选)
   - Composables
   - 大小: ~10KB (gzip)
```

#### 工具与文档

```
- actr-cli (Web 平台支持)
- 项目模板
- 快速开始指南
- API 参考文档
- 示例项目集合
- 故障排查指南
```

---

## 九、技术风险与缓解

### 9.1 风险评估

| 风险 | 影响 | 概率 | 缓解措施 |
|------|------|------|---------|
| WASM 二进制过大 | 加载时间长 | 中 | 代码分割、按需加载、gzip 压缩 |
| IndexedDB 性能 | 延迟高于 SQLite | 中 | 批量操作、缓存策略、预加载 |
| 浏览器兼容性 | 部分浏览器不支持 | 低 | 功能检测、Polyfill、降级方案 |
| 调试复杂度 | 开发效率下降 | 中 | Source Map、专用工具、日志增强 |
| 内存限制 | 大规模应用受限 | 低 | 流式处理、内存监控、主动回收 |

### 9.2 性能指标目标

| 指标 | 目标值 | 测量方法 |
|------|--------|---------|
| WASM 加载时间 | <2s (3G 网络) | Lighthouse |
| 消息往返延迟 | <50ms | 性能测试 |
| 内存占用 | <50MB | Chrome DevTools |
| 帧率 (实时应用) | >30fps | Performance API |

---

## 十、开发者体验目标

### 10.1 核心原则

1. **零配置起步**: `actr-cli init --platform web` 一键创建可运行项目
2. **类型安全**: 100% TypeScript 类型覆盖
3. **开发友好**: HMR、Source Map、清晰错误信息
4. **文档完善**: 快速开始、API 参考、示例代码

### 10.2 学习曲线

```
初学者 (0 经验)
    ↓ 30 分钟: 跟随快速开始
掌握基础 (能创建简单应用)
    ↓ 2 小时: 阅读核心概念
理解架构 (能设计复杂应用)
    ↓ 1 天: 深入文档与示例
精通框架 (能优化性能与调试)
```

### 10.3 示例项目清单

1. **Hello World** (5 分钟)
   - 最简单的 Echo 客户端
   - 展示基础 API 使用

2. **Todo 应用** (30 分钟)
   - CRUD 操作
   - 状态管理

3. **聊天应用** (1 小时)
   - 实时消息
   - 订阅模式

4. **视频会议** (2 小时)
   - WebRTC 媒体流
   - 多人房间

5. **协同编辑** (4 小时)
   - OT/CRDT
   - 冲突解决

---

## 十一、成功标准

### 11.1 技术指标

- ✅ WASM 二进制 <150KB (gzip)
- ✅ 代码复用率 >85%
- ✅ 核心功能测试覆盖率 >90%
- ✅ 支持 Chrome/Firefox/Safari 最新 2 个版本

### 11.2 开发者体验

- ✅ 从零到 Hello World <10 分钟
- ✅ API 文档完整度 100%
- ✅ 示例项目 >5 个
- ✅ 社区反馈积极 (GitHub Stars >100)

### 11.3 性能基准

- ✅ 消息延迟 <50ms (P99)
- ✅ 内存占用 <50MB (典型应用)
- ✅ 首次加载 <2s (3G 网络)

---

## 十二、后续演进方向

### 12.1 短期 (6 个月)

- 优化 WASM 体积 (定制运行时)
- Vue/Svelte 框架集成
- 离线支持与 PWA

### 12.2 中期 (1 年)

- Edge Runtime 支持 (Cloudflare Workers, Deno Deploy)
- React Native 适配
- 性能分析工具

### 12.3 长期 (2 年)

- WASM 多线程支持 (SharedArrayBuffer)
- WebGPU 集成 (高性能计算)
- WebTransport 协议支持

---

## 十三、附录

### 13.1 技术栈清单

#### Rust 依赖

```toml
[dependencies]
wasm-bindgen = "0.2"
wasm-bindgen-futures = "0.4"
js-sys = "0.3"
web-sys = { version = "0.3", features = ["..."] }
indexed_db_futures = "0.5"
tokio = { version = "1", features = ["sync", "macros", "time"], default-features = false }
serde-wasm-bindgen = "0.6"
console_error_panic_hook = "0.1"
wasm-logger = "0.2"
```

#### JavaScript/TypeScript 依赖

```json
{
  "dependencies": {
    "@actr/web": "^0.1.0",
    "protobufjs": "^7.2.0"
  },
  "devDependencies": {
    "typescript": "^5.3.0",
    "vite": "^5.0.0",
    "vite-plugin-wasm": "^3.3.0",
    "vite-plugin-top-level-await": "^1.4.1"
  }
}
```

### 13.2 参考资源

- [Rust WASM Book](https://rustwasm.github.io/docs/book/)
- [wasm-bindgen Guide](https://rustwasm.github.io/wasm-bindgen/)
- [IndexedDB API (MDN)](https://developer.mozilla.org/en-US/docs/Web/API/IndexedDB_API)
- [WebRTC API (MDN)](https://developer.mozilla.org/en-US/docs/Web/API/WebRTC_API)
- [Service Worker API](https://developer.mozilla.org/en-US/docs/Web/API/Service_Worker_API)

### 13.3 术语表

| 术语 | 说明 |
|------|------|
| **Actor** | 独立的状态与行为封装单元 |
| **ActorSystem** | Actor 运行时基础设施 |
| **Workload** | 业务逻辑实现 |
| **State Path** | 可靠有序的消息处理路径 |
| **Fast Path** | 低延迟的流式数据处理路径 |
| **Mailbox** | Actor 的持久化消息队列 |
| **Context** | Actor 与系统交互的接口 |
| **WASM** | WebAssembly，浏览器可执行的二进制格式 |

---

## 文档修订历史

| 版本 | 日期 | 作者 | 变更说明 |
|------|------|------|---------|
| v0.1.0-draft | 2025-01-10 | 架构设计团队 | 初始版本 |

---

**文档状态**: 草案 - 待评审
**下一步**: 技术评审会议，确定实施优先级
