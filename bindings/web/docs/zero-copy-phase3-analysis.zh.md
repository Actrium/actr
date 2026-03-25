# Phase 3 零拷贝优化技术分析

## 概述

本文档详细分析 Phase 3 零拷贝优化的技术可行性、实施策略和风险评估。

---

## 1. 技术方案分析

### 1.1 方案 A：WASM 线性内存直接访问（推荐优先实施）

#### 原理
- WASM 模块有一个连续的线性内存空间
- JavaScript 可以通过 `WebAssembly.Memory` 访问这块内存
- Rust 可以通过 `wasm_bindgen::memory()` 暴露内存访问接口
- 使用 `Uint8Array` 创建视图而非拷贝数据

#### 实现策略

**接收路径优化**：
```rust
// 当前实现（2 次拷贝）
let data = uint8_array.to_vec();  // 拷贝 1
let payload_data = Bytes::copy_from_slice(&data[5..]);  // 拷贝 2

// 优化方案（1 次拷贝）
let len = uint8_array.length() as usize;
let mut buffer = Vec::with_capacity(len);
unsafe {
    buffer.set_len(len);
    // 使用浏览器优化的 memcpy，直接写入 WASM 线性内存
    uint8_array.copy_to(&mut buffer);
}
// 零拷贝：Bytes::from() 转移 Vec 所有权
let payload_data = Bytes::from(buffer.split_off(5));
```

**发送路径优化**：
```rust
// 当前实现（2 次拷贝）
let mut msg = Vec::new();
msg.extend_from_slice(&data);  // 拷贝 1
let js_array = Uint8Array::from(&msg[..]);  // 拷贝 2

// 优化方案（0 次拷贝）
let js_array = unsafe {
    // 直接在 WASM 内存上创建 Uint8Array 视图
    let ptr = data.as_ptr();
    let len = data.len();
    js_sys::Uint8Array::view_mut_raw(ptr, len)
};
```

#### 优势
- ✅ 实施简单，只需修改数据转换代码
- ✅ 无需特殊浏览器配置（COOP/COEP）
- ✅ 所有现代浏览器支持
- ✅ 内存安全（使用 Rust 的生命周期保证）

#### 风险和限制
- ⚠️ 需要使用 `unsafe` 代码块
- ⚠️ 必须确保 JS 使用视图时 Rust 端数据未被释放
- ⚠️ 视图是只读的（发送路径），不能修改

#### 适用场景
- ✅ 所有通信路径（PostMessage、DataChannel、WebSocket）
- ✅ 接收和发送两个方向
- ✅ 所有数据大小

---

### 1.2 方案 B：Transferable Objects（中等优先级）

#### 原理
- `postMessage` 可以转移 ArrayBuffer 的所有权而非拷贝
- 转移后原 buffer 变为 detached，不可再访问
- 接收端获得独占所有权

#### 实现策略

```rust
// PostMessage 发送路径
pub async fn send_with_transfer(&self, data: Bytes) -> LaneResult<()> {
    // 构造消息
    let mut msg = Vec::with_capacity(5 + data.len());
    msg.push(*payload_type as u8);
    msg.extend_from_slice(&(data.len() as u32).to_be_bytes());
    msg.extend_from_slice(&data);

    let js_array = js_sys::Uint8Array::from(&msg[..]);
    let buffer = js_array.buffer();

    // 创建 transferable 数组
    let transferable = js_sys::Array::new();
    transferable.push(&buffer);

    // 使用 transfer 语义
    port.post_message_with_transfer(&js_array.into(), &transferable)
        .map_err(|e| WebError::Transport(format!("Transfer failed: {:?}", e)))?;

    Ok(())
}
```

#### 优势
- ✅ 消除 postMessage 的结构化克隆开销
- ✅ 无需特殊浏览器配置
- ✅ 所有现代浏览器支持

#### 风险和限制
- ⚠️ 只适用于 PostMessage 通信
- ⚠️ 转移后原 buffer 不可用，需要管理生命周期
- ⚠️ 对于小数据（<10KB），转移开销可能大于拷贝
- ⚠️ 不能用于 WebSocket、DataChannel（它们不支持 transfer）

#### 适用场景
- ✅ PostMessage 通信（Service Worker ↔ DOM）
- ✅ 大数据传输（>10KB）
- ❌ WebSocket、DataChannel

---

### 1.3 方案 C：SharedArrayBuffer（高风险，需谨慎）

#### 原理
- SharedArrayBuffer 在多个上下文间共享内存
- 真正的零拷贝，所有上下文看到同一块内存
- 需要 Atomic 操作保证并发安全

#### 实现策略

```rust
// 共享内存池
pub struct SharedMemoryPool {
    buffers: DashMap<String, js_sys::SharedArrayBuffer>,
    allocator: parking_lot::Mutex<PoolAllocator>,
}

impl SharedMemoryPool {
    pub fn allocate(&self, size: usize) -> Option<SharedBuffer> {
        let buffer = js_sys::SharedArrayBuffer::new(size as u32);
        let id = uuid::Uuid::new_v4().to_string();
        self.buffers.insert(id.clone(), buffer.clone());

        Some(SharedBuffer {
            id,
            buffer,
            offset: 0,
            length: size,
        })
    }

    pub fn write(&self, buffer_id: &str, offset: usize, data: &[u8]) -> Result<()> {
        let buffer = self.buffers.get(buffer_id)
            .ok_or(WebError::Transport("Buffer not found".to_string()))?;

        let view = js_sys::Uint8Array::new(buffer.value());
        view.set(&js_sys::Uint8Array::from(data), offset as u32);
        Ok(())
    }
}

// 接收路径
let onmessage_callback = Closure::wrap(Box::new(move |e: MessageEvent| {
    if let Ok(obj) = e.data().dyn_into::<js_sys::Object>() {
        let buffer_id = js_sys::Reflect::get(&obj, &"buffer_id".into()).unwrap();
        let offset = js_sys::Reflect::get(&obj, &"offset".into()).unwrap();
        let length = js_sys::Reflect::get(&obj, &"length".into()).unwrap();

        // 零拷贝：直接访问共享内存
        let buffer = pool.get_buffer(&buffer_id).unwrap();
        let data = buffer.read(offset, length);

        let _ = tx_clone.unbounded_send(data);
    }
}));
```

#### 优势
- ✅ 真正的零拷贝（所有上下文共享同一块内存）
- ✅ 最佳性能（无任何拷贝开销）
- ✅ 适合高频大数据传输

#### 风险和限制
- ❌ **高风险**：需要启用 COOP/COEP HTTP 头
  ```http
  Cross-Origin-Opener-Policy: same-origin
  Cross-Origin-Embedder-Policy: require-corp
  ```
- ❌ 这会破坏跨域资源加载（CDN、第三方脚本等）
- ❌ 需要复杂的并发控制（Atomic 操作）
- ❌ 内存管理复杂（何时释放共享内存？）
- ❌ 浏览器兼容性有限（Chrome 92+, Firefox 95+, Safari 15.2+）
- ❌ 调试困难（数据竞争、内存损坏）

#### 适用场景
- ⚠️ **仅适用于受控环境**（如企业内网应用）
- ⚠️ 需要极致性能的场景（视频会议、云游戏）
- ❌ **不推荐用于公共 Web 应用**

---

### 1.4 方案 D：MediaStreamTrackProcessor（Insertable Streams）

#### 原理
- WebRTC Insertable Streams API 允许访问 encoded frames
- 可以在 JavaScript 中拦截和处理媒体帧
- 支持 VideoFrame 和 AudioFrame 对象

#### 实现策略

**JavaScript 侧**：
```javascript
// 获取 Receiver 的 readable stream
const receiver = peerConnection.getReceivers()[0];
const readableStream = receiver.readable;
const reader = readableStream.getReader();

// 读取 encoded frames
while (true) {
    const {value: encodedFrame, done} = await reader.read();
    if (done) break;

    // 转发到 Rust 端（通过 wasm-bindgen）
    await rust_handle_media_frame(encodedFrame);
}
```

**Rust 侧**：
```rust
#[wasm_bindgen]
pub async fn rust_handle_media_frame(frame: JsValue) {
    // 尝试转换为 RTCEncodedVideoFrame 或 RTCEncodedAudioFrame
    if let Ok(video_frame) = frame.dyn_into::<web_sys::RtcEncodedVideoFrame>() {
        // 获取 frame 数据（ArrayBuffer）
        let data_buffer = video_frame.data();

        // 零拷贝：直接访问 ArrayBuffer
        let uint8_array = js_sys::Uint8Array::new(&data_buffer);

        // ... 处理帧数据
    }
}
```

#### 优势
- ✅ 专为媒体流设计，性能优异
- ✅ 直接访问 encoded frames，无需解码
- ✅ 支持修改和转发帧

#### 风险和限制
- ❌ 浏览器兼容性差（仅 Chrome/Edge 90+）
- ❌ Firefox 和 Safari 不支持
- ❌ API 仍在实验阶段，可能变更
- ⚠️ 只适用于 WebRTC 媒体流

#### 适用场景
- ✅ WebRTC 媒体流处理
- ❌ 其他通信场景

---

## 2. 实施优先级和策略

### 阶段 1：WASM 线性内存优化（推荐立即实施）

**目标**：消除 1 次数据拷贝，从 2 次降到 1 次

**实施步骤**：
1. 创建 `ZeroCopyBuffer` 工具类
2. 修改接收路径：使用 `Bytes::from()` 替代 `copy_from_slice()`
3. 修改发送路径：使用 `Uint8Array::view()` 创建视图
4. 编写单元测试验证正确性
5. 性能基准测试对比

**影响范围**：
- `runtime-sw/src/transport/postmessage.rs`
- `runtime-sw/src/transport/websocket.rs`
- `runtime-dom/src/transport/postmessage.rs`
- `runtime-dom/src/transport/webrtc_datachannel.rs`
- `runtime-dom/src/transport/lane.rs`
- `runtime-sw/src/transport/lane.rs`

**风险评估**：
- 风险等级：**低**
- 需要 `unsafe` 代码，但范围可控
- 可以通过测试充分验证

**预期收益**：
- 性能提升：40-50%
- 延迟降低：30-40%
- CPU 使用降低：20-30%

### 阶段 2：Transferable Objects 优化（可选）

**目标**：消除 PostMessage 的结构化克隆开销

**实施步骤**：
1. 为 PostMessage Lane 添加 `send_with_transfer()` 方法
2. 在 DOM 和 SW 两端实现
3. 处理 detached buffer 的生命周期
4. 编写测试验证转移语义

**影响范围**：
- `runtime-sw/src/transport/lane.rs`
- `runtime-dom/src/transport/lane.rs`

**风险评估**：
- 风险等级：**低-中**
- 需要正确管理 buffer 生命周期
- 对小数据可能性能下降

**预期收益**：
- 性能提升：10-20%（大数据）
- 对小数据（<10KB）可能无提升

### 阶段 3：SharedArrayBuffer 优化（谨慎评估）

**目标**：真正的零拷贝

**实施步骤**：
1. 设计共享内存池架构
2. 实现内存分配器和回收机制
3. 实现 Atomic 操作的并发控制
4. 配置 COOP/COEP HTTP 头
5. 充分的并发测试和压力测试

**影响范围**：
- 整个通信层
- HTTP 服务器配置
- 第三方资源加载策略

**风险评估**：
- 风险等级：**高**
- 破坏性变更（COOP/COEP）
- 复杂的并发问题
- 跨浏览器兼容性问题

**预期收益**：
- 性能提升：80-90%（理论最大值）
- 延迟降低：80-90%
- CPU 使用降低：60-70%

**决策建议**：
- ❌ **不推荐用于 actr-web 公共版本**
- ✅ 可以作为可选特性（feature flag）
- ✅ 适合企业定制版本

### 阶段 4：MediaStreamTrackProcessor（实验性）

**目标**：优化媒体流处理

**实施步骤**：
1. 编写 web-sys 绑定（如果官方未提供）
2. 实现 Insertable Streams 集成
3. 处理 VideoFrame/AudioFrame
4. 添加浏览器特性检测

**影响范围**：
- `runtime-dom/src/transport/webrtc_mediatrack.rs`
- 可能需要新增 JS glue 代码

**风险评估**：
- 风险等级：**中-高**
- 浏览器兼容性极差
- API 不稳定

**预期收益**：
- 媒体流性能提升：60-80%
- 仅适用于 Chrome/Edge

**决策建议**：
- ⚠️ 作为可选特性（feature flag）
- ⚠️ 提供降级方案（不支持的浏览器使用当前实现）

---

## 3. 安全性和正确性保证

### 3.1 内存安全

#### WASM 线性内存访问规则

```rust
// ✅ 安全：使用 Closure::forget() 确保回调生命周期
let callback = Closure::wrap(Box::new(move |e: MessageEvent| {
    // 在回调中使用 WASM 内存视图
    let view = unsafe { ... };
}));
callback.forget();

// ❌ 不安全：视图生命周期超出数据生命周期
let data = vec![1, 2, 3];
let view = unsafe { Uint8Array::view(&data) };
drop(data);  // 数据被释放
// view 现在是悬垂指针！
```

#### 解决方案：使用 Pin 和生命周期标记

```rust
use std::pin::Pin;

pub struct ZeroCopyView<'a> {
    _data: Pin<Box<Vec<u8>>>,  // 固定在堆上，不会移动
    view: js_sys::Uint8Array,
    _marker: PhantomData<&'a ()>,
}

impl<'a> ZeroCopyView<'a> {
    pub fn new(data: Vec<u8>) -> Self {
        let pinned = Box::pin(data);
        let view = unsafe {
            let ptr = pinned.as_ptr();
            let len = pinned.len();
            js_sys::Uint8Array::view_mut_raw(ptr, len)
        };

        Self {
            _data: pinned,
            view,
            _marker: PhantomData,
        }
    }
}
```

### 3.2 并发安全（SharedArrayBuffer）

#### 数据竞争预防

```rust
use std::sync::atomic::{AtomicU32, Ordering};

pub struct SharedBuffer {
    buffer: js_sys::SharedArrayBuffer,
    write_offset: AtomicU32,  // Atomic 写指针
    read_offset: AtomicU32,   // Atomic 读指针
}

impl SharedBuffer {
    pub fn write(&self, data: &[u8]) -> Result<()> {
        let offset = self.write_offset.fetch_add(data.len() as u32, Ordering::AcqRel);

        // 使用 Atomic 操作写入数据
        let view = js_sys::Uint8Array::new(&self.buffer);
        for (i, &byte) in data.iter().enumerate() {
            let idx = (offset as usize + i) as u32;
            js_sys::Atomics::store(&view, idx, byte as i32)?;
        }

        Ok(())
    }
}
```

### 3.3 测试策略

#### 单元测试

```rust
#[cfg(test)]
mod zero_copy_tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn test_zero_copy_receive() {
        // 创建测试数据
        let js_data = js_sys::Uint8Array::from(&[1, 2, 3, 4, 5][..]);

        // 零拷贝接收
        let rust_data = zero_copy_receive(&js_data);

        // 验证数据正确性
        assert_eq!(rust_data.as_ref(), &[1, 2, 3, 4, 5]);
    }

    #[wasm_bindgen_test]
    fn test_zero_copy_send() {
        let data = Bytes::from_static(&[1, 2, 3, 4, 5]);

        // 零拷贝发送
        let js_view = zero_copy_send(&data);

        // 验证视图正确性
        assert_eq!(js_view.length(), 5);
        assert_eq!(js_view.get_index(0), 1);
    }

    #[wasm_bindgen_test]
    fn test_memory_safety() {
        // 测试数据生命周期
        let view = {
            let data = vec![1, 2, 3, 4, 5];
            create_view(&data)  // 应该延长 data 的生命周期
        };

        // view 应该仍然有效
        assert_eq!(view.length(), 5);
    }
}
```

#### 性能基准测试

```rust
#[cfg(test)]
mod benchmarks {
    use super::*;

    #[test]
    fn bench_copy_vs_zero_copy() {
        let data_sizes = vec![1024, 10 * 1024, 100 * 1024, 1024 * 1024];

        for size in data_sizes {
            let data = vec![0u8; size];

            // 测试拷贝模式
            let start = js_sys::Date::now();
            for _ in 0..1000 {
                let _ = copy_mode_receive(&data);
            }
            let copy_time = js_sys::Date::now() - start;

            // 测试零拷贝模式
            let start = js_sys::Date::now();
            for _ in 0..1000 {
                let _ = zero_copy_receive(&data);
            }
            let zero_copy_time = js_sys::Date::now() - start;

            log::info!(
                "Size: {}KB, Copy: {:.2}ms, ZeroCopy: {:.2}ms, Speedup: {:.2}x",
                size / 1024,
                copy_time,
                zero_copy_time,
                copy_time / zero_copy_time
            );
        }
    }
}
```

---

## 4. 实施建议

### 推荐方案：渐进式优化

```
阶段 1 (1-2天，低风险)
  └─> WASM 线性内存优化
       ├─> 接收路径：Bytes::from()
       ├─> 发送路径：Uint8Array::view()
       └─> 预期提升：40-50%

阶段 2 (2-3天，低风险，可选)
  └─> Transferable Objects
       ├─> 仅用于 PostMessage
       ├─> 大数据传输优化
       └─> 预期提升：10-20%

阶段 3 (不推荐，高风险)
  └─> SharedArrayBuffer
       ├─> 仅作为 feature flag
       ├─> 需要 COOP/COEP
       └─> 预期提升：80-90%

阶段 4 (实验性，可选)
  └─> MediaStreamTrackProcessor
       ├─> 仅 Chrome/Edge
       ├─> feature flag
       └─> 预期提升：60-80% (媒体流)
```

### 决策建议

**立即实施**：
- ✅ 阶段 1：WASM 线性内存优化

**根据需求评估**：
- ⚠️ 阶段 2：Transferable Objects（如果有大数据传输场景）

**不推荐/谨慎**：
- ❌ 阶段 3：SharedArrayBuffer（破坏性变更，高风险）
- ⚠️ 阶段 4：MediaStreamTrackProcessor（浏览器兼容性差）

---

## 5. 回退策略

### Feature Flag 设计

```rust
// Cargo.toml
[features]
default = []
zero-copy-wasm = []
zero-copy-transfer = ["zero-copy-wasm"]
zero-copy-shared = ["zero-copy-transfer"]  # 高风险，默认禁用
insertable-streams = []  # 实验性

// 代码中
#[cfg(feature = "zero-copy-wasm")]
pub async fn recv(&self) -> Option<Bytes> {
    // 零拷贝实现
    zero_copy_receive(...)
}

#[cfg(not(feature = "zero-copy-wasm"))]
pub async fn recv(&self) -> Option<Bytes> {
    // 拷贝模式（回退方案）
    copy_mode_receive(...)
}
```

### 浏览器特性检测

```rust
lazy_static! {
    static ref SUPPORTS_SHARED_ARRAY_BUFFER: bool = {
        js_sys::eval("typeof SharedArrayBuffer !== 'undefined'")
            .map(|v| v.is_truthy())
            .unwrap_or(false)
    };
}

pub fn create_lane() -> DataLane {
    if *SUPPORTS_SHARED_ARRAY_BUFFER && cfg!(feature = "zero-copy-shared") {
        // 使用 SharedArrayBuffer
        create_shared_lane()
    } else {
        // 回退到 WASM 线性内存方案
        create_wasm_lane()
    }
}
```

---

## 6. 总结

### 推荐实施路径

**Phase 3 完整实施分解为 4 个子阶段**：

1. **阶段 1（立即实施）**：WASM 线性内存优化
   - 风险：低
   - 收益：高（40-50% 性能提升）
   - 时间：1-2 天
   - **强烈推荐**

2. **阶段 2（可选）**：Transferable Objects
   - 风险：低
   - 收益：中（10-20% 性能提升）
   - 时间：2-3 天
   - 推荐用于大数据传输场景

3. **阶段 3（不推荐）**：SharedArrayBuffer
   - 风险：高
   - 收益：非常高（80-90% 性能提升）
   - 时间：1-2 周
   - **仅适用于受控环境**，作为 feature flag

4. **阶段 4（实验性）**：MediaStreamTrackProcessor
   - 风险：中
   - 收益：高（60-80% 媒体流性能提升）
   - 时间：1 周
   - 浏览器兼容性差，作为可选特性

### 建议

**对于 actr-web 项目**：
- ✅ **立即实施阶段 1**（WASM 线性内存优化）
- ⚠️ **评估后决定阶段 2**（Transferable Objects）
- ❌ **暂不实施阶段 3**（SharedArrayBuffer，风险太高）
- ⚠️ **长期规划阶段 4**（MediaStreamTrackProcessor，待浏览器支持成熟）

---

**文档生成时间**：2026-01-08
**分析人员**：技术架构分析
**审核状态**：待用户确认
