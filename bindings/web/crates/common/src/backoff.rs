//! Exponential Backoff 重试策略
//!
//! 用于连接失败后的重试延迟计算

use std::time::Duration;

/// 指数退避策略
#[derive(Debug, Clone)]
pub struct ExponentialBackoff {
    /// 当前重试次数
    attempt: u32,

    /// 初始延迟（毫秒）
    initial_delay_ms: u64,

    /// 最大延迟（毫秒）
    max_delay_ms: u64,

    /// 倍数因子
    multiplier: f64,

    /// 随机抖动因子 (0.0 - 1.0)
    jitter: f64,
}

impl ExponentialBackoff {
    /// 创建新的指数退避策略
    ///
    /// # 参数
    /// - `initial_delay_ms`: 初始延迟（毫秒）
    /// - `max_delay_ms`: 最大延迟（毫秒）
    pub fn new(initial_delay_ms: u64, max_delay_ms: u64) -> Self {
        Self {
            attempt: 0,
            initial_delay_ms,
            max_delay_ms,
            multiplier: 2.0,
            jitter: 0.1,
        }
    }

    /// 设置倍数因子
    pub fn with_multiplier(mut self, multiplier: f64) -> Self {
        self.multiplier = multiplier;
        self
    }

    /// 设置抖动因子
    pub fn with_jitter(mut self, jitter: f64) -> Self {
        self.jitter = jitter.clamp(0.0, 1.0);
        self
    }

    /// 获取下一次重试的延迟
    pub fn next_delay(&mut self) -> Duration {
        let base_delay = (self.initial_delay_ms as f64 * self.multiplier.powi(self.attempt as i32))
            .min(self.max_delay_ms as f64);

        // 添加简单的抖动（基于attempt）
        let jitter_range = base_delay * self.jitter;
        // 使用 attempt 作为伪随机源
        let pseudo_random = ((self.attempt * 7919) % 100) as f64 / 100.0; // 0.0 - 1.0
        let jitter = (pseudo_random * 2.0 - 1.0) * jitter_range;
        let final_delay = (base_delay + jitter).max(0.0);

        self.attempt += 1;

        Duration::from_millis(final_delay as u64)
    }

    /// 重置重试计数
    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    /// 获取当前重试次数
    pub fn attempt(&self) -> u32 {
        self.attempt
    }
}

impl Default for ExponentialBackoff {
    fn default() -> Self {
        Self::new(1000, 30000) // 1s - 30s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exponential_backoff() {
        let mut backoff = ExponentialBackoff::new(1000, 10000);

        let delay1 = backoff.next_delay();
        assert!(delay1.as_millis() >= 900 && delay1.as_millis() <= 1100);

        let delay2 = backoff.next_delay();
        assert!(delay2.as_millis() >= 1800 && delay2.as_millis() <= 2200);

        let delay3 = backoff.next_delay();
        assert!(delay3.as_millis() >= 3600 && delay3.as_millis() <= 4400);

        // 重置
        backoff.reset();
        let delay4 = backoff.next_delay();
        assert!(delay4.as_millis() >= 900 && delay4.as_millis() <= 1100);
    }

    #[test]
    fn test_max_delay() {
        let mut backoff = ExponentialBackoff::new(1000, 5000);

        for _ in 0..10 {
            backoff.next_delay();
        }

        let delay = backoff.next_delay();
        assert!(delay.as_millis() <= 5500); // 考虑抖动
    }
}
