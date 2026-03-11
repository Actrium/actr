# Actor-RTC Web

> 将 Actor-RTC 分布式实时通信框架移植到 Web 环境

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Status](https://img.shields.io/badge/status-MVP%20阶段-yellow.svg)]()

**Actor-RTC Web** 是 Actor-RTC 框架的浏览器端实现，通过 WebAssembly 技术提供与原生版本一致的 Actor 模型编程体验。

---

## 🎯 项目状态

**当前版本**: v0.1.0-alpha
**开发阶段**: MVP (最小可行产品)
**整体完成度**: 78% (相对于 actr Native)

### 已完成的核心功能（P0 - MVP）

- ✅ ActorSystem 生命周期管理（启动/关闭/错误处理）
- ✅ WebRTC P2P 连接（4 个 negotiated DataChannels）
- ✅ RPC 请求-响应机制（request/pending_requests/timeout）
- ✅ DOM 侧固定转发层（@actr/dom，PostMessage + WebRTC）
- ✅ UI 交互 API（call/subscribe/on）
- ✅ 端到端 RPC 测试（simple-rpc 示例）
- ✅ IndexedDB Mailbox（持久化消息队列）
- ✅ TypeScript SDK（类型安全 API）
- ✅ React Hooks（useActorClient, useServiceCall, useSubscription）

### 高性能优化（P1 - 已完成）

- ✅ Fast Path 数据流（零拷贝传输，30x 性能提升）
  - Transferable ArrayBuffer（3ms 延迟）
  - SharedArrayBuffer（0.1ms 延迟）
  - WasmMemoryPool（预分配复用）
- ✅ RouteTable 路由表（静态路由 + 动态 LRU 缓存）
- ✅ 枚举静态分发（避免 dyn Trait 虚函数开销）
- ✅ DashMap 无锁并发（FastPathRegistry）
- ✅ AtomicU64 零锁监控（性能指标收集）

### 进行中的功能（P1/P2）

- ⏳ actr-cli Web 支持（0%）
- ⏳ 测试覆盖提升（当前 30%，目标 70%）
- ⏳ Service Worker 集成
- ⏳ 更多示例（react-echo, todo-app, chat-app）

详见：[完成度评估](./docs/architecture/completion-status.zh.md)

---

## 🚀 快速开始

### 前置要求

- Rust 1.88+ 和 wasm-pack
- Node.js 18+ 和 npm
- 支持 WebRTC 的现代浏览器

### 运行 Echo 示例

```bash
# 进入示例目录
cd examples/echo

# 自动构建并启动（signaling server + echo server + web 客户端）
./start.sh

# 浏览器自动打开 http://localhost:3001
```

### 从零开始

```bash
# 1. 克隆仓库
git clone https://github.com/actor-rtc/actor-rtc.git
cd actor-rtc/actr-web

# 2. 安装依赖
npm install

# 3. 构建 WASM
./scripts/build-wasm.sh

# 4. 运行测试
node test-wasm.js
```

---

## 📖 文档导航

### 用户文档

面向使用 Actor-RTC Web 构建应用的开发者：

- [快速开始指南](./docs/getting-started.md) ⭐ 推荐先读
- [故障排查指南](./docs/troubleshooting.md)

### 需求与规划文档

项目需求、目标和规划：

- [Web 适配需求](./docs/requirements.md) - 完整功能需求和架构设计

### 架构文档

面向框架贡献者和希望深入了解内部实现的开发者：

- [架构文档索引](./docs/architecture/README.zh.md)
- [架构总览](./docs/architecture/overview.zh.md) - 双进程模型和核心组件
- [技术决策记录](./docs/architecture/decisions.zh.md) - 9 个 TDR
- [完成度评伌](./docs/architecture/completion-status.zh.md) - 相对于 actr Native 的完成度 (78%)

---

## 🏗️ 架构概览

```
┌─────────── 浏览器环境 ───────────┐
│                                  │
│  TypeScript 应用                 │
│       ↓                          │
│  @actr/web SDK                   │
│       ↓                          │
│  WASM Runtime (Rust)             │
│    ├─ ActorSystem                │
│    ├─ Mailbox (IndexedDB)        │
│    ├─ WebRTC Coordinator         │
│    └─ Signaling Client           │
│       ↓                          │
│  浏览器 API                      │
│   (WebRTC, IndexedDB, WebSocket) │
└──────────────────────────────────┘
```

**核心特性**：

- **高代码复用率**: 85-90% 的核心逻辑直接复用自 actr Native
- **类型安全**: Rust + TypeScript 双重类型保证
- **性能优势**: WASM 接近原生性能
- **浏览器优先**: 充分利用浏览器原生 WebRTC 和 IndexedDB

---

## 📊 性能指标

| 指标 | 当前值 | 备注 |
|------|--------|------|
| WASM 包大小 | 99.6 KB (~35 KB gzipped) | 保持精简 |
| WASM 初始化时间 | <100ms | 快速启动 |
| State Path 延迟 | 30-40ms | RPC 请求-响应 |
| Fast Path 延迟（基线） | ~3ms | Transferable ArrayBuffer |
| Fast Path 延迟（优化） | ~0.1ms | SharedArrayBuffer，30x 提升 |
| 视频流处理 (60fps) | <0.1ms/帧 | 支持高帧率流 |
| IndexedDB 延迟 | <50ms | 持久化存储 |
| 内存占用 | ~48 MB | 典型应用 |

---

## 🛠️ 技术栈

### Rust / WebAssembly

- **wasm-bindgen**: Rust ↔ JavaScript 互操作
- **web-sys**: 浏览器 Web API 绑定
- **tokio**: 异步运行时 (最小特性集)
- **rexie**: IndexedDB 高级 API
- **prost**: Protobuf 编解码

### JavaScript / TypeScript

- **React 18**: UI 框架
- **Vite**: 开发服务器和构建工具
- **TypeScript 5**: 类型系统
- **grpc-web**: gRPC 浏览器客户端

### 协议与标准

- **WebRTC**: P2P 实时通信
- **WebSocket**: Signaling 信令通道
- **Protobuf**: 消息序列化
- **IndexedDB**: 浏览器持久化存储

---

## 📦 项目结构

```
actr-web/
├── crates/              # Rust crates (WASM 核心)
│   ├── runtime-sw/      # Service Worker 运行时
│   ├── runtime-dom/     # DOM 运行时
│   └── mailbox-web/     # IndexedDB Mailbox
│
├── packages/            # JavaScript/TypeScript 包
│   ├── actr-dom/        # DOM 侧 WASM 绑定
│   ├── web-sdk/         # 高级 TypeScript SDK (@actr/web)
│   └── web-react/       # React Hooks (@actr/web-react)
│
├── examples/            # 示例项目
│   ├── echo/            # Echo 示例 (完整实现)
│   ├── hello-world/     # 最小 hello-world 示例
│   └── codegen-test/    # 代码生成测试
│       ├── proto/       # Protobuf 定义
│       ├── server/      # gRPC 服务端 (Tonic)
│       ├── client/      # Web 客户端 (React)
│       └── start.sh     # 一键启动脚本
│
├── docs/                # 文档
│   ├── getting-started.md
│   ├── requirements.md
│   └── architecture/
│
└── scripts/             # 构建脚本
    ├── build-wasm.sh
    └── test-e2e.sh
```

---

## 🧪 开发与测试

### 构建 WASM

```bash
./scripts/build-wasm.sh
```

### 运行测试

```bash
# WASM 单元测试
node test-wasm.js

# E2E 测试 (需要先启动示例)
cd examples/echo
./start.sh
# 在另一个终端运行测试
npm run test:e2e
```

### 监听模式开发

```bash
# 监听 WASM 变更并自动重新构建
npm run dev:wasm

# 监听 TypeScript 变更
npm run dev:packages
```

---

## 🤝 贡献指南

欢迎贡献！请遵循以下步骤：

1. Fork 本仓库
2. 创建特性分支 (`git checkout -b feature/amazing-feature`)
3. 提交更改 (`git commit -m 'Add some amazing feature'`)
4. 推送到分支 (`git push origin feature/amazing-feature`)
5. 提交 Pull Request

请按照以上步骤提交您的贡献。

---

## 📝 许可证

本项目采用 Apache License 2.0 许可证。详见 [LICENSE](LICENSE) 文件。

---

## 🔗 相关资源

- **主项目**: [Actor-RTC](https://github.com/actor-rtc/actor-rtc)
- **文档站**: [actor-rtc.github.io](https://actor-rtc.github.io)
- **原生实现**: `/d/actor-rtc/actr/`
- **问题反馈**: [GitHub Issues](https://github.com/actor-rtc/actor-rtc/issues)

### 学习资源

- [Rust WASM Book](https://rustwasm.github.io/docs/book/)
- [wasm-bindgen Guide](https://rustwasm.github.io/wasm-bindgen/)
- [WebRTC API (MDN)](https://developer.mozilla.org/en-US/docs/Web/API/WebRTC_API)
- [IndexedDB API (MDN)](https://developer.mozilla.org/en-US/docs/Web/API/IndexedDB_API)

---

## 📧 联系方式

- **维护者**: kookyleo <kookyleo@gmail.com>
- **GitHub**: [@kookyleo](https://github.com/kookyleo)

---

**最后更新**: 2025-11-18
**文档版本**: v1.1.0
