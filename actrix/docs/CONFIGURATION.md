# Actrix 配置参考

本文档详细说明所有配置选项及其用途。

## 配置文件格式

使用 TOML 格式，通常命名为 `config.toml`。

## 核心配置

### enable (必需)

**类型**: `u8` (0-255)  
**默认值**: `31` (所有服务)  
**用途**: 位掩码控制服务启用

```toml
# Binary representation:
#   xxxxx
#   ││││└─ Signaling (0b00001 = 1)  [Currently disabled]
#   │││└── STUN      (0b00010 = 2)
#   ││└─── TURN      (0b00100 = 4)
#   │└──── AIS       (0b01000 = 8)  [Currently disabled]
#   └───── KS        (0b10000 = 16)

# 示例:
enable = 6   # STUN + TURN
enable = 22  # KS + TURN + STUN
enable = 2   # STUN only
enable = 4   # TURN only
```

### name (必需)

**类型**: `String`  
**默认值**: `"actrix-01"`  
**用途**: 服务实例标识，用于监控和追踪

```toml
name = "actrix-prod-beijing-01"
```

### env (必需)

**类型**: `String`  
**允许值**: `"dev"`, `"prod"`, `"test"`  
**默认值**: `"dev"`  
**用途**: 运行环境，影响验证规则

```toml
env = "prod"  # 强制 HTTPS, 文件日志
env = "dev"   # 允许 HTTP, 控制台日志
env = "test"  # 测试环境
```

**环境差异**:
- `prod`: 要求 HTTPS, 建议文件日志和轮转
- `dev`: 允许 HTTP, 宽松验证
- `test`: 用于自动化测试

### sqlite_path (必需)

**类型**: `String` (目录路径)  
**默认值**: `"database"`  
**用途**: SQLite 数据库文件存储目录路径。主数据库文件将存储为 `{sqlite_path}/actrix.db`

```toml
sqlite_path = "/var/lib/actrix"
sqlite_path = "database"  # 相对路径
```

**权限建议**: `chmod 755 {sqlite_path}` (目录权限)

### actrix_shared_key (必需)

**类型**: `String`  
**默认值**: (包含 "default" 会触发警告)  
**用途**: 服务间通信的共享密钥

```toml
# ❌ 不安全 (默认值)
actrix_shared_key = "default-actrix-shared-key-change-in-production"

# ✅ 安全 (生成强随机密钥)
actrix_shared_key = "a4f8c9d2e7b1f6a3c5e9d8f7b2a6c4e1d9f3b7a5c2e8d6f1b4a9c7e2d5f8b3a6"
```

**生成密钥**:
```bash
openssl rand -hex 32
```

**集群部署注意**:
在集群环境中，所有服务节点（实例）的 `auxes_shared_key` **必须完全一致**。这是确保内部服务之间可以成功认证和通信的前提。

**验证规则**:
- 长度 >= 16 字符
- 不包含 "default" 或 "change"

### location_tag (必需)

**类型**: `String`  
**默认值**: `"default-location"`  
**用途**: 地理位置或逻辑分组标签

```toml
location_tag = "aws,us-west-2,zone-a"
location_tag = "aliyun,beijing,zone-b"
location_tag = "office,beijing,rack-01"
```

## 可观测性配置

### observability.filter_level (必需)

**类型**: `String`  
**允许值**: `"trace"`, `"debug"`, `"info"`, `"warn"`, `"error"` (支持 EnvFilter 语法，如 `info,hyper=warn`)  
**默认值**: `"info"`  
**用途**: 统一的日志与追踪过滤规则。若设置了 `RUST_LOG` 环境变量，则优先生效。

```toml
[observability]
filter_level = "info"          # 默认过滤级别
# RUST_LOG=debug,hyper=info    # 环境变量覆盖配置
```

### observability.log.output (可选)

**类型**: `String`  
**允许值**: `"console"`, `"file"`  
**默认值**: `"console"`  
**用途**: 日志输出目标

```toml
[observability.log]
output = "console"  # 控制台 (开发)
output = "file"     # 文件 (生产)
```

### observability.log.rotate (可选)

**类型**: `bool`  
**默认值**: `false`  
**用途**: 启用按天日志轮转 (仅当 output = "file" 时)

```toml
[observability.log]
output = "file"
rotate = true   # actrix-2025-01-15.log
rotate = false  # actrix.log (追加)
```

### observability.log.path (可选)

**类型**: `String` (目录路径)  
**默认值**: `"logs/"`  
**用途**: 日志文件目录 (仅当 output = "file" 时)

```toml
[observability.log]
output = "file"
path = "/var/log/actrix/"
```

### observability.tracing (可选)

**用途**: OpenTelemetry 分布式追踪配置（需要 `opentelemetry` feature）

```toml
[observability.tracing]
enable = true
service_name = "actrix"
endpoint = "http://127.0.0.1:4317"
```

## 进程管理 (可选)

### pid (可选)

**类型**: `String` (文件路径)  
**默认值**: `Some("logs/actrix.pid")`  
**用途**: PID 文件路径

```toml
pid = "/var/run/actrix.pid"
```

### user (可选)

**类型**: `String`  
**默认值**: `None`  
**用途**: 运行用户 (绑定端口后切换)

```toml
user = "actrix"
```

### group (可选)

**类型**: `String`  
**默认值**: `None`  
**用途**: 运行用户组

```toml
group = "actrix"
```

## 网络绑定配置

### bind.http (可选)

**用途**: HTTP 服务绑定 (仅开发环境)

```toml
[bind.http]
domain_name = "localhost"
advertised_ip = "127.0.0.1"   # 对外宣告 IP
ip = "127.0.0.1"              # 实际绑定 IP
port = 8080
```

**字段说明**:
- `domain_name`: 域名
- `advertised_ip`: 客户端连接的 IP (NAT 环境为公网 IP)
- `ip`: 实际监听的网络接口 ("0.0.0.0" 监听所有)
- `port`: 端口号

### bind.https (可选, 生产环境推荐)

**用途**: HTTPS 服务绑定

```toml
[bind.https]
domain_name = "actrix.example.com"
advertised_ip = "203.0.113.10"
ip = "0.0.0.0"
port = 8443
cert = "certificates/server.crt"
key = "certificates/server.key"
```

**额外字段**:
- `cert`: TLS 证书路径
- `key`: TLS 私钥路径

**生产环境**: 强制要求 HTTPS (env = "prod")

### bind.ice (可选)

**用途**: ICE 服务 (STUN/TURN) 绑定

```toml
[bind.ice]
domain_name = "ice.example.com"
advertised_ip = "203.0.113.10"
ip = "0.0.0.0"
port = 3478  # 标准 STUN/TURN 端口
```

## TURN 配置

### turn.advertised_ip (必需, 当 TURN 启用时)

**类型**: `String` (IP 地址)  
**用途**: TURN 服务器公网 IP

```toml
[turn]
advertised_ip = "203.0.113.10"  # 必须是可路由的公网 IP
```

**验证**: 必须是有效的 IPv4/IPv6 地址

### turn.advertised_port (必需)

**类型**: `u16`  
**默认值**: `3478`  
**用途**: TURN 公网端口

```toml
advertised_port = 3478
```

### turn.relay_port_range (必需)

**类型**: `String`  
**格式**: `"start-end"`  
**默认值**: `"49152-65535"`  
**用途**: 中继端口范围

```toml
relay_port_range = "49152-65535"  # 推荐范围
relay_port_range = "50000-60000"  # 自定义范围
```

**注意**:
- 范围越大，并发会话越多
- 需要在防火墙开放

### turn.realm (必需)

**类型**: `String`  
**用途**: TURN 认证域

```toml
realm = "actrix.example.com"
```

## OpenTelemetry 追踪 (可选)

### observability.tracing.enable (可选)

**类型**: `bool`  
**默认值**: `false`  
**用途**: 启用 OpenTelemetry 追踪

```toml
[observability.tracing]
enable = true
```

**注意**: 需要编译时启用 feature:
```bash
cargo build --features opentelemetry
```

### observability.tracing.service_name (可选)

**类型**: `String`  
**默认值**: `"actrix"`  
**用途**: 服务名称 (在 Jaeger 等显示)

```toml
[observability.tracing]
service_name = "actrix-prod-01"
```

### observability.tracing.endpoint (可选)

**类型**: `String` (URL)  
**默认值**: `"http://127.0.0.1:4317"`  
**用途**: OTLP gRPC 端点

```toml
[observability.tracing]
endpoint = "http://127.0.0.1:4317"  # Jaeger
endpoint = "http://tempo:4317"      # Grafana Tempo
endpoint = "http://otel-collector:4317"  # OTel Collector
```

**验证**: 必须以 `http://` 或 `https://` 开头

## Supervisor 配置 (可选)

**用途**: 与 Supervisor 管理平台的 gRPC 集成，用于服务注册、状态上报和远程管理

```toml
[supervisor]
connect_timeout_secs = 30
status_report_interval_secs = 60
health_check_interval_secs = 30
enable_tls = false
# tls_domain = "supervisor.example.com"       # required when enable_tls = true
# client_cert = "/path/to/client-cert.pem"    # optional (mTLS)
# client_key = "/path/to/client-key.pem"      # optional (mTLS)
# ca_cert = "/path/to/ca-cert.pem"            # optional
max_clock_skew_secs = 300

[supervisor.supervisord]
node_name = "actrix-node"
ip = "0.0.0.0"
port = 50055
advertised_ip = "127.0.0.1"

[supervisor.client]
node_id = "actrix-node-01"
# name = "actrix-edge-01"                     # optional
endpoint = "http://supervisor.example.com:50051"
shared_secret = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
```

### supervisor.connect_timeout_secs (可选)

**类型**: `u64`  \
**默认值**: `30`  \
**用途**: 连接超时时间（秒）

### supervisor.status_report_interval_secs (可选)

**类型**: `u64`  \
**默认值**: `60`  \
**用途**: 状态上报间隔（秒）

### supervisor.health_check_interval_secs (可选)

**类型**: `u64`  \
**默认值**: `30`  \
**用途**: 健康检查间隔（秒）

### supervisor.enable_tls / supervisor.tls_domain / supervisor.client_cert / supervisor.client_key / supervisor.ca_cert (可选)

**用途**: 控制连接到 Supervisor 时的 TLS/mTLS 行为；当 `enable_tls = true` 时需要配置 `tls_domain`，配置 mTLS 时需要同时提供 `client_cert` 与 `client_key`

### supervisor.max_clock_skew_secs (可选)

**类型**: `u64`  \
**默认值**: `300`  \
**用途**: nonce-auth 允许的最大时间偏差（秒）

### supervisor.supervisord.node_name (可选)

**类型**: `String`  \
**用途**: Supervisord 服务展示名称，未配置时回退到 `supervisor.client.name` 或 `node_id`

### supervisor.supervisord.ip / port / advertised_ip (必需)

**用途**: Supervisord gRPC 服务监听地址与对外曝光地址；`advertised_ip:port` 将写入注册信息，供 Supervisor 回连

### supervisor.client.node_id (必需)

**类型**: `String`  \
**用途**: 节点唯一标识符，在 Supervisor 平台中用于识别此服务实例

### supervisor.client.name (可选)

**类型**: `String`  \
**用途**: 节点展示名称，未配置时回退到 `node_id`

### supervisor.client.endpoint (必需)

**类型**: `String` (URL)  \
**用途**: Supervisor 平台的 gRPC 入口地址  \
**格式**: `http://hostname:port` 或 `https://hostname:port`

### supervisor.client.shared_secret (必需)

**类型**: `String` (Hex)  \
**用途**: 共享密钥，用于 nonce-auth 认证的 HMAC 签名  \
**要求**: 至少 64 个 hex 字符（32 字节），建议使用 `openssl rand -hex 32` 生成

## KS (Key Server) 配置 (可选)

**当前状态**: KS 服务可用，配置待完善

```toml
[ks]
# 未来配置选项
```

## 配置验证

### 验证命令

```bash
# 测试配置有效性
cargo run -- test config.toml
./actrix test config.toml
```

### 验证规则

#### 错误 (阻止启动)
- ❌ 必需字段缺失
- ❌ 无效的 IP 地址格式
- ❌ 无效的环境值 (非 dev/prod/test)
- ❌ 无效的过滤级别
- ❌ TURN 启用但缺少配置
- ❌ KS 启用但缺少配置

#### 警告 (允许启动)
- ⚠️ 使用默认 actrix_shared_key
- ⚠️ 密钥长度 < 16
- ⚠️ 生产环境使用控制台日志
- ⚠️ 生产环境未启用日志轮转

## 配置示例

### 最小配置 (STUN only)

```toml
enable = 2
name = "actrix-stun"
env = "dev"
sqlite_path = "database"
actrix_shared_key = "my-secure-key-min-16-chars"
location_tag = "dev,local"

[observability]
filter_level = "info"

[observability.log]
output = "console"

[bind.ice]
domain_name = "localhost"
advertised_ip = "127.0.0.1"
ip = "127.0.0.1"
port = 3478
```

### 生产配置 (TURN + STUN + KS)

```toml
enable = 22  # KS + TURN + STUN
name = "actrix-prod-01"
env = "prod"
sqlite_path = "/var/lib/actrix"
actrix_shared_key = "REPLACE_WITH_STRONG_32_CHAR_HEX_KEY"
location_tag = "aws,us-west-2,zone-a"

[observability]
filter_level = "info"

[observability.log]
output = "file"
rotate = true
path = "/var/log/actrix/"

pid = "/var/run/actrix.pid"
user = "actrix"
group = "actrix"

[bind.https]
domain_name = "actrix.example.com"
advertised_ip = "203.0.113.10"
ip = "0.0.0.0"
port = 8443
cert = "/etc/actrix/tls/fullchain.pem"
key = "/etc/actrix/tls/privkey.pem"

[bind.ice]
domain_name = "ice.example.com"
advertised_ip = "203.0.113.10"
ip = "0.0.0.0"
port = 3478

[turn]
advertised_ip = "203.0.113.10"
advertised_port = 3478
relay_port_range = "49152-65535"
realm = "actrix.example.com"

[observability.tracing]
enable = true
service_name = "actrix-prod-01"
endpoint = "http://otel-collector.internal:4317"

[supervisor]
connect_timeout_secs = 30
status_report_interval_secs = 60
health_check_interval_secs = 30
enable_tls = true
tls_domain = "supervisor.example.com"
max_clock_skew_secs = 300

[supervisor.supervisord]
node_name = "actrix-prod-01"
ip = "0.0.0.0"
port = 50055
advertised_ip = "203.0.113.10"

[supervisor.client]
node_id = "actrix-prod-01"
endpoint = "https://supervisor.example.com:50051"
shared_secret = "REPLACE_WITH_HEX_SECRET"
```

### 开发配置

```toml
enable = 6  # TURN + STUN
name = "actrix-dev"
env = "dev"
sqlite_path = "database"
actrix_shared_key = "dev-key-16-chars-min"
location_tag = "local,dev"

[observability]
filter_level = "debug"

[observability.log]
output = "console"

[bind.http]
domain_name = "localhost"
advertised_ip = "127.0.0.1"
ip = "127.0.0.1"
port = 8080

[bind.ice]
domain_name = "localhost"
advertised_ip = "127.0.0.1"
ip = "127.0.0.1"
port = 3478

[turn]
advertised_ip = "127.0.0.1"
advertised_port = 3478
relay_port_range = "49152-65535"
realm = "localhost"

[observability.tracing]
enable = false  # 开发时可选
```

## 环境变量

### RUST_LOG

覆盖 `observability.filter_level` 配置:

```bash
RUST_LOG=debug ./actrix
RUST_LOG=actrix=trace,ks=debug ./actrix
```

### RUST_BACKTRACE

启用错误回溯:

```bash
RUST_BACKTRACE=1 ./actrix
RUST_BACKTRACE=full ./actrix
```

## 配置热重载

**当前状态**: 不支持

**计划**: 未来版本支持通过 SIGHUP 重载配置

## 配置安全

### 1. 文件权限

```bash
chmod 600 config.toml
chown actrix:actrix config.toml
```

### 2. 密钥管理

- ✅ 使用强随机密钥
- ✅ 定期轮转密钥
- ✅ 不提交到版本控制
- ✅ 使用环境变量或密钥管理服务

### 3. 生产清单

- [ ] 修改 actrix_shared_key
- [ ] 修改 supervisor.shared_secret
- [ ] 使用有效 TLS 证书
- [ ] 启用文件日志和轮转
- [ ] 设置 user/group
- [ ] 配置防火墙规则
- [ ] 测试配置: `actrix test config.toml`

## 故障排查

### 配置解析失败

```
Error: TOML parse error...
```

**解决**: 检查 TOML 语法，使用 TOML 验证工具

### 配置验证失败

查看详细错误信息，逐项修复:

```bash
$ actrix test config.toml
❌ 配置验证发现问题:
  1. ❌ Security warning: actrix_shared_key appears to be a default value
  2. ⚠️ Warning: Production environment should use file logging
```

### 端口冲突

检查端口占用:

```bash
netstat -tlnp | grep 8443
lsof -i :3478
```

## 参考

- [config.example.toml](../config.example.toml) - 完整示例
- [DEVELOPMENT.md](DEVELOPMENT.md) - 开发指南
- [deploy/README.md](../deploy/README.md) - 部署指南
