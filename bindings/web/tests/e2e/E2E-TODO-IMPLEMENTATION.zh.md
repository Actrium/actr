# E2E 测试完整实现待办清单

## 📋 概述

当前 E2E 测试只覆盖了基础设施层（WASM 加载、浏览器 API 可用性等），**尚未测试 Actor-RTC 的核心功能**。本文档列出需要补齐的完整功能测试。

## 🎯 测试分类

### ✅ 已完成 (8 个基础测试)
- 页面加载
- WASM 模块加载  
- 浏览器 API 支持
- ActorClient 初始化尝试（仅验证 UI 更新，不验证连接成功）
- IndexedDB 基础操作
- UI 元素渲染
- 性能指标收集

### ⚠️ 待实现 (34+ 个集成测试)

## 📊 测试用例清单

### 1. 信令连接测试 (3 个)
**文件**: `tests/e2e/puppeteer/basic.test.ts:188-211`

| 测试用例 | 优先级 | 依赖 | 预计工作量 |
|---------|--------|------|-----------|
| `should connect to signaling server via WebSocket` | P0 | Mock 信令服务器 | 2-3h |
| `should handle signaling server reconnection` | P1 | Mock 服务器 + 重连逻辑 | 2h |
| `should send and receive signaling messages` | P0 | Mock 服务器 | 1-2h |

**依赖**:
- Mock WebSocket 信令服务器 (可用 `ws` 库实现)
- 信令协议定义（需要文档或代码）

### 2. WebRTC 连接测试 (3 个)
**文件**: `tests/e2e/puppeteer/basic.test.ts:217-240`

| 测试用例 | 优先级 | 依赖 | 预计工作量 |
|---------|--------|------|-----------|
| `should establish WebRTC peer connection` | P0 | STUN/TURN 服务器或 Mock | 3-4h |
| `should create and open data channel` | P0 | WebRTC 连接 | 1-2h |
| `should handle WebRTC connection failure and retry` | P1 | 网络模拟工具 | 2h |

**依赖**:
- STUN 服务器（可用公共的，如 Google STUN）
- TURN 服务器（可选，用于 NAT 穿透测试）
- 双客户端测试环境

### 3. RPC 调用测试 (4 个)
**文件**: `tests/e2e/puppeteer/basic.test.ts:246-277`

| 测试用例 | 优先级 | 依赖 | 预计工作量 |
|---------|--------|------|-----------|
| `should call remote service via RPC` | P0 | 完整连接 + Echo 服务 | 2-3h |
| `should handle RPC call timeout` | P1 | 超时模拟 | 1h |
| `should handle concurrent RPC calls` | P1 | - | 1-2h |
| `should measure RPC call latency` | P2 | 性能监控 | 1h |

**依赖**:
- 完整的信令 + WebRTC 连接
- Mock Echo 服务实现
- 性能监控工具

### 4. Actor 系统测试 (4 个)
**文件**: `tests/e2e/puppeteer/basic.test.ts:283-317`

| 测试用例 | 优先级 | 依赖 | 预计工作量 |
|---------|--------|------|-----------|
| `should create and destroy actor` | P0 | Actor 运行时 | 2h |
| `should send messages between actors` | P0 | Actor 消息传递 | 2-3h |
| `should handle actor mailbox overflow` | P1 | 背压机制 | 2h |
| `should supervise child actors` | P2 | 监督策略 | 3h |

**依赖**:
- Actor 运行时完整实现
- Mailbox 和监督机制

### 5. 状态同步测试 (2 个)
**文件**: `tests/e2e/puppeteer/basic.test.ts:323-337`

| 测试用例 | 优先级 | 依赖 | 预计工作量 |
|---------|--------|------|-----------|
| `should sync actor state across peers` | P1 | 状态同步协议 | 3-4h |
| `should handle state conflict resolution` | P2 | CRDT 或冲突解决策略 | 4h |

**依赖**:
- 状态同步机制
- 冲突解决策略（CRDT/LWW 等）

### 6. 性能和压力测试 (2 个)
**文件**: `tests/e2e/puppeteer/basic.test.ts:343-359`

| 测试用例 | 优先级 | 依赖 | 预计工作量 |
|---------|--------|------|-----------|
| `should handle high message throughput` | P2 | 性能测试环境 | 2h |
| `should handle multiple concurrent connections` | P2 | 多客户端环境 | 2-3h |

**依赖**:
- 性能监控工具
- 负载生成工具

### 7. IndexedDB 完整测试 (2 个)
**文件**: `tests/e2e/puppeteer/basic.test.ts:365-379`

| 测试用例 | 优先级 | 依赖 | 预计工作量 |
|---------|--------|------|-----------|
| `should persist actor state to IndexedDB` | P1 | 持久化层 | 2h |
| `should query IndexedDB with complex filters` | P2 | 查询 API | 1-2h |

**依赖**:
- IndexedDB 持久化实现
- 索引和查询 API

### 8. 错误恢复测试 (2 个)
**文件**: `tests/e2e/puppeteer/basic.test.ts:385-401`

| 测试用例 | 优先级 | 依赖 | 预计工作量 |
|---------|--------|------|-----------|
| `should recover from network interruption` | P1 | 网络中断模拟 | 2h |
| `should handle browser tab visibility changes` | P2 | Page Visibility API | 1h |

**依赖**:
- 网络故障模拟能力
- 重连机制

### 9. 跨浏览器和 UI 测试 (12 个)
**文件**: `tests/e2e/browser/actor-client.spec.ts:138-289`

| 分类 | 测试数量 | 优先级 | 预计工作量 |
|-----|---------|--------|-----------|
| 真实连接测试 | 3 | P0 | 3-4h |
| 跨浏览器特性 | 2 | P1 | 2h |
| UI 响应式 | 2 | P2 | 1-2h |
| 网络条件 | 2 | P1 | 2h |
| 并发测试 | 2 | P2 | 2h |
| 可访问性 | 2 | P3 | 2h |
| 错误场景 | 2 | P1 | 2h |

**依赖**:
- Playwright 浏览器安装
- Mock 服务器
- 网络模拟工具

## 🔧 实现前置条件

### 必需组件

#### 1. Mock 信令服务器
**描述**: 用于测试的 WebSocket 信令服务器

**实现方案**:
```typescript
// tests/e2e/mock-server/signaling.ts
import WebSocket from 'ws';

export class MockSignalingServer {
  private wss: WebSocket.Server;
  
  start(port: number = 9000) {
    this.wss = new WebSocket.Server({ port });
    this.wss.on('connection', this.handleConnection);
  }
  
  private handleConnection(ws: WebSocket) {
    // 实现信令逻辑
  }
  
  stop() {
    this.wss.close();
  }
}
```

**工作量**: 1-2 天

#### 2. Mock Echo 服务
**描述**: 简单的回声服务，用于测试 RPC 调用

**实现方案**:
```typescript
// tests/e2e/mock-server/echo-service.ts
export class MockEchoService {
  async handleEcho(message: string): Promise<string> {
    return `Echo: ${message}`;
  }
}
```

**工作量**: 0.5 天

#### 3. 测试工具函数
**描述**: 通用的测试辅助函数

**实现方案**:
```typescript
// tests/e2e/helpers/test-utils.ts
export async function waitForConnection(page: Page, timeout = 10000) {
  await page.waitForFunction(
    () => document.getElementById('status')?.textContent?.includes('已连接'),
    { timeout }
  );
}

export async function mockNetworkFailure(page: Page) {
  await page.setOfflineMode(true);
}
```

**工作量**: 1 天

### 可选组件

#### 1. TURN 服务器
用于 NAT 穿透测试，可先使用公共 STUN 服务器

#### 2. 性能监控面板
实时查看测试性能指标

#### 3. CI/CD 集成
在 GitHub Actions 中运行完整测试

## 📅 实现路线图

### Phase 1: 基础集成 (1-2 周) - P0
**目标**: 完成最基础的端到端流程

- [ ] 实现 Mock 信令服务器
- [ ] 实现 Mock Echo 服务  
- [ ] 完成信令连接测试 (3 个)
- [ ] 完成基础 RPC 调用测试 (1 个)
- [ ] 完成真实连接 UI 测试 (3 个)

**交付物**: 能够测试完整的"连接 → 发送消息 → 接收响应"流程

### Phase 2: WebRTC 和 Actor (2-3 周) - P0/P1
**目标**: 测试核心 WebRTC 和 Actor 功能

- [ ] 完成 WebRTC 连接测试 (3 个)
- [ ] 完成 Actor 系统测试 (2 个)
- [ ] 完成错误恢复测试 (2 个)
- [ ] 完成跨浏览器基础测试 (5 个)

**交付物**: WebRTC 和 Actor 核心功能验证通过

### Phase 3: 完善和优化 (1-2 周) - P1/P2
**目标**: 补充高级功能和边缘场景

- [ ] 完成状态同步测试 (2 个)
- [ ] 完成性能测试 (4 个)
- [ ] 完成 IndexedDB 完整测试 (2 个)
- [ ] 完成其余 UI 和可访问性测试 (7 个)

**交付物**: 完整测试套件，覆盖所有核心和高级功能

### Phase 4: CI/CD 和文档 (1 周) - P2/P3
**目标**: 生产就绪

- [ ] GitHub Actions 集成
- [ ] 测试报告生成
- [ ] 性能基准建立
- [ ] 测试文档完善

**交付物**: 可在 CI 中自动运行的完整测试套件

## 🚀 快速开始实现指南

### 步骤 1: 创建 Mock 服务器目录
```bash
mkdir -p tests/e2e/mock-server
cd tests/e2e/mock-server
```

### 步骤 2: 安装依赖
```bash
pnpm add -D ws @types/ws
```

### 步骤 3: 实现 Mock 信令服务器
参考上述"Mock 信令服务器"实现方案

### 步骤 4: 修改测试用例
将第一个 `it.todo` 改为 `it`:
```typescript
it('should connect to signaling server via WebSocket', async () => {
  const mockServer = new MockSignalingServer();
  mockServer.start(9000);
  
  // 测试逻辑...
  
  mockServer.stop();
});
```

### 步骤 5: 运行测试
```bash
pnpm test
```

## 📚 参考资料

### 测试工具文档
- [Puppeteer API](https://pptr.dev/)
- [Playwright API](https://playwright.dev/)
- [Vitest API](https://vitest.dev/)

### WebRTC 资源
- [WebRTC 示例](https://webrtc.github.io/samples/)
- [STUN/TURN 服务器](https://gist.github.com/sagivo/3a4b2f2c7ac6e1b5267c2f1f59ac6c6b)

### Actor 模型
- [Akka Testing](https://doc.akka.io/docs/akka/current/testing.html)
- [Orleans Testing](https://learn.microsoft.com/en-us/dotnet/orleans/host/testing)

## 📝 注意事项

1. **Mock 服务器生命周期**: 确保每个测试后正确清理 Mock 服务器
2. **端口冲突**: 使用动态端口分配避免测试冲突
3. **超时设置**: WebRTC 连接可能需要较长时间，适当增加超时
4. **浏览器兼容性**: Safari 对某些 API 有限制，需要特殊处理
5. **CI 环境**: Headless 模式可能不支持某些 API，注意测试选择

## 🎯 成功指标

完成所有测试后，应该能够：

- ✅ 自动测试完整的连接建立流程
- ✅ 验证消息在客户端间正确传递
- ✅ 测试各种故障场景和恢复机制
- ✅ 在 CI 中自动运行所有测试
- ✅ 生成详细的测试报告和性能指标
- ✅ 在三大浏览器（Chrome/Firefox/Safari）中验证兼容性

## 📮 问题反馈

如果在实现过程中遇到问题：
1. 检查本文档的"注意事项"部分
2. 查看 `tests/e2e/README.zh.md` 基础说明
3. 参考 `tests/e2e/TESTING_LOGIC.zh.md` 测试架构
4. 提交 Issue 到项目仓库
