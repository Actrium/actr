//! PostMessage Lane - DOM 端（接收来自 Service Worker的消息）
//!
//! DOM 端的 PostMessage Lane，用于从 Service Worker 接收消息。

use super::lane::{DataLane, LaneResult};
use actr_web_common::PayloadType;
use actr_web_common::zero_copy::{
    extract_payload_zero_copy, parse_message_header, receive_zero_copy,
};
use futures::channel::mpsc;
use parking_lot::Mutex;
use std::sync::Arc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{MessageEvent, MessagePort};

/// PostMessage Lane 构建器（DOM 端）
pub struct PostMessageLaneBuilder {
    port: MessagePort,
    payload_type: PayloadType,
    buffer_size: usize,
}

impl PostMessageLaneBuilder {
    /// 创建新的 PostMessage Lane 构建器
    ///
    /// # 参数
    /// - `port`: MessagePort 对象（MessageChannel 的一端）
    /// - `payload_type`: 该 Lane 传输的 PayloadType
    pub fn new(port: MessagePort, payload_type: PayloadType) -> Self {
        Self {
            port,
            payload_type,
            buffer_size: 256,
        }
    }

    /// 设置接收缓冲区大小
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }

    /// 构建 PostMessage Lane
    pub fn build(self) -> LaneResult<DataLane> {
        // 创建接收通道
        let (tx, rx) = mpsc::unbounded();
        let rx = Arc::new(Mutex::new(rx));

        // 设置 onmessage 回调（零拷贝优化）
        let tx_clone = tx.clone();
        let onmessage_callback = Closure::wrap(Box::new(move |e: MessageEvent| {
            // 尝试获取 Uint8Array 数据
            if let Ok(uint8_array) = e.data().dyn_into::<js_sys::Uint8Array>() {
                // ✅ 零拷贝接收：1 次拷贝（JS → WASM 线性内存）
                let data = receive_zero_copy(&uint8_array);

                // 解析消息头部
                if let Some((payload_type_byte, length, _offset)) = parse_message_header(&data) {
                    log::trace!(
                        "PostMessage Lane (DOM) 接收消息: payload_type={}, size={} bytes",
                        payload_type_byte,
                        length
                    );

                    // ✅ 零拷贝提取 payload：转移 Vec 所有权到 Bytes
                    let payload_data = extract_payload_zero_copy(data, 5);
                    let _ = tx_clone.unbounded_send(payload_data);
                }
            } else if let Ok(array_buffer) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
                // 兼容 ArrayBuffer 格式
                let uint8_array = js_sys::Uint8Array::new(&array_buffer);

                // ✅ 零拷贝接收
                let data = receive_zero_copy(&uint8_array);

                if let Some((payload_type_byte, length, _offset)) = parse_message_header(&data) {
                    log::trace!(
                        "PostMessage Lane (DOM) 接收消息: payload_type={}, size={} bytes",
                        payload_type_byte,
                        length
                    );

                    // ✅ 零拷贝提取 payload
                    let payload_data = extract_payload_zero_copy(data, 5);
                    let _ = tx_clone.unbounded_send(payload_data);
                }
            }
        }) as Box<dyn FnMut(MessageEvent)>);

        self.port
            .set_onmessage(Some(onmessage_callback.as_ref().unchecked_ref()));
        onmessage_callback.forget();

        // 启动 MessagePort
        self.port.start();

        log::info!(
            "PostMessage Lane (DOM) 创建成功: payload_type={:?}",
            self.payload_type
        );

        Ok(DataLane::PostMessage {
            port: Arc::new(self.port),
            payload_type: self.payload_type,
            rx,
        })
    }
}
