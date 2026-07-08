# Actor-RTC Web 架构文档

本目录包含 actr-web 的架构设计文档，面向框架贡献者和希望深入了解内部实现的开发者。

## 📚 文档索引

### 当前可信入口（优先阅读）

1. **[架构总览](./overview.zh.md)** ⭐ 推荐先读
   - 当前 Option U / wasm-bindgen 浏览器主路径
   - Service Worker 总控、DOM WebRTC coordinator、FastPathForwarder 的真实边界
   - `.actr` + `.wbg` 旁车、CLI embedded assets、`.wbg` companion routes 的当前关系

2. **[架构全览图](./overview.zh.md#架构全览图)** 🎨 **最清晰**
   - **SVG 版本**：矢量图，可缩放无损，适合打印和展示
   - **Mermaid 版本**：交互式图表，可在 GitHub/Markdown 查看器中直接渲染
   - 三层结构：远程节点、SW 总控、DOM WebRTC
   - 颜色编码：蓝色（WebSocket）、紫色（WebRTC）、橙色（转发）
   - Fast Path 当前为 DOM → FastPathForwarder → SW `handle_dom_fast_path` → SW stream handlers
   - **推荐先看这个！**

3. **[WASM-DOM 集成架构](./wasm-dom-integration.zh.md)** 🔥 **核心决策**
   - 当前 Option U 流程：WIT → `tools/wit-compile-web` / `actr-web-abi` → wasm-bindgen/wasm-pack no-modules
   - 产物形态：`.actr` + `.wbg/guest.js` + `.wbg/guest_bg.wasm`
   - SW 入口：`packages/web-sdk/src/actor.sw.js`
   - **阅读此文了解为什么采用当前架构**

4. **[SW WebRTC 设计](./sw-webrtc-design.zh.md)** 🔧 **重要设计**
   - SW 如何通过 MessagePort 使用 DOM 创建的 WebRTC DataChannel
   - `register_datachannel_port` 注入 WirePool 的当前路径
   - 出站 WebRTC 仍由 SW 决策，DOM 执行 DataChannel I/O

5. **[消息 I/O 详解](./message-io-entry-exit.zh.md)**
   - WebSocket、WebRTC DataChannel、PostMessage 的入口/出口边界
   - Fast Path 转发入口和 SW handler dispatch

### 历史/快照文档（不作为当前事实入口）

这些文档保留设计演进背景。若与上面的当前入口冲突，以 `overview.zh.md` 和源码锚点为准。

6. **[消息流全览图](./message-flow-visual.zh.md)** 🎨
   - 历史可视化文档，已加过时说明
   - 当前 Fast Path 以 [架构总览](./overview.zh.md) 为准

7. **[消息流完整图](./message-flow-complete.zh.md)** 📋
   - 历史详细图，保留原始推导上下文
   - 当前实现路径以 [消息 I/O 详解](./message-io-entry-exit.zh.md) 为准

8. **[双层架构设计](./dual-layer.zh.md)**
   - State Path vs Fast Path
   - 已更新为当前 SW handler dispatch 模型

9. **[API 层设计](./api-layer.zh.md)**
   - Gate (Host + Peer)
   - Context (RuntimeContext)
   - ActrRef (Actor 引用)

### 技术决策与讨论

10. **[技术决策记录](./decisions.zh.md)**
   - 9 个关键技术决策 (TDR)
   - 异步运行时、持久化、网络层等选型
   - 决策背景和理由

11. **[WASM-DOM 架构讨论记录](./decisions-wasm-dom-qa.zh.md)** 📝
    - 架构设计过程的完整讨论
    - 核心问题和方案对比
    - 已标注为历史讨论，当前事实以 Option U 文档和源码为准

### 实现与进度

12. **[完成度评估](./completion-status.zh.md)**
    - 历史完成度快照，不作为当前状态矩阵
    - 文首保留当前源码派生摘要和 canonical pointers

### 架构评估与反思

13. **[架构评估报告](./architecture-evaluation.zh.md)** 🔍
    - 历史评估快照，文首保留当前修正
    - 不再作为当前完成度、性能数字或 TODO 的事实来源

## 🎯 快速导航

### 我想了解...

#### 整体架构和设计理念
→ 【详细了解】先读 [架构总览](./overview.zh.md)
→ 【核心决策】必读 [WASM-DOM 集成架构](./wasm-dom-integration.zh.md)

#### 消息如何在 SW 和 DOM 间流转？
→ 【当前事实】[架构总览](./overview.zh.md)
→ 【入口出口】[消息 I/O 详解](./message-io-entry-exit.zh.md)

#### SW 如何使用 WebRTC？
→ 【核心设计】[SW WebRTC 设计](./sw-webrtc-design.zh.md) 🔧
→ 通过 MessagePort 桥接机制

#### 为什么采用"统一在 WASM"的方案？
→ 阅读 [WASM-DOM 集成架构](./wasm-dom-integration.zh.md)
→ 详细讨论见 [WASM-DOM 架构讨论记录](./decisions-wasm-dom-qa.zh.md)

#### State Path 和 Fast Path 的区别
→ 阅读 [双层架构设计](./dual-layer.zh.md)
→ 以 [架构总览](./overview.zh.md) 的当前路径为准

#### 如何实现 Gate/Context/ActrRef
→ 阅读 [API 层设计](./api-layer.zh.md)

#### UI 如何和 WASM 交互？
→ 阅读 [WASM-DOM 集成架构 - UI 交互 API](./wasm-dom-integration.zh.md#四、ui-交互-api-设计)

#### 当前进度和后续计划
→ 阅读 [完成度评估](./completion-status.zh.md) 文首的当前源码派生摘要；旧矩阵仅作历史参考

#### 架构设计是否合理？有什么问题？
→ 【批判性思考】[架构评估报告](./architecture-evaluation.zh.md) 🔍
→ 该文是历史评估，当前实现差异以文首修正和源码为准

## 🏗️ 架构概览

```
actr-web/
├── crates/
│   ├── common/              # 通用类型和工具
│   ├── actr-web-abi/     # Option U WIT → wasm-bindgen ABI
│   ├── sw-host/          # Service Worker Runtime（主控）
│   │   ├── transport/       # 传输层
│   │   ├── inbound/         # 入站处理
│   │   ├── outbound/        # 出站处理
│   │   └── context.rs       # RuntimeContext
│   ├── dom-bridge/         # DOM Runtime（生命周期/桥接）
│   ├── mailbox-web/         # IndexedDB Mailbox
│   └── platform-web/        # Web platform glue
├── packages/
│   ├── actr-dom/            # DOM 侧固定转发层（HAL）
│   ├── web-sdk/             # TypeScript SDK (@actrium/actr-web)
│   └── web-react/           # React Hooks（如启用）
└── docs/
    ├── getting-started.md   # 用户文档
    └── architecture/        # 架构文档（本目录）
```

## 📊 当前源码派生状态

| 领域 | 当前事实 |
|------|----------|
| 浏览器 guest 路径 | Option U 是唯一路径：WIT → `tools/wit-compile-web` / `actr-web-abi` → wasm-bindgen / wasm-pack no-modules → `.actr` + `.wbg` |
| Service Worker 入口 | `packages/web-sdk/src/actor.sw.js` 加载 sw-host WASM、guest `.wbg` 胶水，并安装 `actrHost*` bridge |
| CLI Web assets | `cli/src/web_assets.rs` 嵌入 sw-host WASM/JS、`actor.sw.js` 和 host HTML；`cli/src/commands/run.rs` 挂载 `/packages/<name>.wbg/*` |
| DOM 固定层 | `packages/actr-dom` 提供 WebRTC coordinator、SW bridge 和 `FastPathForwarder` |
| Fast Path | DOM 收到 WebRTC 数据后转发 `fast_path_data`；SW `handle_dom_fast_path` 分发到 runtime fast path 和 stream handlers |
| 生命周期 | DOM 和 SW lifecycle baseline 已实现；仍有细粒度恢复和重注册缺口 |

当前路线图以 [TODO](../../TODO.zh.md) 为准。

## 💡 贡献指南

想要贡献代码？

1. **必读**: [WASM-DOM 集成架构](./wasm-dom-integration.zh.md) 了解核心设计决策
2. 阅读 [架构总览](./overview.zh.md) 了解整体设计
3. 查看 [完成度评估](./completion-status.zh.md) 文首当前摘要，再回到源码确认可贡献模块
4. 阅读 [API 层设计](./api-layer.zh.md) 了解 API 约定

### 当前最需要的贡献

1. **Fast Path 端到端验证**
   - DOM `FastPathForwarder` → SW `handle_dom_fast_path` → stream handlers
   - DataChunk、MediaTrack 的真实场景覆盖

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
**拆分完成日期**: 2026-01-08（sw-host + dom-bridge 独立完成）
