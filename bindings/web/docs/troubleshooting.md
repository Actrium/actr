# 故障排查指南

本指南帮助您解决使用 Actor-RTC Web 时遇到的常见问题。

## 构建问题

### WASM 编译失败

**问题:** `wasm-pack` 构建失败

**解决方案:**
1. 确保已安装 Rust 和 wasm-pack:
   ```bash
   rustup target add wasm32-unknown-unknown
   cargo install wasm-pack
   ```

2. 检查 Rust 版本:
   ```bash
   rustc --version
   # 应该是 1.88+ 或更新
   ```

3. 清理并重新构建:
   ```bash
   cargo clean
   npm run build:wasm
   ```

### Vite 配置问题

**问题:** Vite 无法加载 WASM 模块

**解决方案:**
确保安装了必需的插件:

```bash
npm install --save-dev vite-plugin-wasm vite-plugin-top-level-await
```

并在 `vite.config.ts` 中正确配置:

```typescript
import wasm from 'vite-plugin-wasm';
import topLevelAwait from 'vite-plugin-top-level-await';

export default defineConfig({
  plugins: [wasm(), topLevelAwait()],
});
```

### CORS 错误

**问题:** 浏览器显示 CORS 错误

**解决方案:**
在开发服务器中添加必要的 headers:

```typescript
// vite.config.ts
export default defineConfig({
  server: {
    headers: {
      'Cross-Origin-Opener-Policy': 'same-origin',
      'Cross-Origin-Embedder-Policy': 'require-corp',
    },
  },
});
```

## 运行时问题

### 连接失败

**问题:** 无法连接到信令服务器

**检查项:**
1. 信令服务器URL是否正确
2. 网络连接是否正常
3. 防火墙是否阻止 WebSocket 连接

**调试:**
```typescript
const actor = await createActor({
  signalingUrl: 'wss://signal.example.com',
  realm: 'demo',
  debug: true, // 启用调试日志
});
```

### IndexedDB 错误

**问题:** IndexedDB 操作失败

**可能原因:**
1. 浏览器隐私模式可能禁用 IndexedDB
2. 存储空间不足
3. 浏览器不支持 IndexedDB

**解决方案:**
```typescript
// 检查 IndexedDB 是否可用
if (!window.indexedDB) {
  console.error('IndexedDB is not supported');
}
```

### WebRTC 连接问题

**问题:** WebRTC 连接建立失败

**检查项:**
1. 检查 STUN/TURN 服务器配置
2. 检查防火墙设置
3. 检查 NAT 类型

**配置 TURN 服务器:**
```typescript
const actor = await createActor({
  signalingUrl: 'wss://signal.example.com',
  realm: 'demo',
  iceServers: [
    { urls: 'stun:stun.l.google.com:19302' },
    {
      urls: 'turn:turn.example.com:3478',
      username: 'user',
      credential: 'pass',
    },
  ],
});
```

## 性能问题

### WASM 加载缓慢

**问题:** WASM 文件加载时间过长

**优化方案:**
1. 确保启用了 gzip 压缩
2. 使用 CDN 加速
3. 实施预加载策略

```html
<!-- 预加载 WASM -->
<link rel="preload" href="/actr_runtime_web_bg.wasm" as="fetch" crossorigin>
```

### 内存泄漏

**问题:** 应用运行时内存持续增长

**检查项:**
1. 确保正确取消订阅
2. 及时关闭客户端连接
3. 清理事件监听器

```typescript
// 正确的清理
useEffect(() => {
  const unsubscribe = await actor.subscribe(...);

  return () => {
    unsubscribe(); // 清理订阅
  };
}, [actor]);
```

## React 相关问题

### Hook 无限重渲染

**问题:** 组件不断重新渲染

**原因:** 配置对象在每次渲染时都是新对象

**解决方案:**
```tsx
// ❌ 错误
function App() {
  const { actor } = useActor({
    signalingUrl: 'wss://signal.example.com',
    realm: 'demo',
  }); // 每次渲染都创建新对象
}

// ❤ 正确
function App() {
  const config = useMemo(() => ({
    signalingUrl: 'wss://signal.example.com',
    realm: 'demo',
  }), []); // 只创建一次

  const { actor } = useActor(config);
}
```

## 浏览器兼容性

### 不支持的浏览器

**支持的浏览器:**
- Chrome/Edge 90+
- Firefox 88+
- Safari 15+

**检查浏览器支持:**
```typescript
function checkBrowserSupport() {
  const features = {
    wasm: typeof WebAssembly !== 'undefined',
    webrtc: 'RTCPeerConnection' in window,
    indexedDB: 'indexedDB' in window,
    serviceWorker: 'serviceWorker' in navigator,
  };

  const unsupported = Object.entries(features)
    .filter(([, supported]) => !supported)
    .map(([feature]) => feature);

  if (unsupported.length > 0) {
    console.error('Unsupported features:', unsupported);
    return false;
  }

  return true;
}
```

## 错误代码参考

| 错误代码 | 描述 | 可能原因 |
|---------|------|---------|
| `NETWORK_ERROR` | 网络错误 | 网络连接问题,服务器不可达 |
| `TIMEOUT_ERROR` | 超时错误 | 请求超时,网络延迟过高 |
| `CONNECTION_ERROR` | 连接错误 | WebRTC连接失败,信令错误 |
| `SERIALIZATION_ERROR` | 序列化错误 | 数据格式错误,类型不匹配 |
| `SERVICE_NOT_FOUND` | 服务未找到 | 服务名称错误,服务未注册 |
| `METHOD_NOT_FOUND` | 方法未找到 | 方法名称错误,方法不存在 |
| `INTERNAL_ERROR` | 内部错误 | 框架内部错误,需要检查日志 |
| `CONFIG_ERROR` | 配置错误 | 配置参数无效 |

## 调试技巧

### 启用详细日志

```typescript
const actor = await createActor({
  signalingUrl: 'wss://signal.example.com',
  realm: 'demo',
  debug: true, // 启用调试模式
});
```

### 监控连接状态

```typescript
actor.on('stateChange', (state) => {
  console.log('Connection state changed:', state);
});
```

### 检查 WASM 加载

```typescript
// 在浏览器控制台
console.log('WASM loaded:', typeof window.actr_runtime_web !== 'undefined');
```

## 获取帮助

如果问题仍未解决:

1. 查看 [GitHub Issues](https://github.com/actor-rtc/actr/issues)
2. 加入社区讨论
3. 提交 Bug 报告(附带详细信息和复现步骤)

## 常见问题 FAQ

**Q: 为什么首次加载很慢?**
A: WASM 文件需要下载和编译。建议启用 gzip 压缩和使用 CDN。

**Q: 可以在 Node.js 中使用吗?**
A: 当前版本专为浏览器设计,Node.js 支持在规划中。

**Q: Service Worker 和 Web Worker 有什么区别?**
A: Service Worker 具有持久化和后台能力,推荐生产环境使用。Web Worker 调试更方便,适合开发环境。

**Q: 如何处理重连?**
A: 设置 `autoReconnect: true` 启用自动重连,或使用 `reconnect()` 方法手动重连。
