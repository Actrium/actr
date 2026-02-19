# KS (Key Server) 完全指南

**版本**: v0.1.0
**最后更新**: 2025-11-03
**状态**: ✅ 生产就绪 (内部使用)

---

## 📋 目录

1. [概述](#概述)
2. [核心概念](#核心概念)
3. [系统架构](#系统架构)
4. [工作流程](#工作流程)
5. [API 详解](#api-详解)
6. [安全机制](#安全机制)
7. [数据存储](#数据存储)
8. [客户端集成](#客户端集成)
9. [运维指南](#运维指南)
10. [故障排查](#故障排查)

---

## 概述

### 什么是 KS?

**KS (Key Server)** 是 Actrix 项目中的**椭圆曲线密钥管理服务**，为整个 Actor-RTC 生态系统提供加密密钥的生成、存储和查询能力。

### 核心功能

```
┌─────────────────────────────────────────────────────────┐
│                    KS 核心功能                          │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  🔑 密钥生成     → 生成 ECIES 椭圆曲线密钥对           │
│  💾 安全存储     → SQLite 持久化存储                   │
│  🔍 密钥查询     → 基于 key_id 快速检索                │
│  ⏰ 生命周期管理  → 自动过期和清理                      │
│  🛡️  防重放攻击   → Nonce + PSK 双重保护               │
│  📊 状态监控     → 健康检查和统计信息                   │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

### 使用场景

```
场景 1: AIS 服务需要加密 Actor ID Token
   ├─> AIS 向 KS 请求公钥
   ├─> KS 生成密钥对，返回公钥
   ├─> AIS 使用公钥加密 Token
   └─> 验证服务从 KS 获取私钥解密

场景 2: 内部服务间加密通信
   ├─> 服务 A 从 KS 获取公钥
   ├─> 服务 A 使用公钥加密敏感数据
   ├─> 服务 B 从 KS 获取私钥解密
   └─> 实现端到端加密
```

### 关键特性

| 特性             | 说明                          | 文件位置             |
| ---------------- | ----------------------------- | -------------------- |
| **ECIES 加密**   | 基于椭圆曲线的集成加密方案    | `storage.rs:196`     |
| **自动递增 ID**  | SQLite AUTOINCREMENT 避免冲突 | `storage.rs:61`      |
| **PSK 认证**     | Pre-Shared Key 认证机制       | `handlers.rs:35-57`  |
| **Nonce 防重放** | 基于 nonce-auth v0.6.1        | `nonce_storage.rs`   |
| **密钥过期**     | 可配置 TTL，自动清理          | `storage.rs:226-244` |
| **RESTful API**  | 标准 HTTP JSON 接口           | `handlers.rs:86-92`  |

---

## 核心概念

### 1. 椭圆曲线加密 (ECIES)

```
ECIES (Elliptic Curve Integrated Encryption Scheme)
╔═══════════════════════════════════════════════════════╗
║                                                       ║
║  私钥 (Secret Key)  →  32 字节随机数                 ║
║           ↓                                           ║
║  公钥 (Public Key)  →  从私钥推导 (33 字节压缩格式)  ║
║                                                       ║
║  特点:                                                ║
║  • 小密钥尺寸 (256-bit)                              ║
║  • 高安全性 (等效 RSA 3072-bit)                      ║
║  • 快速运算                                          ║
║  • 非对称加密                                        ║
║                                                       ║
╚═══════════════════════════════════════════════════════╝
```

**代码实现**:
```rust
// 文件: crates/services/ks/src/storage.rs:196
let (secret_key, public_key) = ecies::utils::generate_keypair();
```

### 2. 密钥 ID (key_id)

```
┌─────────────────────────────────────────────────┐
│  key_id 的作用                                  │
├─────────────────────────────────────────────────┤
│                                                 │
│  1. 唯一标识符 → 每个密钥对的唯一 ID          │
│  2. 自动递增   → SQLite AUTOINCREMENT          │
│  3. 查询索引   → 快速检索密钥                  │
│  4. 版本控制   → 支持密钥轮转                  │
│                                                 │
│  示例:                                          │
│  key_id=1  →  第一次生成的密钥对               │
│  key_id=2  →  第二次生成的密钥对               │
│  key_id=N  →  第 N 次生成的密钥对              │
│                                                 │
└─────────────────────────────────────────────────┘
```

### 3. 密钥生命周期

```
密钥生命周期状态机
═════════════════════════════════════════════════

   创建 (Created)
        │
        │ generate_and_store_key()
        ↓
   活跃 (Active)  ←─────────┐
        │                   │
        │ 时间推移          │ 查询刷新
        │                   │
        ↓                   │
   过期 (Expired)           │
        │                   │
        │ cleanup_expired_keys()
        ↓
   删除 (Deleted)
        │
        ↓
   [已清除]


时间线示例 (TTL = 3600 秒):
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 t=0        t=3600         t=7200
  │           │              │
  │  活跃期   │   过期期     │
  │←────────→│←───────────→│
  │           │              │
创建         过期           清理
```

---

## 系统架构

### 整体架构图

```
┌─────────────────────────────────────────────────────────────────┐
│                        Actrix System                            │
│                                                                 │
│  ┌──────────────────┐         ┌──────────────────┐            │
│  │  其他服务         │         │   KS 客户端       │            │
│  │  (AIS, 验证服务)  │  HTTP   │   (Client)       │            │
│  │                  │◄────────┤                  │            │
│  └────────┬─────────┘  HTTPS  └────────┬─────────┘            │
│           │                            │                       │
│           │ POST /ks/generate          │                       │
│           │ GET  /ks/secret/{id}       │                       │
│           │ GET  /ks/health            │                       │
│           ↓                            ↓                       │
│  ┌─────────────────────────────────────────────────────┐      │
│  │           KS HTTP API (Axum Router)                 │      │
│  │  ┌────────────────────────────────────────────┐     │      │
│  │  │  handlers.rs                               │     │      │
│  │  │  • generate_key_handler()                  │     │      │
│  │  │  • get_secret_key_handler()                │     │      │
│  │  │  • health_check_handler()                  │     │      │
│  │  └────────────────┬───────────────────────────┘     │      │
│  │                   │                                 │      │
│  │                   ↓                                 │      │
│  │  ┌────────────────────────────────────────────┐     │      │
│  │  │  KSState (服务状态)                        │     │      │
│  │  │  • storage: KeyStorage                     │     │      │
│  │  │  • nonce_storage: SqliteNonceStorage       │     │      │
│  │  │  • psk: actrix_shared_key                   │     │      │
│  │  └────────────────┬───────────────────────────┘     │      │
│  └───────────────────┼─────────────────────────────────┘      │
│                      │                                         │
│         ┌────────────┴────────────┐                            │
│         ↓                         ↓                            │
│  ┌─────────────────┐    ┌─────────────────────┐               │
│  │  KeyStorage     │    │ SqliteNonceStorage  │               │
│  │  (storage.rs)   │    │ (nonce_storage.rs)  │               │
│  └────────┬────────┘    └────────┬────────────┘               │
│           │                      │                             │
│           ↓                      ↓                             │
│  ┌─────────────────────────────────────────────┐               │
│  │         SQLite Database                     │               │
│  │  ┌──────────────┐  ┌──────────────────┐    │               │
│  │  │  keys 表     │  │  nonce 表        │    │               │
│  │  │              │  │                  │    │               │
│  │  │ • key_id     │  │ • nonce          │    │               │
│  │  │ • public_key │  │ • timestamp      │    │               │
│  │  │ • secret_key │  │ • expiry_time    │    │               │
│  │  │ • created_at │  │                  │    │               │
│  │  │ • expires_at │  │                  │    │               │
│  │  └──────────────┘  └──────────────────┘    │               │
│  │                                             │               │
│  │  文件: /var/lib/actrix/ks.db               │               │
│  └─────────────────────────────────────────────┘               │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 模块依赖关系

```
┌────────────────────────────────────────────────┐
│           crates/services/ks/                           │
├────────────────────────────────────────────────┤
│                                                │
│  lib.rs (公共接口)                             │
│    │                                           │
│    ├─► handlers.rs (HTTP 处理器)              │
│    │     ├─► KSState                          │
│    │     ├─► generate_key_handler()           │
│    │     ├─► get_secret_key_handler()         │
│    │     └─► health_check_handler()           │
│    │                                           │
│    ├─► storage.rs (存储层)                     │
│    │     ├─► KeyStorage                       │
│    │     ├─► generate_and_store_key()         │
│    │     ├─► get_secret_key()                 │
│    │     └─► cleanup_expired_keys()           │
│    │                                           │
│    ├─► nonce_storage.rs (Nonce 存储)          │
│    │     └─► SqliteNonceStorage               │
│    │                                           │
│    ├─► client.rs (KS 客户端)                  │
│    │     ├─► Client                           │
│    │     └─► fetch_secret_key()               │
│    │                                           │
│    ├─► types.rs (数据类型)                     │
│    │     ├─► GenerateKeyRequest/Response      │
│    │     ├─► GetSecretKeyRequest/Response     │
│    │     └─► KeyPair, KeyRecord               │
│    │                                           │
│    ├─► config.rs (配置)                        │
│    │     └─► KeyServerConfig                  │
│    │                                           │
│    └─► error.rs (错误类型)                     │
│          └─► KsError                           │
│                                                │
└────────────────────────────────────────────────┘

外部依赖:
  • ecies v0.2.9         → 椭圆曲线加密
  • rusqlite v0.35.0     → SQLite 数据库
  • nonce-auth v0.6.1    → Nonce 认证
  • axum v0.8.0          → Web 框架
  • reqwest v0.12.0      → HTTP 客户端
```

---

## 工作流程

### 1. 密钥生成流程

```
客户端请求生成密钥
         │
         ↓
┌─────────────────────────────────────────────────────────┐
│ 步骤 1: 构建请求                                        │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  GenerateKeyRequest {                                   │
│      credential: NonceCredential {                      │
│          nonce: "uuid-v4",                              │
│          timestamp: 1730611200,                         │
│          signature: "hmac-sha256..."                    │
│      }                                                  │
│  }                                                      │
│                                                         │
└──────────────────┬──────────────────────────────────────┘
                   │
                   ↓
         POST /ks/generate
                   │
                   ↓
┌─────────────────────────────────────────────────────────┐
│ 步骤 2: 服务端验证 (handlers.rs:106-129)               │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  2.1 验证 Nonce (防重放)                                │
│       ├─ 检查 nonce 是否已使用                         │
│       └─ 检查 timestamp 是否在窗口内 (±300秒)          │
│                                                         │
│  2.2 验证签名                                           │
│       ├─ 使用 PSK 重新计算签名                         │
│       ├─ 比对签名是否匹配                              │
│       └─ 签名 payload: "generate_key"                  │
│                                                         │
│  2.3 标记 nonce 为已使用                                │
│                                                         │
└──────────────────┬──────────────────────────────────────┘
                   │
                   ↓ (验证通过)
┌─────────────────────────────────────────────────────────┐
│ 步骤 3: 生成密钥对 (storage.rs:194-213)                │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  3.1 生成 ECIES 密钥对                                  │
│       ┌──────────────────────────────┐                 │
│       │ ecies::utils::generate_keypair()              │
│       │   ↓                          │                 │
│       │ secret_key: [u8; 32]         │                 │
│       │ public_key: [u8; 33]         │                 │
│       └──────────────────────────────┘                 │
│                                                         │
│  3.2 Base64 编码                                        │
│       secret_key_b64 = BASE64(secret_key)              │
│       public_key_b64 = BASE64(public_key)              │
│                                                         │
│  3.3 存储到数据库                                       │
│       INSERT INTO keys VALUES (                        │
│           key_id: AUTOINCREMENT,                       │
│           public_key: public_key_b64,                  │
│           secret_key: secret_key_b64,                  │
│           created_at: now,                             │
│           expires_at: now + TTL                        │
│       )                                                │
│                                                         │
│  3.4 获取自动生成的 key_id                             │
│       key_id = last_insert_rowid()                     │
│                                                         │
└──────────────────┬──────────────────────────────────────┘
                   │
                   ↓
┌─────────────────────────────────────────────────────────┐
│ 步骤 4: 返回响应                                        │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  GenerateKeyResponse {                                  │
│      key_id: 123,                                       │
│      public_key: "BHxN7Q8vK...",  // Base64            │
│      expires_at: 1730614800       // Unix 时间戳       │
│  }                                                      │
│                                                         │
│  注意: 不返回 secret_key!                              │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

**时序图**:

```
客户端           KS API          KeyStorage      SQLite
  │                │                 │              │
  │  POST /generate│                 │              │
  ├───────────────>│                 │              │
  │                │                 │              │
  │                │ verify_nonce()  │              │
  │                ├────────────────>│              │
  │                │      OK         │              │
  │                │<────────────────┤              │
  │                │                 │              │
  │                │ generate_key()  │              │
  │                ├────────────────>│              │
  │                │                 │  INSERT      │
  │                │                 ├─────────────>│
  │                │                 │  key_id=123  │
  │                │                 │<─────────────┤
  │                │   KeyPair       │              │
  │                │<────────────────┤              │
  │                │                 │              │
  │   Response     │                 │              │
  │<───────────────┤                 │              │
  │                │                 │              │
```

### 2. 密钥查询流程

```
验证服务请求私钥
         │
         ↓
┌─────────────────────────────────────────────────────────┐
│ 步骤 1: 构建请求                                        │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  GetSecretKeyRequest {                                  │
│      key_id: 123,                                       │
│      credential: NonceCredential {                      │
│          nonce: "uuid-v4",                              │
│          timestamp: 1730612000,                         │
│          signature: "hmac-sha256..."                    │
│      }                                                  │
│  }                                                      │
│                                                         │
│  签名 payload: "get_secret_key:123"                     │
│                                                         │
└──────────────────┬──────────────────────────────────────┘
                   │
                   ↓
     GET /ks/secret/123?credential={...}
                   │
                   ↓
┌─────────────────────────────────────────────────────────┐
│ 步骤 2: 验证请求 (handlers.rs:132-168)                 │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  2.1 验证路径参数和查询参数一致                         │
│       if path_key_id != query_key_id:                  │
│           return 400 Bad Request                       │
│                                                         │
│  2.2 验证 Nonce + 签名                                  │
│       (同生成流程)                                      │
│                                                         │
└──────────────────┬──────────────────────────────────────┘
                   │
                   ↓ (验证通过)
┌─────────────────────────────────────────────────────────┐
│ 步骤 3: 查询密钥 (storage.rs:134-158)                  │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  3.1 触发清理检查                                       │
│       if now - last_cleanup > 3600:                    │
│           cleanup_expired_keys()                       │
│                                                         │
│  3.2 从数据库查询                                       │
│       SELECT secret_key FROM keys                      │
│       WHERE key_id = 123                               │
│                                                         │
│  3.3 检查结果                                           │
│       ┌─ 找到密钥 → 返回 secret_key                    │
│       └─ 未找到   → 返回 404 Not Found                 │
│                                                         │
└──────────────────┬──────────────────────────────────────┘
                   │
                   ↓ (找到密钥)
┌─────────────────────────────────────────────────────────┐
│ 步骤 4: 返回响应                                        │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  GetSecretKeyResponse {                                 │
│      key_id: 123,                                       │
│      secret_key: "VxPz9m2...",  // Base64              │
│      expires_at: 1730614800                            │
│  }                                                      │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

### 3. 密钥过期和清理流程

```
自动清理机制 (storage.rs:249-275)
═══════════════════════════════════════════════════

┌─────────────────────────────────────────┐
│  触发条件                               │
├─────────────────────────────────────────┤
│                                         │
│  1. 每次 get_secret_key() 调用时       │
│  2. 距离上次清理 >= 1 小时 (3600秒)    │
│                                         │
└────────────────┬────────────────────────┘
                 │
                 ↓
┌─────────────────────────────────────────┐
│  清理过程                               │
├─────────────────────────────────────────┤
│                                         │
│  DELETE FROM keys                       │
│  WHERE expires_at > 0                   │
│    AND expires_at < now                 │
│                                         │
│  返回: 删除的密钥数量                  │
│                                         │
└────────────────┬────────────────────────┘
                 │
                 ↓
┌─────────────────────────────────────────┐
│  更新清理时间戳                         │
├─────────────────────────────────────────┤
│                                         │
│  last_cleanup_time = now                │
│                                         │
└─────────────────────────────────────────┘


时间线示例:
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
t=0       t=3600      t=7200      t=10800
 │          │           │            │
 │          │           │            │
 │  密钥1   │   过期    │   清理     │
 ├─────────>│──────────>│───────────>│
 │          │           │            │
 │          │  密钥2    │   过期     │   清理
 │          ├──────────>│───────────>│────────>
 │          │           │            │
 │          │           │  密钥3     │   过期
 │          │           ├───────────>│────────>
 │          │           │            │

清理触发点:
  ✓ 每小时触发一次清理
  ✓ 删除所有已过期的密钥
  ✓ TTL=0 的密钥永不过期
```

---

## API 详解

### API 端点总览

```
┌────────────────────────────────────────────────────────┐
│  KS API Endpoints                                      │
├────────────────────────────────────────────────────────┤
│                                                        │
│  POST   /ks/generate                                   │
│         生成新的密钥对                                 │
│                                                        │
│  GET    /ks/secret/{key_id}                            │
│         获取指定密钥的私钥                             │
│                                                        │
│  GET    /ks/health                                     │
│         健康检查                                       │
│                                                        │
└────────────────────────────────────────────────────────┘

所有 API 都挂载在 /ks 路由前缀下
完整路径示例: https://actrix.example.com/ks/generate
```

### 1. 生成密钥对 API

**端点**: `POST /ks/generate`

**请求**:
```http
POST /ks/generate HTTP/1.1
Host: actrix.example.com
Content-Type: application/json

{
  "credential": {
    "nonce": "550e8400-e29b-41d4-a716-446655440000",
    "timestamp": 1730611200,
    "signature": "qMZ8vL3xYH2k..."
  }
}
```

**请求结构**:
```rust
// 文件: crates/services/ks/src/types.rs:18-22
pub struct GenerateKeyRequest {
    pub credential: NonceCredential,
}

// NonceCredential 来自 nonce-auth 库
pub struct NonceCredential {
    pub nonce: String,        // UUID v4
    pub timestamp: i64,       // Unix 时间戳
    pub signature: String,    // HMAC-SHA256 签名
}
```

**签名计算**:
```rust
// Payload 固定为 "generate_key"
let payload = "generate_key";
let signature = HMAC-SHA256(psk, nonce + timestamp + payload);
```

**响应 200 OK**:
```json
{
  "key_id": 123,
  "public_key": "BHxN7Q8vK9m2...",
  "expires_at": 1730614800
}
```
```json
{
  "key_id": 123,
  "public_key": "BHxN7Q8vK9m2...",
  "expires_at": 1730614800,
  "tolerance_seconds": 3600
}
```

**响应结构**:
```rust
// 文件: crates/services/ks/src/types.rs:25-33
pub struct GenerateKeyResponse {
    pub key_id: u32,
    pub public_key: String,      // Base64 编码
    pub expires_at: u64,         // Unix 时间戳
  pub tolerance_seconds: u64,  // 容忍期（秒）
}
```

**错误响应**:

| 状态码 | 错误                     | 原因                 |
| ------ | ------------------------ | -------------------- |
| 400    | `Invalid request`        | 请求格式错误         |
| 401    | `Invalid signature`      | PSK 错误或签名不匹配 |
| 403    | `Nonce already used`     | 重放攻击检测         |
| 403    | `Timestamp out of range` | 时间戳超出窗口       |
| 500    | `Database error`         | 数据库操作失败       |

**curl 示例**:
```bash
# 1. 生成签名 (伪代码)
NONCE=$(uuidgen)
TIMESTAMP=$(date +%s)
PAYLOAD="generate_key"
SIGNATURE=$(echo -n "${NONCE}${TIMESTAMP}${PAYLOAD}" | \
            openssl dgst -sha256 -hmac "your-psk" -binary | base64)

# 2. 发送请求
curl -X POST https://actrix.example.com/ks/generate \
  -H "Content-Type: application/json" \
  -d "{
    \"credential\": {
      \"nonce\": \"${NONCE}\",
      \"timestamp\": ${TIMESTAMP},
      \"signature\": \"${SIGNATURE}\"
    }
  }"
```

### 2. 获取私钥 API

**端点**: `GET /ks/secret/{key_id}`

**请求**:
```http
GET /ks/secret/123?key_id=123&credential=%7B%22nonce%22%3A%22...%22%7D HTTP/1.1
Host: actrix.example.com
```

**URL 参数**:
- `{key_id}` (路径参数): 要查询的密钥 ID
- `key_id` (查询参数): 必须与路径参数相同
- `credential` (查询参数): URL 编码的 JSON 凭证

**请求结构**:
```rust
// 文件: crates/services/ks/src/types.rs:36-42
pub struct GetSecretKeyRequest {
    pub key_id: u32,
    pub credential: NonceCredential,
}
```

**签名 Payload**:
```rust
// Payload 包含 key_id
let payload = format!("get_secret_key:{}", key_id);
let signature = HMAC-SHA256(psk, nonce + timestamp + payload);
```

**响应 200 OK**:
```json
{
  "key_id": 123,
  "secret_key": "VxPz9m2nLw8...",
  "expires_at": 1730614800
}
```

**响应结构**:
```rust
// 文件: crates/services/ks/src/types.rs:45-53
pub struct GetSecretKeyResponse {
    pub key_id: u32,
    pub secret_key: String,      // Base64 编码
    pub expires_at: u64,
}
```

**错误响应**:

| 状态码 | 错误                 | 原因                 |
| ------ | -------------------- | -------------------- |
| 400    | `key_id mismatch`    | 路径和查询参数不匹配 |
| 401    | `Invalid signature`  | 认证失败             |
| 403    | `Nonce already used` | 重放攻击             |
| 404    | `Key not found`      | 密钥不存在或已过期   |
| 500    | `Database error`     | 数据库错误           |

**curl 示例**:
```bash
KEY_ID=123
NONCE=$(uuidgen)
TIMESTAMP=$(date +%s)
PAYLOAD="get_secret_key:${KEY_ID}"
SIGNATURE=$(echo -n "${NONCE}${TIMESTAMP}${PAYLOAD}" | \
            openssl dgst -sha256 -hmac "your-psk" -binary | base64)

CREDENTIAL=$(cat <<EOF | jq -c '.' | jq -sRr @uri
{
  "nonce": "${NONCE}",
  "timestamp": ${TIMESTAMP},
  "signature": "${SIGNATURE}"
}
EOF
)

curl "https://actrix.example.com/ks/secret/${KEY_ID}?key_id=${KEY_ID}&credential=${CREDENTIAL}"
```

### 3. 健康检查 API

**端点**: `GET /ks/health`

**请求**:
```http
GET /ks/health HTTP/1.1
Host: actrix.example.com
```

**特点**:
- ❌ 不需要认证
- ✅ 快速响应
- ✅ 包含服务统计信息

**响应 200 OK**:
```json
{
  "status": "healthy",
  "service": "ks",
  "key_count": 42,
  "timestamp": 1730611200
}
```

**字段说明**:
- `status`: 服务状态 (`healthy` / `unhealthy`)
- `service`: 服务名称 (`ks`)
- `key_count`: 当前存储的密钥总数
- `timestamp`: 当前服务器时间戳

**使用场景**:
```bash
# 监控脚本
while true; do
  STATUS=$(curl -s http://localhost:8443/ks/health | jq -r '.status')
  if [ "$STATUS" != "healthy" ]; then
    echo "KS service unhealthy!" | mail -s "Alert" admin@example.com
  fi
  sleep 60
done
```

---

## 安全机制

### 1. PSK (Pre-Shared Key) 认证

```
PSK 认证架构
═══════════════════════════════════════════════════

┌─────────────────────────────────────────────────┐
│  配置文件 (config.toml)                         │
├─────────────────────────────────────────────────┤
│                                                 │
│  actrix_shared_key = "your-strong-key-here"      │
│                                                 │
│  ⚠️  所有内部服务共享此密钥                     │
│  ⚠️  必须在部署前更改默认值                     │
│  ⚠️  密钥长度建议 ≥ 32 字符                     │
│                                                 │
└─────────────────────────────────────────────────┘
         │
         ↓ 加载到内存
┌─────────────────────────────────────────────────┐
│  KSState                                        │
├─────────────────────────────────────────────────┤
│                                                 │
│  pub psk: String                                │
│                                                 │
└────────────┬────────────────────────────────────┘
             │
             ↓ 用于验证
┌──────────────────────────────────────────────────┐
│  CredentialVerifier::verify()                   │
├──────────────────────────────────────────────────┤
│                                                  │
│  1. 提取请求中的 signature                      │
│  2. 使用 psk 重新计算签名                       │
│  3. 比对两个签名是否一致                        │
│                                                  │
│  计算公式:                                       │
│  signature = HMAC-SHA256(psk, data)             │
│  data = nonce + timestamp + payload             │
│                                                  │
└──────────────────────────────────────────────────┘
```

**代码实现**:
```rust
// 文件: crates/services/ks/src/handlers.rs:35-57
pub async fn verify_credential(
    &self,
    credential: &nonce_auth::NonceCredential,
    request_payload: &str,
) -> Result<(), KsError> {
    let verify_result = CredentialVerifier::new(self.nonce_storage.clone())
        .with_secret(self.psk.as_bytes())  // ← PSK
        .verify(credential, request_payload.as_bytes())
        .await;

    // 错误处理...
}
```

### 2. Nonce 防重放攻击

```
Nonce 防重放机制工作原理
═══════════════════════════════════════════════════

┌─────────────────────────────────────────────────┐
│  第一次请求                                     │
├─────────────────────────────────────────────────┤
│                                                 │
│  1. 客户端生成唯一 nonce                       │
│     nonce = "550e8400-e29b-41d4-a716-..."      │
│                                                 │
│  2. 服务器检查 nonce 是否存在                  │
│     SELECT * FROM nonce WHERE nonce = ?        │
│     结果: 不存在 ✓                             │
│                                                 │
│  3. 服务器标记 nonce 为已使用                  │
│     INSERT INTO nonce VALUES (nonce, ...)      │
│                                                 │
│  4. 处理请求                                    │
│                                                 │
└─────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────┐
│  重放攻击尝试 (使用相同 nonce)                 │
├─────────────────────────────────────────────────┤
│                                                 │
│  1. 攻击者重放相同请求                         │
│     nonce = "550e8400-e29b-41d4-a716-..."      │
│                                                 │
│  2. 服务器检查 nonce                            │
│     SELECT * FROM nonce WHERE nonce = ?        │
│     结果: 已存在 ✗                             │
│                                                 │
│  3. 拒绝请求                                    │
│     返回 403 Forbidden                         │
│     错误: "Nonce already used"                 │
│                                                 │
└─────────────────────────────────────────────────┘


Nonce 表结构:
┌──────────────────────────────────────────┐
│  nonce 表                                │
├──────────────────────────────────────────┤
│  nonce        TEXT PRIMARY KEY           │
│  timestamp    INTEGER NOT NULL           │
│  expiry_time  INTEGER NOT NULL           │
└──────────────────────────────────────────┘

清理策略:
  • 定期清理过期 nonce (expiry_time < now)
  • 默认有效期: 300 秒 (5 分钟)
```

**代码实现**:
```rust
// 文件: crates/services/ks/src/nonce_storage.rs
impl StorageBackend for SqliteNonceStorage {
    async fn store_nonce(&self, nonce: &str, timestamp: i64)
        -> Result<(), NonceError> {
        // 存储 nonce
    }

    async fn check_nonce(&self, nonce: &str)
        -> Result<bool, NonceError> {
        // 检查 nonce 是否已存在
    }

    async fn cleanup_expired(&self, before_timestamp: i64)
        -> Result<(), NonceError> {
        // 清理过期 nonce
    }
}
```

### 3. 时间戳验证

```
时间窗口验证
═══════════════════════════════════════════════════

允许的时间窗口: ±300 秒 (5 分钟)

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    t-300        t-0         t+300
      │           │            │
      │←──────────┼──────────→│
      │   有效窗口 │            │
      │           │            │
      │           │            │
   ✗ 太旧       当前时间     ✗ 太新


示例:
  当前时间: 1730611200

  ✓ 有效请求:
    timestamp = 1730610900  (now - 300)
    timestamp = 1730611200  (now)
    timestamp = 1730611500  (now + 300)

  ✗ 无效请求:
    timestamp = 1730610800  (now - 400, 太旧)
    timestamp = 1730611700  (now + 500, 太新)


拒绝原因:
  • 防止重放过期请求
  • 防止时钟不同步攻击
  • 限制 nonce 存储时间
```

**实现细节**:
```rust
// nonce-auth 库内部实现
const TIME_WINDOW: i64 = 300;  // 秒

fn validate_timestamp(request_ts: i64, server_ts: i64) -> bool {
    let diff = (request_ts - server_ts).abs();
    diff <= TIME_WINDOW
}
```

### 4. 完整认证流程

```
完整认证流程图
═══════════════════════════════════════════════════

客户端                           KS 服务器
  │                                  │
  │ 1. 准备请求数据                  │
  │    • nonce = UUID v4            │
  │    • timestamp = now()          │
  │    • payload = "generate_key"   │
  │                                  │
  │ 2. 计算签名                      │
  │    data = nonce + ts + payload  │
  │    sig = HMAC-SHA256(psk, data) │
  │                                  │
  │ 3. 构建凭证                      │
  │    credential = {               │
  │      nonce, timestamp, sig      │
  │    }                             │
  │                                  │
  │ POST /ks/generate                │
  ├─────────────────────────────────>│
  │                                  │
  │                                  │ 4. 验证时间戳
  │                                  │    if |ts - now| > 300:
  │                                  │      return 403
  │                                  │
  │                                  │ 5. 检查 nonce
  │                                  │    if nonce_exists:
  │                                  │      return 403
  │                                  │
  │                                  │ 6. 验证签名
  │                                  │    data = nonce + ts + payload
  │                                  │    sig' = HMAC(psk, data)
  │                                  │    if sig != sig':
  │                                  │      return 401
  │                                  │
  │                                  │ 7. 标记 nonce 已使用
  │                                  │    INSERT INTO nonce...
  │                                  │
  │                                  │ 8. 处理业务逻辑
  │                                  │    generate_key()
  │                                  │
  │         Response                 │
  │<─────────────────────────────────┤
  │                                  │
```

### 5. 安全最佳实践

```
┌─────────────────────────────────────────────────┐
│  部署前安全检查清单                             │
├─────────────────────────────────────────────────┤
│                                                 │
│  ✅ 更改默认 PSK                                │
│     • 长度 ≥ 32 字符                           │
│     • 使用强随机生成器                         │
│     • 定期轮转 (建议每 90 天)                  │
│                                                 │
│  ✅ 文件权限                                    │
│     • config.toml: 600 (rw-------)             │
│     • *.db: 600                                │
│     • 目录: 700                                │
│                                                 │
│  ✅ 网络隔离                                    │
│     • 仅内网访问                               │
│     • 不暴露到公网                             │
│     • 使用防火墙规则                           │
│                                                 │
│  ✅ HTTPS/TLS                                   │
│     • 生产环境强制 HTTPS                       │
│     • 使用有效证书                             │
│     • TLS 1.2+                                 │
│                                                 │
│  ✅ 日志和审计                                  │
│     • 记录所有密钥访问                         │
│     • 监控异常请求                             │
│     • 定期审查日志                             │
│                                                 │
└─────────────────────────────────────────────────┘
```

---

## 数据存储

### 数据库表结构

```sql
-- 密钥表
CREATE TABLE keys (
    key_id INTEGER PRIMARY KEY AUTOINCREMENT,
    public_key TEXT NOT NULL,
    secret_key TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);

-- 索引
CREATE INDEX idx_keys_expires_at ON keys(expires_at);

-- Nonce 表 (防重放)
CREATE TABLE nonce (
    nonce TEXT PRIMARY KEY,
    timestamp INTEGER NOT NULL,
    expiry_time INTEGER NOT NULL
);
```

### 字段说明

**keys 表**:

| 字段         | 类型    | 说明                   | 示例           |
| ------------ | ------- | ---------------------- | -------------- |
| `key_id`     | INTEGER | 自动递增主键           | 1, 2, 3...     |
| `public_key` | TEXT    | Base64 编码的公钥      | "BHxN7Q8vK..." |
| `secret_key` | TEXT    | Base64 编码的私钥      | "VxPz9m2nL..." |
| `created_at` | INTEGER | 创建时间 (Unix 时间戳) | 1730611200     |
| `expires_at` | INTEGER | 过期时间 (0=永不过期)  | 1730614800     |

**存储格式**:
```
公钥长度: 33 字节 → Base64 编码后 44 字符
私钥长度: 32 字节 → Base64 编码后 44 字符
```

### 数据存储流程

```
写入流程 (storage.rs:85-107)
═══════════════════════════════════════════════

┌────────────────────────────────────────────┐
│  1. 生成椭圆曲线密钥对                     │
│     (secret_key, public_key)               │
│                                            │
│  2. Base64 编码                            │
│     secret_key_b64 = BASE64(secret_key)    │
│     public_key_b64 = BASE64(public_key)    │
│                                            │
│  3. 计算过期时间                           │
│     if key_ttl_seconds == 0:              │
│         expires_at = 0  (永不过期)        │
│     else:                                  │
│         expires_at = now + key_ttl_seconds│
│                                            │
│  4. 插入数据库                             │
│     INSERT INTO keys VALUES (             │
│         NULL,  -- key_id 自动生成         │
│         public_key_b64,                    │
│         secret_key_b64,                    │
│         now,                               │
│         expires_at                         │
│     )                                      │
│                                            │
│  5. 获取自动生成的 key_id                  │
│     key_id = last_insert_rowid()          │
│                                            │
└────────────────────────────────────────────┘


读取流程 (storage.rs:134-158)
═══════════════════════════════════════════════

┌────────────────────────────────────────────┐
│  1. 触发清理检查                           │
│     maybe_cleanup()                        │
│                                            │
│  2. 从数据库查询                           │
│     SELECT secret_key                      │
│     FROM keys                              │
│     WHERE key_id = ?                       │
│                                            │
│  3. 检查结果                               │
│     • 找到 → 返回 secret_key_b64          │
│     • 未找到 → 返回 None                  │
│                                            │
└────────────────────────────────────────────┘
```

### 存储优化

```
优化措施
═══════════════════════════════════════════════

✅ 索引优化
   CREATE INDEX idx_keys_expires_at ON keys(expires_at)

   用途:
   • 加速过期密钥查询
   • 提升清理操作性能

✅ 自动递增
   key_id INTEGER PRIMARY KEY AUTOINCREMENT

   优点:
   • 无需手动管理 ID
   • 避免 ID 冲突
   • 保证全局唯一

✅ 定期清理
   每小时触发一次清理

   DELETE FROM keys
   WHERE expires_at > 0 AND expires_at < now

   效果:
   • 控制数据库大小
   • 提升查询性能
   • 释放存储空间

⚠️  安全考虑
   私钥存储: Base64 编码 (未加密)

   缓解措施:
   • 文件权限 600
   • 仅内网访问
   • 定期审计

   未来改进:
   • 使用操作系统密钥环
   • 集成 KMS (Key Management Service)
   • 实现密钥分片存储
```

### 数据库文件位置

```
默认位置:
  {sqlite_path}/ks_keys.db (通过 StorageConfig 配置)

配置方式:
  [services.ks.storage.sqlite]
  path = "ks_keys.db"  # 相对于 sqlite_path 目录，或使用绝对路径

查看数据库信息:
  sqlite3 {sqlite_path}/ks_keys.db "
    SELECT
      COUNT(*) as total_keys,
      SUM(CASE WHEN expires_at = 0 THEN 1 ELSE 0 END) as permanent_keys,
      SUM(CASE WHEN expires_at > 0 AND expires_at > strftime('%s', 'now')
               THEN 1 ELSE 0 END) as active_keys,
      SUM(CASE WHEN expires_at > 0 AND expires_at < strftime('%s', 'now')
               THEN 1 ELSE 0 END) as expired_keys
    FROM keys;
  "
```

---

## 客户端集成

### 1. Rust 客户端

**注意**: `Client` 和 `ClientConfig` (HTTP 客户端) 仅在测试模式下可用 (`#[cfg(test)]`)。  
生产代码应使用 `GrpcClient` 和 `GrpcClientConfig` (gRPC 客户端)。

**测试代码示例**:
```rust
#[cfg(test)]
use ks::{Client, ClientConfig};

let config = ClientConfig {
    endpoint: "https://ks.example.com".to_string(),
    psk: actrix_shared_key.clone(),  // 从全局配置获取
    timeout_seconds: 30,
    cache_db_path: None,
};

let client = Client::new(&config);
```

**获取私钥**:
```rust
// 文件: crates/services/ks/src/client.rs:67-118
use ecies::SecretKey;

let key_id = 123;
let (secret_key, expires_at) = client.fetch_secret_key(key_id).await?;

// secret_key 是 ecies::SecretKey 类型，可直接用于解密
// expires_at 是 Unix 时间戳
```

**完整示例**:
```rust
use ks::{Client, ClientConfig};
use ecies::decrypt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. 创建客户端
    let actrix_shared_key = "your-actrix-shared-key";  // 从全局配置获取
    let client = Client::new(&ClientConfig {
        endpoint: "https://ks.example.com".to_string(),
        psk: actrix_shared_key.to_string(),
        timeout_seconds: 30,
        cache_db_path: None,
    });

    // 2. 获取私钥
    let key_id = 123;
    let (secret_key, expires_at) = client.fetch_secret_key(key_id).await?;

    println!("Got secret key, expires at: {}", expires_at);

    // 3. 使用私钥解密
    let encrypted_data = vec![/* ... */];
    let decrypted = decrypt(
        &secret_key.serialize(),
        &encrypted_data
    )?;

    println!("Decrypted: {:?}", decrypted);

    Ok(())
}
```

### 2. HTTP 客户端 (任意语言)

**Python 示例**:
```python
import requests
import hmac
import hashlib
import base64
import time
import uuid
import json

class KSClient:
    def __init__(self, endpoint: str, psk: str):
        self.endpoint = endpoint
        self.psk = psk.encode('utf-8')

    def _create_credential(self, payload: str) -> dict:
        """创建认证凭证"""
        nonce = str(uuid.uuid4())
        timestamp = int(time.time())

        # 计算签名
        data = f"{nonce}{timestamp}{payload}".encode('utf-8')
        signature = base64.b64encode(
            hmac.new(self.psk, data, hashlib.sha256).digest()
        ).decode('utf-8')

        return {
            "nonce": nonce,
            "timestamp": timestamp,
            "signature": signature
        }

    def generate_key(self) -> dict:
        """生成新密钥对"""
        url = f"{self.endpoint}/generate"
        credential = self._create_credential("generate_key")

        response = requests.post(url, json={
            "credential": credential
        })
        response.raise_for_status()
        return response.json()

    def get_secret_key(self, key_id: int) -> dict:
        """获取私钥"""
        url = f"{self.endpoint}/secret/{key_id}"
        payload = f"get_secret_key:{key_id}"
        credential = self._create_credential(payload)

        params = {
            "key_id": key_id,
            "credential": json.dumps(credential)
        }

        response = requests.get(url, params=params)
        response.raise_for_status()
        return response.json()

# 使用示例
client = KSClient("https://ks.example.com/ks", "your-psk")

# 生成密钥
result = client.generate_key()
print(f"Generated key_id: {result['key_id']}")

# 获取私钥
secret = client.get_secret_key(result['key_id'])
print(f"Secret key: {secret['secret_key']}")
```

**JavaScript/Node.js 示例**:
```javascript
const crypto = require('crypto');
const axios = require('axios');
const { v4: uuidv4 } = require('uuid');

class KSClient {
    constructor(endpoint, psk) {
        this.endpoint = endpoint;
        this.psk = psk;
    }

    createCredential(payload) {
        const nonce = uuidv4();
        const timestamp = Math.floor(Date.now() / 1000);

        // 计算签名
        const data = `${nonce}${timestamp}${payload}`;
        const signature = crypto
            .createHmac('sha256', this.psk)
            .update(data)
            .digest('base64');

        return { nonce, timestamp, signature };
    }

    async generateKey() {
        const url = `${this.endpoint}/generate`;
        const credential = this.createCredential('generate_key');

        const response = await axios.post(url, { credential });
        return response.data;
    }

    async getSecretKey(keyId) {
        const url = `${this.endpoint}/secret/${keyId}`;
        const payload = `get_secret_key:${keyId}`;
        const credential = this.createCredential(payload);

        const response = await axios.get(url, {
            params: {
                key_id: keyId,
                credential: JSON.stringify(credential)
            }
        });
        return response.data;
    }
}

// 使用示例
(async () => {
    const client = new KSClient('https://ks.example.com/ks', 'your-psk');

    // 生成密钥
    const result = await client.generateKey();
    console.log(`Generated key_id: ${result.key_id}`);

    // 获取私钥
    const secret = await client.getSecretKey(result.key_id);
    console.log(`Secret key: ${secret.secret_key}`);
})();
```

---

## 运维指南

### 1. 部署配置

**配置文件** (`config.toml`):
```toml
# 启用 KS 服务
enable = 16  # 或包含 16 的组合，如 22 (KS + STUN + TURN)

# KS 服务配置
[services.ks]
# Note: Service enablement is controlled by the bitmask (enable field)
# Set ENABLE_KS bit (16) in the enable field to enable this service
nonce_db_file = "/var/lib/actrix/nonce.db"  # Optional: Nonce database file path

[services.ks.storage]
backend = "sqlite"
key_ttl_seconds = 3600      # 密钥 TTL (秒), 0=永不过期

[services.ks.storage.sqlite]
path = "ks_keys.db"  # Relative to sqlite_path, or use absolute path

# 全局配置
actrix_shared_key = "your-strong-key-change-me"  # ⚠️ 必须更改!
sqlite_path = "/var/lib/actrix"  # 数据库存储目录，主数据库文件为 {sqlite_path}/actrix.db

# 日志配置
[observability]
filter_level = "info"   # 可被 RUST_LOG 覆盖

[observability.log]
output = "file"
path = "/var/log/actrix"
```

### 2. 启动和停止

**systemd 服务**:
```bash
# 启动服务
sudo systemctl start actrix

# 停止服务
sudo systemctl stop actrix

# 重启服务
sudo systemctl restart actrix

# 查看状态
sudo systemctl status actrix

# 查看日志
sudo journalctl -u actrix -f
```

**手动启动**:
```bash
# 开发模式
cargo run -- --config config.toml

# 生产模式
./actrix --config /etc/actrix/config.toml
```

### 3. 监控和健康检查

**健康检查脚本**:
```bash
#!/bin/bash
# check_ks_health.sh

ENDPOINT="http://localhost:8443/ks/health"
ALERT_EMAIL="admin@example.com"

# 检查服务健康状态
RESPONSE=$(curl -s "$ENDPOINT")
STATUS=$(echo "$RESPONSE" | jq -r '.status')

if [ "$STATUS" != "healthy" ]; then
    echo "KS service unhealthy: $RESPONSE" | \
        mail -s "KS Alert" "$ALERT_EMAIL"
    exit 1
fi

# 检查密钥数量
KEY_COUNT=$(echo "$RESPONSE" | jq -r '.key_count')
echo "KS is healthy, key_count: $KEY_COUNT"

# 可选: 监控密钥数量增长
# 如果密钥数过多，可能需要清理
if [ "$KEY_COUNT" -gt 10000 ]; then
    echo "Warning: Too many keys ($KEY_COUNT)" | \
        mail -s "KS Warning" "$ALERT_EMAIL"
fi

exit 0
```

**cron 定时检查**:
```bash
# 每 5 分钟检查一次
*/5 * * * * /usr/local/bin/check_ks_health.sh
```

### 4. 数据库维护

**查看密钥统计**:
```bash
sqlite3 /var/lib/actrix/ks.db <<EOF
.mode column
.headers on

SELECT
    COUNT(*) as total_keys,
    SUM(CASE WHEN expires_at = 0 THEN 1 ELSE 0 END) as permanent,
    SUM(CASE WHEN expires_at > 0 THEN 1 ELSE 0 END) as with_ttl
FROM keys;

SELECT
    MIN(created_at) as oldest_key,
    MAX(created_at) as newest_key,
    COUNT(*) as total
FROM keys;
EOF
```

**手动清理过期密钥**:
```bash
sqlite3 /var/lib/actrix/ks.db <<EOF
-- 查看过期密钥
SELECT COUNT(*) FROM keys
WHERE expires_at > 0 AND expires_at < strftime('%s', 'now');

-- 删除过期密钥
DELETE FROM keys
WHERE expires_at > 0 AND expires_at < strftime('%s', 'now');

-- 压缩数据库
VACUUM;
EOF
```

**备份数据库**:
```bash
#!/bin/bash
# backup_ks_db.sh

BACKUP_DIR="/backup/actrix"
DB_PATH="/var/lib/actrix/ks.db"
DATE=$(date +%Y%m%d_%H%M%S)
BACKUP_FILE="$BACKUP_DIR/ks_${DATE}.db"

# 创建备份目录
mkdir -p "$BACKUP_DIR"

# 在线备份 (不锁表)
sqlite3 "$DB_PATH" ".backup $BACKUP_FILE"

# 压缩备份
gzip "$BACKUP_FILE"

# 删除 7 天前的备份
find "$BACKUP_DIR" -name "ks_*.db.gz" -mtime +7 -delete

echo "Backup completed: ${BACKUP_FILE}.gz"
```

### 5. 性能优化

**数据库优化**:
```sql
-- 分析表统计信息
ANALYZE keys;

-- 重建索引
REINDEX idx_keys_expires_at;

-- 压缩数据库 (回收空间)
VACUUM;
```

**系统配置**:
```bash
# 增加文件描述符限制
ulimit -n 65536

# 调整 SQLite 参数
export SQLITE_TMPDIR=/tmp
```

---

## 故障排查

### 常见问题

#### 1. 认证失败 (401 Unauthorized)

**症状**:
```json
{
  "error": "Invalid signature"
}
```

**可能原因**:
1. PSK 不匹配
2. 签名计算错误
3. Payload 构造错误

**排查步骤**:
```bash
# 1. 检查配置文件中的 PSK
grep actrix_shared_key /etc/actrix/config.toml

# 2. 检查客户端使用的 PSK
# 确保客户端和服务端使用相同的密钥

# 3. 启用 debug 日志查看详细信息
RUST_LOG=debug ./actrix --config config.toml

# 4. 验证签名计算
# 确保 payload 格式正确:
#   生成密钥: "generate_key"
#   获取私钥: "get_secret_key:<key_id>"
```

#### 2. 重放攻击检测 (403 Forbidden)

**症状**:
```json
{
  "error": "Nonce already used"
}
```

**可能原因**:
1. 客户端重用了 nonce
2. 请求被重放
3. Nonce 数据库损坏

**解决方案**:
```bash
# 1. 确保每次请求生成新的 nonce
# nonce 必须是全局唯一的 UUID

# 2. 检查系统时间同步
ntpdate -q pool.ntp.org

# 3. 清理 nonce 数据库 (谨慎操作)
sqlite3 /var/lib/actrix/nonce.db "DELETE FROM nonce;"
```

#### 3. 密钥不存在 (404 Not Found)

**症状**:
```json
{
  "error": "Key not found: 123"
}
```

**可能原因**:
1. key_id 不存在
2. 密钥已过期被清理
3. 数据库损坏

**排查步骤**:
```bash
# 1. 查询密钥是否存在
sqlite3 /var/lib/actrix/ks.db \
  "SELECT * FROM keys WHERE key_id = 123;"

# 2. 检查密钥是否过期
sqlite3 /var/lib/actrix/ks.db \
  "SELECT key_id, expires_at,
          strftime('%s', 'now') as now,
          CASE
            WHEN expires_at = 0 THEN 'never'
            WHEN expires_at > strftime('%s', 'now') THEN 'active'
            ELSE 'expired'
          END as status
   FROM keys WHERE key_id = 123;"

# 3. 查看所有密钥
sqlite3 /var/lib/actrix/ks.db \
  "SELECT key_id, created_at, expires_at FROM keys ORDER BY key_id;"
```

#### 4. 数据库锁定 (500 Internal Server Error)

**症状**:
```json
{
  "error": "Database error: database is locked"
}
```

**可能原因**:
1. 多个进程同时访问
2. 长时间事务未提交
3. 磁盘 I/O 性能问题

**解决方案**:
```bash
# 1. 确保只有一个 actrix 实例运行
ps aux | grep actrix
killall -9 actrix  # 如果有多个实例

# 2. 检查数据库文件权限
ls -l /var/lib/actrix/*.db

# 3. 修复数据库 (如果损坏)
sqlite3 /var/lib/actrix/ks.db "PRAGMA integrity_check;"

# 4. 备份并重建数据库
cp /var/lib/actrix/ks.db /backup/ks.db.backup
sqlite3 /var/lib/actrix/ks.db "VACUUM;"
```

### 日志分析

**查看 KS 相关日志**:
```bash
# 查看最近的错误
journalctl -u actrix | grep -i error | tail -20

# 查看 KS 密钥生成日志
journalctl -u actrix | grep "Generated key pair"

# 查看认证失败日志
journalctl -u actrix | grep "Authentication error"

# 实时监控
journalctl -u actrix -f | grep -E "KS|key_id"
```

### 性能问题

#### 密钥查询慢

**诊断**:
```bash
# 查询执行计划
sqlite3 /var/lib/actrix/ks.db <<EOF
EXPLAIN QUERY PLAN
SELECT secret_key FROM keys WHERE key_id = 123;
EOF

# 应该看到: SEARCH TABLE keys USING INTEGER PRIMARY KEY
```

**优化**:
```sql
-- 重建索引
REINDEX;

-- 分析表
ANALYZE;

-- 压缩数据库
VACUUM;
```

#### 清理操作慢

**原因**: 过期密钥太多

**解决**:
```bash
# 调整 TTL 配置
# 减小 key_ttl_seconds，避免积累过多密钥

# 手动触发清理
sqlite3 /var/lib/actrix/ks.db \
  "DELETE FROM keys WHERE expires_at > 0 AND expires_at < strftime('%s', 'now');"
```

---

## 附录

### A. 配置参数完整列表

| 参数                              | 类型   | 默认值       | 说明                                     |
| --------------------------------- | ------ | ------------ | ---------------------------------------- |
| `ks.ip`                           | String | "127.0.0.1"  | 监听地址                                 |
| `ks.port`                         | u16    | 8081         | 监听端口                                 |
| `services.ks.storage.sqlite.path` | String | "ks_keys.db" | 密钥数据库文件路径（相对于 sqlite_path） |
| `services.ks.nonce_db_file`       | String | None         | Nonce 数据库文件路径（可选）             |
| `ks.key_ttl_seconds`              | u64    | 3600         | 密钥 TTL (0=永不过期)                    |
| `actrix_shared_key`               | String | -            | PSK (必须配置)                           |

### B. 错误代码参考

| 错误代码         | HTTP 状态 | 说明         |
| ---------------- | --------- | ------------ |
| `InvalidRequest` | 400       | 请求格式错误 |
| `Authentication` | 401       | 签名验证失败 |
| `ReplayAttack`   | 403       | Nonce 已使用 |
| `KeyNotFound`    | 404       | 密钥不存在   |
| `Database`       | 500       | 数据库错误   |
| `Internal`       | 500       | 内部错误     |

### C. 相关文档

- [CRATES.md](./CRATES.md) - KS 实现细节
- [API.md](./API.md) - KS API 参考
- [CONFIGURATION.md](./CONFIGURATION.md) - 配置指南
- [SERVICES.md](./SERVICES.md) - 服务部署

### D. 文件位置索引

| 模块        | 文件路径                         | 说明          |
| ----------- | -------------------------------- | ------------- |
| 库入口      | `crates/services/ks/src/lib.rs`           | 公共接口      |
| HTTP 处理器 | `crates/services/ks/src/handlers.rs`      | API 实现      |
| 存储层      | `crates/services/ks/src/storage.rs`       | 数据库操作    |
| 客户端      | `crates/services/ks/src/client.rs`        | Rust 客户端   |
| 数据类型    | `crates/services/ks/src/types.rs`         | 请求/响应类型 |
| 错误定义    | `crates/services/ks/src/error.rs`         | 错误类型      |
| 配置        | `crates/services/ks/src/config.rs`        | 配置结构      |
| Nonce 存储  | `crates/services/ks/src/nonce_storage.rs` | 防重放实现    |

---

**文档版本**: v1.0
**最后更新**: 2025-11-03
**维护者**: Actrix Team
**反馈**: https://github.com/Actrium/actrix/issues
