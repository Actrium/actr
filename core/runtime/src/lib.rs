//! # actr-runtime — 业务分发层
//!
//! 精简后的 `actr-runtime` 仅包含纯业务分发逻辑，
//! **不依赖** tokio、WebRTC、wasmtime 等平台相关库，
//! 可在 native 和 `wasm32-unknown-unknown` 目标上编译。
//!
//! ## 职责划分
//!
//! ```text
//! actr-hyper   ← 基础设施层（transport, wire, signaling, WASM engine …）
//! actr-runtime ← 业务分发层（ACL + dispatch + lifecycle hooks）  ← 你在这里
//! actr-framework ← SDK 接口层（trait 定义：Workload, Context, MessageDispatcher）
//! actr-protocol  ← 数据定义层（protobuf 类型）
//! ```
//!
//! ## 核心类型
//!
//! - [`ActrDispatch`] — 持有 `Arc<Workload>` + 可选 ACL，提供 `dispatch()` 入口
//! - [`check_acl_permission`] — 纯函数 ACL 权限判定
//!
//! ## 使用示例
//!
//! ```rust,ignore
//! use actr_runtime::ActrDispatch;
//!
//! let dispatch = ActrDispatch::new(Arc::new(workload), acl);
//!
//! // 生命周期
//! dispatch.on_start(&ctx).await?;
//!
//! // 消息分发
//! let response = dispatch.dispatch(&self_id, caller_id.as_ref(), envelope, &ctx).await?;
//!
//! // 关闭
//! dispatch.on_stop(&ctx).await?;
//! ```

pub mod acl;
pub mod dispatch;

// ── 核心导出 ──
pub use acl::check_acl_permission;
pub use dispatch::ActrDispatch;

// ── 转导出 actr-framework 的核心 trait，方便下游一站式引入 ──
pub use actr_framework::{Context, MessageDispatcher, Workload};

// ── 转导出 actr-protocol 常用类型 ──
pub use actr_protocol::{Acl, ActrError, ActrId, ActrType, ActorResult, RpcEnvelope};
