# Actor-RTC Web 架构文档

本目录包含 actr-web 的架构设计文档，面向框架贡献者和希望深入了解内部实现的开发者。

## 📚 文档索引

### 核心架构

1. **[架构总览](./overview.zh.md)** ⭐ 推荐先读
   - 双进程模型（Service Worker + DOM）
   - 核心组件和职责划分
   - 与 actr (Native) 的对比

2. **[架构全览图](./overview.zh.md#架构全览图)** 🎨 **最清晰**
   - **SVG 版本**：矢量图，可缩放无损，适合打印和展示
   - **Mermaid 版本**：交互式图表，可在 GitHub/Markdown 查看器中直接渲染
   - 三层结构：远程节点、SW 总控、DOM WebRTC
   - 颜色编码：蓝色（WebSocket）、紫色（WebRTC）、橙色（转发）
   - 包含图例、核心要点、性能对比
   - **推荐先看这个！**

2.5. **[消息流全览图](./message-flow-visual.zh.md)** 🎨 **超直观**
   - 一图看懂所有消息如何在 SW 和 DOM 间穿梭
   - 5 个关键流程详解（RPC/Stream/Media/转发/控制）
   - 性能对比和设计精髓
   - 文字版详细说明

3. **[消息流完整图](./message-flow-complete.zh.md)** 📋 **技术详细版**
   - 完整的技术架构图
   - 所有组件的交互细节
   - 适合深入研究

3.5. **[SW WebRTC 设计](./sw-webrtc-design.zh.md)** 🔧 **重要设计**
   - SW 如何通过 MessagePort 使用 WebRTC
   - MessagePort 桥接机制详解
   - 优先级和连接管理
   - 当前实现状态和 TODO

4. **[WASM-DOM 集成架构](./wasm-dom-integration.zh.md)** 🔥 **核心决策**
   - 用户代码统一在 WASM 的架构设计
   - DOM 侧固定转发层设计
   - UI 交互 API 设计（call/subscribe/on）
   - 性能权衡和多语言支持路径
   - **阅读此文了解为什么采用当前架构**

5. **[双层架构设计](./dual-layer.zh.md)**
   - State Path vs Fast Path
   - 消息路由决策
   - Web 环境适配

6. **[API 层设计](./api-layer.zh.md)**
   - Gate (Host + Peer)
   - Context (RuntimeContext)
   - ActrRef (Actor 引用)

### 技术决策与讨论

7. **[技术决策记录](./decisions.zh.md)**
   - 9 个关键技术决策 (TDR)
   - 异步运行时、持久化、网络层等选型
   - 决策背景和理由

8. **[WASM-DOM 架构讨论记录](./decisions-wasm-dom-qa.zh.md)** 📝
    - 架构设计过程的完整讨论
    - 核心问题和方案对比
    - Q&A 要点（谁触发谁、API 设计、性能权衡）
    - 经验总结和心智模型

### 实现与进度

9. **[完成度评估](./completion-status.zh.md)**
    - 相对 actr 的完成度分析（78%）
    - 关键缺失功能列表
    - 开发路线图

### 架构评估与反思

10. **[架构评估报告](./architecture-evaluation.zh.md)** 🔍 **批判性思考**
    - 整体架构评分：8.3/10
    - 架构优势分析（职责分离、MessagePort 桥接、双路径设计）
    - 架构问题与风险（复杂度、性能收益、状态同步）
    - 技术选型评估（IndexedDB、PostMessage、WASM）
    - 改进建议（短期 P0、中期 P1、长期 P2）
    - **推荐在深入实现前阅读**

## 🎯 快速导航

### 我想了解...

#### 整体架构和设计理念
→ 【快速理解】先看 [消息流全览图](./message-flow-visual.zh.md) 🎨
→ 【详细了解】阅读 [架构总览](./overview.zh.md)
→ 【核心决策】必读 [WASM-DOM 集成架构](./wasm-dom-integration.zh.md)

#### 消息如何在 SW 和 DOM 间流转？
→ 【最直观】[消息流全览图](./message-flow-visual.zh.md) 🎨
→ 【技术详细】[消息流完整图](./message-flow-complete.zh.md)
→ 【入口出口】[消息 I/O 详解](./message-io-entry-exit.zh.md)

#### SW 如何使用 WebRTC？
→ 【核心设计】[SW WebRTC 设计](./sw-webrtc-design.zh.md) 🔧
→ 通过 MessagePort 桥接机制

#### 为什么采用"统一在 WASM"的方案？
→ 阅读 [WASM-DOM 集成架构](./wasm-dom-integration.zh.md)
→ 详细讨论见 [WASM-DOM 架构讨论记录](./decisions-wasm-dom-qa.zh.md)

#### State Path 和 Fast Path 的区别
→ 阅读 [双层架构设计](./dual-layer.zh.md)
→ 或查看 [消息流全览图](./message-flow-visual.zh.md) 中的对比

#### 如何实现 Gate/Context/ActrRef
→ 阅读 [API 层设计](./api-layer.zh.md)

#### UI 如何和 WASM 交互？
→ 阅读 [WASM-DOM 集成架构 - UI 交互 API](./wasm-dom-integration.zh.md#四、ui-交互-api-设计)

#### 当前进度和后续计划
→ 阅读 [完成度评估](./completion-status.zh.md)

#### 架构设计是否合理？有什么问题？
→ 【批判性思考】[架构评估报告](./architecture-evaluation.zh.md) 🔍
→ 整体评分 8.3/10，了解架构优势与风险
→ 获取改进建议和优化方向

## 🏗️ 架构概览

```
actr-web/
├── crates/
│   ├── common/              # 通用类型和工具
│   ├── runtime-sw/          # Service Worker Runtime（主控）
│   │   ├── transport/       # 传输层
│   │   ├── inbound/         # 入站处理
│   │   ├── outbound/        # 出站处理
│   │   └── context.rs       # RuntimeContext
│   ├── runtime-dom/         # DOM Runtime（Fast Path）
│   │   ├── webrtc/          # WebRTC 管理
│   │   └── fastpath/        # Fast Path Registry
│   ├── mailbox-web/         # IndexedDB Mailbox
│   └── web-protoc-codegen/  # Protobuf 代码生成工具
├── packages/
│   ├── actr-dom/            # DOM 侧固定 JS 层
│   ├── web-sdk/             # TypeScript SDK (@actr/web)
│   └── web-react/           # React Hooks (@actr/web-react)
└── docs/
    ├── getting-started.md   # 用户文档
    └── architecture/        # 架构文档（本目录）
```

## 📊 完成度概览

| 维度 | 完成度 | 状态 |
|------|--------|------|
| 核心架构层 | 85% | ✅ 主要实现 |
| 持久化与调度 | 95% | ✅ 已完成 |
| Fast Path 支持 | 50% | ⚠️ 框架完成，集成待完善 |
| DOM 固定转发层 | 70% | ✅ MessageChannel 桥接已实现 |
| UI API 层 | 80% | ✅ TypeScript SDK + React Hooks |
| 代码生成支持 | 25% | ❌ 未实现 |
| **整体完成度** | **78%** | ⚠️ 接近 MVP |

详细分析见 [完成度评估](./completion-status.zh.md)。

## 🚀 后续工作

### 🔴 P0: 阻塞使用（核心架构实现）

**基于 [WASM-DOM 集成架构](./wasm-dom-integration.zh.md) 的实施计划**：

0. ~~**实现 DOM 侧固定转发层**~~ ✅ 已完成 (MessageChannel 桥接 + register_datachannel_port)

1. ~~**完善 WebRTC 连接逻辑**~~ ✅ 已完成 (完整传输栈 + ICE restart + 状态监控)

2. **完善 UI 交互 API 层** (80% → 90%)
   - `call()` - ✅ 已实现
   - `subscribe()` - ⚠️ register_stream 为 TODO stub
   - `on()` - ⚠️ 系统事件监听待完善

3. ~~**实现 ActorSystem 生命周期**~~ ⚠️ 部分完成 (System ~233 行，具备 MessageHandler + Gate)

4. ~~**实现调度器**~~ ✅ 已完成 (串行调度 + 优先级 + 事件驱动)

### 🟡 P1: 影响体验

5. **完善 Fast Path（统一在 WASM）** (50% → 90%)
   - WASM 侧 handle_fast_path 已实现
   - register_stream/register_media 回调待完善
   - 性能目标：<10ms

6. ~~**实现 RouteTable**~~ ✅ 已完成 (~300 行)
7. **actr-cli Web 平台支持** (0% → 80%)

### 🟢 P2: 增强功能

8. **多语言支持验证** (0% → 60%)
   - 定义统一 WASM 接口
   - TinyGo 示例验证
   - Emscripten (C++) 示例验证

9. 实现 DeadLetterQueue
10. 清理遗留代码 (send_rpc_to_remote)
11. 创建更多示例
12. 完善文档和教程

## 💡 贡献指南

想要贡献代码？

1. **必读**: [WASM-DOM 集成架构](./wasm-dom-integration.zh.md) 了解核心设计决策
2. 阅读 [架构总览](./overview.zh.md) 了解整体设计
3. 查看 [完成度评估](./completion-status.zh.md) 找到可以贡献的模块
4. 阅读 [API 层设计](./api-layer.zh.md) 了解 API 约定

### 当前最需要的贡献

1. **Fast Path 集成完善**（P1.5）
   - register_stream / register_media 回调实现
   - DataStream 端到端验证

2. **actr-cli Web 支持**（P1.7）
   - Protobuf → TypeScript 生成
   - Web 平台脖手架

## 🔗 相关资源

- **核心架构决策**: [WASM-DOM 集成架构](./wasm-dom-integration.zh.md)
- **用户文档**: `../getting-started.md`
- **示例代码**: `../../examples/`
- **原型实现**: `/d/actor-rtc/actr/` (Native Rust)
- **开发任务**: `../../TODO.zh.md`

## 📖 阅读顺序建议

### 初次了解（最快路径）
1. [架构全览图](./overview.zh.md#架构全览图) 🎨 ← **从这里开始！清晰无重叠**
   - SVG 版本（静态，精美）
   - Mermaid 版本（交互式，可直接在 GitHub 查看）
2. [消息流全览图](./message-flow-visual.zh.md)（文字详细版）
3. [架构总览](./overview.zh.md)
4. [WASM-DOM 集成架构](./wasm-dom-integration.zh.md) 🔥

### 深入实现
4. [消息流完整图](./message-flow-complete.zh.md)（技术详细版）
5. [双层架构设计](./dual-layer.zh.md)
6. [API 层设计](./api-layer.zh.md)
7. [完成度评估](./completion-status.zh.md)

### 理解决策过程
8. [技术决策记录](./decisions.zh.md)
9. [WASM-DOM 架构讨论记录](./decisions-wasm-dom-qa.zh.md)

### 批判性评估
10. [架构评估报告](./architecture-evaluation.zh.md) 🔍 ← **开发前必读**
    - 整体架构评分 8.3/10
    - 优势与风险分析
    - 改进建议（P0/P1/P2）

---

**维护者**: Actor-RTC Team
**最后更新**: 2026-02-28
**关键架构决策日期**: 2025-11-11
**拆分完成日期**: 2026-01-08（runtime-sw + runtime-dom 独立完成）
