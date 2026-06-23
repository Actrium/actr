//! Actor Identity Service (AIS) - ActrId registration and credential issuance.
//!
//! # Overview
//!
//! AIS is the core Actrix identity service. It allocates ActrId serial numbers,
//! issues signed AIdCredential tokens, generates time-limited TURN credentials,
//! and returns renewal tokens for `/ais/renew`.
//!
//! # 架构设计
//!
//! ```text
//! ┌──────────────┐
//! │   Client     │
//! └──────┬───────┘
//!        │ POST /ais/register (protobuf)
//!        ▼
//! ┌──────────────────────────────────────────┐
//! │  AIS Service                             │
//! │  ┌────────────┐      ┌────────────────┐ │
//! │  │  Handlers  │─────▶│  AIdIssuer     │ │
//! │  └────────────┘      └────────┬───────┘ │
//! │                               │         │
//! │  ┌──────────────────┐  ┌─────▼──────┐  │
//! │  │ SN Generator     │  │ KeyStorage │  │
//! │  │ (Snowflake)      │  │ (SQLite)   │  │
//! │  └──────────────────┘  └────────────┘  │
//! └──────────────┬───────────────────────────┘
//!                │ KS Client
//!                ▼
//!         ┌─────────────┐
//!         │ KS Service  │ (密钥生成)
//!         └─────────────┘
//! ```
//!
//! # Core Flow
//!
//! ## Registration
//!
//! 1. Receive a protobuf `RegisterRequest`.
//! 2. Allocate a 54-bit serial number with Snowflake.
//! 3. Sign identity claims through Signer and build an AIdCredential.
//! 4. Generate a time-limited TURN credential.
//! 5. Generate and persist a renewal token hash.
//! 6. Return `RegisterResponse` with ActrId, access credential, TURN credential,
//!    and renewal token.
//!
//! ## 密钥管理
//!
//! - **获取**：启动时从本地 SQLite 加载缓存密钥，如果过期则从 Signer 获取
//! - **刷新**：后台任务每 10 分钟检查，提前 10 分钟刷新即将过期的密钥
//! - **容忍**：密钥过期后 24 小时内仍可使用（避免时钟偏差导致服务中断）
//!
//! # 使用示例
//!
//! ```no_run
//! use ais::create_ais_router;
//! use platform::config::{AisConfig, ActrixConfig};
//! use tokio_util::sync::CancellationToken;
//!
//! # async fn example() -> anyhow::Result<()> {
//! // 创建配置
//! let global_config = ActrixConfig::default();
//! let ais_config = AisConfig::default();
//! let cancel = CancellationToken::new();
//!
//! // 创建 AIS 路由器
//! let router = create_ais_router(&ais_config, &global_config, cancel).await?;
//!
//! // 集成到主 HTTP 服务
//! // let app = Router::new().nest("/ais", router);
//! # Ok(())
//! # }
//! ```
//!
//! # 安全考虑
//!
//! - **Renewal token rotation**: renewal tokens are stored as hashes and rotated on use.
//! - **加密传输**：Token 使用 ECIES 加密，只有持有私钥的服务才能解密
//! - **序列号唯一性**：Snowflake 算法保证分布式环境下的全局唯一性
//! - **Key rotation**: old keys remain verifiable during the tolerance window.
//!
//! # 配置选项
//!
//! 参见 [`platform::config::AisConfig`] 获取完整配置说明。
#![deny(clippy::disallowed_macros)]

pub mod handlers;
pub mod issuer;
pub mod ratelimit;
pub mod renewal;
pub mod signer_client_wrapper;
mod sn;
mod storage;

pub use issuer::{AIdIssuer, IssuerConfig, KeyCacheInfo};

use crate::handlers::{AISState, create_router};
use crate::signer_client_wrapper::create_signer_client;
use anyhow::{Context, Result};
use axum::Router;
use platform::config::AisConfig;
use platform::monitoring::ServiceCounters;
use std::sync::Arc;

/// 创建 AIS 路由器，遵循项目的 HttpRouterService 架构
pub async fn create_ais_router(
    config: &AisConfig,
    global_config: &platform::config::ActrixConfig,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<Router> {
    create_ais_router_with_counters(config, global_config, cancel, None).await
}

/// Create AIS router with optional service counters for metrics collection.
pub async fn create_ais_router_with_counters(
    config: &AisConfig,
    global_config: &platform::config::ActrixConfig,
    cancel: tokio_util::sync::CancellationToken,
    counters: Option<Arc<ServiceCounters>>,
) -> Result<Router> {
    platform::recording::info!("Creating AIS router with config");

    // 获取 Signer 客户端配置
    let signer_client_config = config
        .get_signer_client_config(global_config)
        .context("Failed to get Signer client config. Ensure Signer is enabled or ais.dependencies.signer is configured.")?;

    // 创建 KS gRPC 客户端
    let signer_client =
        create_signer_client(&signer_client_config, &global_config.actrix_shared_key)
            .await
            .context("Failed to create Signer gRPC client")?;

    platform::recording::info!("Signer gRPC client created successfully");

    // 创建 Issuer 配置
    if config.server.renewal_token_secret.is_empty() {
        anyhow::bail!("services.ais.server.renewal_token_secret is required");
    }
    let renewal_token_secret = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &config.server.renewal_token_secret,
    )
    .context("Failed to decode services.ais.server.renewal_token_secret from base64")?;
    if renewal_token_secret.len() < 32 {
        anyhow::bail!(
            "services.ais.server.renewal_token_secret must decode to at least 32 bytes, got {} byte(s)",
            renewal_token_secret.len()
        );
    }

    let issuer_config = IssuerConfig {
        token_ttl_secs: config.server.token_ttl_secs,
        signaling_heartbeat_interval_secs: config.server.signaling_heartbeat_interval_secs,
        key_refresh_interval_secs: 3600, // 1 小时
        key_storage_file: global_config.sqlite_path.join("ais_keys.db"),
        enable_periodic_rotation: false, // 默认禁用，可通过配置文件开启
        key_rotation_interval_secs: 86400, // 24 小时
        turn_secret: global_config.turn.turn_secret.clone(),
        sqlite_path: global_config.sqlite_path.clone(),
        renewal_token_ttl_secs: config.server.renewal_token_ttl_secs,
        renewal_rotation_window_secs: config.server.renewal_rotation_window_secs,
        renewal_token_secret,
    };

    // 创建 AId Token 签发器
    let issuer = AIdIssuer::new(signer_client, issuer_config, cancel)
        .await
        .context("Failed to create AIS issuer")?;

    let state = if let Some(ctr) = counters {
        AISState::new(issuer).with_counters(ctr)
    } else {
        AISState::new(issuer)
    };

    // 创建路由器
    let router = create_router(state);

    platform::recording::info!("AIS router created successfully");
    Ok(router)
}

#[cfg(test)]
mod tests {

    // Note: 完整的集成测试需要 KS 服务运行
    // 这里仅做基本的单元测试
    // 实际测试在主程序启动后通过 HTTP 端点进行
}
