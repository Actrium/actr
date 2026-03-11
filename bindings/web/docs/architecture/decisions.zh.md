# 技术决策记录 (Technical Decision Records)

## 文档说明

本文档记录 Actor-RTC Web 适配过程中的关键技术决策、背景、考虑因素和最终选择。

---

## TDR-001: 异步运行时选择

**日期**: 2025-01-10
**状态**: 已决策
**决策者**: 架构团队

### 背景

Actor-RTC 原生版本使用 tokio 作为异步运行时。在 Web 环境中，需要决定是继续使用 tokio (编译为 WASM) 还是替换为 JS 原生运行时。

### 考虑因素

1. **WASM 二进制大小**
   - 完整 tokio: ~850KB (未压缩)
   - 最小 tokio: ~420KB (未压缩)
   - 纯 JS: ~180KB (未压缩)

2. **开发成本**
   - 最小 tokio: 2 周
   - 定制运行时: 10 周
   - 纯 JS 重写: 20 周

3. **代码复用率**
   - 最小 tokio: 90%
   - 定制运行时: 75%
   - 纯 JS: 50%

4. **维护成本**
   - 最小 tokio: 1x (统一代码库)
   - 定制运行时: 1.3x (两套实现)
   - 纯 JS: 2x (完全分叉)

### 决策

**采用渐进式混合方案**：

**Phase 1 (立即)**: 使用最小 tokio
- 只启用必需的 features: `sync`, `macros`, `time`
- 禁用 `default-features`
- 预期大小: ~350KB (未压缩), ~120KB (gzip)

**Phase 2 (长期)**: 定制运行时
- 替换 `tokio::sync::mpsc` 为 JS 驱动的实现
- 替换 `tokio::sync::Mutex` 为 JS Mutex
- 替换 `tokio::time` 为 `setTimeout`/`setInterval`
- 预期大小: ~250KB (未压缩), ~85KB (gzip)

### 理由

1. **快速验证**: Phase 1 只需 2 周，风险极低
2. **灵活渐进**: 可根据实际需求决定是否进入 Phase 2
3. **平衡优化**: Phase 2 提供 30% 的体积优化，开发成本可接受
4. **避免过度**: 纯 JS 方案维护成本过高，不值得

### 后果

- ✅ 快速上线 MVP
- ✅ 保持高代码复用率
- ⚠️ 初期 WASM 体积较大 (但可接受)
- ✅ 为长期优化保留空间

---

## TDR-002: 持久化存储方案

**日期**: 2025-01-10
**状态**: 已决策
**决策者**: 架构团队

### 背景

原生版本使用 SQLite 作为 Mailbox 的持久化后端。Web 环境没有 SQLite，需要选择替代方案。

### 候选方案

| 方案 | 优点 | 缺点 |
|------|------|------|
| **IndexedDB** | 浏览器原生、事务支持、异步 API | 性能略低于 SQLite |
| **LocalStorage** | 简单、同步 API | 5MB 限制、无事务、阻塞主线程 |
| **内存队列** | 性能最优 | 无持久化、刷新丢失 |

### 决策

**使用 IndexedDB**

### 理由

1. **事务支持**: 提供 ACID 语义，与 SQLite 一致
2. **容量充足**: 无固定上限 (取决于磁盘空间)
3. **异步 API**: 与 tokio 配合良好
4. **复合索引**: 支持 `(priority DESC, status, created_at ASC)` 复合索引
5. **浏览器支持**: 所有现代浏览器原生支持

### 实现策略

**接口保持不变**: 复用现有 `Mailbox` trait

```rust
// 原生平台
#[cfg(not(target_arch = "wasm32"))]
use actr_mailbox::SqliteMailbox;

// Web 平台
#[cfg(target_arch = "wasm32")]
use actr_mailbox_web::IndexedDbMailbox;
```

**关键特性映射**:

| SQLite 特性 | IndexedDB 对应 |
|------------|---------------|
| 双优先级表 | 复合索引 (priority) |
| ACID 事务 | IDBTransaction |
| SQL 查询 | IDBCursor 遍历 |
| 批量读取 | IDBCursor.advance() |

### 后果

- ✅ 完整的持久化能力
- ✅ 业务代码无需修改
- ⚠️ 性能略低于 SQLite (~20% 延迟增加)
- ✅ 通过批量操作缓解性能影响

---

## TDR-003: 网络层适配策略

**日期**: 2025-01-10
**状态**: 已决策
**决策者**: 架构团队

### 背景

原生版本使用 Rust `webrtc` crate。Web 环境应使用浏览器原生 WebRTC API。

### 决策

**通过 `web-sys` 绑定浏览器原生 API**

### 实现方案

```rust
use web_sys::{RtcPeerConnection, RtcDataChannel};

pub struct WebRtcConnection {
    peer_connection: RtcPeerConnection,
    data_channels: DashMap<String, RtcDataChannel>,
}
```

**绑定层**:
- `wasm-bindgen`: Rust ↔ JS 互操作桥梁
- `web-sys`: 浏览器 Web API 类型绑定
- `js-sys`: JavaScript 标准对象绑定

### 理由

1. **零额外体积**: 复用浏览器内置实现
2. **原生性能**: 无需 Rust WebRTC 栈
3. **自动更新**: 随浏览器更新
4. **标准兼容**: 遵循 W3C 标准

### 后果

- ✅ WASM 体积最小化 (无需打包 WebRTC 实现)
- ✅ 性能最优 (原生代码)
- ⚠️ 需要处理浏览器 API 差异
- ✅ 通过 `web-sys` 提供统一抽象

---

## TDR-004: Worker 类型选择

**日期**: 2025-01-10
**状态**: 已决策
**决策者**: 架构团队

### 背景

WASM 运行时需要在 Worker 中运行，以避免阻塞主线程。需要选择 Service Worker 或 Web Worker。

### 对比分析

| 特性 | Service Worker | Web Worker |
|------|---------------|------------|
| 生命周期 | 独立于页面 | 随页面销毁 |
| 后台运行 | 支持 (Push API) | 不支持 |
| 拦截网络 | 支持 (Fetch API) | 不支持 |
| 调试复杂度 | 较高 | 较低 |
| 浏览器支持 | 需要 HTTPS | 无限制 |

### 决策

**提供双模式支持，默认优先 Service Worker**

```typescript
export interface ActorClientConfig {
    workerType?: 'service-worker' | 'web-worker';
    // ...
}
```

### 理由

1. **生产环境**: Service Worker 提供持久化和后台能力
2. **开发环境**: Web Worker 调试更方便
3. **灵活性**: 让开发者根据场景选择

### 实现策略

```typescript
if (config.workerType === 'service-worker') {
    // 注册 Service Worker
    const registration = await navigator.serviceWorker.register('/sw.js');
} else {
    // 启动 Web Worker
    const worker = new Worker('/worker.js', { type: 'module' });
}
```

### 后果

- ✅ 生产环境最优体验
- ✅ 开发环境易于调试
- ⚠️ 需要维护两种模式
- ✅ 通过配置切换降低复杂度

---

## TDR-005: TypeScript 类型生成方案

**日期**: 2025-01-10
**状态**: 已决策
**决策者**: 架构团队

### 背景

需要为 JS/TS 开发者提供完整的类型定义，包括 Protobuf 消息类型和 WASM API 类型。

### 决策

**多层次自动生成**:

1. **Protobuf → TypeScript**: 使用 `protobuf.js` 生成消息类型
2. **Rust → TypeScript**: 使用 `wasm-bindgen` 生成 WASM 绑定类型
3. **高级封装**: 使用 `actr-cli` 生成服务客户端接口

### 生成流程

```
.proto 文件
    ↓ protobuf.js
echo.v1.d.ts (消息类型)
    ↓
actr-bindings.d.ts (WASM 类型)
    ↓ actr-cli
services.d.ts (客户端接口)
```

### 理由

1. **完整覆盖**: 从 Protobuf 到 WASM 到 SDK 全链路类型
2. **自动同步**: 修改 `.proto` 后自动更新类型
3. **零手写**: 开发者无需编写类型定义
4. **工具成熟**: `protobuf.js` 和 `wasm-bindgen` 是成熟工具

### 示例输出

```typescript
// echo.v1.d.ts (protobuf.js 生成)
export namespace echo.v1 {
    interface EchoRequest {
        message?: string;
    }
    interface EchoResponse {
        reply?: string;
        timestamp?: number;
    }
}

// actr-bindings.d.ts (wasm-bindgen 生成)
export class ActorSystem {
    constructor(config: ActorSystemConfig): Promise<ActorSystem>;
    call(service: string, method: string, request: Uint8Array): Promise<Uint8Array>;
}

// services.d.ts (actr-cli 生成)
export interface EchoServiceClient {
    sendEcho(request: echo.v1.EchoRequest): Promise<echo.v1.EchoResponse>;
}
```

### 后果

- ✅ 100% 类型覆盖
- ✅ 编译时错误检查
- ✅ IDE 自动补全
- ✅ 与 Rust 端完全同步

---

## TDR-006: 客户端 vs 服务端模式

**日期**: 2025-01-10
**状态**: 已决策
**决策者**: 架构团队

### 背景

Web Actor 可以作为客户端连接后端服务，也可以作为服务端接受其他 Actor 的连接。

### 决策

**双角色支持，客户端优先**

**Phase 1**: 专注客户端模式 (90% 场景)
- 连接后端 Actor 服务
- 调用远程服务
- 订阅数据流

**Phase 2**: 支持服务端模式 (10% 场景)
- 通过信令服务器注册服务
- 接受其他 Actor 连接
- 实现真正的 P2P 应用

### 理由

1. **需求分布**: 90% 的 Web 应用只需客户端能力
2. **渐进实施**: 客户端模式更简单，快速验证
3. **架构前瞻**: 保留服务端能力的扩展性
4. **P2P 潜力**: 充分发挥 WebRTC P2P 优势

### 实现策略

```typescript
// 统一 API：每个 Actor 同时具备客户端和服务端能力
const actor = await createActor({
    signalingUrl: 'wss://signal.example.com',
    realm: 'demo',
});

// 带 WASM handler 的 Actor（可处理本地和远程请求）
const serverActor = await createActor({
    signalingUrl: 'wss://signal.example.com',
    realm: 'demo',
    wasmUrl: '/echo_server_bg.wasm',
});
```

### 后果

- ✅ 统一 API，无需区分客户端/服务端
- ✅ 降低开发者学习成本
- ✅ 充分发挥 WebRTC P2P 优势
- ✅ WASM handler 可选，灵活扩展

---

## TDR-007: 开发框架集成策略

**日期**: 2025-01-10
**状态**: 已决策
**决策者**: 架构团队

### 背景

需要决定是否为主流前端框架 (React, Vue, Svelte) 提供专用集成。

### 决策

**分层提供，React 优先**

**核心层**: 框架无关的 TypeScript SDK
```typescript
@actr/web - 纯 TS，无框架依赖
```

**集成层**: 框架专用封装
```typescript
@actr/web-react   - React Hooks (Phase 1)
@actr/web-vue     - Vue Composables (Phase 2)
@actr/web-svelte  - Svelte Stores (Phase 3)
```

### 理由

1. **分层解耦**: 核心 SDK 保持框架中立
2. **优先级**: React 市场占有率最高 (~40%)
3. **渐进实施**: 根据需求逐步支持其他框架
4. **原生支持**: 纯 TS SDK 也可在任何框架中使用

### React Hooks 示例

```typescript
export function useActor(config: ActorConfig);
export function useServiceCall<TReq, TRes>(...);
export function useSubscription<T>(...);
```

### 后果

- ✅ 核心 SDK 通用性
- ✅ React 开发者最佳体验
- ⚠️ 其他框架需等待或自行封装
- ✅ 社区可贡献其他框架集成

---

## TDR-008: 构建工具选择

**日期**: 2025-01-10
**状态**: 已决策
**决策者**: 架构团队

### 背景

需要选择合适的构建工具来支持 WASM + TypeScript 的开发。

### 候选方案

| 工具 | 优点 | 缺点 |
|------|------|------|
| **Vite** | 快速、现代、WASM 支持好 | 较新 |
| **Webpack** | 成熟、生态丰富 | 配置复杂、慢 |
| **Rollup** | 轻量、Tree-shaking 好 | WASM 支持一般 |

### 决策

**默认使用 Vite，提供 Webpack 配置示例**

### 理由

1. **开发体验**: Vite 的 HMR 极快
2. **WASM 支持**: 通过 `vite-plugin-wasm` 开箱即用
3. **现代化**: 原生 ESM，Top-level await
4. **社区趋势**: React/Vue 官方推荐

### 配置模板

```typescript
// vite.config.ts
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import wasm from 'vite-plugin-wasm';
import topLevelAwait from 'vite-plugin-top-level-await';

export default defineConfig({
    plugins: [react(), wasm(), topLevelAwait()],
});
```

### 后果

- ✅ 极佳开发体验
- ✅ WASM 支持简单
- ⚠️ Webpack 用户需自行配置
- ✅ 提供文档和示例

---

## TDR-009: 错误处理策略

**日期**: 2025-01-10
**状态**: 已决策
**决策者**: 架构团队

### 背景

需要设计跨越 Rust/WASM/JS 边界的统一错误处理机制。

### 决策

**分层错误处理**

**Rust 层**: 使用 `Result<T, ActorError>`
```rust
pub enum ActorError {
    NetworkError(String),
    SerializationError(String),
    TimeoutError,
    // ...
}
```

**WASM 边界**: 转换为 `JsValue`
```rust
#[wasm_bindgen]
pub async fn call(...) -> Result<Vec<u8>, JsValue> {
    inner_call().await
        .map_err(|e| JsValue::from_str(&e.to_string()))
}
```

**TypeScript 层**: 转换为强类型错误
```typescript
export class ActorError extends Error {
    constructor(
        message: string,
        public code: ErrorCode,
        public details?: any
    ) {}
}
```

### 理由

1. **类型安全**: 各层都有强类型错误
2. **调试友好**: 保留完整错误信息
3. **标准兼容**: TypeScript 层使用标准 Error
4. **可序列化**: 错误可跨边界传递

### 示例

```typescript
try {
    await client.echo.sendEcho({ message: 'test' });
} catch (error) {
    if (error instanceof ActorError) {
        switch (error.code) {
            case ErrorCode.NetworkError:
                console.error('网络错误:', error.message);
                break;
            case ErrorCode.TimeoutError:
                console.error('超时');
                break;
        }
    }
}
```

### 后果

- ✅ 完整错误信息传递
- ✅ 类型安全的错误处理
- ✅ 调试友好
- ⚠️ 需要维护错误码映射

---

## 决策总结矩阵

| 决策 | 优先级 | 复杂度 | 风险 | 状态 |
|------|--------|--------|------|------|
| TDR-001: 异步运行时 | 🔴 高 | 🟡 中 | 🟢 低 | ✅ 已决策 |
| TDR-002: 持久化存储 | 🔴 高 | 🟡 中 | 🟢 低 | ✅ 已决策 |
| TDR-003: 网络层适配 | 🔴 高 | 🟡 中 | 🟢 低 | ✅ 已决策 |
| TDR-004: Worker 选择 | 🟡 中 | 🟢 低 | 🟢 低 | ✅ 已决策 |
| TDR-005: 类型生成 | 🔴 高 | 🟡 中 | 🟢 低 | ✅ 已决策 |
| TDR-006: 客户端/服务端 | 🟡 中 | 🟢 低 | 🟢 低 | ✅ 已决策 |
| TDR-007: 框架集成 | 🟢 低 | 🟢 低 | 🟢 低 | ✅ 已决策 |
| TDR-008: 构建工具 | 🟢 低 | 🟢 低 | 🟢 低 | ✅ 已决策 |
| TDR-009: 错误处理 | 🟡 中 | 🟢 低 | 🟢 低 | ✅ 已决策 |

---

## 后续行动

### 立即执行

1. 基于 TDR-001, 配置最小 tokio
2. 基于 TDR-002, 开始 IndexedDB Mailbox 实现
3. 基于 TDR-003, 设计网络层抽象接口

### 近期计划

4. 基于 TDR-005, 实现类型生成流程
5. 基于 TDR-007, 实现 React Hooks
6. 基于 TDR-008, 配置 Vite 模板

### 长期跟踪

7. TDR-001 Phase 2: 定制运行时实现
8. TDR-006 Phase 2: 服务端模式支持
9. TDR-007 Phase 2/3: Vue/Svelte 集成

---

**文档维护**: 每个新的重要技术决策都应在此文档中记录
