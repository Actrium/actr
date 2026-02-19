# Actrix gRPC 安全设计方案

## 一、当前安全分析

### 1.1 现有机制

**传输层安全（可选）：**
- TLS 1.3 加密（通过 `enable_tls` 配置启用）
- 服务端证书验证（通过 `tls_domain` 指定）
- 保护传输过程中的数据不被窃听和篡改

**现有配置：**
```rust
SupervisorConfig {
    connect_timeout_secs: u64,
    status_report_interval_secs: u64,
    health_check_interval_secs: u64,
    enable_tls: bool,
    tls_domain: Option<String>,
    max_clock_skew_secs: u64,
    supervisord: Option<SupervisordConfig>,
    client: Option<SupervisorClientConfig>,
}
```

### 1.2 安全不足

#### P0 - 关键缺陷

1. **无应用层认证**
   - 任何知道 `endpoint` 的客户端都可以连接
   - 无法验证客户端身份
   - 风险：冒充节点、数据投毒

2. **无防重放保护**
   - 没有 nonce 或 timestamp 验证
   - 攻击者可以重放合法请求
   - 风险：重复执行操作（如重复创建 Realm）

3. **缺少消息完整性校验**
   - 依赖 TLS 的 MAC
   - 一旦 TLS 被绕过（如内网环境），无保护
   - 风险：消息篡改

#### P1 - 重要缺陷

4. **无请求签名**
   - 无法追溯请求来源
   - 审计困难
   - 风险：难以定位安全事件

5. **密钥管理缺失**
   - 没有密钥轮换机制
   - 密钥泄露后无法撤销
   - 风险：长期妥协

6. **无访问控制**
   - 所有认证节点权限相同
   - 无法限制特定节点只能访问特定功能
   - 风险：权限滥用

---

## 二、威胁模型

### 2.1 攻击场景

| 威胁           | 攻击者能力         | 影响           | 当前防护 | 建议防护               |
| -------------- | ------------------ | -------------- | -------- | ---------------------- |
| **中间人攻击** | 网络嗅探、流量篡改 | 数据泄露、篡改 | ✅ TLS    | ✅ TLS (已足够)         |
| **节点冒充**   | 获取 endpoint      | 完全控制       | ❌ 无     | ⚠️ 需要 mTLS 或 Token   |
| **重放攻击**   | 截获合法请求       | 重复执行操作   | ❌ 无     | ⚠️ 需要 Nonce/Timestamp |
| **权限提升**   | 获取低权限凭证     | 访问高权限功能 | ❌ 无     | ⚠️ 需要 RBAC            |
| **内网攻击**   | 内网访问           | 绕过 TLS       | ❌ 无     | ⚠️ 需要应用层认证       |
| **DDoS**       | 大量连接           | 服务不可用     | ❌ 无     | ✅ gRPC 限流            |

### 2.2 信任边界

```
┌─────────────────────────────────────────────────────────────┐
│  信任边界 1: Internet (不可信)                                │
│                                                               │
│  ┌─────────────────────────────────────────────────────┐   │
│  │  TLS 保护层                                           │   │
│  │  ┌────────────────────────────────────────────┐     │   │
│  │  │  信任边界 2: 内网 (部分可信)                 │     │   │
│  │  │                                              │     │   │
│  │  │  ┌────────────────────────────────────┐   │     │   │
│  │  │  │  应用层认证保护                     │   │     │   │
│  │  │  │                                      │   │     │   │
│  │  │  │  actrix-node ←─gRPC─→ supervisor  │   │     │   │
│  │  │  │  (已认证)              (已认证)    │   │     │   │
│  │  │  │                                      │   │     │   │
│  │  │  └────────────────────────────────────┘   │     │   │
│  │  │                                              │     │   │
│  │  └────────────────────────────────────────────┘     │   │
│  │                                                       │   │
│  └─────────────────────────────────────────────────────┘   │
│                                                               │
└─────────────────────────────────────────────────────────────┘
```

**关键发现：**
- 当前仅保护信任边界 1（TLS）
- **缺失**：信任边界 2 保护（应用层认证）
- **风险**：内网攻击者可绕过所有安全措施

---

## 三、安全方案设计

### 3.1 方案选型对比

| 方案                | 安全性 | 复杂度 | 性能  | 适用场景               |
| ------------------- | ------ | ------ | ----- | ---------------------- |
| **mTLS (双向 TLS)** | ⭐⭐⭐⭐⭐  | ⭐⭐⭐⭐   | ⭐⭐⭐⭐⭐ | 企业内部、证书管理成熟 |
| **JWT Token**       | ⭐⭐⭐⭐   | ⭐⭐⭐    | ⭐⭐⭐⭐  | 互联网服务、动态节点   |
| **HMAC 签名**       | ⭐⭐⭐⭐   | ⭐⭐     | ⭐⭐⭐⭐⭐ | 固定节点、对称密钥     |
| **API Key**         | ⭐⭐⭐    | ⭐      | ⭐⭐⭐⭐⭐ | 简单场景、信任环境     |

### 3.2 推荐方案：**分层混合安全架构**

#### 🔐 L1: 传输层安全（必选）

**技术：mTLS (Mutual TLS)**

```toml
[supervisor]
enable_tls = true
tls_domain = "supervisor.example.com"
max_clock_skew_secs = 300

[supervisor.supervisord]
ip = "0.0.0.0"
port = 50055
advertised_ip = "203.0.113.10"

[supervisor.client]
node_id = "actrix-01"
endpoint = "https://supervisor.example.com:50051"
shared_secret = "<replace-with-hex>"

# 客户端证书认证（可选）
client_cert = "/etc/actrix/certs/client.crt"
client_key = "/etc/actrix/certs/client.key"
ca_cert = "/etc/actrix/certs/ca.crt"
```

**优势：**
- ✅ 双向身份验证（服务端验证客户端证书）
- ✅ gRPC 原生支持，性能开销小
- ✅ 自动密钥协商和更新
- ✅ 防中间人、防窃听、防篡改

**实现：**
```rust
// 客户端配置
let tls_config = ClientTlsConfig::new()
    .domain_name(&config.tls_domain)
    .ca_certificate(Certificate::from_pem(&ca_cert))
    .identity(Identity::from_pem(&client_cert, &client_key));

let channel = Endpoint::from_shared(config.endpoint)?
    .tls_config(tls_config)?
    .connect()
    .await?;
```

#### 🔑 L2: 应用层认证（必选）

**技术：gRPC Metadata + HMAC-SHA256 签名**

**设计原理：**
1. 每个节点分配唯一的 `node_id` 和 `shared_secret`
2. 每个请求在 metadata 中携带签名
3. 服务端验证签名有效性

**签名算法：**
```
signature = HMAC-SHA256(shared_secret, node_id + timestamp + request_hash)
```

**Metadata 格式：**
```
x-node-id: actrix-01
x-timestamp: 1699999999
x-signature: base64(hmac_result)
x-nonce: random_uuid
```

**实现：**
```rust
// 拦截器实现
pub struct AuthInterceptor {
    node_id: String,
    shared_secret: Vec<u8>,
}

impl Interceptor for AuthInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        let timestamp = Utc::now().timestamp();
        let nonce = Uuid::new_v4().to_string();

        // 计算请求内容哈希
        let request_hash = sha256(request.get_ref());

        // 生成签名
        let sign_content = format!("{}{}{}", self.node_id, timestamp, request_hash);
        let signature = hmac_sha256(&self.shared_secret, &sign_content);

        // 添加到 metadata
        request.metadata_mut().insert("x-node-id", self.node_id.parse()?);
        request.metadata_mut().insert("x-timestamp", timestamp.to_string().parse()?);
        request.metadata_mut().insert("x-signature", base64::encode(&signature).parse()?);
        request.metadata_mut().insert("x-nonce", nonce.parse()?);

        Ok(request)
    }
}
```

#### 🛡️ L3: 防重放保护（必选）

**技术：Timestamp + Nonce + 服务端缓存**

**验证逻辑：**
```rust
pub struct ReplayProtection {
    nonce_cache: Arc<RwLock<LruCache<String, Instant>>>,
    max_clock_skew: Duration,  // 允许的时钟偏差，如 5 分钟
}

impl ReplayProtection {
    pub fn verify(&self, timestamp: i64, nonce: &str) -> Result<()> {
        // 1. 检查时间戳（防止过期请求）
        let now = Utc::now().timestamp();
        let age = (now - timestamp).abs();

        if age > self.max_clock_skew.as_secs() as i64 {
            return Err(anyhow!("Request expired or clock skew too large"));
        }

        // 2. 检查 nonce（防止重放）
        let mut cache = self.nonce_cache.write().unwrap();

        if cache.contains(nonce) {
            return Err(anyhow!("Duplicate nonce detected - replay attack"));
        }

        // 3. 记录 nonce（带过期清理）
        cache.put(nonce.to_string(), Instant::now());

        Ok(())
    }
}
```

**配置：**
```toml
[supervisor.security]
max_clock_skew_secs = 300  # 5 分钟
nonce_cache_size = 10000   # 缓存 1 万个 nonce
nonce_ttl_secs = 600       # nonce 10 分钟后自动清理
```

#### 🎯 L4: 访问控制（推荐）

**技术：基于 node_id 的 RBAC**

**权限定义：**
```rust
#[derive(Debug, Clone)]
pub enum Permission {
    // 状态上报
    ReportStatus,

    // Realm 管理
    RealmCreate,
    RealmRead,
    RealmUpdate,
    RealmDelete,

    // 配置管理
    ConfigRead,
    ConfigUpdate,

    // 健康检查（所有节点）
    HealthCheck,
}

pub struct NodePermissions {
    permissions: HashMap<String, HashSet<Permission>>,
}

impl NodePermissions {
    pub fn check(&self, node_id: &str, permission: Permission) -> bool {
        self.permissions
            .get(node_id)
            .map(|perms| perms.contains(&permission))
            .unwrap_or(false)
    }
}
```

**配置示例：**
```toml
# 普通节点
[[supervisor.nodes]]
node_id = "actrix-node-01"
shared_secret = "hex_encoded_secret"
permissions = ["ReportStatus", "HealthCheck", "RealmRead"]

# 管理节点
[[supervisor.nodes]]
node_id = "actrix-admin-01"
shared_secret = "hex_encoded_admin_secret"
permissions = ["*"]  # 所有权限
```

---

## 四、完整实现方案

### 4.1 配置结构

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupervisorConfig {
    pub connect_timeout_secs: u64,
    pub status_report_interval_secs: u64,
    pub health_check_interval_secs: u64,
    pub enable_tls: bool,
    pub tls_domain: Option<String>,
    pub client_cert: Option<String>,
    pub client_key: Option<String>,
    pub ca_cert: Option<String>,
    pub max_clock_skew_secs: u64,
    pub supervisord: Option<SupervisordConfig>,
    pub client: Option<SupervisorClientConfig>,
}

pub struct SupervisorClientConfig {
    pub node_id: String,
    pub name: Option<String>,
    pub endpoint: String,
    pub shared_secret: Option<String>,
}

pub struct SupervisordConfig {
    pub node_name: Option<String>,
    pub ip: String,
    pub port: u16,
    pub advertised_ip: String,
}
```

### 4.2 客户端实现

```rust
pub struct AdminClient {
    config: AdminConfig,
    client: Option<GrpcSupervisorClient<InterceptedService<Channel, AuthInterceptor>>>,
    auth_interceptor: AuthInterceptor,
}

impl AdminClient {
    pub async fn connect(&mut self) -> Result<()> {
        // 1. 配置 TLS
        let mut endpoint = Endpoint::from_shared(self.config.endpoint.clone())?;

        if self.config.enable_tls {
            let tls_config = self.build_tls_config()?;
            endpoint = endpoint.tls_config(tls_config)?;
        }

        // 2. 建立连接
        let channel = endpoint.connect().await?;

        // 3. 添加认证拦截器
        let client = GrpcSupervisorClient::with_interceptor(
            channel,
            self.auth_interceptor.clone(),
        );

        self.client = Some(client);
        Ok(())
    }

    fn build_tls_config(&self) -> Result<ClientTlsConfig> {
        let ca_cert = std::fs::read(&self.config.ca_cert.as_ref().unwrap())?;
        let client_cert = std::fs::read(&self.config.client_cert.as_ref().unwrap())?;
        let client_key = std::fs::read(&self.config.client_key.as_ref().unwrap())?;

        Ok(ClientTlsConfig::new()
            .domain_name(self.config.tls_domain.as_ref().unwrap())
            .ca_certificate(Certificate::from_pem(&ca_cert))
            .identity(Identity::from_pem(&client_cert, &client_key)))
    }
}
```

### 4.3 服务端验证

```rust
pub struct SupervisorAuthService {
    permissions: Arc<NodePermissions>,
    replay_protection: Arc<ReplayProtection>,
    secrets: Arc<HashMap<String, Vec<u8>>>,
}

impl SupervisorAuthService {
    pub fn verify_request<T>(&self, request: &Request<T>) -> Result<String> {
        // 1. 提取 metadata
        let metadata = request.metadata();
        let node_id = metadata.get("x-node-id")
            .ok_or_else(|| anyhow!("Missing node_id"))?
            .to_str()?;
        let timestamp = metadata.get("x-timestamp")
            .ok_or_else(|| anyhow!("Missing timestamp"))?
            .to_str()?
            .parse::<i64>()?;
        let signature = metadata.get("x-signature")
            .ok_or_else(|| anyhow!("Missing signature"))?
            .to_str()?;
        let nonce = metadata.get("x-nonce")
            .ok_or_else(|| anyhow!("Missing nonce"))?
            .to_str()?;

        // 2. 防重放检查
        self.replay_protection.verify(timestamp, nonce)?;

        // 3. 验证签名
        let shared_secret = self.secrets.get(node_id)
            .ok_or_else(|| anyhow!("Unknown node_id"))?;

        let request_hash = sha256(request.get_ref());
        let sign_content = format!("{}{}{}", node_id, timestamp, request_hash);
        let expected_signature = hmac_sha256(shared_secret, &sign_content);

        if base64::encode(&expected_signature) != signature {
            return Err(anyhow!("Invalid signature"));
        }

        // 4. 返回已认证的 node_id
        Ok(node_id.to_string())
    }
}
```

---

## 五、安全配置指南

### 5.1 密钥生成

```bash
# 1. 生成 CA 证书（仅一次）
openssl req -x509 -newkey rsa:4096 -nodes \
    -keyout ca-key.pem -out ca-cert.pem -days 3650 \
    -subj "/CN=Actrix CA"

# 2. 生成服务端证书
openssl req -newkey rsa:4096 -nodes \
    -keyout server-key.pem -out server-req.pem \
    -subj "/CN=supervisor.example.com"
openssl x509 -req -in server-req.pem -CA ca-cert.pem -CAkey ca-key.pem \
    -CAcreateserial -out server-cert.pem -days 365

# 3. 生成客户端证书（每个节点）
openssl req -newkey rsa:4096 -nodes \
    -keyout client-actrix-01-key.pem -out client-actrix-01-req.pem \
    -subj "/CN=actrix-01"
openssl x509 -req -in client-actrix-01-req.pem -CA ca-cert.pem -CAkey ca-key.pem \
    -CAcreateserial -out client-actrix-01-cert.pem -days 365

# 4. 生成 shared_secret（每个节点）
openssl rand -hex 32  # 输出 64 字符的 hex 字符串
```

### 5.2 配置示例

**节点配置（actrix-node-01）：**
```toml
[supervisor]
connect_timeout_secs = 30
status_report_interval_secs = 60
health_check_interval_secs = 30
enable_tls = true
tls_domain = "supervisor.example.com"
max_clock_skew_secs = 300

[supervisor.supervisord]
node_name = "actrix-01"
ip = "0.0.0.0"
port = 50055
advertised_ip = "203.0.113.10"

[supervisor.client]
node_id = "actrix-01"
endpoint = "https://supervisor.example.com:50051"
shared_secret = "a1b2c3d4e5f6...64位hex字符串"
client_cert = "/etc/actrix/certs/client-actrix-01-cert.pem"
client_key = "/etc/actrix/certs/client-actrix-01-key.pem"
ca_cert = "/etc/actrix/certs/ca-cert.pem"
```

**Supervisor 配置：**
```toml
[server]
bind_addr = "0.0.0.0:50051"

# L1: TLS
enable_tls = true
server_cert = "/etc/supervisor/certs/server-cert.pem"
server_key = "/etc/supervisor/certs/server-key.pem"
ca_cert = "/etc/supervisor/certs/ca-cert.pem"
require_client_cert = true

# L3: 防重放
max_clock_skew_secs = 300
nonce_cache_size = 10000
nonce_ttl_secs = 600

# L4: 访问控制
[[nodes]]
node_id = "actrix-01"
shared_secret = "a1b2c3d4e5f6...64位hex字符串"
permissions = ["ReportStatus", "HealthCheck", "RealmRead"]

[[nodes]]
node_id = "actrix-admin"
shared_secret = "另一个64位hex字符串"
permissions = ["*"]
```

---

## 六、安全最佳实践

### 6.1 部署建议

1. **强制 TLS**
   - 生产环境必须启用 `enable_tls = true`
   - 使用有效的 CA 证书
   - 定期轮换证书（建议每年）

2. **密钥管理**
   - `shared_secret` 存储在安全的密钥管理系统（如 HashiCorp Vault）
   - 证书和私钥文件权限设置为 600
   - 定期轮换 shared_secret（建议每季度）

3. **网络隔离**
   - Supervisor 端口不对公网开放
   - 使用 VPC/VPN 限制访问
   - 配置防火墙规则

4. **监控告警**
   - 记录所有认证失败事件
   - 监控异常请求频率
   - 设置重放攻击告警

### 6.2 应急响应

**密钥泄露应对：**
```bash
# 1. 立即撤销泄露节点的访问权限（修改 supervisor 配置）
# 2. 生成新的 shared_secret
openssl rand -hex 32 > new_secret.txt

# 3. 更新节点配置
# 4. 重启节点和 supervisor
# 5. 审计日志，查找异常访问
```

---

## 七、性能影响评估

| 安全措施       | 延迟增加 | CPU 开销 | 内存开销         |
| -------------- | -------- | -------- | ---------------- |
| **mTLS**       | ~1-2ms   | ~3%      | 最小             |
| **HMAC 签名**  | ~0.1ms   | ~1%      | 最小             |
| **防重放缓存** | ~0.05ms  | ~0.5%    | ~10MB (1万nonce) |
| **总计**       | ~1-3ms   | ~4-5%    | ~10MB            |

**结论：**安全开销在可接受范围内，对高吞吐量场景影响很小。

---

## 八、与 Auxes 的对比

| 特性         | Auxes (ECIES + HMAC) | Actrix (mTLS + HMAC) |
| ------------ | -------------------- | -------------------- |
| **加密方式** | ECIES (应用层)       | TLS 1.3 (传输层)     |
| **签名方式** | HMAC-SHA256          | HMAC-SHA256 (相同)   |
| **防重放**   | Timestamp (5分钟)    | Timestamp + Nonce    |
| **身份认证** | shared_secret        | mTLS + shared_secret |
| **密钥管理** | 手动轮换             | 自动协商 (TLS)       |
| **性能**     | 较慢 (非对称加密)    | 更快 (对称加密)      |
| **复杂度**   | 高 (手动实现)        | 中 (依赖 TLS)        |

**优势：**
- ✅ Actrix 方案性能更好
- ✅ 安全性不降低（多层防护）
- ✅ 实现复杂度更低（复用 TLS）
- ✅ 更好的密钥管理（自动协商）

---

## 九、实施路线图

### Phase 1: 核心安全（2 周）
- [ ] 实现 mTLS 支持
- [ ] 实现 HMAC 认证拦截器
- [ ] 实现防重放保护
- [ ] 单元测试

### Phase 2: 访问控制（1 周）
- [ ] 实现基于 node_id 的 RBAC
- [ ] 实现权限配置加载
- [ ] 集成测试

### Phase 3: 监控审计（1 周）
- [ ] 添加安全事件日志
- [ ] 实现告警机制
- [ ] 性能测试

### Phase 4: 文档与培训（3 天）
- [ ] 编写部署文档
- [ ] 编写应急响应手册
- [ ] 团队培训

**总计：约 4 周完成完整安全加固**
