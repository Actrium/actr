# Actor-RTC Framework

[![Rust](https://img.shields.io/badge/rust-1.70+-orange.svg)](https://rustup.rs/)
[![Crates.io](https://img.shields.io/crates/v/actor-rtc-framework.svg)](https://crates.io/crates/actor-rtc-framework)
[![Documentation](https://docs.rs/actor-rtc-framework/badge.svg)](https://docs.rs/actor-rtc-framework)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](../LICENSE)

基于 WebRTC 和 Actor 模型的分布式实时通信框架。

## 🚀 特性

- **宏观 Actor 模型**: 进程级别的 Actor 抽象，简化分布式系统设计
- **WebRTC 原生支持**: 内置 NAT 穿透和点对点直连能力
- **双路径处理**: 状态路径(可靠) + 快车道(低延迟)
- **类型安全**: 基于 Protobuf 的契约驱动开发
- **ACL 感知**: 访问控制的安全发现机制
- **高性能**: 优化的消息调度和路由系统

## 📦 安装

添加到您的 `Cargo.toml`:

```toml
[dependencies]
actor-rtc-framework = "0.1.0"
tokio = { version = "1.0", features = ["full"] }
anyhow = "1.0"
```

## 🔧 快速开始

```rust
use actor_rtc_framework::prelude::*;

// 定义您的 Actor
#[derive(Default)]
struct MyActor {
    name: String,
}

// 实现生命周期
#[async_trait]
impl ILifecycle for MyActor {
    async fn on_start(&self, ctx: Arc<Context>) {
        ctx.log_info("Actor started!");
    }
}

// 实现消息处理
#[async_trait]
impl MessageHandler<MyRequest> for MyActor {
    type Response = MyResponse;
    
    async fn handle(&self, request: MyRequest, ctx: Arc<Context>) -> ActorResult<Self::Response> {
        ctx.log_info(&format!("处理请求: {}", request.data));
        Ok(MyResponse { result: format!("处理完成: {}", request.data) })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 创建 Actor
    let actor = Arc::new(MyActor::default());
    let actor_id = ActorId::new(1001, ActorTypeCode::Authenticated, "my_service".to_string());
    
    // 配置信令
    let signaling = Box::new(WebSocketSignaling::new("ws://localhost:8080")?);
    
    // 启动系统
    ActorSystem::new(actor_id)
        .with_signaling(signaling)
        .attach(actor)
        .start()
        .await?;
        
    Ok(())
}
```

## 🏗️ 架构概述

Actor-RTC 框架采用"宏观 Actor"模型，其中每个进程作为一个独立的 Actor：

```
┌─────────────────┐    WebRTC P2P    ┌─────────────────┐
│   Actor A       │◄──────────────────► │   Actor B       │
│  ┌───────────┐  │                  │  ┌───────────┐  │
│  │State Path │  │                  │  │State Path │  │
│  │(Reliable) │  │   Signaling      │  │(Reliable) │  │
│  └───────────┘  │   ┌─────────┐    │  └───────────┘  │
│  ┌───────────┐  │◄──│Signaling│────► │  ┌───────────┐  │
│  │Fast Path  │  │   │ Server  │    │  │Fast Path  │  │
│  │(Low Lat.) │  │   └─────────┘    │  │(Low Lat.) │  │
│  └───────────┘  │                  │  └───────────┘  │
└─────────────────┘                  └─────────────────┘
```

### 核心组件

- **ActorSystem**: Actor 运行时环境
- **Context**: Actor 与系统交互的接口
- **MessageScheduler**: 双路径消息调度器
- **WebRTCManager**: WebRTC 连接管理
- **SignalingAdapter**: 信令协议适配器

## 📋 主要 API

### 生命周期管理

```rust
#[async_trait]
impl ILifecycle for MyActor {
    async fn on_start(&self, ctx: Arc<Context>) { /* 启动时调用 */ }
    async fn on_stop(&self, ctx: Arc<Context>) { /* 停止前调用 */ }
    async fn on_peer_connected(&self, ctx: Arc<Context>, peer_id: &str) { /* 连接建立 */ }
    async fn on_actor_discovered(&self, ctx: Arc<Context>, actor_id: &ActorId) -> bool { /* 发现新 Actor */ }
}
```

### 消息处理

```rust
// 状态路径消息处理（可靠有序）
#[async_trait]
impl MessageHandler<MyMessage> for MyActor {
    type Response = MyResponse;
    async fn handle(&self, msg: MyMessage, ctx: Arc<Context>) -> ActorResult<Self::Response> { /* ... */ }
}

// 快车道消息处理（低延迟）
#[async_trait]
impl StreamMessageHandler<StreamData> for MyActor {
    async fn handle_stream(&self, data: StreamData, ctx: Arc<Context>) -> ActorResult<()> { /* ... */ }
}
```

### 上下文操作

```rust
// 在消息处理器中
async fn handle(&self, request: MyRequest, ctx: Arc<Context>) -> ActorResult<Self::Response> {
    // 发送单向消息
    ctx.tell(&target_actor_id, SomeMessage { data: "hello".to_string() }).await?;
    
    // 发送请求并等待响应
    let response: SomeResponse = ctx.call(&target_actor_id, SomeRequest { query: "info".to_string() }).await?;
    
    // 延迟发送消息
    ctx.schedule_tell(&target_actor_id, DelayedMessage { content: "later".to_string() }, Duration::from_secs(5)).await?;
    
    // 日志记录
    ctx.log_info("处理完成");
    
    Ok(MyResponse { result: "success".to_string() })
}
```

## 🎯 使用示例

查看 [`examples/`](../examples/) 目录获取完整的使用示例：

- [`echo-demo`](../examples/echo-demo/): 基础回声服务
- [`media-demo`](../examples/media-demo/): 媒体流处理 (规划中)
- [`file-transfer-demo`](../examples/file-transfer-demo/): 文件传输 (规划中)

## 🔧 高级特性

### 自定义信令适配器

```rust
use actor_rtc_framework::signaling::SignalingAdapter;

struct MySignalingAdapter;

#[async_trait]
impl SignalingAdapter for MySignalingAdapter {
    async fn connect(&mut self) -> ActorResult<()> { /* 实现连接逻辑 */ }
    async fn register_actor(&mut self, actor_id: &ActorId) -> ActorResult<()> { /* 注册 Actor */ }
    // ... 其他方法
}
```

### 性能调优

框架提供了多种性能优化选项：

- **消息优先级**: 控制消息处理顺序
- **快车道处理**: 流式数据绕过队列直接处理
- **连接池管理**: 复用 WebRTC 连接
- **批量消息处理**: 提高吞吐量

## 📊 性能基准

基于本地测试环境的性能数据：

- **消息吞吐量**: ~10K msg/sec (状态路径)
- **流式数据**: ~100MB/sec (快车道)
- **连接延迟**: <50ms (本地网络)
- **内存占用**: ~10MB (基础运行时)

## 🔒 安全特性

- **ACL 控制**: 基于 Actor 类型的访问控制
- **身份验证**: 结构化的 Actor 身份系统
- **加密通信**: WebRTC 内置端到端加密
- **安全发现**: "发现即授权" 原则

## 🧪 测试

运行测试：

```bash
cargo test
cargo test --features integration-tests  # 集成测试
cargo bench                             # 性能基准测试
```

## 📚 文档

- [API 文档](https://docs.rs/actor-rtc-framework)
- [架构设计](../docs/1-Concepts-and-Architecture.zh.md)
- [开发指南](../docs/2-Developer-Guide.zh.md)
- [协议定义](../docs/1.2-Framework-Internal-Protocols-zh.md)

## 🤝 贡献

欢迎贡献代码、报告问题或提出改进建议！

1. Fork 项目
2. 创建特性分支 (`git checkout -b feature/AmazingFeature`)
3. 提交更改 (`git commit -m 'Add AmazingFeature'`)
4. 推送分支 (`git push origin feature/AmazingFeature`)
5. 开启 Pull Request

## 📄 许可证

本项目采用 MIT 许可证 - 查看 [LICENSE](../LICENSE) 文件了解详情。