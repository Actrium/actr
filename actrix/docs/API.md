# Actrix API 参考

**版本**: v0.1.0
**最后更新**: 2025-11-03
**基础 URL**: `https://actrix.example.com` (生产) / `http://localhost:8443` (开发)

本文档记录 Actrix 对外提供的 HTTP/WebSocket API 端点。

---

## 📋 目录

- [认证机制](#认证机制)
- [KS - Key Server API](#ks---key-server-api)
- [错误响应](#错误响应)
- [速率限制](#速率限制)

---

## 认证机制

### Nonce-Auth 签名认证

所有 API 请求使用 PSK (Pre-Shared Key) + Nonce 签名机制防重放攻击。

**流程**:

```
1. 客户端生成 nonce (随机 UUID)
2. 客户端生成 timestamp (当前 Unix 时间戳)
3. 计算签名: HMAC-SHA256(psk, nonce + timestamp + payload)
4. 构造 NonceCredential 对象
5. 发送请求
```

**NonceCredential 结构**:

```json
{
  "nonce": "550e8400-e29b-41d4-a716-446655440000",
  "timestamp": 1730611200,
  "signature": "base64-encoded-hmac-sha256"
}
```

**时间窗口**: ±300 秒 (5 分钟)
**Nonce 唯一性**: 每个 nonce 只能使用一次

**Rust 客户端示例**:

```rust
use nonce_auth::CredentialSigner;

let psk = "my-shared-key";
let payload = "generate_key";

let credential = CredentialSigner::new()
    .with_secret(psk.as_bytes())
    .sign(payload.as_bytes())?;
```

---

## KS - Key Server API

**路由前缀**: `/ks`
**认证**: Nonce-Auth (PSK: `actrix_shared_key`)

### 1. 生成密钥对

生成新的 ECIES 椭圆曲线密钥对,返回公钥。

**端点**: `POST /ks/generate`
**认证负载**: `"generate_key"`

**请求**:

```http
POST /ks/generate HTTP/1.1
Host: actrix.example.com
Content-Type: application/json

{
  "credential": {
    "nonce": "550e8400-e29b-41d4-a716-446655440000",
    "timestamp": 1730611200,
    "signature": "qMZ8vL3x..."
  }
}
```

**响应 200 OK**:

```json
{
  "key_id": 123,
  "public_key": "BHxN7Q8vK...",
  "expires_at": 1730614800
}
```

**字段说明**:
- `key_id`: 密钥唯一标识符
- `public_key`: Base64 编码的 ECIES 公钥
- `expires_at`: 过期时间 (Unix 时间戳),0 表示永不过期

**curl 示例**:

```bash
# 使用 nonce-auth CLI 工具生成凭证
credential=$(nonce-auth sign --secret "my-psk" --payload "generate_key")

curl -X POST https://actrix.example.com/ks/generate \
  -H "Content-Type: application/json" \
  -d "{\"credential\": $credential}"
```

---

### 2. 获取私钥

根据 key_id 查询对应的私钥 (用于解密)。

**端点**: `GET /ks/secret/{key_id}`
**认证负载**: `"get_secret_key:{key_id}"`

**请求**:

```http
GET /ks/secret/123?credential=%7B%22nonce%22%3A%22...%22%7D HTTP/1.1
Host: actrix.example.com
```

**Query 参数**:
- `key_id`: (path) 密钥 ID
- `credential`: (query) URL 编码的 NonceCredential JSON

**响应 200 OK**:

```json
{
  "key_id": 123,
  "secret_key": "VxPz9m2...",
  "expires_at": 1730614800
}
```

**字段说明**:
- `key_id`: 密钥标识符
- `secret_key`: Base64 编码的 ECIES 私钥
- `expires_at`: 过期时间 (Unix 时间戳)

**curl 示例**:

```bash
KEY_ID=123
credential=$(nonce-auth sign --secret "my-psk" --payload "get_secret_key:$KEY_ID")

curl "https://actrix.example.com/ks/secret/$KEY_ID?credential=$(urlencode $credential)"
```

**Rust 客户端示例**:

```rust
use ks::{Client, ClientConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let actrix_shared_key = "my-shared-key";  // 从全局配置获取
    let client = Client::new(&ClientConfig {
        endpoint: "https://actrix.example.com/ks".to_string(),
        psk: actrix_shared_key.to_string(),
        timeout_seconds: 30,
        cache_db_path: None,
    });

    // 生成密钥对
  let (key_id, public_key, expires_at, tolerance_seconds) = client.generate_key().await?;
    println!("Generated key_id: {}", key_id);

    // 获取私钥
    let (secret_key, expires_at) = client.fetch_secret_key(key_id).await?;
    println!("Secret key fetched, expires at: {}", expires_at);

    Ok(())
}
```

---

### 3. 健康检查

检查 Signer 服务状态和数据库连接。

**端点**: `GET /ks/health`
**认证**: 无需认证

**响应 200 OK**:

```json
{
  "status": "healthy",
  "version": "0.1.0",
  "key_count": 42
}
```

**字段说明**:
- `status`: 服务状态 (`healthy` / `unhealthy`)
- `version`: 服务版本号
- `key_count`: 当前存储的密钥总数

**curl 示例**:

```bash
curl https://actrix.example.com/ks/health
```

---

## 错误响应

### 通用错误格式

```json
{
  "error": "错误描述信息"
}
```

### HTTP 状态码

| 状态码                        | 说明           | 示例                              |
| ----------------------------- | -------------- | --------------------------------- |
| **200 OK**                    | 请求成功       | -                                 |
| **400 Bad Request**           | 请求参数错误   | `{"error": "Invalid key_id"}`     |
| **401 Unauthorized**          | 认证失败       | `{"error": "Invalid signature"}`  |
| **403 Forbidden**             | 重放攻击检测   | `{"error": "Nonce already used"}` |
| **404 Not Found**             | 资源不存在     | `{"error": "Key not found: 123"}` |
| **500 Internal Server Error** | 服务器内部错误 | `{"error": "Database error"}`     |

### 认证错误详情

**401 Unauthorized - 签名错误**:
```json
{
  "error": "Authentication error: Invalid signature"
}
```

**原因**:
- PSK 不匹配
- payload 构造错误
- 签名算法错误

**403 Forbidden - 重放攻击**:
```json
{
  "error": "Replay attack detected: Nonce already used"
}
```

**原因**:
- Nonce 已使用过
- 时间戳超出允许窗口 (±300 秒)

**404 Not Found - 密钥不存在**:
```json
{
  "error": "Key not found: 123"
}
```

**原因**:
- key_id 不存在
- 密钥已过期并被清理

---

## 速率限制

当前版本无全局速率限制,建议根据部署环境在反向代理层实现。

**推荐限制** (nginx 示例):

```nginx
limit_req_zone $binary_remote_addr zone=ks_generate:10m rate=10r/s;
limit_req_zone $binary_remote_addr zone=ks_secret:10m rate=100r/s;

location /ks/generate {
    limit_req zone=ks_generate burst=5 nodelay;
    proxy_pass http://actrix_backend;
}

location /ks/secret {
    limit_req zone=ks_secret burst=20 nodelay;
    proxy_pass http://actrix_backend;
}
```

---

## 🔒 安全最佳实践

### 1. PSK 管理

```bash
# 生成强随机密钥
openssl rand -hex 32 > /etc/actrix/secrets/psk.key

# 设置严格权限
chmod 600 /etc/actrix/secrets/psk.key
chown actrix:actrix /etc/actrix/secrets/psk.key
```

### 2. TLS 强制

生产环境必须使用 HTTPS:

```toml
env = "prod"

[bind.https]
ip = "0.0.0.0"
port = 8443
cert = "/etc/actrix/certs/server.crt"
key = "/etc/actrix/certs/server.key"
```

### 3. 密钥轮转

定期清理过期密钥:

```bash
# 查询过期密钥数量
sqlite3 /var/lib/actrix/ks.db "
  SELECT COUNT(*) FROM keys
  WHERE expires_at > 0 AND expires_at < strftime('%s', 'now');
"

# 清理过期密钥
sqlite3 /var/lib/actrix/ks.db "
  DELETE FROM keys
  WHERE expires_at > 0 AND expires_at < strftime('%s', 'now');
"
```

### 4. 审计日志

启用详细日志记录:

```toml
[recording]
filter_level = "info"   # 可被 RUST_LOG 覆盖
sink = "file:///var/log/actrix/actrix.log"
service_name = "actrix-prod"

[recording.audit]
sink = "otlp+grpc://jaeger:4317"
```

---

## 📚 相关文档

- [CRATES.md](./CRATES.md) - KS 实现细节
- [SERVICES.md](./SERVICES.md) - 服务部署和管理
- [CONFIGURATION.md](./CONFIGURATION.md) - 配置参考

**文件**: `crates/services/ks/src/types.rs` - API 数据类型定义
**文件**: `crates/services/ks/src/handlers.rs` - API 处理器实现

**最后验证时间**: 2025-11-03
**代码版本**: v0.1.0+enhancements
