//! Signaling 服务速率限制
//!
//! 实现双重速率限制：
//! 1. **连接速率限制**：限制每个 IP 建立新 WebSocket 连接的速率
//! 2. **消息速率限制**：限制每个连接发送消息的速率
//!
//! 使用 governor crate 实现，支持配置化

use governor::{DefaultDirectRateLimiter, Quota, RateLimiter};
use platform::config::signaling::{ConnectionRateLimit, MessageRateLimit};
use std::collections::HashMap;
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// 连接速率限制器（基于 IP）
#[derive(Debug)]
pub struct ConnectionRateLimiter {
    /// 配置
    config: ConnectionRateLimit,
    /// 每个 IP 的速率限制器
    limiters: Arc<RwLock<HashMap<IpAddr, DefaultDirectRateLimiter>>>,
    /// 每个 IP 的当前连接数
    connections: Arc<RwLock<HashMap<IpAddr, u32>>>,
}

impl ConnectionRateLimiter {
    /// 创建新的连接速率限制器
    pub fn new(config: ConnectionRateLimit) -> Self {
        Self {
            config,
            limiters: Arc::new(RwLock::new(HashMap::new())),
            connections: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 检查是否允许新连接
    ///
    /// 返回 Ok(()) 如果允许，否则返回 Err
    pub async fn check_connection(&self, ip: IpAddr) -> Result<(), String> {
        if !self.config.enabled {
            return Ok(());
        }

        // 检查并发连接数
        let connections = self.connections.read().await;
        if let Some(&count) = connections.get(&ip)
            && count >= self.config.max_concurrent_per_ip
        {
            warn!(
                "IP {} exceeded max concurrent connections: {}/{}",
                ip, count, self.config.max_concurrent_per_ip
            );
            return Err(format!(
                "Too many concurrent connections from your IP: {}/{}",
                count, self.config.max_concurrent_per_ip
            ));
        }
        drop(connections);

        // 检查连接速率
        let mut limiters = self.limiters.write().await;
        let limiter = limiters.entry(ip).or_insert_with(|| {
            // 每分钟 per_minute 个连接，转换为每秒
            let per_second =
                NonZeroU32::new((self.config.per_minute as f64 / 60.0).ceil().max(1.0) as u32)
                    .unwrap();

            let quota = Quota::per_second(per_second)
                .allow_burst(NonZeroU32::new(self.config.burst_size).unwrap());

            RateLimiter::direct(quota)
        });

        match limiter.check() {
            Ok(_) => {
                debug!("IP {} passed connection rate limit check", ip);
                Ok(())
            }
            Err(_) => {
                warn!("IP {} exceeded connection rate limit", ip);
                Err(format!(
                    "Too many connection attempts. Limit: {} connections/minute",
                    self.config.per_minute
                ))
            }
        }
    }

    /// 增加连接计数
    pub async fn increment_connection(&self, ip: IpAddr) {
        if !self.config.enabled {
            return;
        }

        let mut connections = self.connections.write().await;
        *connections.entry(ip).or_insert(0) += 1;
        debug!(
            "IP {} connection count: {}",
            ip,
            connections.get(&ip).unwrap()
        );
    }

    /// 减少连接计数
    pub async fn decrement_connection(&self, ip: IpAddr) {
        if !self.config.enabled {
            return;
        }

        let mut connections = self.connections.write().await;
        if let Some(count) = connections.get_mut(&ip) {
            *count = count.saturating_sub(1);
            debug!("IP {} connection count decreased to: {}", ip, count);

            // 如果连接数为 0，移除记录以节省内存
            if *count == 0 {
                connections.remove(&ip);
            }
        }
    }

    /// 获取统计信息
    pub async fn stats(&self) -> (usize, usize) {
        let limiters = self.limiters.read().await;
        let connections = self.connections.read().await;
        (limiters.len(), connections.len())
    }
}

/// 消息速率限制器（基于连接 ID）
#[derive(Debug)]
pub struct MessageRateLimiter {
    /// 配置
    config: MessageRateLimit,
    /// 每个连接的速率限制器
    limiters: Arc<RwLock<HashMap<String, DefaultDirectRateLimiter>>>,
}

impl MessageRateLimiter {
    /// 创建新的消息速率限制器
    pub fn new(config: MessageRateLimit) -> Self {
        Self {
            config,
            limiters: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 检查是否允许发送消息
    ///
    /// 返回 Ok(()) 如果允许，否则返回 Err
    pub async fn check_message(&self, connection_id: &str) -> Result<(), String> {
        if !self.config.enabled {
            return Ok(());
        }

        let mut limiters = self.limiters.write().await;
        let limiter = limiters
            .entry(connection_id.to_string())
            .or_insert_with(|| {
                let per_second = NonZeroU32::new(self.config.per_second).unwrap();
                let quota = Quota::per_second(per_second)
                    .allow_burst(NonZeroU32::new(self.config.burst_size).unwrap());

                RateLimiter::direct(quota)
            });

        match limiter.check() {
            Ok(_) => {
                debug!(
                    "Connection {} passed message rate limit check",
                    connection_id
                );
                Ok(())
            }
            Err(_) => {
                warn!("Connection {} exceeded message rate limit", connection_id);
                Err(format!(
                    "Too many messages. Limit: {} messages/second",
                    self.config.per_second
                ))
            }
        }
    }

    /// 移除连接的速率限制器（连接关闭时调用）
    pub async fn remove_connection(&self, connection_id: &str) {
        if !self.config.enabled {
            return;
        }

        let mut limiters = self.limiters.write().await;
        limiters.remove(connection_id);
        debug!("Removed rate limiter for connection {}", connection_id);
    }

    /// 获取统计信息
    pub async fn stats(&self) -> usize {
        let limiters = self.limiters.read().await;
        limiters.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[tokio::test]
    async fn test_connection_rate_limiter_creation() {
        let config = ConnectionRateLimit::default();
        let _limiter = ConnectionRateLimiter::new(config);
    }

    #[tokio::test]
    async fn test_message_rate_limiter_creation() {
        let config = MessageRateLimit::default();
        let _limiter = MessageRateLimiter::new(config);
    }

    #[tokio::test]
    async fn test_connection_increment_decrement() {
        let config = ConnectionRateLimit::default();
        let limiter = ConnectionRateLimiter::new(config);
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        limiter.increment_connection(ip).await;
        let (_, conn_count) = limiter.stats().await;
        assert_eq!(conn_count, 1);

        limiter.decrement_connection(ip).await;
        let (_, conn_count) = limiter.stats().await;
        assert_eq!(conn_count, 0);
    }

    #[tokio::test]
    async fn test_message_limiter_removal() {
        let config = MessageRateLimit::default();
        let limiter = MessageRateLimiter::new(config);
        let conn_id = "test-connection-1";

        // 发送一条消息以创建限制器
        let _ = limiter.check_message(conn_id).await;
        assert_eq!(limiter.stats().await, 1);

        // 移除连接
        limiter.remove_connection(conn_id).await;
        assert_eq!(limiter.stats().await, 0);
    }
}
