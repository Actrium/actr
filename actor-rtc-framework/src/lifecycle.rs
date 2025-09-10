//! Actor 生命周期管理

use crate::context::Context;
use async_trait::async_trait;
use std::sync::Arc;

/// Actor 生命周期 trait
///
/// 实现此 trait 以处理 Actor 的启动和停止事件。
/// 这些方法在 Actor 系统的关键时刻被调用。
#[async_trait]
pub trait ILifecycle: Send + Sync {
    /// Actor 启动时调用
    ///
    /// 在 Actor 被附加到系统并且系统启动后调用。
    /// 可以在这里进行初始化工作，如连接到外部服务、注册回调等。
    ///
    /// # 参数
    /// - `ctx`: Actor 上下文，用于与系统交互
    async fn on_start(&self, ctx: Arc<Context>) {
        ctx.log_info("Actor started with default lifecycle implementation");
    }

    /// Actor 停止前调用
    ///
    /// 在 Actor 系统关闭前调用，用于清理资源。
    /// 这是执行清理工作的最后机会，如关闭连接、保存状态等。
    ///
    /// # 参数  
    /// - `ctx`: Actor 上下文，用于与系统交互
    async fn on_stop(&self, ctx: Arc<Context>) {
        ctx.log_info("Actor stopped with default lifecycle implementation");
    }

    /// Actor 与新的对等节点建立连接时调用
    ///
    /// 当成功建立到另一个 Actor 的 WebRTC 连接时调用。
    /// 可以在这里进行特定于连接的初始化。
    ///
    /// # 参数
    /// - `ctx`: Actor 上下文
    /// - `peer_id`: 对等 Actor 的 ID
    async fn on_peer_connected(&self, ctx: Arc<Context>, peer_id: &str) {
        ctx.log_info(&format!("Connected to peer: {}", peer_id));
    }

    /// Actor 与对等节点断开连接时调用
    ///
    /// 当与另一个 Actor 的 WebRTC 连接断开时调用。
    /// 可以在这里进行连接相关的清理工作。
    ///
    /// # 参数
    /// - `ctx`: Actor 上下文  
    /// - `peer_id`: 对等 Actor 的 ID
    async fn on_peer_disconnected(&self, ctx: Arc<Context>, peer_id: &str) {
        ctx.log_info(&format!("Disconnected from peer: {}", peer_id));
    }

    /// 发现新的 Actor 时调用
    ///
    /// 当通过信令服务器发现新的可连接 Actor 时调用。
    /// 可以在这里决定是否要主动连接到新发现的 Actor。
    ///
    /// # 参数
    /// - `ctx`: Actor 上下文
    /// - `actor_id`: 新发现的 Actor ID
    ///
    /// # 返回值
    /// - `true`: 主动连接到此 Actor
    /// - `false`: 不主动连接，等待对方连接
    async fn on_actor_discovered(
        &self,
        ctx: Arc<Context>,
        actor_id: &shared_protocols::actor::ActorId,
    ) -> bool {
        ctx.log_info(&format!("Discovered new actor: {}", actor_id.serial_number));
        // 默认策略：ID 较小的主动连接ID较大的
        actor_id.serial_number > ctx.actor_id.serial_number
    }

    /// 处理未知消息类型时调用
    ///
    /// 当接收到无法路由到具体处理器的消息时调用。
    /// 可以在这里实现通用的消息处理逻辑。
    ///
    /// # 参数
    /// - `ctx`: Actor 上下文
    /// - `message_type`: 消息类型标识
    /// - `payload`: 消息载荷
    async fn on_unknown_message(&self, ctx: Arc<Context>, message_type: &str, payload: &[u8]) {
        ctx.log_warn(&format!(
            "Received unknown message type: {}, payload size: {} bytes",
            message_type,
            payload.len()
        ));
    }
}

// 注意：不提供泛型默认实现，以避免与具体实现冲突
// 如果需要默认实现，可以为具体类型单独实现 ILifecycle trait
