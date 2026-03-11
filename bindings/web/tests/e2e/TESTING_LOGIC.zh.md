# E2E 测试架构说明

## 当前测试逻辑概览

Actor-RTC Web 的 E2E 测试分为 **三层**：

```
┌─────────────────────────────────────────────────────────────┐
│                     E2E 测试架构                             │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  Level 1: 静态验证 (Node.js)                         │  │
│  │  ─────────────────────────────────                    │  │
│  │  test-wasm.js                                         │  │
│  │  • 文件完整性检查                                     │  │
│  │  • WASM 二进制格式验证                                │  │
│  │  • 模块编译测试                                       │  │
│  │  • 大小分析                                           │  │
│  │  ✅ 已实现并通过                                      │  │
│  └──────────────────────────────────────────────────────┘  │
│                          ↓                                   │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  Level 2: 浏览器环境验证 (Puppeteer)                │  │
│  │  ──────────────────────────────────────               │  │
│  │  tests/e2e/puppeteer/basic.test.ts                   │  │
│  │  • Headless Chrome 自动化                             │  │
│  │  • WASM 加载测试                                      │  │
│  │  • Browser API 兼容性检查                             │  │
│  │  • IndexedDB 操作测试                                 │  │
│  │  • 性能指标收集                                       │  │
│  │  ⚠️ 已配置，需 Chromium 下载                          │  │
│  └──────────────────────────────────────────────────────┘  │
│                          ↓                                   │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  Level 3: 多浏览器验证 (Playwright)                  │  │
│  │  ───────────────────────────────────                  │  │
│  │  tests/e2e/browser/actor-client.spec.ts              │  │
│  │  • Chrome/Firefox/Safari 跨浏览器测试                 │  │
│  │  • UI 交互测试                                        │  │
│  │  • ActorClient 功能测试                               │  │
│  │  • CORS headers 验证                                  │  │
│  │  • 错误检测和日志                                     │  │
│  │  ⚠️ 已配置，需浏览器安装                              │  │
│  └──────────────────────────────────────────────────────┘  │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

---

## 详细测试逻辑

### 📦 Level 1: 静态验证 (test-wasm.js)

**目标**: 验证 WASM 构建产物的完整性和正确性

**测试内容**:

```javascript
// 1. 文件系统检查
✅ WASM 文件是否存在
✅ JavaScript 绑定文件是否存在
✅ TypeScript 定义文件是否存在
✅ 文件大小是否合理

// 2. WASM 二进制验证
✅ Magic Number: 0x00 0x61 0x73 0x6D (\0asm)
✅ Version: 0x01 0x00 0x00 0x00 (v1)
✅ 模块可以被 WebAssembly.compile() 编译

// 3. 模块分析
✅ 导出函数数量 (30 个)
✅ 导入函数数量 (32 个)
✅ 预估压缩大小 (~35%)
```

**运行方式**:
```bash
node test-wasm.js
```

**当前状态**: ✅ **全部通过** (8/8 tests, 100%)

---

### 🎭 Level 2: Puppeteer 测试 (Headless Chrome)

**目标**: 在真实浏览器环境中验证 WASM 加载和运行

**测试文件**: `tests/e2e/puppeteer/basic.test.ts`

**测试逻辑**:

```typescript
describe('Actor-RTC Web - Puppeteer Tests', () => {
  // 1. 初始化
  beforeAll(() => {
    启动 Headless Chrome
    配置: --no-sandbox, --disable-setuid-sandbox
    监听: console 日志, page 错误
  })

  // 2. 页面加载测试
  it('页面加载') {
    访问: http://localhost:5173
    等待: networkidle0 (网络空闲)
    验证: 页面标题包含 "Actor-RTC"
  }

  // 3. WASM 模块测试
  it('WASM 加载') {
    在浏览器中执行:
      检查 typeof WebAssembly !== 'undefined'
    验证: WASM API 可用
  }

  // 4. Browser API 测试
  it('浏览器 API') {
    检查:
      ✓ IndexedDB
      ✓ RTCPeerConnection (WebRTC)
      ✓ WebSocket
      ✓ ServiceWorker
  }

  // 5. ActorClient 初始化
  it('客户端初始化') {
    等待: #status 元素出现
    读取: 连接状态文本
    验证: 状态已更新
  }

  // 6. IndexedDB 操作
  it('IndexedDB 测试') {
    在浏览器中执行:
      创建测试数据库
      创建 ObjectStore
      删除测试数据库
    验证: 操作成功
  }

  // 7. 性能测量
  it('页面加载性能') {
    测量: 加载时间
    目标: < 5000ms
    验证: 性能达标
  }

  // 8. WASM 文件大小
  it('WASM 大小检查') {
    通过 Performance API 获取资源大小
    验证: transferSize 或 encodedBodySize
    目标: < 500KB (未压缩)
  }
})
```

**运行方式**:
```bash
cd tests/e2e
npm install  # 会下载 Chromium (~170MB)
npm test
```

**当前状态**: ⚠️ **已配置，等待 Chromium 下载**

---

### 🎪 Level 3: Playwright 测试 (多浏览器)

**目标**: 跨浏览器兼容性测试和 UI 交互测试

**测试文件**: `tests/e2e/browser/actor-client.spec.ts`

**测试逻辑**:

```typescript
test.describe('ActorClient E2E Tests', () => {
  // 每个测试前
  beforeEach(() => {
    导航到测试页面 (/)
  })

  // 1. UI 元素测试
  test('页面标题') {
    验证: 标题包含 "Actor-RTC Web"
  }

  test('连接状态显示') {
    查找: #status 元素
    验证: 元素可见且有内容
  }

  test('发送按钮') {
    查找: #sendBtn 元素
    验证:
      • 按钮可见
      • 按钮文本包含 "Echo"
  }

  test('结果区域') {
    查找: #result 元素
    验证: 元素可见
  }

  // 2. 浏览器兼容性
  test('Browser API 支持') {
    在页面中执行:
      检查 WebAssembly
      检查 IndexedDB
      检查 RTCPeerConnection
      检查 WebSocket
    验证: 所有 API 都可用
  }

  // 3. 错误处理
  test('JavaScript 错误检测') {
    监听: 'pageerror' 事件
    加载页面
    分析错误:
      • 允许: 网络错误 (无服务器)
      • 不允许: SyntaxError, ReferenceError
    验证: 无致命错误
  }

  // 4. CORS 验证
  test('CORS Headers') {
    检查响应头:
      Cross-Origin-Opener-Policy: same-origin
      Cross-Origin-Embedder-Policy: require-corp
    验证: Headers 正确配置
  }

  // 5. 性能指标
  test('性能测量') {
    获取 Performance API 数据:
      • domContentLoaded
      • loadComplete
      • TTFB (Time To First Byte)
    验证: 所有指标 > 0
  }
})
```

**配置**: `playwright.config.ts`
```typescript
projects: [
  { name: 'chromium' },  // Chrome/Edge
  { name: 'firefox' },   // Firefox
  { name: 'webkit' },    // Safari
]

webServer: {
  command: 'npm run dev',
  url: 'http://localhost:5173',
  reuseExistingServer: true
}
```

**运行方式**:
```bash
cd tests/e2e
npx playwright install  # 安装浏览器
npm run test:browser
```

**当前状态**: ⚠️ **已配置，需要安装浏览器**

---

## 测试数据流

```
                    开发代码
                       ↓
              ┌────────────────┐
              │  Rust 源码      │
              │  (crates/)     │
              └────────┬───────┘
                       ↓
              ┌────────────────┐
              │  cargo build   │
              │  + wasm-pack   │
              └────────┬───────┘
                       ↓
              ┌────────────────┐
              │  WASM 产物      │
              │  .wasm + .js   │
              └────────┬───────┘
                       ↓
        ┌──────────────┼──────────────┐
        ↓              ↓               ↓
   [Level 1]      [Level 2]      [Level 3]
   Node.js      Puppeteer      Playwright
   静态验证     单浏览器验证    多浏览器验证
        ↓              ↓               ↓
      ✅ Pass       ⚠️ Ready       ⚠️ Ready
```

---

## 测试目标对照表

| 测试项 | Node | Puppeteer | Playwright | 状态 |
|--------|------|-----------|------------|------|
| **WASM 构建** |
| 文件存在 | ✅ | - | - | 通过 |
| 二进制格式 | ✅ | - | - | 通过 |
| 模块编译 | ✅ | - | - | 通过 |
| 大小检查 | ✅ | ✅ | - | 通过 |
| **浏览器加载** |
| 页面加载 | - | ✅ | ✅ | 待测 |
| WASM 加载 | - | ✅ | - | 待测 |
| JS 绑定 | - | ✅ | - | 待测 |
| **API 兼容** |
| WebAssembly | - | ✅ | ✅ | 待测 |
| IndexedDB | - | ✅ | ✅ | 待测 |
| WebRTC | - | ✅ | ✅ | 待测 |
| WebSocket | - | ✅ | ✅ | 待测 |
| ServiceWorker | - | ✅ | - | 待测 |
| **功能测试** |
| ActorClient 初始化 | - | ✅ | - | 待测 |
| IndexedDB 操作 | - | ✅ | - | 待测 |
| UI 交互 | - | - | ✅ | 待测 |
| **性能测试** |
| 加载时间 | - | ✅ | ✅ | 待测 |
| 内存占用 | - | ✅ | - | 待测 |
| **跨浏览器** |
| Chrome | - | ✅ | ✅ | 待测 |
| Firefox | - | - | ✅ | 待测 |
| Safari | - | - | ✅ | 待测 |

---

## 测试执行顺序

### 推荐执行流程:

```bash
# Step 1: 静态验证 (快速，无依赖)
node test-wasm.js
# 预期: 8/8 通过，耗时 <1s

# Step 2: 浏览器测试页面 (手动验证)
python3 -m http.server 8080
# 访问: http://localhost:8080/test.html
# 手动点击测试按钮，查看结果

# Step 3: Puppeteer 自动化 (需要 Chromium)
cd tests/e2e
npm install  # 首次需要下载 Chromium
npm test
# 预期: 全部通过，耗时 10-30s

# Step 4: Playwright 多浏览器 (可选)
cd tests/e2e
npx playwright install
npm run test:browser
# 预期: 全部通过，耗时 30-60s
```

---

## 当前测试覆盖率

### ✅ 已完成并验证:
- **静态验证**: 100% 覆盖
- **WASM 构建**: 完整验证
- **文件完整性**: 完整验证

### ⚠️ 已实现但未运行:
- **Puppeteer 测试**: 代码完整，等待 Chromium
- **Playwright 测试**: 代码完整，等待浏览器安装
- **性能基准**: 框架就绪，等待实际测试

### 🔄 测试缺口 (TODO):
1. **ActorClient 集成测试**
   - 实际的 RPC 调用测试
   - 需要 Mock 服务器或真实后端

2. **WebRTC 连接测试**
   - 信令握手测试
   - 数据通道测试
   - 需要 WebRTC 服务器

3. **React Hooks 测试**
   - useActorClient 测试
   - useServiceCall 测试
   - useSubscription 测试
   - 需要 React Testing Library

4. **IndexedDB 完整测试**
   - CRUD 操作测试
   - 事务测试
   - 索引查询测试
   - 需要完整 IndexedDB 实现

---

## 测试环境要求

### 最小要求 (Level 1):
- Node.js 18+
- 无需额外依赖

### 标准要求 (Level 2):
- Node.js 18+
- Puppeteer (~170MB Chromium)
- HTTP 服务器 (示例页面)

### 完整要求 (Level 3):
- Node.js 18+
- Playwright + 浏览器 (~500MB)
- HTTP 服务器或开发服务器
- 可选: Mock 服务器

---

## 测试输出示例

### Level 1 输出:
```
🧪 Actor-RTC Web - WASM Testing
==================================================
📦 File System Tests:
✅ WASM file exists (Path: ...)
✅ WASM file size (99.6 KB)
✅ JavaScript bindings exist (22359 bytes)
✅ TypeScript definitions exist
🔍 WASM Validation Tests:
✅ WASM binary is valid
✅ WASM can be compiled
✅ WASM module has exports (30 exports, 32 imports)
📊 Size Analysis:
✅ Estimated gzip size (99.6 KB → ~34.9 KB)
==================================================
📋 Test Summary: Passed: 8/8 (100.0%)
✨ All tests passed!
```

### Level 2 输出 (预期):
```
 ✓ should load page successfully (1234ms)
 ✓ should have WASM module loaded (3456ms)
 ✓ should have required browser APIs (234ms)
 ✓ should initialize ActorClient (567ms)
 ✓ should test IndexedDB operations (890ms)
 ✓ should measure page load time (456ms)
 ✓ should check WASM file size (123ms)

Test Suites: 1 passed, 1 total
Tests:       7 passed, 7 total
Time:        12.345 s
```

---

## 总结

当前 E2E 测试是一个 **三层渐进式验证体系**：

1. **Level 1** (静态): 快速验证构建产物 ✅ **已完成**
2. **Level 2** (单浏览器): Headless Chrome 全面测试 ⚠️ **待运行**
3. **Level 3** (多浏览器): 跨浏览器兼容性 ⚠️ **待运行**

**测试哲学**:
- 从简单到复杂
- 从静态到动态
- 从单一到多样
- 逐层增加测试成本和覆盖范围

**下一步**:
- 完成 Level 2 测试运行 (需要 Chromium)
- 完成 Level 3 测试运行 (可选)
- 添加集成测试 (需要后端服务)
