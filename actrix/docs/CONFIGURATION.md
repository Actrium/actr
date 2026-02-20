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
- `prod`: 要求 HTTPS, 建议配置 `file://` 记录出口
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

## Recording 管线配置

### recording.filter_level (必需)

**类型**: `String`  
**允许值**: `"trace"`, `"debug"`, `"info"`, `"warn"`, `"error"` (支持 EnvFilter 语法，如 `info,hyper=warn`)  
**默认值**: `"info"`  
**用途**: 统一的日志与追踪过滤规则。若设置了 `RUST_LOG` 环境变量，则优先生效。

```toml
[recording]
filter_level = "info"          # 默认过滤级别
# RUST_LOG=debug,hyper=info    # 环境变量覆盖配置
```

### recording.sink (可选)

**类型**: `String` (URI)  
**格式**: `file://...` / `otlp+http://...` / `otlp+grpc://...`  
**用途**: 全局记录出口。作为所有通道默认 sink。

```toml
[recording]
sink = "file:///var/log/actrix/actrix.log"
```

### recording.service_name (可选)

**类型**: `String`  
**默认值**: `"actrix"`  
**用途**: OTLP 导出使用的 `service.name`

```toml
[recording]
service_name = "actrix-prod-01"
```

### recording.<channel>.sink (可选)

**通道**: `observability` / `audit` / `security` / `operations`  
**用途**: 分通道覆盖。采用“global + spec 覆盖”规则：先读 `[recording]`，再用 `[recording.<channel>]` 覆盖。

```toml
[recording]
sink = "file:///var/log/actrix/actrix.log"

[recording.audit]
sink = "otlp+http://127.0.0.1:4318/v1/logs"    # 仅 audit 覆盖

[recording.security]
sink = "otlp+grpc://127.0.0.1:4317"            # 仅 security 覆盖
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

通过 `recording.sink` 或 `recording.<channel>.sink` 中的 `otlp+http://...` / `otlp+grpc://...` 启用。未配置 OTLP sink 时不导出。

```toml
[recording]
service_name = "actrix-prod-01"
sink = "otlp+grpc://127.0.0.1:4317"  # Jaeger/Tempo/OTel Collector
```

**注意**: 需要编译时启用 `opentelemetry` feature:
```bash
cargo build --features opentelemetry
```

## Control 配置（常驻）

**用途**: 控制面始终启用，复用主 HTTP/HTTPS 端口，不单独开 control 端口。

```toml
[control]
head = "admin_ui"  # admin_ui | grpc_api

[control.grpc_api]
node_id = "actrix-node-01"
node_name = "actrix-edge-01"
shared_secret = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
max_clock_skew_secs = 300
```

### control.head（必需，带默认）

**类型**: `String` (`admin_ui` / `grpc_api`)  \
**默认值**: `"admin_ui"`  \
**用途**:
- `admin_ui`: `/admin` 提供本地管理 UI
- `grpc_api`: `/admin.v1.NodeAdminService/*` 提供 NodeAdminService（给 supervisor 使用）
  兼容路径：`/admin/grpc/admin.v1.NodeAdminService/*`

### control.grpc_api.node_id（grpc_api 模式必需）

**类型**: `String`  \
**用途**: gRPC 认证载荷中的节点标识

### control.grpc_api.node_name（可选）

**类型**: `String`  \
**默认值**: `"actrix-node"`  \
**用途**: 节点展示名称；为空时回退到 `node_id`

### control.grpc_api.shared_secret（grpc_api 模式必需）

**类型**: `String` (Hex)  \
**用途**: nonce-auth 共享密钥  \
**要求**: 至少 64 个 hex 字符（32 字节）

### control.grpc_api.max_clock_skew_secs（可选）

**类型**: `u64`  \
**默认值**: `300`  \
**用途**: nonce-auth 允许的最大时间偏差（秒）

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
- ⚠️ 生产环境未配置任何 `file://` 记录出口

## 配置示例

### 最小配置 (STUN only)

```toml
enable = 2
name = "actrix-stun"
env = "dev"
sqlite_path = "database"
actrix_shared_key = "my-secure-key-min-16-chars"
location_tag = "dev,local"

[recording]
filter_level = "info"
# 默认 stdout（未配置 file://）

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

[recording]
filter_level = "info"
sink = "file:///var/log/actrix/actrix.log"
service_name = "actrix-prod-01"

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

[recording.audit]
sink = "otlp+grpc://otel-collector.internal:4317"

[control]
head = "grpc_api"

[control.grpc_api]
node_id = "actrix-prod-01"
node_name = "actrix-prod-01"
shared_secret = "REPLACE_WITH_HEX_SECRET"
max_clock_skew_secs = 300
```

### 开发配置

```toml
enable = 6  # TURN + STUN
name = "actrix-dev"
env = "dev"
sqlite_path = "database"
actrix_shared_key = "dev-key-16-chars-min"
location_tag = "local,dev"

[recording]
filter_level = "debug"
# 默认 stdout（未配置 file://）

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
```

## 环境变量

### RUST_LOG

覆盖 `recording.filter_level` 配置:

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
- [ ] 修改 control.grpc_api.shared_secret（若使用 grpc_api 头）
- [ ] 使用有效 TLS 证书
- [ ] 配置 `recording.sink` 或 `recording.<channel>.sink`
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
  2. ⚠️ Warning: Production environment should configure recording.sink (file://...) or channel-specific recording.<channel>.sink
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
