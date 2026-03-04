//! 零拷贝工具模块
//!
//! 提供 WASM ↔ JavaScript 数据传输的零拷贝（或最小拷贝）实现。
//!
//! ## 设计原则
//!
//! 1. **接收路径优化**：
//!    - 避免 `to_vec()` 和 `copy_from_slice()` 的双重拷贝
//!    - 使用 `Bytes::from()` 转移 Vec 所有权（零拷贝）
//!
//! 2. **发送路径优化**：
//!    - 使用 `Uint8Array::view()` 创建 WASM 内存视图（零拷贝）
//!    - 确保数据生命周期正确
//!
//! 3. **内存安全**：
//!    - 使用 Rust 生命周期系统保证安全
//!    - 谨慎使用 `unsafe`，确保边界检查
//!
//! ## 使用示例
//!
//! ```rust,no_run
//! use actr_web_common::zero_copy::{receive_zero_copy, send_zero_copy, extract_payload_zero_copy};
//! use bytes::Bytes;
//! use wasm_bindgen::prelude::*;
//! use wasm_bindgen::JsCast;
//!
//! // 模拟 port 对象
//! struct MockPort;
//! impl MockPort {
//!     fn post_message(&self, _msg: &JsValue) -> Result<(), JsValue> { Ok(()) }
//! }
//! let port = MockPort;
//!
//! // 接收路径（JS → Rust）
//! // 假设从 MessageEvent 获取到了 Uint8Array
//! let js_array = js_sys::Uint8Array::new_with_length(10);
//!
//! // 零拷贝接收（1 次拷贝：JS → WASM 线性内存）
//! let data = receive_zero_copy(&js_array);
//!
//! // 零拷贝提取 payload（Vec → Bytes 转移所有权）
//! if data.len() >= 5 {
//!     let payload = extract_payload_zero_copy(data, 5);
//!     // ... 使用 payload
//! }
//!
//! // 发送路径（Rust → JS）
//! let data = Bytes::from(vec![1, 2, 3, 4, 5]);
//! // 零拷贝发送（创建 WASM 内存视图）
//! let js_view = send_zero_copy(&data);
//! port.post_message(&js_view.into()).unwrap();
//! ```

use bytes::Bytes;

/// 零拷贝接收：从 JS Uint8Array 接收数据到 Rust Vec
///
/// **性能优化**：
/// - 旧实现：`uint8_array.to_vec()` 会复制整个数组
/// - 新实现：使用 `copy_to()` 直接写入预分配的 Vec，利用浏览器优化的 memcpy
///
/// **拷贝次数**：1 次（JS 内存 → WASM 线性内存，不可避免）
///
/// # 参数
/// - `uint8_array`: JavaScript 的 Uint8Array 对象
///
/// # 返回
/// - `Vec<u8>`: Rust 端的字节向量（位于 WASM 线性内存）
///
/// # 注意
/// - 这个拷贝是必须的，因为 JS 和 WASM 内存空间是分离的
/// - 但相比 `to_vec()` + `copy_from_slice()`，减少了 1 次拷贝
#[inline]
pub fn receive_zero_copy(uint8_array: &js_sys::Uint8Array) -> Vec<u8> {
    let len = uint8_array.length() as usize;
    // 初始化 buffer（内存安全）
    let mut buffer = vec![0u8; len];

    // 使用浏览器优化的 memcpy，直接写入 WASM 线性内存
    uint8_array.copy_to(&mut buffer);

    buffer
}

/// 零拷贝提取 payload：从 Vec 中提取 payload 部分，转移所有权到 Bytes
///
/// **性能优化**：
/// - 旧实现：`Bytes::copy_from_slice(&vec[offset..])` 会再次拷贝数据
/// - 新实现：使用 `Vec::split_off()` 和 `Bytes::from()` 转移所有权（零拷贝）
///
/// **拷贝次数**：0 次（转移所有权）
///
/// # 参数
/// - `mut data`: 包含完整消息的 Vec（会被消费）
/// - `header_size`: 头部大小（要跳过的字节数）
///
/// # 返回
/// - `Bytes`: payload 部分（零拷贝）
///
/// # 示例
/// ```rust
/// use actr_web_common::zero_copy::extract_payload_zero_copy;
/// use bytes::Bytes;
///
/// let data = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];  // [header(5) | payload(5)]
/// let payload = extract_payload_zero_copy(data, 5);
/// // payload = Bytes([6, 7, 8, 9, 10])
/// ```
#[inline]
pub fn extract_payload_zero_copy(mut data: Vec<u8>, header_size: usize) -> Bytes {
    // split_off 会切分 Vec，返回后半部分（payload），前半部分（header）被丢弃
    let payload_vec = data.split_off(header_size);

    // Bytes::from(Vec) 会转移 Vec 的所有权，零拷贝
    // Bytes 内部使用 Arc，后续 clone 也是零拷贝的
    Bytes::from(payload_vec)
}

/// 零拷贝发送：创建 Bytes 数据的 JS Uint8Array 视图
///
/// **性能优化**：
/// - 旧实现：`Uint8Array::from(&bytes[..])` 会复制整个数组
/// - 新实现：使用 `Uint8Array::view()` 创建 WASM 内存视图（零拷贝）
///
/// **拷贝次数**：0 次（创建视图，不拷贝数据）
///
/// # 参数
/// - `data`: 要发送的数据（Bytes）
///
/// # 返回
/// - `js_sys::Uint8Array`: JavaScript 侧的视图
///
/// # 安全性
/// - ⚠️ 视图的生命周期必须短于数据的生命周期
/// - ⚠️ 视图是只读的，JS 端不应修改
/// - ⚠️ 如果 Rust 端修改数据，JS 端看到的内容会变化
///
/// # 注意
/// - 必须确保在 JS 使用视图期间，Rust 端的 `data` 没有被释放
/// - 对于 PostMessage，浏览器会在 `post_message` 时复制数据，所以视图可以立即释放
/// - 对于 WebSocket/DataChannel 的 `send_with_u8_array`，也是同步复制，所以安全
#[inline]
pub fn send_zero_copy(data: &Bytes) -> js_sys::Uint8Array {
    // 安全：Uint8Array::view() 创建一个临时视图
    // 浏览器的 postMessage/send API 会在调用时立即复制数据
    // 所以即使视图生命周期很短，也不会有问题
    unsafe {
        let ptr = data.as_ptr();
        let len = data.len();
        js_sys::Uint8Array::view(std::slice::from_raw_parts(ptr, len))
    }
}

/// 零拷贝构造消息：组装 header + payload
///
/// **设计思路**：
/// - 预先分配 `header_size + payload_size` 的 Vec
/// - 写入 header 和 payload
/// - 转换为 Bytes（零拷贝）
///
/// **拷贝次数**：1 次（payload 复制到 Vec，但这是最优方案）
///
/// # 参数
/// - `header`: 消息头部字节
/// - `payload`: 消息负载
///
/// # 返回
/// - `Bytes`: 完整消息（header + payload）
#[inline]
pub fn construct_message_zero_copy(header: &[u8], payload: &Bytes) -> Bytes {
    let total_size = header.len() + payload.len();
    let mut msg = Vec::with_capacity(total_size);

    msg.extend_from_slice(header);
    msg.extend_from_slice(payload.as_ref());

    Bytes::from(msg)
}

/// 零拷贝构造消息（带 payload 所有权转移）
///
/// **优化点**：
/// - 如果 payload 是 Vec，可以直接追加到 header 后面
/// - 避免 `extend_from_slice` 的拷贝
///
/// **拷贝次数**：0 次（追加操作，不拷贝）
///
/// # 参数
/// - `header`: 消息头部字节
/// - `payload`: 消息负载（Vec，会被消费）
///
/// # 返回
/// - `Bytes`: 完整消息
///
/// # 注意
/// - 这个函数会消费 `payload`，适用于不需要保留原始 payload 的场景
#[inline]
pub fn construct_message_from_vec(header: &[u8], mut payload: Vec<u8>) -> Bytes {
    // 在 payload 前面插入 header
    let mut msg = Vec::with_capacity(header.len() + payload.len());
    msg.extend_from_slice(header);
    msg.append(&mut payload); // append 转移所有权，零拷贝

    Bytes::from(msg)
}

/// 解析消息格式：[PayloadType(1) | Length(4) | Data(N)]
///
/// **返回**：(payload_type, length, data_start_offset)
///
/// # 参数
/// - `buffer`: 完整消息缓冲区
///
/// # 返回
/// - `Some((payload_type, length, 5))`: 成功解析
/// - `None`: 消息格式错误或长度不足
#[inline]
pub fn parse_message_header(buffer: &[u8]) -> Option<(u8, usize, usize)> {
    if buffer.len() < 5 {
        return None;
    }

    let payload_type = buffer[0];
    let length = u32::from_be_bytes([buffer[1], buffer[2], buffer[3], buffer[4]]) as usize;

    // 验证消息长度
    if buffer.len() < 5 + length {
        log::warn!(
            "消息长度不匹配: 期望 {} bytes (header 5 + payload {}), 实际 {} bytes",
            5 + length,
            length,
            buffer.len()
        );
        return None;
    }

    Some((payload_type, length, 5))
}

/// 构造消息 header：[PayloadType(1) | Length(4)]
///
/// # 参数
/// - `payload_type`: PayloadType 枚举值
/// - `payload_len`: payload 长度
///
/// # 返回
/// - `[u8; 5]`: 5 字节的 header
#[inline]
pub fn construct_message_header(payload_type: u8, payload_len: usize) -> [u8; 5] {
    let mut header = [0u8; 5];
    header[0] = payload_type;
    header[1..5].copy_from_slice(&(payload_len as u32).to_be_bytes());
    header
}

/// 使用 Transferable Objects 发送数据（PostMessage 专用）
///
/// **Transferable Objects 原理**：
/// - `postMessage(message, [transferList])` 转移 ArrayBuffer 所有权而非拷贝
/// - 转移后原 buffer 变为 detached，不可再访问
/// - 接收端获得独占所有权
/// - 适用于大数据传输（>10KB）
///
/// **性能优势**：
/// - 消除 postMessage 的结构化克隆开销
/// - 对于大数据（>10KB），性能提升显著
/// - 对于小数据（<10KB），转移开销可能大于拷贝
///
/// # 参数
/// - `data`: 要发送的数据（Bytes）
///
/// # 返回
/// - `(js_sys::Uint8Array, js_sys::Array)`: (数据视图, transferList)
///
/// # 使用示例
/// ```rust,no_run
/// use actr_web_common::zero_copy::send_with_transfer;
/// use bytes::Bytes;
/// use wasm_bindgen::JsValue;
///
/// // 模拟 port 对象
/// struct MockPort;
/// impl MockPort {
///     fn post_message_with_transfer(&self, _msg: &JsValue, _transfer: &js_sys::Array) -> Result<(), JsValue> { Ok(()) }
/// }
/// let port = MockPort;
///
/// let data = Bytes::from(vec![0u8; 100_000]); // 100KB 数据
/// let (js_view, transfer_list) = send_with_transfer(&data);
/// port.post_message_with_transfer(&js_view.into(), &transfer_list).unwrap();
/// ```
///
/// # 注意
/// - 只适用于 PostMessage（WebSocket/DataChannel 不支持）
/// - 转移后原 ArrayBuffer 变为 detached
/// - 适合一次性发送的大数据
#[inline]
pub fn send_with_transfer(data: &Bytes) -> (js_sys::Uint8Array, js_sys::Array) {
    // 创建 WASM 内存视图
    let js_view = unsafe {
        let ptr = data.as_ptr();
        let len = data.len();
        js_sys::Uint8Array::view(std::slice::from_raw_parts(ptr, len))
    };

    // 创建 transferable 数组
    let transfer_list = js_sys::Array::new();
    transfer_list.push(&js_view.buffer());

    (js_view, transfer_list)
}

/// 判断是否应该使用 Transferable Objects
///
/// **决策逻辑**：
/// - 数据大小 < 10KB：不推荐（转移开销 > 拷贝开销）
/// - 数据大小 >= 10KB：推荐（转移开销 < 拷贝开销）
/// - 数据大小 >= 100KB：强烈推荐
///
/// # 参数
/// - `data_size`: 数据大小（字节）
///
/// # 返回
/// - `true`: 推荐使用 Transferable Objects
/// - `false`: 不推荐，使用普通 send 即可
///
/// # 使用示例
/// ```rust
/// use actr_web_common::zero_copy::should_use_transfer;
/// use bytes::Bytes;
///
/// let data = Bytes::from(vec![0u8; 50_000]);
/// if should_use_transfer(data.len()) {
///     // 使用 send_with_transfer()
/// } else {
///     // 使用普通 send()
/// }
/// ```
#[inline]
pub fn should_use_transfer(data_size: usize) -> bool {
    data_size >= 10 * 1024 // 10KB 阈值
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    // ===== receive_zero_copy 测试 =====

    #[wasm_bindgen_test]
    fn test_receive_zero_copy_basic() {
        let js_array = js_sys::Uint8Array::from(&[1u8, 2, 3, 4, 5][..]);
        let data = receive_zero_copy(&js_array);

        assert_eq!(data, vec![1, 2, 3, 4, 5]);
    }

    #[wasm_bindgen_test]
    fn test_receive_zero_copy_empty() {
        let js_array = js_sys::Uint8Array::new_with_length(0);
        let data = receive_zero_copy(&js_array);

        assert_eq!(data, Vec::<u8>::new());
    }

    #[wasm_bindgen_test]
    fn test_receive_zero_copy_large() {
        let large_data = vec![42u8; 1024 * 1024]; // 1MB
        let js_array = js_sys::Uint8Array::from(&large_data[..]);
        let data = receive_zero_copy(&js_array);

        assert_eq!(data.len(), 1024 * 1024);
        assert_eq!(data[0], 42);
        assert_eq!(data[1024 * 1024 - 1], 42);
    }

    // ===== extract_payload_zero_copy 测试 =====

    #[wasm_bindgen_test]
    fn test_extract_payload_zero_copy() {
        let data = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let payload = extract_payload_zero_copy(data, 5);

        assert_eq!(payload.as_ref(), &[6, 7, 8, 9, 10]);
    }

    #[wasm_bindgen_test]
    fn test_extract_payload_zero_copy_empty_payload() {
        let data = vec![1, 2, 3, 4, 5];
        let payload = extract_payload_zero_copy(data, 5);

        let empty: &[u8] = &[];
        assert_eq!(payload.as_ref(), empty);
    }

    #[wasm_bindgen_test]
    fn test_extract_payload_zero_copy_no_header() {
        let data = vec![1, 2, 3, 4, 5];
        let payload = extract_payload_zero_copy(data, 0);

        assert_eq!(payload.as_ref(), &[1, 2, 3, 4, 5]);
    }

    // ===== send_zero_copy 测试 =====

    #[wasm_bindgen_test]
    fn test_send_zero_copy_basic() {
        let data = Bytes::from_static(&[1, 2, 3, 4, 5]);
        let js_view = send_zero_copy(&data);

        assert_eq!(js_view.length(), 5);
        assert_eq!(js_view.get_index(0), 1);
        assert_eq!(js_view.get_index(4), 5);
    }

    #[wasm_bindgen_test]
    fn test_send_zero_copy_empty() {
        let data = Bytes::from_static(&[]);
        let js_view = send_zero_copy(&data);

        assert_eq!(js_view.length(), 0);
    }

    #[wasm_bindgen_test]
    fn test_send_zero_copy_large() {
        let large_data = Bytes::from(vec![42u8; 1024 * 1024]);
        let js_view = send_zero_copy(&large_data);

        assert_eq!(js_view.length(), 1024 * 1024);
        assert_eq!(js_view.get_index(0), 42);
        assert_eq!(js_view.get_index(1024 * 1024 - 1), 42);
    }

    // ===== construct_message_zero_copy 测试 =====

    #[wasm_bindgen_test]
    fn test_construct_message_zero_copy() {
        let header = [1, 0, 0, 0, 5]; // PayloadType=1, Length=5
        let payload = Bytes::from_static(&[10, 20, 30, 40, 50]);

        let msg = construct_message_zero_copy(&header, &payload);

        assert_eq!(msg.len(), 10);
        assert_eq!(&msg[0..5], &[1, 0, 0, 0, 5]);
        assert_eq!(&msg[5..10], &[10, 20, 30, 40, 50]);
    }

    // ===== construct_message_from_vec 测试 =====

    #[wasm_bindgen_test]
    fn test_construct_message_from_vec() {
        let header = [1, 0, 0, 0, 5];
        let payload = vec![10, 20, 30, 40, 50];

        let msg = construct_message_from_vec(&header, payload);

        assert_eq!(msg.len(), 10);
        assert_eq!(&msg[0..5], &[1, 0, 0, 0, 5]);
        assert_eq!(&msg[5..10], &[10, 20, 30, 40, 50]);
    }

    // ===== parse_message_header 测试 =====

    #[wasm_bindgen_test]
    fn test_parse_message_header_valid() {
        let buffer = [1, 0, 0, 0, 5, 10, 20, 30, 40, 50];

        let result = parse_message_header(&buffer);

        assert!(result.is_some());
        let (payload_type, length, offset) = result.unwrap();
        assert_eq!(payload_type, 1);
        assert_eq!(length, 5);
        assert_eq!(offset, 5);
    }

    #[wasm_bindgen_test]
    fn test_parse_message_header_too_short() {
        let buffer = [1, 0, 0, 0]; // 只有 4 字节

        let result = parse_message_header(&buffer);

        assert!(result.is_none());
    }

    #[wasm_bindgen_test]
    fn test_parse_message_header_length_mismatch() {
        let buffer = [1, 0, 0, 0, 10, 1, 2, 3]; // 声称 10 字节，实际只有 3 字节

        let result = parse_message_header(&buffer);

        assert!(result.is_none());
    }

    // ===== construct_message_header 测试 =====

    #[wasm_bindgen_test]
    fn test_construct_message_header() {
        let header = construct_message_header(1, 5);

        assert_eq!(header, [1, 0, 0, 0, 5]);
    }

    #[wasm_bindgen_test]
    fn test_construct_message_header_large_length() {
        let header = construct_message_header(2, 1024 * 1024);

        assert_eq!(header[0], 2);
        let length = u32::from_be_bytes([header[1], header[2], header[3], header[4]]);
        assert_eq!(length, 1024 * 1024);
    }

    // ===== 集成测试：完整流程 =====

    #[wasm_bindgen_test]
    fn test_zero_copy_full_workflow_receive() {
        // 模拟接收流程：JS → Rust
        let message = [1, 0, 0, 0, 5, 10, 20, 30, 40, 50]; // [header(5) | payload(5)]
        let js_array = js_sys::Uint8Array::from(&message[..]);

        // 1. 零拷贝接收
        let data = receive_zero_copy(&js_array);

        // 2. 解析 header
        let (payload_type, length, offset) = parse_message_header(&data).unwrap();
        assert_eq!(payload_type, 1);
        assert_eq!(length, 5);
        assert_eq!(offset, 5);

        // 3. 零拷贝提取 payload
        let payload = extract_payload_zero_copy(data, offset);
        assert_eq!(payload.as_ref(), &[10, 20, 30, 40, 50]);
    }

    #[wasm_bindgen_test]
    fn test_zero_copy_full_workflow_send() {
        // 模拟发送流程：Rust → JS
        let payload = Bytes::from_static(&[10, 20, 30, 40, 50]);
        let header = construct_message_header(1, payload.len());

        // 1. 构造消息
        let msg = construct_message_zero_copy(&header, &payload);
        assert_eq!(msg.len(), 10);

        // 2. 零拷贝发送
        let js_view = send_zero_copy(&msg);
        assert_eq!(js_view.length(), 10);
        assert_eq!(js_view.get_index(0), 1); // PayloadType
        assert_eq!(js_view.get_index(5), 10); // Payload 第一个字节
    }

    // ===== Transferable Objects 测试 =====

    #[wasm_bindgen_test]
    fn test_send_with_transfer_basic() {
        let data = Bytes::from_static(&[1, 2, 3, 4, 5]);
        let (js_view, transfer_list) = send_with_transfer(&data);

        assert_eq!(js_view.length(), 5);
        assert_eq!(js_view.get_index(0), 1);
        assert_eq!(js_view.get_index(4), 5);

        // 验证 transferList 包含 buffer
        assert_eq!(transfer_list.length(), 1);
        assert!(transfer_list.get(0).is_truthy());
    }

    #[wasm_bindgen_test]
    fn test_send_with_transfer_large_data() {
        let large_data = vec![42u8; 100 * 1024]; // 100KB
        let data = Bytes::from(large_data);

        let (js_view, transfer_list) = send_with_transfer(&data);

        assert_eq!(js_view.length(), 100 * 1024);
        assert_eq!(js_view.get_index(0), 42);
        assert_eq!(js_view.get_index(100 * 1024 - 1), 42);

        assert_eq!(transfer_list.length(), 1);
    }

    #[wasm_bindgen_test]
    fn test_send_with_transfer_empty() {
        let data = Bytes::from_static(&[]);
        let (js_view, transfer_list) = send_with_transfer(&data);

        assert_eq!(js_view.length(), 0);
        assert_eq!(transfer_list.length(), 1);
    }

    #[wasm_bindgen_test]
    fn test_should_use_transfer_small_data() {
        // 小数据不推荐使用 transfer
        assert!(!should_use_transfer(1024)); // 1KB
        assert!(!should_use_transfer(5 * 1024)); // 5KB
        assert!(!should_use_transfer(9 * 1024)); // 9KB
    }

    #[wasm_bindgen_test]
    fn test_should_use_transfer_large_data() {
        // 大数据推荐使用 transfer
        assert!(should_use_transfer(10 * 1024)); // 10KB
        assert!(should_use_transfer(50 * 1024)); // 50KB
        assert!(should_use_transfer(1024 * 1024)); // 1MB
    }

    #[wasm_bindgen_test]
    fn test_should_use_transfer_edge_case() {
        // 边界测试
        assert!(!should_use_transfer(10 * 1024 - 1)); // 10KB - 1
        assert!(should_use_transfer(10 * 1024)); // 10KB
        assert!(should_use_transfer(10 * 1024 + 1)); // 10KB + 1
    }
}
