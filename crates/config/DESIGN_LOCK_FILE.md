# Lock File Design: Embedded Proto Content

## 设计理念

与 cargo/npm 等包管理器不同，我们的 lock 文件**直接嵌入 proto 内容**，而不是使用独立的 cache 目录。

## 设计对比

### 传统设计（cargo/npm 模式）

```
actr.lock.toml (5KB)
  - metadata
  - dependencies (只有引用)

~/.cache/actr/
  user-service/
    user.v1.proto (3KB)
    common.v1.proto (2KB)
  payment-service/
    payment.v1.proto (4KB)
```

**优势**：
- 适合大包（几百 MB）
- 多项目复用
- 减少重复存储

**劣势**：
- 复杂的 cache 管理
- 需要路径/URI 转换
- 容易出现不一致
- 不利于版本控制

### 新设计（内嵌模式）

```
actr.lock.toml (15KB)
  - metadata
  - dependencies
    - service_spec
      - files[].content (包含所有 proto)
```

**优势**：
- ✅ **极简架构**：无需 cache 目录和管理代码
- ✅ **单一数据源**：所有信息在一个文件中
- ✅ **原子性强**：复制项目只需复制 lock 文件
- ✅ **版本控制友好**：git diff 直接显示 proto 变化
- ✅ **易于调试**：打开一个文件即可查看所有依赖
- ✅ **代码简化**：减少 50% 代码量（482 → 367 行）

**适用性**：
- Proto 文件很小（单个 2-10KB）
- 总量可控（50 个文件 ≈ 250KB）
- 不需要跨项目复用

## Lock 文件格式

```toml
[metadata]
version = 1
generated_at = "2025-01-15T10:30:00Z"

[[dependency]]
name = "user-service"
actr_type = "acme+user-service"
description = "User management service"
fingerprint = "service_semantic:a1b2c3d4e5f6"
published_at = 1705315800
tags = ["latest", "stable"]
cached_at = "2025-01-15T10:30:00Z"

  [[dependency.files]]
  uri = "actr://101:acme+user-service@v1/user.v1.proto"
  fingerprint = "semantic:xyz123"
  content = """
syntax = "proto3";

package user.v1;

message User {
  uint64 id = 1;
  string name = 2;
  string email = 3;
}

service UserService {
  rpc GetUser(GetUserRequest) returns (GetUserResponse);
  rpc CreateUser(CreateUserRequest) returns (CreateUserResponse);
}
"""

  [[dependency.files]]
  uri = "actr://101:acme+user-service@v1/common.v1.proto"
  fingerprint = "semantic:abc789"
  content = """
syntax = "proto3";

package common.v1;

message Empty {}
"""
```

## 核心数据结构

```rust
/// Lock file structure
pub struct LockFile {
    pub metadata: Option<LockMetadata>,
    pub dependencies: Vec<LockedDependency>,
}

/// Locked dependency with embedded proto content
pub struct LockedDependency {
    pub name: String,
    pub actr_type: String,
    pub spec: ServiceSpecMeta,
    pub cached_at: String,
}

/// Service specification with embedded content
pub struct ServiceSpecMeta {
    pub description: Option<String>,
    pub fingerprint: String,
    pub protobufs: Vec<ProtoFileWithContent>,  // 直接包含 content
    pub published_at: Option<i64>,
    pub tags: Vec<String>,
}

/// Proto file with content (not a reference)
pub struct ProtoFileWithContent {
    pub uri: String,
    pub fingerprint: String,
    pub content: String,  // 直接嵌入内容
}
```

## 双向转换

```rust
// ServiceSpec → ServiceSpecMeta (直接映射，无需 cache)
impl From<ServiceSpec> for ServiceSpecMeta {
    fn from(spec: ServiceSpec) -> Self {
        Self {
            description: spec.description,
            fingerprint: spec.fingerprint,
            protobufs: spec.protobufs.into_iter().map(|proto| {
                ProtoFileWithContent {
                    uri: proto.uri,
                    fingerprint: proto.fingerprint,
                    content: proto.content,  // 直接复制
                }
            }).collect(),
            published_at: spec.published_at,
            tags: spec.tags,
        }
    }
}

// ServiceSpecMeta → ServiceSpec (直接映射，无需 cache)
impl From<ServiceSpecMeta> for ServiceSpec {
    fn from(meta: ServiceSpecMeta) -> Self {
        Self {
            description: meta.description,
            fingerprint: meta.fingerprint,
            protobufs: meta.protobufs.into_iter().map(|proto| {
                service_spec::Protobuf {
                    uri: proto.uri,
                    content: proto.content,  // 直接使用
                    fingerprint: proto.fingerprint,
                }
            }).collect(),
            published_at: meta.published_at,
            tags: meta.tags,
        }
    }
}
```

## 移除的代码

以下复杂组件已完全移除：

- ❌ `ProtoCache` trait
- ❌ `MemoryCache` 实现
- ❌ `FsProtoCache` 实现
- ❌ `ServiceSpecBuilder`
- ❌ `from_spec_with_cache()` 方法
- ❌ `restore_with_cache()` 方法
- ❌ `ProtoFileRef`（被 `ProtoFileWithContent` 替代）
- ❌ 所有 path ↔ URI 转换逻辑

## 文件大小分析

### 实际案例

假设一个项目有 10 个服务依赖：

```
每个服务：
- 5 个 proto 文件
- 每个文件 5KB
- 总计: 5KB × 5 = 25KB

10 个服务：
- 总计: 25KB × 10 = 250KB
```

**结论**：250KB 的 lock 文件在现代开发环境中完全可以接受。

### TOML 开销

TOML 序列化会增加约 20-30% 的大小（主要是 `"""` 标记和缩进），但：

```
250KB 原始内容 → ~320KB TOML 文件
```

这对于现代系统微不足道。

## 版本控制优势

### Git Diff 示例

```diff
  [[dependency.files]]
  uri = "actr://101:acme+user-service@v1/user.v1.proto"
  fingerprint = "semantic:xyz123"
  content = """
  syntax = "proto3";

  package user.v1;

  message User {
    uint64 id = 1;
    string name = 2;
-   string email = 3;
+   string email = 3;
+   bool verified = 4;
  }
  """
```

**优势**：
- 直接看到 proto 变化
- 不需要查看多个文件
- Code review 更容易
- 历史追溯清晰

## 与其他工具的对比

| 特性          | cargo   | npm     | actr (新设计) |
| ------------- | ------- | ------- | ------------- |
| 包大小        | 几百 MB | 几十 MB | 几 KB         |
| 需要 cache    | ✓       | ✓       | ✗             |
| lock 文件大小 | ~100KB  | ~500KB  | ~300KB        |
| 包含内容      | ✗       | ✗       | ✓             |
| 版本控制友好  | 一般    | 一般    | 优秀          |
| 复杂度        | 高      | 高      | 低            |

## 使用示例

```rust
use actr_config::lock::*;

// 创建 lock 文件
let mut lock_file = LockFile::new();

// 添加依赖
let spec_meta = ServiceSpecMeta {
    description: Some("User service".to_string()),
    fingerprint: "service_semantic:abc123".to_string(),
    protobufs: vec![
        ProtoFileWithContent {
            uri: "actr://101:acme+user-service@v1/user.v1.proto".to_string(),
            fingerprint: "semantic:xyz".to_string(),
            content: "syntax = \"proto3\";\n...".to_string(),
        }
    ],
    published_at: Some(1705315800),
    tags: vec!["latest".to_string()],
};

let dep = LockedDependency::new(
    "user-service".to_string(),
    "acme+user-service".to_string(),
    spec_meta,
);

lock_file.add_dependency(dep);

// 保存
lock_file.save_to_file("actr.lock.toml")?;

// 加载
let restored = LockFile::from_file("actr.lock.toml")?;

// 转换为 ServiceSpec
let service_spec = restored.dependencies[0].to_service_spec();
```

## 总结

这个设计充分考虑了 Actor-RTC 的实际使用场景：

1. **Proto 文件小** → 可以直接嵌入
2. **不需要跨项目复用** → 不需要 cache
3. **需要版本控制** → 嵌入内容更友好
4. **追求简洁** → 减少架构复杂度

结果是一个更简单、更直观、更易维护的设计。
