# E2E 测试

Actor-RTC Web 的端到端测试套件。

## 测试工具

- **Puppeteer**: Headless Chrome 自动化测试
- **Playwright**: 多浏览器测试(Chrome, Firefox, Safari)
- **Vitest**: 测试框架和断言

## 测试结构

```
tests/e2e/
├── puppeteer/          # Puppeteer 测试
│   └── basic.test.ts   # 基础功能测试
├── browser/            # Playwright 测试
│   └── actor-client.spec.ts  # ActorClient E2E 测试
├── package.json
├── vitest.config.ts
└── playwright.config.ts
```

## 运行测试

### 1. 安装依赖

```bash
cd tests/e2e
npm install
```

Puppeteer 会自动下载 Chromium (~170MB)。

### 2. 启动开发服务器

在项目根目录:

```bash
cd ../../examples/hello-world
npm install
npm run dev
```

保持服务器运行在 http://localhost:5173

### 3. 运行 Puppeteer 测试

在另一个终端:

```bash
cd tests/e2e
npm test
```

### 4. 运行 Playwright 测试

```bash
# 首次运行需要安装浏览器
npx playwright install

# 运行测试
npm run test:browser
```

## 测试内容

### Puppeteer 测试 (puppeteer/basic.test.ts)

- ✅ 页面加载
- ✅ WASM 模块加载
- ✅ 浏览器 API 支持检查
- ✅ ActorClient 初始化
- ✅ IndexedDB 操作
- ✅ 页面加载性能
- ✅ WASM 文件大小检查

### Playwright 测试 (browser/actor-client.spec.ts)

- ✅ 页面标题
- ✅ 连接状态显示
- ✅ 按钮交互
- ✅ 浏览器 API 支持
- ✅ JavaScript 错误检查
- ✅ CORS headers 验证
- ✅ 性能指标

## 调试

### Puppeteer 调试

```typescript
// 在 beforeAll 中设置
browser = await puppeteer.launch({
  headless: false,    // 显示浏览器
  devtools: true,     // 打开 DevTools
  slowMo: 100,        // 减慢操作速度
});
```

### Playwright 调试

```bash
# 调试模式运行
npm run test:debug

# UI 模式
npm run test:ui
```

## 环境要求

- Node.js 18+
- 足够的磁盘空间 (~200MB for browsers)
- Linux/macOS: 可能需要额外的系统依赖

### Ubuntu/Debian 系统依赖

```bash
sudo apt-get update
sudo apt-get install -y \
  libnss3 \
  libatk1.0-0 \
  libatk-bridge2.0-0 \
  libcups2 \
  libdrm2 \
  libxkbcommon0 \
  libxcomposite1 \
  libxdamage1 \
  libxfixes3 \
  libxrandr2 \
  libgbm1 \
  libasound2
```

## CI/CD 集成

### GitHub Actions 示例

```yaml
name: E2E Tests

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: actions/setup-node@v3
        with:
          node-version: 18

      - name: Install dependencies
        run: |
          npm install
          cd tests/e2e && npm install

      - name: Build WASM
        run: npm run build:wasm

      - name: Start dev server
        run: |
          cd examples/hello-world
          npm install
          npm run dev &
          sleep 5

      - name: Run Puppeteer tests
        run: cd tests/e2e && npm test

      - name: Install Playwright browsers
        run: cd tests/e2e && npx playwright install --with-deps

      - name: Run Playwright tests
        run: cd tests/e2e && npm run test:browser
```

## 常见问题

### Q: Chromium 下载失败

A: 设置 Puppeteer 镜像:

```bash
export PUPPETEER_DOWNLOAD_HOST=https://registry.npmmirror.com/-/binary/chromium-browser-snapshots
npm install puppeteer
```

### Q: 测试超时

A: 增加超时时间:

```typescript
test('test name', async () => {
  // ...
}, 60000); // 60 秒
```

### Q: WASM 加载失败

A: 检查:
1. WASM 文件是否构建成功
2. Vite 配置是否正确
3. CORS headers 是否设置

### Q: IndexedDB 测试失败

A: 某些 headless 环境可能不完全支持 IndexedDB。使用 `headless: false` 调试。

## 性能基准

预期性能指标:

| 指标 | 目标 | 测试方法 |
|-----|------|---------|
| 页面加载 | <2s | Puppeteer 测量 |
| WASM 大小 | <150KB (gzip) | Network 面板 |
| 消息延迟 | <50ms | RPC 调用测试 |
| 内存占用 | <50MB | Chrome DevTools |

## 贡献

添加新测试时:

1. 在 `puppeteer/` 或 `browser/` 下创建新文件
2. 使用描述性的测试名称
3. 添加适当的错误处理
4. 更新本 README

## 参考

- [Puppeteer 文档](https://pptr.dev/)
- [Playwright 文档](https://playwright.dev/)
- [Vitest 文档](https://vitest.dev/)
