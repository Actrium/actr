# Actor-RTC Framework Demo

![Rust](https://img.shields.io/badge/rust-1.88+-orange.svg)
![Node.js](https://img.shields.io/badge/node.js-16+-green.svg)
![WebRTC](https://img.shields.io/badge/webrtc-enabled-blue.svg)
![License](https://img.shields.io/badge/license-MIT-green.svg)

基于 WebRTC 和 Actor 模型的分布式实时通信框架演示程序。

## 📖 概述

这个项目展示了一个创新的分布式系统架构，将经典的 Actor 模型与现代 WebRTC 技术相结合。通过"宏观 Actor"的设计理念，每个进程作为一个独立的 Actor，通过 WebRTC 进行点对点通信，同时内置了双路径处理模型来优化不同类型数据的传输。

### 🎯 核心特性

- **宏观 Actor 模型**: 进程级别的 Actor 抽象，简化分布式系统设计
- **WebRTC 原生支持**: 内置 NAT 穿透和点对点直连能力
- **双路径处理**: 
  - **状态路径**: 可靠有序的控制消息处理
  - **快车道**: 低延迟的流式数据处理
- **类型安全**: 基于 Protobuf 的契约驱动开发
- **ACL 感知**: 访问控制列表支持的安全发现机制

---

## 📚 文档中心

我们提供了一套完整的设计与开发文档，帮助您深入理解和使用本框架。推荐按以下顺序阅读：

#### **第一部分：核心概念与架构 (Concepts & Architecture)**
*这部分是所有开发者的必读内容，用于建立对框架的宏观理解。*

1.  **[生态系统综述](./docs/0-Ecosystem-Overview.zh.md)** (建议首先阅读)
2.  **[理念与架构](./docs/1-Concepts-and-Architecture.zh.md)**
3.  **[ActorSystem 与 Actor](./docs/1.1-ActorSystem-and-Actor.zh.md)**
4.  **[与外部世界交互](./docs/1.3-Interacting-with-the-Outside-World.zh.md)**
5.  **[Actor 间通信模式](./docs/1.4-Inter-Actor-Communication-Patterns.zh.md)**
6.  **[框架内部协议 (参考)](./docs/1.2-Framework-Internal-Protocols-zh.md)**

#### **第二部分：开发者指南与实践 (Guides & Practices)**
*这部分是动手实践的内容，指导开发者如何使用框架进行开发。*

1.  **[开发者指南](./docs/2-Developer-Guide.zh.md)** (快速入门教程)
2.  **[项目清单与命令行工具](./docs/2.4-Project-Manifest-and-CLI.zh.md)** (CLI 参考手册)
3.  **[Actor 食谱](./docs/2.2-Actor-Cookbook.zh.md)** (进阶开发模式)
4.  **[测试 Actor](./docs/2.3-Testing-Your-Actors.zh.md)**
5.  **[媒体源与轨道](./docs/2.1-Media-Sources-and-Tracks.zh.md)** (特定应用场景)

#### **第三部分：实现内幕 (How It Works)**
*这部分是为希望深入理解框架、或为其贡献代码的开发者准备的。*

1.  **[框架实现内幕](./docs/3-How-it-works.zh.md)** (内部机制概览)
2.  **[所有专题解析](./docs/)** (包含 `3.1` 至 `3.13` 的所有深度解析文档)

#### **附录**

*   **[名词解释表](./docs/appendix-a-glossary.zh.md)**

---

## 🚀 快速开始

### 前置要求

- **Rust**: 1.88+ ([安装指南](https://rustup.rs/))
- **Node.js**: 16+ ([下载地址](https://nodejs.org/))
- **protoc**: Protocol Buffer 编译器
  ```bash
  # Ubuntu/Debian
  sudo apt install protobuf-compiler
  
  # macOS
  brew install protobuf
  ```

### 一键演示

```bash
# 1. 设置项目（安装依赖、构建）
./run_demo.sh setup

# 2. 运行完整演示
./run_demo.sh demo
```

## 📁 项目结构

```
actor-rtc/
├── docs/                          # 框架设计文档
├── proto/                         # Protobuf 协议定义
├── actor-rtc-framework/          # 🔥 框架核心 crate
├── signaling-server/             # Node.js 信令服务器
├── examples/                     # 🎯 示例程序（使用框架）
└── run_demo.sh                   # 自动化脚本
```

## 🤝 贡献指南

我们欢迎各种形式的贡献！

1. Fork 本项目
2. 创建特性分支 (`git checkout -b feature/AmazingFeature`)
3. 提交更改 (`git commit -m 'Add some AmazingFeature'`)
4. 推送到分支 (`git push origin feature/AmazingFeature`)
5. 开启 Pull Request

## Release Train (Maintainers)

Use the manual workflow `Release Train (Basic)` for the monorepo-managed
foundation crates, protoc tools, supported SDK crates, and `actr-cli` with one
shared stable version.

- Workflow file: `.github/workflows/release-train-cli-protoc.yml`
- Local/CI entrypoint: `scripts/release-train-cli-protoc.sh`
- Required secrets: `CARGO_REGISTRY_TOKEN`, `PYPI_API_TOKEN`
- Reports are generated under `release/reports/` and uploaded as workflow
  artifacts.
- Components without a monorepo-native publish path yet are recorded as skipped
  in the report and do not block the basic train.

TypeScript package releases use the separate manual workflow
`Publish TypeScript Package`, which publishes `@actrium/actr` from
`bindings/typescript` using npm trusted publishing via GitHub Actions OIDC.
The package must be published manually once before enabling the trusted
publisher for `.github/workflows/publish-typescript.yml`.

## 📄 许可证

本项目采用 MIT 许可证。
