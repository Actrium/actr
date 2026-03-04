# 代码生成器完整功能测试

这个示例用于验证 `actr-web-protoc-codegen` 的完整功能。

## 测试内容

### Proto 文件特性

- ✅ Package 声明
- ✅ Service 定义
- ✅ 普通 RPC 方法
- ✅ 流式 RPC 方法（stream）
- ✅ Message 定义
- ✅ 基本类型（string, int32, int64, bool）
- ✅ Optional 字段
- ✅ Repeated 字段
- ✅ 嵌套消息类型

### 代码生成功能

#### Rust 代码生成
- ✅ Message 结构定义
- ✅ Actor 结构定义
- ✅ RPC 方法签名
- ✅ 流式方法支持
- ✅ 类型转换（Proto → Rust）
- ✅ wasm-bindgen 注解
- ✅ Serde 注解

#### TypeScript 代码生成
- ✅ Interface 定义
- ✅ Optional 字段（?:）
- ✅ Repeated 字段（[]）
- ✅ ActorRef 类
- ✅ RPC 方法包装
- ✅ 流式方法包装（subscribe）
- ✅ React Hooks
- ✅ useCallback 优化
- ✅ 自动导出（index.ts）

## 运行测试

```bash
cd examples/codegen-test
cargo run
```

## 预期输出

程序将生成以下文件：

```
generated-rust/
├── mod.rs
└── user_service.rs

generated-ts/
├── index.ts
├── user-service.types.ts
├── user-service.actor-ref.ts
└── use-user-service.ts
```

## 检查生成的代码

### 1. 查看 Rust 代码

```bash
cat generated-rust/user_service.rs
```

应该包含：
- User, GetUserRequest, GetUserResponse 等消息定义
- UserServiceActor 结构
- get_user, create_user 等方法

### 2. 查看 TypeScript 类型

```bash
cat generated-ts/user-service.types.ts
```

应该包含：
- User, GetUserRequest 等接口定义
- Optional 字段标记（?:）
- Repeated 字段标记（[]）

### 3. 查看 ActorRef

```bash
cat generated-ts/user-service.actor-ref.ts
```

应该包含：
- UserServiceActorRef 类
- getUser, createUser 等方法
- subscribeListUsers, subscribeWatchUsers 等流式方法

### 4. 查看 React Hook

```bash
cat generated-ts/use-user-service.ts
```

应该包含：
- useUserService Hook
- useCallback 包装的方法

## 验证

### 验证 Rust 代码

```bash
# 检查语法（不编译，因为缺少运行时依赖）
rustfmt --check generated-rust/*.rs
```

### 验证 TypeScript 代码

```bash
# 检查语法
npx tsc --noEmit generated-ts/*.ts
```

## 测试覆盖

| 功能 | 测试 | 状态 |
|------|------|------|
| Proto 解析 | user_service.proto | ✅ |
| Rust 消息生成 | User, GetUserRequest 等 | ✅ |
| Rust 方法生成 | get_user, create_user 等 | ✅ |
| 流式方法（Rust） | list_users, watch_users | ✅ |
| TypeScript 类型 | Interface 定义 | ✅ |
| Optional 字段 | age?: number | ✅ |
| Repeated 字段 | tags: string[] | ✅ |
| ActorRef 方法 | async getUser() | ✅ |
| 流式订阅 | subscribeListUsers() | ✅ |
| React Hooks | useUserService | ✅ |
| 代码格式化 | rustfmt, prettier | ⏸️  (可选) |

## 故障排查

### 问题：生成的文件为空

检查 Proto 文件路径是否正确：
```bash
ls -la proto/user_service.proto
```

### 问题：类型映射错误

查看日志输出，检查 Proto 类型是否受支持。

### 问题：格式化失败

格式化失败不会阻塞生成，可以手动运行：
```bash
rustfmt generated-rust/*.rs
npx prettier --write generated-ts/*.ts
```
