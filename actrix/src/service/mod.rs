//! 服务管理模块
//!
//! 管理各种辅助服务的生命周期
//! # Service Management Abstraction
//!
//! 提供通用的服务管理抽象，用于细粒度地管理不同类型的服务（STUN、TURN、Signaling、Admin等）
//!
//! ## 核心概念
//!
//! - `HttpRouterService`: HTTP路由服务的核心 trait，提供 axum 路由器
//! - `IceService`: ICE服务的核心 trait，独立的 UDP 服务器
//! - `ServiceInfo`: 服务的基本信息
//! - `ServiceManager`: 服务管理器，负责管理多个服务的生命周期

pub mod container;
pub mod grpc;
pub mod http;
pub mod ice;
pub mod manager;
pub mod trace;

use actrix_common::{ServiceInfo, ServiceState};
use anyhow::Result;
use async_trait::async_trait;
use axum::Router;
use std::fmt::Debug;
use tracing::info;
use url::Url;

// 重新导出服务实现
pub use grpc::{KsGrpcService, SupervisordGrpcService};
pub use http::{AisService, KsHttpService, SignalingService};
pub use ice::{StunService, TurnService};

// 重新导出核心组件
pub use container::ServiceContainer;
pub use manager::ServiceManager;

/// HTTP路由服务的核心 trait - 为 axum 提供路由器
#[async_trait]
pub trait HttpRouterService: Send + Sync + Debug {
    /// 获取服务信息
    fn info(&self) -> &ServiceInfo;

    /// 获取可变的服务信息
    fn info_mut(&mut self) -> &mut ServiceInfo;

    /// 构建axum路由器
    async fn build_router(&mut self) -> Result<Router>;

    /// 服务启动回调（路由器已构建并启动后调用）
    async fn on_start(&mut self, base_url: Url) -> Result<()> {
        self.info_mut().set_running(base_url);
        Ok(())
    }

    /// 服务停止回调
    async fn on_stop(&mut self) -> Result<()> {
        info!("HTTP router service '{}' stopped", self.info().name);
        self.info_mut().status = ServiceState::Unknown;
        Ok(())
    }

    /// 获取路由前缀（如 "/admin", "/authority" 等）
    fn route_prefix(&self) -> &str;
}

/// ICE服务的核心 trait - 独立的 UDP 服务器
#[async_trait]
pub trait IceService: Send + Sync + Debug {
    /// 获取服务信息
    fn info(&self) -> &ServiceInfo;

    /// 获取可变的服务信息
    fn info_mut(&mut self) -> &mut ServiceInfo;

    /// 启动ICE服务
    async fn start(
        &mut self,
        shutdown_rx: tokio::sync::broadcast::Receiver<()>,
        oneshot_tx: tokio::sync::oneshot::Sender<ServiceInfo>,
    ) -> Result<()>;

    /// 停止ICE服务
    async fn stop(&mut self) -> Result<()> {
        info!("ICE service '{}' stopped", self.info().name);
        self.info_mut().status = ServiceState::Unknown;
        Ok(())
    }

    /// 获取服务健康状态
    #[allow(dead_code)]
    async fn health_check(&self) -> Result<bool> {
        Ok(self.info().is_running())
    }
}
