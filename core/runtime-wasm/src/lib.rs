//! # actr-runtime-wasm
//!
//! Actor-RTC WASM guest-side runtime，运行在 `wasm32-unknown-unknown` 目标上。
//!
//! 本 crate 是 `actr-runtime`（native 宿主侧）的 WASM 客体侧对应物：
//! - **`actr-runtime`**：native 宿主，驱动 WASM 模块执行（Wasmtime / WAMR）
//! - **`actr-runtime-wasm`**：WASM 客体侧，被编译进 WASM 二进制，提供 `Context` 实现
//!
//! ## 架构位置
//!
//! ```text
//! actor 业务代码（actr-framework 接口）
//!         ↓ 编译为 wasm32
//! actr-runtime-wasm（本 crate，编译进 WASM 二进制）
//!         ↓ host imports
//! actr-runtime（native，宿主侧 WasmHost/WasmInstance）
//! ```
//!
//! ## 使用方式
//!
//! ```rust,ignore
//! use actr_runtime_wasm::entry;
//!
//! // 1. 实现 Handler（通过 actr-framework 接口）
//! struct MyService;
//! // impl EchoServiceHandler for MyService { ... }
//!
//! // 2. 注册 Workload，生成 WASM ABI 导出
//! entry!(EchoServiceWorkload<MyService>);
//! // 或自定义初始化：
//! // entry!(EchoServiceWorkload<MyService>, EchoServiceWorkload(MyService::new()));
//! ```
//!
//! ## asyncify 透明挂起
//!
//! `WasmContext::call(...)` 等通信方法内部调用同步 host import。
//! WASM 二进制在编译后经 `wasm-opt --asyncify` 转换，使得 host import 调用点
//! 可被宿主透明地挂起/恢复，无需修改业务代码。

pub mod abi;
pub mod context;
pub mod executor;
pub mod imports;

// 便捷重导出
pub use context::WasmContext;
