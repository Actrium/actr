# AGENTS.md - AI 开发助手指南

本文档专为 AI 代码助手（如 Claude Code、GitHub Copilot、Cursor 等）设计，提供项目的开发规范和最佳实践。

## Critical invariants
- Key serialization: all secp256k1 public keys must be stored/sent as the 33-byte compressed output of `public_key.serialize_compressed()` (Base64-encoded). AIS/KS write paths and clients enforce this length; any 65-byte uncompressed key will be rejected.
- HTTP port sharing: all HTTP-based services (AIS, KS HTTP API, Signaling) share the same instance-level port defined by `bind.http` or `bind.https`.

## 项目概览

**Actrix** 是 Actor-RTC 生态系统的 WebRTC 辅助服务集合，提供 STUN、TURN、身份认证等核心服务。

- **代码库**: https://github.com/actor-rtc/actrix
- **许可证**: Apache 2.0
- **Rust 版本**: 1.88+ (Edition 2024)
- **架构**: Workspace 多 crate 架构

## 代码规范

### 1. 命名规范

```rust
// ✅ 正确：使用描述性名称
pub struct TurnAuthenticator {
    cache: LruCache<u128, Vec<u8>>,
}

// ❌ 错误：使用缩写或不清晰的名称
pub struct TurnAuth {
    c: LruCache<u128, Vec<u8>>,
}

// ✅ 正确：函数名清晰表达意图
fn compute_auth_key(username: &str, realm: &str, psk: &str) -> Vec<u8>

// ❌ 错误：函数名过于简略
fn compute_key(u: &str, r: &str, p: &str) -> Vec<u8>
```

### 2. 错误处理

```rust
// ✅ 正确：使用 Result 和自定义错误类型
pub enum TurnError {
    AuthenticationFailed(String),
    InvalidConfiguration(String),
    NetworkError(std::io::Error),
}

impl std::fmt::Display for TurnError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            TurnError::AuthenticationFailed(msg) => write!(f, "TURN 认证失败: {}", msg),
            TurnError::InvalidConfiguration(msg) => write!(f, "配置无效: {}", msg),
            TurnError::NetworkError(e) => write!(f, "网络错误: {}", e),
        }
    }
}

// ❌ 错误：使用字符串或 panic
fn authenticate(username: &str) -> String {
    if username.is_empty() {
        panic!("Username cannot be empty!");  // 不要使用 panic
    }
    "ok".to_string()  // 不要使用字符串表示结果
}
```

### 3. 日志记录

**按重要性选择日志级别**：

```rust
use tracing::{error, warn, info, debug, trace};

// error!：导致服务不可用的严重错误
error!("无法启动 TURN 服务器: {}", e);

// warn!：可恢复的错误或潜在问题
warn!("TURN 认证失败: username={}, src={}", username, src_addr);

// info!：重要的业务事件
info!("TURN 服务器启动成功: bind={}:{}", ip, port);

// debug!：调试信息（开发时使用）
debug!("缓存命中: key_id={}, cache_size={}", key_id, cache_size);

// trace!：详细的执行流程（性能调试时使用）
trace!("进入函数 compute_auth_key: username={}", username);
```

**日志内容规范**：

```rust
// ✅ 正确：提供上下文信息
info!(
    "TURN 认证成功: username={}, realm={}, src={}, cache_hit={}",
    username, realm, src_addr, cache_hit
);

// ❌ 错误：信息不足
info!("Authentication successful");

// ✅ 正确：使用结构化日志
debug!(
    target: "turn::auth",
    username = %username,
    cache_size = cache_size,
    "认证密钥缓存命中"
);

// ❌ 错误：泄露敏感信息
debug!("PSK: {}, Secret Key: {:?}", psk, secret_key);  // 不要记录密钥！
```

### 4. 测试规范

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;  // 全局状态测试需要串行化

    // ✅ 正确：清晰的测试名称
    #[test]
    fn test_authenticator_caches_auth_keys() {
        // Arrange: 准备测试数据
        Authenticator::clear_cache();
        let username = "user1";
        let realm = "realm1";
        let psk = "psk1";

        // Act: 执行操作
        let key1 = Authenticator::compute_auth_key(username, realm, psk);
        let key2 = Authenticator::compute_auth_key(username, realm, psk);

        // Assert: 验证结果
        assert_eq!(key1, key2);
        assert_eq!(Authenticator::cache_stats().0, 1);
    }

    // ✅ 正确：测试全局状态时使用 serial
    #[test]
    #[serial]
    fn test_lru_cache_eviction() {
        Authenticator::clear_cache();

        // 填满缓存
        for i in 0..1000 {
            Authenticator::compute_auth_key(
                &format!("user{}", i),
                &format!("realm{}", i),
                &format!("psk{}", i),
            );
        }

        assert_eq!(Authenticator::cache_stats().0, 1000);

        // 触发 LRU 淘汰
        Authenticator::compute_auth_key("new_user", "new_realm", "new_psk");
        assert_eq!(Authenticator::cache_stats().0, 1000);
    }

    // ❌ 错误：测试名称不清晰
    #[test]
    fn test1() {
        assert_eq!(1 + 1, 2);
    }
}
```

### 5. 性能优化

```rust
// ✅ 正确：使用 LRU 缓存优化重复计算
use lru::LruCache;
use once_cell::sync::Lazy;
use std::sync::Mutex;

static AUTH_KEY_CACHE: Lazy<Mutex<LruCache<u128, Vec<u8>>>> = Lazy::new(|| {
    let capacity = NonZeroUsize::new(1000).unwrap();
    Mutex::new(LruCache::new(capacity))
});

// ✅ 正确：避免不必要的克隆
fn get_config_value(&self) -> &str {
    &self.config_value  // 返回引用
}

// ❌ 错误：不必要的克隆
fn get_config_value(&self) -> String {
    self.config_value.clone()  // 避免不必要的克隆
}

// ✅ 正确：使用 non-blocking I/O
let (non_blocking, worker_guard) = tracing_appender::non_blocking(file_appender);
let subscriber = tracing_subscriber::fmt()
    .with_writer(non_blocking)
    .finish();

// ❌ 错误：阻塞式 I/O
std::fs::write("log.txt", log_message)?;  // 阻塞主线程
```

## Git 提交规范

### 提交消息格式

```
<type>: <subject>

<body>

<footer>
```

**类型（type）**：

- `feat`: 新功能
- `fix`: 修复 bug
- `perf`: 性能优化
- `refactor`: 重构（不改变功能）
- `docs`: 文档更新
- `test`: 测试相关
- `chore`: 构建/工具相关
- `style`: 代码格式（不影响功能）

**示例**：

```
feat: add LRU cache for TURN authentication

Implement LRU cache with 1000-entry capacity to optimize MD5
computation in TURN authentication. This improves performance
from 10,000 req/s to 14,000 req/s (+40%).

- Add lru dependency (0.12)
- Implement global AUTH_KEY_CACHE
- Add cache_stats() and clear_cache() methods
- Add comprehensive tests for caching behavior
```

**注意事项**：

- ✅ 使用英文编写提交消息
- ✅ 主题行不超过 72 字符
- ✅ 正文解释"为什么"而不仅是"做了什么"
- ❌ 不要提及 AI 工具（Claude、Copilot 等）
- ❌ 不要使用模糊的描述（"fix stuff", "update code"）

## 配置管理

### 配置层次

```
1. config.toml           - 生产配置（不提交到 git）
2. config.example.toml   - 配置模板（提交到 git）
3. ActrixConfig::default() - 代码默认值（开发环境）
```

### 添加新配置项

```rust
// 1. 在 crates/base/src/config/mod.rs 中添加字段
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ActrixConfig {
    // 现有字段...

    /// 新配置项说明
    ///
    /// 详细的配置说明，包括：
    /// - 用途和影响范围
    /// - 可选值和默认值
    /// - 使用场景和示例
    #[serde(default = "default_new_field")]
    pub new_field: String,
}

fn default_new_field() -> String {
    "default_value".to_string()
}

// 2. 在 config.example.toml 中添加注释和示例
# New field description
# - option1: Description of option1
# - option2: Description of option2
new_field = "default_value"

// 3. 更新 Default 实现
impl Default for ActrixConfig {
    fn default() -> Self {
        Self {
            // ...
            new_field: default_new_field(),
        }
    }
}

// 4. 添加单元测试
#[cfg(test)]
mod tests {
    #[test]
    fn test_new_field_default() {
        let config = ActrixConfig::default();
        assert_eq!(config.new_field, "default_value");
    }

    #[test]
    fn test_new_field_from_toml() {
        let toml = r#"
            enable = 31
            name = "test"
            env = "test"
            # ... 其他必需字段 ...
            new_field = "custom_value"
        "#;
        let config = ActrixConfig::from_toml(toml).unwrap();
        assert_eq!(config.new_field, "custom_value");
    }
}
```

## 架构模式

### 服务启动流程

```rust
// 1. 加载配置
let config = ActrixConfig::from_file("config.toml")?;

// 2. 初始化可观测性（日志 + 追踪）
let _guard = init_observability(config.observability_config())?;

// 3. 广播通道 + 服务管理器
let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(10);
let mut service_manager = ServiceManager::new(config.clone(), shutdown_tx.clone());

// 4. 根据配置注册服务
if config.is_turn_enabled() {
    service_manager.add_service(ServiceContainer::turn(TurnService::new(config.clone())));
}

if config.is_stun_enabled() {
    service_manager.add_service(ServiceContainer::stun(StunService::new(config.clone())));
}

// 5. 启动所有服务并收集任务句柄
let mut handles = service_manager.start_all().await?;

// 6. 附加 KS gRPC 等其他任务
if config.is_ks_enabled() {
    let mut ks_grpc = KsGrpcService::new(config.clone());
    handles.push(
        ks_grpc
            .start("127.0.0.1:50052".parse()?, shutdown_tx.clone())
            .await?,
    );
}

// 7. 顺序等待所有任务，出错即广播关闭
for handle in handles {
    if let Err(e) = handle.await {
        tracing::error!("background task failed: {}", e);
        let _ = shutdown_tx.send(());
    }
}

// 8. 优雅关闭（广播已触发，stop_all 清理资源）
service_manager.stop_all().await?;
```

> **运行时语义**：任何依附在 `shutdown_tx` 上的服务（包括 KS gRPC）退出后都会调用 `send(())`，从而驱动整套服务快速收敛，避免出现部分组件“僵死”但仍对外暴露健康状态的情况。

### 错误传播

```rust
// ✅ 正确：使用 ? 操作符传播错误
pub async fn start_turn_server(config: &TurnConfig) -> Result<TurnServer, TurnError> {
    let socket = UdpSocket::bind(&config.bind_address)
        .await
        .map_err(TurnError::NetworkError)?;

    let authenticator = Authenticator::new()
        .map_err(TurnError::AuthenticationError)?;

    Ok(TurnServer {
        socket,
        authenticator,
    })
}

// ❌ 错误：忽略错误或使用 unwrap
pub async fn start_turn_server(config: &TurnConfig) -> TurnServer {
    let socket = UdpSocket::bind(&config.bind_address)
        .await
        .unwrap();  // 不要使用 unwrap！

    TurnServer { socket }
}
```

## 依赖管理

### Workspace 依赖

```toml
# Cargo.toml（根目录）
[workspace.dependencies]
tokio = { version = "1.47", features = ["full"] }
tracing = "0.1"
serde = { version = "1.0", features = ["derive"] }

# crates/turn/Cargo.toml
[dependencies]
tokio = { workspace = true }
tracing = { workspace = true }
serde = { workspace = true }
```

### Feature Flags

```toml
# 可选特性（如 OpenTelemetry）
[features]
default = []
opentelemetry = [
    "dep:opentelemetry",
    "dep:opentelemetry_sdk",
    "dep:opentelemetry-otlp",
]

[dependencies]
opentelemetry = { workspace = true, optional = true }
opentelemetry_sdk = { workspace = true, optional = true }
opentelemetry-otlp = { workspace = true, optional = true }
```

```rust
// 代码中使用 feature flag
#[cfg(feature = "opentelemetry")]
use opentelemetry::trace::TracerProvider;

#[cfg(feature = "opentelemetry")]
fn init_tracing() {
    // OpenTelemetry 初始化
}

#[cfg(not(feature = "opentelemetry"))]
fn init_tracing() {
    // 默认追踪初始化
}
```

## 常见陷阱和解决方案

### 1. 全局状态测试冲突

**问题**：多个测试并发访问全局缓存导致失败

```rust
// ❌ 错误：测试会并发执行，相互干扰
#[test]
fn test_cache_a() {
    GLOBAL_CACHE.clear();
    assert_eq!(GLOBAL_CACHE.len(), 0);
}

#[test]
fn test_cache_b() {
    GLOBAL_CACHE.clear();
    assert_eq!(GLOBAL_CACHE.len(), 0);
}
```

**解决方案**：使用 `serial_test` crate

```rust
// ✅ 正确：测试串行执行
use serial_test::serial;

#[test]
#[serial]
fn test_cache_a() {
    GLOBAL_CACHE.clear();
    assert_eq!(GLOBAL_CACHE.len(), 0);
}

#[test]
#[serial]
fn test_cache_b() {
    GLOBAL_CACHE.clear();
    assert_eq!(GLOBAL_CACHE.len(), 0);
}
```

### 2. 包名冲突

**问题**：本地 crate 名称与依赖包名称冲突

```toml
# ❌ 错误：crate 名为 turn，依赖也叫 turn
[package]
name = "turn"

[dependencies]
turn = "0.10.0"  # 冲突！
```

**解决方案**：使用 package 重命名

```toml
# ✅ 正确：重命名依赖包
[package]
name = "turn"

[dependencies]
turn_crate = { package = "turn", version = "0.10.0" }
```

### 3. 日志阻塞主线程

**问题**：文件 I/O 阻塞主线程

```rust
// ❌ 错误：同步文件写入会阻塞
let file = std::fs::File::create("app.log")?;
let subscriber = tracing_subscriber::fmt()
    .with_writer(file)  // 阻塞式写入
    .finish();
```

**解决方案**：使用 non-blocking writer

```rust
// ✅ 正确：非阻塞写入
use tracing_appender::non_blocking;

let file_appender = tracing_appender::rolling::daily("logs/", "app.log");
let (non_blocking, _guard) = non_blocking(file_appender);

let subscriber = tracing_subscriber::fmt()
    .with_writer(non_blocking)  // 非阻塞写入
    .finish();
```

## 性能基准

### TURN 认证性能

```rust
// 无缓存: ~10,000 req/s
// 有缓存 (命中率 95%): ~14,000 req/s (+40%)

#[bench]
fn bench_auth_without_cache(b: &mut Bencher) {
    Authenticator::clear_cache();
    b.iter(|| {
        Authenticator::compute_auth_key("user", "realm", "psk")
    });
}

#[bench]
fn bench_auth_with_cache(b: &mut Bencher) {
    Authenticator::clear_cache();
    // 预热缓存
    Authenticator::compute_auth_key("user", "realm", "psk");

    b.iter(|| {
        Authenticator::compute_auth_key("user", "realm", "psk")
    });
}
```

## OpenTelemetry 集成

### 启用追踪

```bash
# 1. 启动 Jaeger
docker-compose -f docker/jaeger-compose.yml up -d

# 2. 编译启用 OpenTelemetry
cargo build --features opentelemetry

# 3. 配置追踪端点
# config.toml:
[observability.tracing]
enable = true
service_name = "actrix"
endpoint = "http://127.0.0.1:4317"

# 4. 访问 Jaeger UI
http://localhost:16686
```

### 添加自定义 Span

```rust
use tracing::{info_span, instrument};

#[instrument(skip(config))]
async fn start_turn_server(config: &TurnConfig) -> Result<(), TurnError> {
    let span = info_span!("turn_server_init",
        public_ip = %config.public_ip,
        port = config.public_port
    );
    let _enter = span.enter();

    info!("初始化 TURN 服务器");
    // ...

    Ok(())
}
```

## 文档规范

### 模块文档

```rust
//! TURN 认证器
//!
//! 实现 TURN 服务器的认证和授权功能，带 LRU 缓存优化。
//!
//! # 使用示例
//!
//! ```rust
//! use turn::Authenticator;
//!
//! let auth = Authenticator::new()?;
//! let auth_key = auth.auth_handle("username", "realm", addr)?;
//! ```
//!
//! # 性能
//!
//! - 无缓存: ~10,000 req/s
//! - 有缓存 (命中率 95%): ~14,000 req/s (+40%)
```

### 函数文档

```rust
/// 计算认证密钥，带 LRU 缓存优化
///
/// 计算 MD5(username:realm:psk)，结果会被缓存以提升性能。
///
/// # 参数
///
/// - `username`: 用户名
/// - `realm`: 认证域
/// - `psk`: 预共享密钥
///
/// # 返回值
///
/// 返回 MD5 哈希值（16 字节）
///
/// # 性能
///
/// - 缓存命中: O(1)
/// - 缓存未命中: O(n) 其中 n 是输入字符串长度
fn compute_auth_key(username: &str, realm: &str, psk: &str) -> Vec<u8>
```

## 开发工作流

### 本地开发

```bash
# 1. 格式化代码
cargo fmt

# 2. 运行 Clippy 检查
cargo clippy -- -D warnings

# 3. 运行所有测试
cargo test

# 4. 检查特定 crate
cargo test -p turn
cargo clippy -p turn

# 5. 运行服务（开发模式）
cargo run -- --config config.toml

# 6. 构建发布版本
cargo build --release
```

### 代码质量检查

```bash
# Makefile 提供了便捷命令
make fmt      # 格式化检查
make clippy   # Lint 检查
make test     # 运行测试
make coverage # 生成覆盖率报告
make all      # 运行所有检查
```

## 安全注意事项

### 1. 敏感信息处理

```rust
// ❌ 错误：记录敏感信息
debug!("PSK: {}, Secret Key: {:?}", psk, secret_key);
debug!("Auth token: {}", token);

// ✅ 正确：不记录敏感信息
debug!("认证成功: username={}", username);
debug!("密钥已生成: key_id={}", key_id);

// ✅ 正确：使用 zeroize 清理敏感数据
use zeroize::Zeroize;

let mut psk = get_psk();
// 使用 psk
psk.zeroize();  // 使用后立即清零
```

### 2. 输入验证

```rust
// ✅ 正确：验证所有外部输入
pub fn authenticate(username: &str, realm: &str) -> Result<(), AuthError> {
    if username.is_empty() {
        return Err(AuthError::InvalidUsername("Username cannot be empty".into()));
    }

    if username.len() > 255 {
        return Err(AuthError::InvalidUsername("Username too long".into()));
    }

    // 验证字符集
    if !username.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(AuthError::InvalidUsername("Invalid characters in username".into()));
    }

    // ...
}

// ❌ 错误：不验证输入
pub fn authenticate(username: &str, realm: &str) -> Result<(), AuthError> {
    // 直接使用未验证的输入
    let query = format!("SELECT * FROM users WHERE username = '{}'", username);
    // SQL 注入风险！
}
```

### 3. 错误信息

```rust
// ❌ 错误：泄露内部信息
return Err(format!("Database query failed: {}", sql));
return Err(format!("Invalid key at path: {}", key_path));

// ✅ 正确：通用错误信息
return Err("Authentication failed".into());
return Err("Invalid credentials".into());

// ✅ 正确：内部错误详细记录，外部错误通用
warn!("Database query failed: {}, sql={}", e, sql);
return Err("Internal server error".into());
```

## 资源清理

### RAII 模式

```rust
// ✅ 正确：使用 Drop trait 自动清理
pub struct ObservabilityGuard {
    #[cfg(feature = "opentelemetry")]
    tracer_provider: Option<SdkTracerProvider>,
    log_guard: Option<WorkerGuard>,
}

impl Drop for ObservabilityGuard {
    fn drop(&mut self) {
        #[cfg(feature = "opentelemetry")]
        if let Some(provider) = self.tracer_provider.take() {
            if let Err(e) = provider.shutdown() {
                eprintln!("Failed to shutdown tracer provider: {:?}", e);
            }
        }

        // log_guard 自动 Drop
    }
}

// 使用：
let _guard = init_observability(config.observability_config())?;
// _guard 离开作用域时自动清理资源
```

## 总结

遵循本指南中的规范和最佳实践，可以帮助 AI 助手生成更高质量、更一致的代码。

**核心原则**：

1. **清晰性** > 简洁性：优先使用描述性名称和详细注释
2. **安全性** > 性能：先确保正确和安全，再优化性能
3. **可维护性** > 灵活性：优先使用简单直接的实现
4. **测试覆盖**：所有公开 API 必须有单元测试
5. **文档完整**：所有公开 API 必须有文档注释

**持续改进**：

本指南会随着项目演进不断更新。如果发现新的模式或反模式，请及时更新本文档。

---

**版本**: 1.0.0
**最后更新**: 2025-10-28
**维护者**: Actor-RTC Team
