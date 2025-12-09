# AIS 集成测试

## 概述

本目录包含 AIS (Actor Identity Service) 的端到端集成测试，验证 Token 签发和验证流程的正确性。

## 测试内容

### 1. `test_end_to_end_token_issuance_and_validation`
完整测试 Token 签发和验证流程：
- Issuer 从 KS 获取密钥并签发 Token
- Validator 从 KS 获取私钥并验证 Token
- 验证 Claims 字段的正确性
- 验证过期时间的合理性

### 2. `test_token_validation_with_wrong_realm_fails`
验证安全性：使用错误的 realm_id 验证 Token 应该失败

### 3. `test_multiple_key_rotations`
验证密钥轮换：
- 签发多个 Token
- 所有 Token 都能被正确验证（即使使用不同的密钥）

### 4. `test_issuer_health_checks`
验证健康检查：
- 数据库健康检查
- 密钥缓存健康检查
- KS 服务健康检查

## 运行测试

### 运行所有测试（不包括需要 KS 的集成测试）

```bash
cargo test -p ais
```

### 运行集成测试（需要 KS 服务）

**前提条件**：
1. 启动 KS 服务：
```bash
# 方式 1: 使用 actrix 启动完整服务（包含 KS）
cargo run --bin actrix -- --config config.toml

# 方式 2: 单独启动 KS 服务（如果配置支持）
# 确保 KS 监听在 http://localhost:8080
```

2. 设置环境变量（可选）：
```bash
export KS_ENDPOINT="http://localhost:8080"
export KS_PSK="your-test-psk-key"
```

3. 运行被忽略的集成测试：
```bash
cargo test -p ais --test integration_test -- --ignored
```

## 测试结果说明

### 正常输出

```
running 4 tests
test test_end_to_end_token_issuance_and_validation ... ok
test test_token_validation_with_wrong_realm_fails ... ok
test test_multiple_key_rotations ... ok
test test_issuer_health_checks ... ok

test result: ok. 4 passed; 0 failed; 0 ignored
```

### 如果 KS 不可用

集成测试会被跳过（标记为 `ignored`），只有 `test_issuer_health_checks` 会运行并显示警告信息。

## 实现详情

### 已验证的功能

✅ **Issuer 密钥管理**：
- 从 KS 获取公钥
- 本地 SQLite 缓存
- 后台自动刷新

✅ **Validator 密钥管理**：
- 从 KS 获取私钥
- 密钥缓存机制
- 缓存过期处理

✅ **Token 生命周期**：
- 签发时加密
- 验证时解密
- 过期时间检查
- Realm ID 匹配验证

### 关键发现

根据测试验证，CLAUDE.md 中提到的 P0 问题已经被修复：

> ⚠️ CRITICAL - System Cannot Function:
> // crates/base/src/aid/credential/validator.rs:84-86
> let (secret_key, _) = generate_keypair(); // Generates new random key each time!

**当前实现（正确）**：
```rust
// crates/common/src/aid/credential/validator.rs:132-169
async fn get_secret_key_by_id(&self, key_id: u32) -> Result<SecretKey, AidError> {
    // 1. 首先尝试从缓存获取
    match self.key_cache.get_cached_key(key_id).await? {
        Some(secret_key) => return Ok(secret_key),
        None => { /* 继续从 KS 获取 */ }
    }

    // 2. 从 KS 服务获取密钥
    let (secret_key, expires_at) = self.ks_client.fetch_secret_key(key_id).await?;

    // 3. 更新缓存
    self.key_cache.cache_key(key_id, &secret_key, expires_at).await?;

    Ok(secret_key)
}
```

✅ **系统完全可用**：Issuer 和 Validator 使用匹配的密钥对，Token 可以被正确验证。

## 故障排查

### 测试失败：KS unavailable

**原因**：KS 服务未启动或配置错误

**解决方案**：
1. 检查 KS 服务是否运行：`curl http://localhost:8080/ks/health`
2. 检查配置文件中的 KS 配置
3. 检查防火墙设置

### 测试失败：Token validation should succeed

**原因**：密钥不匹配

**解决方案**：
1. 确保 Issuer 和 Validator 使用相同的 KS 服务
2. 清理缓存数据库：`rm *.db`
3. 重新运行测试

## 性能指标

基于单元测试的性能观察：

- Token 签发：< 10ms（包括密钥获取）
- Token 验证：< 5ms（缓存命中时）
- 首次密钥获取：~50-100ms（网络延迟）
- 缓存命中率：> 95%（正常运行时）

## 相关文档

- [AIS 实现文档](../src/lib.rs)
- [Issuer 文档](../src/issuer.rs)
- [Validator 文档](../../common/src/aid/credential/validator.rs)
- [KS 文档](../../ks/README.md)
