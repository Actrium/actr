# 零拷贝方案对比示例

演示三种零拷贝策略的性能和使用场景。

---

## 方案对比

### 方案 0：基线（Transferable + to_vec）

**适用场景**: 小数据 (<4KB)

```javascript
// DOM 端
const data = new Uint8Array([1, 2, 3, 4]);
port.postMessage({
  type: 'fastpath_data',
  stream_id: 'test:small',
  data: data.buffer,
  timestamp: Date.now()
}, [data.buffer]); // Transferable

// Service Worker (WASM)
const fastpathData = new FastPathData(stream_id, uint8Array, timestamp);
const chunk = fastpathData.to_chunk(); // 1 次拷贝
actorSystem.dispatch_fastpath(chunk);
```

**性能**:
- 拷贝次数: 1
- 延迟: ~1μs (可忽略)
- 适用: 控制消息、传感器数据

---

### 方案 1：SharedArrayBuffer（完全零拷贝）

**适用场景**: 大数据高频 (>=1MB, >=30fps)

```javascript
// DOM 端
const sab = new SharedArrayBuffer(1920 * 1080 * 3); // 6.2MB (1080p RGB)
const view = new Uint8Array(sab);

// 获取视频帧
canvas.getContext('2d').getImageData(0, 0, 1920, 1080);
view.set(imageData.data, 0);

// 发送（SharedArrayBuffer 直接传递，非 Transferable）
port.postMessage({
  type: 'fastpath_zerocopy',
  stream_id: 'peer:video',
  buffer: sab,      // 共享内存
  offset: 0,
  length: view.length,
  timestamp: Date.now()
});

// Service Worker (WASM)
const fastpathData = new FastPathDataZeroCopy(
  stream_id,
  buffer,
  offset,
  length,
  timestamp
);

// 零拷贝访问
const view = fastpathData.as_view(); // Uint8Array 视图（零拷贝！）

// 直接处理，无需拷贝
for (let i = 0; i < view.length; i += 4) {
  const r = view[i];
  const g = view[i + 1];
  const b = view[i + 2];
  // 处理像素...
}
```

**性能**:
- 拷贝次数: 0
- 延迟: ~0.1ms (6.2MB)
- 适用: 视频流、大音频块

**限制**:
- 需要 COOP/COEP headers
- 浏览器兼容性: Chrome 92+, Firefox 79+

---

### 方案 2：WASM 内存池（内存复用）

**适用场景**: 中等数据 (4KB-1MB)

```javascript
// 初始化（WASM 端）
const memoryPool = new WasmMemoryPool(
  1024 * 1024, // 每个缓冲区 1MB
  10           // 池容量 10 个
);

// JS 端使用
async function processAudioChunk(audioData) {
  // 1. 分配 WASM 缓冲区
  const buffer = memoryPool.allocate();
  const ptr = buffer.ptr();
  const capacity = buffer.capacity();

  // 2. 创建 WASM 内存视图
  const wasmMemory = new Uint8Array(
    wasm.memory.buffer,
    ptr,
    capacity
  );

  // 3. 直接写入 WASM 内存（JS 侧拷贝）
  wasmMemory.set(new Uint8Array(audioData), 0);

  // 4. 通知 WASM 数据就绪
  buffer.set_length(audioData.byteLength);

  // 5. WASM 端处理（零拷贝）
  const bytes = buffer.to_bytes(); // 零拷贝包装
  actorSystem.dispatch_fastpath_bytes(stream_id, bytes);

  // 6. 处理完毕后回收
  memoryPool.recycle(buffer);
}
```

**性能**:
- 拷贝次数: 1 (JS 侧)
- 延迟: ~2ms (1MB)
- 适用: 音频块、中等图片

**优势**:
- 无浏览器限制
- 内存复用（减少 GC 压力）

---

## 混合策略（推荐）

自动根据数据大小选择最优策略：

```javascript
class FastPathAdapter {
  constructor() {
    // SharedArrayBuffer 池（大数据）
    this.sabPool = new SharedBufferPool(6 * 1024 * 1024, 5); // 5 个 6MB

    // WASM 内存池（中等数据）
    this.wasmPool = new WasmMemoryPool(1024 * 1024, 10); // 10 个 1MB

    // 策略选择器
    this.strategy = new StrategySelector();
  }

  send(streamId, data) {
    const size = data.byteLength;
    const strategy = this.strategy.choose(size, this.getFrequency(streamId));

    switch (strategy) {
      case 'direct':
        this.sendDirect(streamId, data);
        break;

      case 'shared':
        this.sendShared(streamId, data);
        break;

      case 'pool':
        this.sendPool(streamId, data);
        break;
    }
  }

  sendDirect(streamId, data) {
    // 小数据：Transferable
    port.postMessage({
      type: 'fastpath_data',
      stream_id: streamId,
      data: data,
      timestamp: Date.now()
    }, [data]);
  }

  sendShared(streamId, data) {
    // 大数据：SharedArrayBuffer
    const sab = this.sabPool.get(data.byteLength);
    new Uint8Array(sab).set(new Uint8Array(data));

    port.postMessage({
      type: 'fastpath_zerocopy',
      stream_id: streamId,
      buffer: sab,
      offset: 0,
      length: data.byteLength,
      timestamp: Date.now()
    });
  }

  sendPool(streamId, data) {
    // 中等数据：WASM Memory Pool
    const buffer = this.wasmPool.allocate();
    const view = new Uint8Array(
      wasm.memory.buffer,
      buffer.ptr(),
      buffer.capacity()
    );
    view.set(new Uint8Array(data));

    buffer.set_length(data.byteLength);
    actorSystem.dispatch_from_pool(streamId, buffer);
  }

  getFrequency(streamId) {
    // 估算发送频率（fps）
    return this.streamStats.get(streamId)?.fps || 0;
  }
}

class StrategySelector {
  choose(size, frequency) {
    // 小数据：直接拷贝
    if (size < 4096) {
      return 'direct';
    }

    // 大数据高频：SharedArrayBuffer
    if (size >= 1024 * 1024 && frequency >= 30) {
      return 'shared';
    }

    // 中等数据或低频：WASM Pool
    return 'pool';
  }
}
```

---

## 性能测试

### 延迟对比（不同数据大小）

```javascript
async function benchmarkLatency() {
  const sizes = [1024, 4096, 16384, 65536, 262144, 1048576, 6291456]; // 1KB - 6MB
  const results = [];

  for (const size of sizes) {
    const data = new Uint8Array(size);

    // 方案 0：Transferable
    const t0 = performance.now();
    sendDirect('bench:direct', data.buffer);
    const directLatency = performance.now() - t0;

    // 方案 1：SharedArrayBuffer
    const t1 = performance.now();
    sendShared('bench:shared', data.buffer);
    const sharedLatency = performance.now() - t1;

    // 方案 2：WASM Pool
    const t2 = performance.now();
    sendPool('bench:pool', data.buffer);
    const poolLatency = performance.now() - t2;

    results.push({
      size,
      direct: directLatency,
      shared: sharedLatency,
      pool: poolLatency
    });
  }

  console.table(results);
}
```

**预期结果**:

| 大小 | Direct (ms) | Shared (ms) | Pool (ms) | 最优 |
|------|-------------|-------------|-----------|------|
| 1KB | 0.01 | 0.05 | 0.03 | Direct |
| 4KB | 0.02 | 0.05 | 0.03 | Direct |
| 16KB | 0.08 | 0.05 | 0.10 | Shared |
| 64KB | 0.30 | 0.05 | 0.35 | Shared |
| 256KB | 1.20 | 0.05 | 1.40 | Shared |
| 1MB | 4.80 | 0.05 | 5.50 | Shared |
| 6MB | 28.00 | 0.10 | 32.00 | Shared |

### 吞吐测试（60fps 视频流）

```javascript
async function benchmarkThroughput() {
  const frameSize = 1920 * 1080 * 3; // 6.2MB
  const fps = 60;
  const duration = 10; // 10 秒
  const totalFrames = fps * duration;

  const strategies = ['direct', 'shared', 'pool'];
  const results = {};

  for (const strategy of strategies) {
    const startTime = performance.now();
    let framesProcessed = 0;

    for (let i = 0; i < totalFrames; i++) {
      const frame = generateFrame(frameSize);
      await sendFrame(strategy, frame);
      framesProcessed++;
    }

    const endTime = performance.now();
    const elapsedTime = endTime - startTime;
    const actualFps = framesProcessed / (elapsedTime / 1000);

    results[strategy] = {
      framesProcessed,
      elapsedTime,
      actualFps,
      avgLatency: elapsedTime / framesProcessed
    };
  }

  console.table(results);
}
```

**预期结果**:

| 策略 | 帧数 | 耗时 (s) | 实际 fps | 平均延迟 (ms) |
|------|------|----------|----------|---------------|
| Direct | 600 | 16.8 | 35.7 | 28.0 |
| Shared | 600 | 10.1 | 59.4 | 0.1 |
| Pool | 600 | 19.2 | 31.3 | 32.0 |

**结论**: SharedArrayBuffer 在大数据高频场景下性能提升 **40-60%**

---

## 浏览器兼容性

### 启用 SharedArrayBuffer

需要配置服务器 headers：

```nginx
# nginx.conf
add_header Cross-Origin-Opener-Policy "same-origin";
add_header Cross-Origin-Embedder-Policy "require-corp";
```

或使用开发服务器：

```javascript
// dev-server.js
const express = require('express');
const app = express();

app.use((req, res, next) => {
  res.setHeader('Cross-Origin-Opener-Policy', 'same-origin');
  res.setHeader('Cross-Origin-Embedder-Policy', 'require-corp');
  next();
});

app.use(express.static('.'));
app.listen(8000);
```

### Polyfill（降级方案）

```javascript
class FastPathAdapter {
  constructor() {
    // 检测 SharedArrayBuffer 支持
    this.sharedSupported = typeof SharedArrayBuffer !== 'undefined';

    if (this.sharedSupported) {
      console.log('✅ SharedArrayBuffer supported - using zero-copy');
      this.sabPool = new SharedBufferPool(6 * 1024 * 1024, 5);
    } else {
      console.warn('⚠️ SharedArrayBuffer not supported - using fallback');
      // 降级到 WASM Pool
      this.wasmPool = new WasmMemoryPool(6 * 1024 * 1024, 5);
    }
  }

  send(streamId, data) {
    const size = data.byteLength;

    if (size < 4096) {
      return this.sendDirect(streamId, data);
    }

    if (this.sharedSupported && size >= 1024 * 1024) {
      return this.sendShared(streamId, data);
    }

    return this.sendPool(streamId, data);
  }
}
```

---

## 运行示例

1. **启动服务器**（支持 COOP/COEP）
   ```bash
   node dev-server.js
   ```

2. **访问示例**
   ```
   http://localhost:8000/examples/zerocopy-comparison/
   ```

3. **运行性能测试**
   - 打开浏览器控制台
   - 执行 `benchmarkLatency()`
   - 执行 `benchmarkThroughput()`

---

## 相关文档

- [零拷贝优化方案](../../docs/architecture/zerocopy-optimization.md)
- [Fast Path 实现总结](../../docs/P1_FASTPATH_ROUTETABLE_COMPLETION.md)
- [SharedArrayBuffer MDN](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/SharedArrayBuffer)
