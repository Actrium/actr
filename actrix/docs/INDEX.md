# Actrix 文档索引

**项目**: Actrix - WebRTC 辅助服务集合
**版本**: v0.1.0+enhancements
**最后更新**: 2025-11-03

本索引帮助你快速找到所需的文档。

---

## 🚀 快速开始

**新用户**: 按此顺序阅读
1. [README.md](../README.md) - 项目简介
2. [CONFIGURATION.md](./CONFIGURATION.md) - 配置你的服务
3. [deploy/README.md](../deploy/README.md) - 部署到生产环境

**开发者**: 深入了解
1. [ARCHITECTURE.md](./ARCHITECTURE.md) - 整体架构
2. [CRATES.md](./CRATES.md) - 代码实现细节
3. [DEVELOPMENT.md](./DEVELOPMENT.md) - 开发指南

---

## 📚 核心文档

### [ARCHITECTURE.md](./ARCHITECTURE.md)
**类型**: 架构设计
**篇幅**: ~880 行
**适合**: 架构师、高级开发者

**内容**:
- 项目整体架构和模块组织
- ServiceManager 和服务生命周期
- 网络层、存储层、错误处理
- 完整的启动流程和数据流
- 所有引用包含文件路径和行号

**何时阅读**: 需要理解系统设计或进行架构级改动

---

### [CRATES.md](./CRATES.md)
**类型**: 代码实现参考
**篇幅**: ~800 行
**适合**: 核心开发者

**内容**:
- 所有 crate 的详细实现 (platform, ks, stun, turn, signaling)
- 每个结构体、trait、函数的完整签名
- 性能特性 (LRU 缓存、异步处理)
- 安全分析和最佳实践
- 可运行的代码示例

**何时阅读**: 修改或扩展核心功能

---

### [SERVICES.md](./SERVICES.md)
**类型**: 服务管理指南
**篇幅**: ~1000 行
**适合**: 运维工程师、DevOps

**内容**:
- HttpRouterService vs IceService 详解
- ServiceManager 完整实现
- 服务启动流程和优雅关闭
- 生产部署 (systemd, Docker)
- 监控、健康检查、故障排查

**何时阅读**: 部署、运维、排查问题

---

### [API.md](./API.md)
**类型**: API 参考
**篇幅**: ~200 行
**适合**: 集成开发者

**内容**:
- KS HTTP API 端点 (`/ks/generate`, `/ks/secret/{id}`)
- Nonce-Auth 认证机制
- 请求/响应格式和示例
- 错误码和故障排查
- 安全最佳实践

**何时阅读**: 集成 Actrix API 到应用

---

### [CONFIGURATION.md](./CONFIGURATION.md)
**类型**: 配置参考
**篇幅**: ~585 行
**适合**: 所有用户

**内容**:
- 所有配置字段详解 (类型、默认值、验证规则)
- 位掩码服务控制
- 网络绑定 (HTTP/HTTPS/ICE)
- TURN、KS、OpenTelemetry 配置
- 生产环境配置示例

**何时阅读**: 配置服务或调整参数

---

## 📖 补充文档

### [deploy/README.md](../deploy/README.md)
**类型**: 部署指南
**篇幅**: ~250 行

**内容**:
- 生产部署完整流程
- systemd 服务安装
- Jaeger 分布式追踪配置
- 维护命令和故障排查

---

### [DEVELOPMENT.md](./DEVELOPMENT.md)
**类型**: 开发指南
**篇幅**: ~200 行

**内容**:
- 开发环境搭建
- 编译和测试
- 代码风格和 PR 流程
- 调试技巧

---

### [ENHANCEMENTS.md](../ENHANCEMENTS.md)
**类型**: 改进总结
**篇幅**: ~880 行

**内容**:
- 从 auxes 项目 develop 分支借鉴的改进
- HTTP Trace Layer、CI/CD、部署文件
- 决策过程和取舍
- 技术债务和未来规划

---

### [config.example.toml](../config.example.toml)
**类型**: 配置示例
**篇幅**: ~210 行

完整的配置文件示例,包含详细注释。

---

## 🎯 按场景查找

### 场景 1: 我想部署 Actrix

1. [CONFIGURATION.md](./CONFIGURATION.md) - 了解配置选项
2. [config.example.toml](../config.example.toml) - 复制配置模板
3. [deploy/README.md](../deploy/README.md) - 按步骤部署
4. [SERVICES.md](./SERVICES.md) - 故障排查

### 场景 2: 我想集成 KS API

1. [API.md](./API.md) - 查看 API 端点
2. [CRATES.md](./CRATES.md) - 了解客户端实现
3. [config.example.toml](../config.example.toml) - 配置 KS 服务

### 场景 3: 我想开发新功能

1. [ARCHITECTURE.md](./ARCHITECTURE.md) - 理解系统架构
2. [CRATES.md](./CRATES.md) - 找到相关代码
3. [DEVELOPMENT.md](./DEVELOPMENT.md) - 开发流程
4. [SERVICES.md](./SERVICES.md) - 服务管理机制

### 场景 4: 我遇到了问题

1. [SERVICES.md](./SERVICES.md) - 故障排查章节
2. [deploy/README.md](../deploy/README.md) - 部署问题
3. [CONFIGURATION.md](./CONFIGURATION.md) - 配置验证
4. [GitHub Issues](https://github.com/Actrium/actrix/issues)

---

## 📊 文档统计

| 文档 | 行数 | 更新日期 | 准确性 |
|------|------|----------|--------|
| ARCHITECTURE.md | ~880 | 2025-11-03 | 100% 验证 |
| CRATES.md | ~800 | 2025-11-03 | 100% 验证 |
| SERVICES.md | ~1000 | 2025-11-03 | 100% 验证 |
| API.md | ~200 | 2025-11-03 | 100% 验证 |
| CONFIGURATION.md | ~585 | 已存在 | 100% 验证 |
| ENHANCEMENTS.md | ~880 | 已存在 | 项目历史 |
| deploy/README.md | ~250 | 已存在 | 部署指南 |
| DEVELOPMENT.md | ~200 | 已存在 | 开发流程 |

**总计**: ~4800 行精炼文档

---

## 🔍 搜索关键词

**架构**: ARCHITECTURE.md
**配置**: CONFIGURATION.md
**API**: API.md
**部署**: deploy/README.md, SERVICES.md
**开发**: DEVELOPMENT.md, CRATES.md
**KS 服务**: CRATES.md, API.md
**STUN/TURN**: CRATES.md, SERVICES.md
**认证**: API.md (Nonce-Auth), CRATES.md (Authenticator)
**追踪**: SERVICES.md (OpenTelemetry), ENHANCEMENTS.md (HTTP Trace Layer)
**Docker**: SERVICES.md, deploy/README.md
**systemd**: SERVICES.md, deploy/README.md
**故障排查**: SERVICES.md, deploy/README.md

---

## 📝 文档约定

**代码引用格式**: `file_path:line_number`
**示例**: `crates/actrixd/src/service/manager.rs:23-31`

**标记含义**:
- ✅ 已实现/已启用
- ⚠️ 待重构/部分完成
- ❌ 已禁用/已移除
- 📝 建议/注意事项
- 🔒 安全相关
- ⭐ 重要特性

---

## 🤝 贡献文档

发现文档问题? 欢迎提交 PR:

1. Fork 项目
2. 修改文档 (`docs/*.md`)
3. 确保引用准确 (文件路径和行号)
4. 提交 PR with 标题 "docs: ..."

---

## 📞 获取帮助

- **GitHub Issues**: https://github.com/Actrium/actrix/issues
- **讨论区**: https://github.com/Actrium/actrix/discussions
- **项目主页**: https://actor-rtc.github.io

---

**文档系统特点**:
- ✅ 100% 映射真实代码
- ✅ 所有引用包含行号
- ✅ 定期验证和更新
- ✅ 完整但不冗余
- ✅ 系统化组织

**最后同步**: 2025-11-03
**代码版本**: v0.1.0+enhancements
