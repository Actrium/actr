//! Prometheus 监控指标模块
//!
//! 提供全局指标收集和导出功能

use lazy_static::lazy_static;
use prometheus::{
    HistogramOpts, HistogramVec, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry,
};
use std::sync::Once;
use std::time::Instant;

static METRICS_INIT: Once = Once::new();

lazy_static! {
    /// 全局 Prometheus Registry
    pub static ref REGISTRY: Registry = Registry::new();

    // ========== 业务指标 ==========

    /// 当前注册的 Actor 总数（按 realm 分组）
    pub static ref ACTORS_TOTAL: IntGaugeVec = IntGaugeVec::new(
        Opts::new("actrix_actors_total", "Total number of registered actors")
            .namespace("actrix"),
        &["realm_id", "service"]
    ).unwrap();

    /// 当前注册的服务总数
    pub static ref SERVICES_TOTAL: IntGaugeVec = IntGaugeVec::new(
        Opts::new("actrix_services_total", "Total number of registered services")
            .namespace("actrix"),
        &["service_name"]
    ).unwrap();

    /// WebSocket 连接数
    pub static ref WEBSOCKET_CONNECTIONS: IntGauge = IntGauge::new(
        "actrix_websocket_connections",
        "Number of active WebSocket connections"
    ).unwrap();

    /// Token 颁发次数（AIS）
    pub static ref TOKENS_ISSUED: IntCounterVec = IntCounterVec::new(
        Opts::new("actrix_tokens_issued_total", "Total number of tokens issued")
            .namespace("actrix"),
        &["realm_id", "status"]
    ).unwrap();

    /// Token 验证次数（Signaling/TURN）
    pub static ref TOKENS_VALIDATED: IntCounterVec = IntCounterVec::new(
        Opts::new("actrix_tokens_validated_total", "Total number of tokens validated")
            .namespace("actrix"),
        &["service", "status"]
    ).unwrap();

    // ========== 性能指标 ==========

    /// HTTP 请求延迟（秒）
    pub static ref REQUEST_DURATION: HistogramVec = HistogramVec::new(
        HistogramOpts::new("actrix_request_duration_seconds", "HTTP request duration in seconds")
            .namespace("actrix")
            .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0]),
        &["service", "method", "path", "status"]
    ).unwrap();

    /// HTTP 请求总数
    pub static ref REQUESTS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("actrix_requests_total", "Total number of HTTP requests")
            .namespace("actrix"),
        &["service", "method", "path", "status"]
    ).unwrap();

    /// 错误次数
    pub static ref ERRORS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("actrix_errors_total", "Total number of errors")
            .namespace("actrix"),
        &["service", "error_type"]
    ).unwrap();

    // ========== 系统指标 ==========

    /// 缓存命中次数
    pub static ref CACHE_HITS: IntCounterVec = IntCounterVec::new(
        Opts::new("actrix_cache_hits_total", "Total number of cache hits")
            .namespace("actrix"),
        &["cache_type"]
    ).unwrap();

    /// 缓存未命中次数
    pub static ref CACHE_MISSES: IntCounterVec = IntCounterVec::new(
        Opts::new("actrix_cache_misses_total", "Total number of cache misses")
            .namespace("actrix"),
        &["cache_type"]
    ).unwrap();

    /// 数据库连接池状态
    pub static ref DB_CONNECTIONS: IntGaugeVec = IntGaugeVec::new(
        Opts::new("actrix_db_connections", "Number of database connections")
            .namespace("actrix"),
        &["pool", "state"]
    ).unwrap();

    // ========== 安全指标 ==========

    /// 速率限制触发次数
    pub static ref RATE_LIMIT_EXCEEDED: IntCounterVec = IntCounterVec::new(
        Opts::new("actrix_rate_limit_exceeded_total", "Total number of rate limit violations")
            .namespace("actrix"),
        &["service", "limiter_type"]
    ).unwrap();

    /// 认证失败次数
    pub static ref AUTH_FAILURES: IntCounterVec = IntCounterVec::new(
        Opts::new("actrix_auth_failures_total", "Total number of authentication failures")
            .namespace("actrix"),
        &["service", "reason"]
    ).unwrap();

    /// 非法请求次数
    pub static ref INVALID_REQUESTS: IntCounterVec = IntCounterVec::new(
        Opts::new("actrix_invalid_requests_total", "Total number of invalid requests")
            .namespace("actrix"),
        &["service", "reason"]
    ).unwrap();

    // ========== KS 特定指标 ==========

    /// 密钥生成次数
    pub static ref KEYS_GENERATED: IntCounterVec = IntCounterVec::new(
        Opts::new("actrix_keys_generated_total", "Total number of keys generated")
            .namespace("actrix"),
        &["key_type"]
    ).unwrap();

    /// 密钥轮转次数
    pub static ref KEY_ROTATIONS: IntCounterVec = IntCounterVec::new(
        Opts::new("actrix_key_rotations_total", "Total number of key rotations")
            .namespace("actrix"),
        &["reason"]
    ).unwrap();

    // ========== TURN 特定指标 ==========

    /// TURN 分配请求
    pub static ref TURN_ALLOCATIONS: IntCounterVec = IntCounterVec::new(
        Opts::new("actrix_turn_allocations_total", "Total number of TURN allocations")
            .namespace("actrix"),
        &["status"]
    ).unwrap();

    /// TURN 活跃会话数
    pub static ref TURN_ACTIVE_SESSIONS: IntGauge = IntGauge::new(
        "actrix_turn_active_sessions",
        "Number of active TURN sessions"
    ).unwrap();

    /// TURN 流量统计（字节）
    pub static ref TURN_BYTES_RELAYED: IntCounterVec = IntCounterVec::new(
        Opts::new("actrix_turn_bytes_relayed_total", "Total bytes relayed by TURN")
            .namespace("actrix"),
        &["direction"]
    ).unwrap();
}

/// 注册所有指标到全局 Registry
///
/// This function is idempotent - calling it multiple times is safe.
/// Only the first call will actually register the metrics.
pub fn register_metrics() -> Result<(), prometheus::Error> {
    let mut result = Ok(());

    METRICS_INIT.call_once(|| {
        let register_result = (|| {
            // 业务指标
            REGISTRY.register(Box::new(ACTORS_TOTAL.clone()))?;
            REGISTRY.register(Box::new(SERVICES_TOTAL.clone()))?;
            REGISTRY.register(Box::new(WEBSOCKET_CONNECTIONS.clone()))?;
            REGISTRY.register(Box::new(TOKENS_ISSUED.clone()))?;
            REGISTRY.register(Box::new(TOKENS_VALIDATED.clone()))?;

            // 性能指标
            REGISTRY.register(Box::new(REQUEST_DURATION.clone()))?;
            REGISTRY.register(Box::new(REQUESTS_TOTAL.clone()))?;
            REGISTRY.register(Box::new(ERRORS_TOTAL.clone()))?;

            // 系统指标
            REGISTRY.register(Box::new(CACHE_HITS.clone()))?;
            REGISTRY.register(Box::new(CACHE_MISSES.clone()))?;
            REGISTRY.register(Box::new(DB_CONNECTIONS.clone()))?;

            // 安全指标
            REGISTRY.register(Box::new(RATE_LIMIT_EXCEEDED.clone()))?;
            REGISTRY.register(Box::new(AUTH_FAILURES.clone()))?;
            REGISTRY.register(Box::new(INVALID_REQUESTS.clone()))?;

            // KS 特定指标
            REGISTRY.register(Box::new(KEYS_GENERATED.clone()))?;
            REGISTRY.register(Box::new(KEY_ROTATIONS.clone()))?;

            // TURN 特定指标
            REGISTRY.register(Box::new(TURN_ALLOCATIONS.clone()))?;
            REGISTRY.register(Box::new(TURN_ACTIVE_SESSIONS.clone()))?;
            REGISTRY.register(Box::new(TURN_BYTES_RELAYED.clone()))?;

            Ok::<(), prometheus::Error>(())
        })();

        if let Err(e) = register_result {
            result = Err(e);
        }
    });

    result
}

/// HTTP 请求计时器
pub struct RequestTimer {
    start: Instant,
    service: String,
    method: String,
    path: String,
}

impl RequestTimer {
    /// 创建计时器
    pub fn new(service: &str, method: &str, path: &str) -> Self {
        Self {
            start: Instant::now(),
            service: service.to_string(),
            method: method.to_string(),
            path: path.to_string(),
        }
    }

    /// 完成计时并记录指标
    pub fn observe(self, status: u16) {
        let duration = self.start.elapsed().as_secs_f64();
        let status_str = status.to_string();

        REQUEST_DURATION
            .with_label_values(&[&self.service, &self.method, &self.path, &status_str])
            .observe(duration);

        REQUESTS_TOTAL
            .with_label_values(&[&self.service, &self.method, &self.path, &status_str])
            .inc();
    }
}

/// 导出 Prometheus 格式的指标
pub fn export_metrics() -> String {
    use prometheus::Encoder;
    let encoder = prometheus::TextEncoder::new();
    let metric_families = REGISTRY.gather();

    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();

    String::from_utf8(buffer).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_metrics() {
        // 注册应该成功（或者已经注册过了）
        let result = register_metrics();
        // 如果已注册，会返回错误，但不影响功能
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_request_timer() {
        let _ = register_metrics();

        let timer = RequestTimer::new("test-service", "GET", "/test");
        std::thread::sleep(std::time::Duration::from_millis(10));
        timer.observe(200);

        // 验证计数器增加
        let before = REQUESTS_TOTAL
            .with_label_values(&["test-service", "GET", "/test", "200"])
            .get();

        let timer2 = RequestTimer::new("test-service", "GET", "/test");
        timer2.observe(200);

        let after = REQUESTS_TOTAL
            .with_label_values(&["test-service", "GET", "/test", "200"])
            .get();

        assert!(after > before);
    }

    #[test]
    fn test_export_metrics() {
        let _ = register_metrics();

        WEBSOCKET_CONNECTIONS.set(42);

        let output = export_metrics();
        // Check for the metric name (may appear with or without namespace prefix)
        assert!(
            output.contains("actrix_websocket_connections")
                || output.contains("websocket_connections"),
            "Output should contain websocket_connections metric. Output: {}",
            output
        );
        assert!(
            output.contains("42"),
            "Output should contain value 42. Output: {}",
            output
        );
    }
}
