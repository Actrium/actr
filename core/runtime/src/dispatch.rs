//! 业务消息分发层
//!
//! `ActrDispatch` 是新 actr-runtime 的核心结构，负责：
//! 1. ACL 权限检查
//! 2. 路由键 → Workload handler 的静态分发
//! 3. Handler panic 捕获与报告
//! 4. 生命周期钩子委托（on_start / on_stop）
//!
//! 本模块**不包含**任何 IO、网络、transport 逻辑，
//! 可在 native 和 wasm32 目标上编译运行。

use std::sync::Arc;

use actr_framework::{Context, MessageDispatcher, Workload};
use actr_protocol::{Acl, ActrError, ActrId, ActrIdExt as _, ActorResult, RpcEnvelope};
use bytes::Bytes;
use futures_util::FutureExt as _;

use crate::acl::check_acl_permission;

/// 纯业务分发器
///
/// 持有 `Arc<W>` workload 实例与可选 ACL 规则，
/// 对外暴露 `dispatch()` 与生命周期方法。
pub struct ActrDispatch<W: Workload> {
    workload: Arc<W>,
    acl: Option<Acl>,
}

impl<W: Workload> ActrDispatch<W> {
    /// 创建分发器
    ///
    /// # 参数
    /// - `workload`: 业务 Workload 实例（`Arc` 包装）
    /// - `acl`: 可选 ACL 规则集；为 `None` 时默认放行所有调用
    pub fn new(workload: Arc<W>, acl: Option<Acl>) -> Self {
        Self { workload, acl }
    }

    /// 获取 Workload 引用
    pub fn workload(&self) -> &W {
        &self.workload
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 生命周期
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    /// 转发 on_start 生命周期钩子
    pub async fn on_start<C: Context>(&self, ctx: &C) -> ActorResult<()> {
        self.workload.on_start(ctx).await
    }

    /// 转发 on_stop 生命周期钩子
    pub async fn on_stop<C: Context>(&self, ctx: &C) -> ActorResult<()> {
        self.workload.on_stop(ctx).await
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 消息分发
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    /// 分发入站消息：ACL 检查 → 路由 → handler 执行
    ///
    /// # 参数
    /// - `self_id`: 当前 Actor 的 ID
    /// - `caller_id`: 调用方 ID（本地调用时为 `None`）
    /// - `envelope`: RPC 信封（包含 route_key 和 payload）
    /// - `ctx`: 执行上下文（泛型，由上层传入）
    ///
    /// # 返回值
    /// 序列化后的响应字节，或 `ActrError`
    pub async fn dispatch<C: Context>(
        &self,
        self_id: &ActrId,
        caller_id: Option<&ActrId>,
        envelope: RpcEnvelope,
        ctx: &C,
    ) -> ActorResult<Bytes> {
        // ── ACL 检查 ──
        let allowed = check_acl_permission(caller_id, self_id, self.acl.as_ref())
            .map_err(|e| ActrError::Internal(format!("ACL check failed: {e}")))?;

        if !allowed {
            tracing::warn!(
                severity = 5,
                error_category = "acl_denied",
                request_id = %envelope.request_id,
                route_key = %envelope.route_key,
                "ACL: permission denied",
            );
            return Err(ActrError::PermissionDenied(format!(
                "ACL denied: {} -> {}",
                caller_id
                    .map(|c| c.to_string_repr())
                    .unwrap_or_else(|| "<unknown>".into()),
                self_id.to_string_repr(),
            )));
        }

        // ── 静态分发 + panic 捕获 ──
        self.do_dispatch(envelope, ctx).await
    }

    /// 内部分发：调用 `MessageDispatcher::dispatch`，捕获 handler panic
    async fn do_dispatch<C: Context>(
        &self,
        envelope: RpcEnvelope,
        ctx: &C,
    ) -> ActorResult<Bytes> {
        let route_key = envelope.route_key.clone();
        let request_id = envelope.request_id.clone();

        let result = std::panic::AssertUnwindSafe(W::Dispatcher::dispatch(
            &self.workload,
            envelope,
            ctx,
        ))
        .catch_unwind()
        .await;

        match result {
            Ok(r) => r,
            Err(panic_payload) => {
                let info = extract_panic_info(panic_payload);
                tracing::error!(
                    severity = 8,
                    error_category = "handler_panic",
                    route_key = %route_key,
                    request_id = %request_id,
                    "handler panicked: {}", info,
                );
                // 通知 workload 的 on_error 钩子
                let _ = self
                    .workload
                    .on_error(ctx, format!("handler panicked: {info}"))
                    .await;
                Err(ActrError::DecodeFailure(format!(
                    "handler panicked: {info}"
                )))
            }
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 工具函数
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 从 panic payload 提取可读字符串
fn extract_panic_info(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic>".to_string()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// trait impls
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

impl<W: Workload> Clone for ActrDispatch<W> {
    fn clone(&self) -> Self {
        Self {
            workload: Arc::clone(&self.workload),
            acl: self.acl.clone(),
        }
    }
}

impl<W: Workload> std::fmt::Debug for ActrDispatch<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActrDispatch")
            .field("has_acl", &self.acl.is_some())
            .finish()
    }
}
