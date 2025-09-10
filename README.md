# Actor-RTC Framework Demo

![Rust](https://img.shields.io/badge/rust-1.70+-orange.svg)
![Node.js](https://img.shields.io/badge/node.js-16+-green.svg)
![WebRTC](https://img.shields.io/badge/webrtc-enabled-blue.svg)
![License](https://img.shields.io/badge/license-MIT-green.svg)

基于 WebRTC 和 Actor 模型的分布式实时通信框架演示程序。

## 📖 概述

这个项目展示了一个创新的分布式系统架构，将经典的 Actor 模型与现代 WebRTC 技术相结合。通过"宏观 Actor"的设计理念，每个进程作为一个独立的 Actor，通过 WebRTC 进行点对点通信，同时内置了双路径处理模型来优化不同类型数据的传输。

### 🎯 核心特性

- **宏观 Actor 模型**: 进程级别的 Actor 抽象，简化分布式系统设计
- **WebRTC 原生支持**: 内置 NAT 穿透和点对点直连能力
- **双路径处理**: 
  - **状态路径**: 可靠有序的控制消息处理
  - **快车道**: 低延迟的流式数据处理
- **类型安全**: 基于 Protobuf 的契约驱动开发
- **ACL 感知**: 访问控制列表支持的安全发现机制

### 🏗️ 架构设计

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

## 🚀 快速开始

### 前置要求

- **Rust**: 1.70+ ([安装指南](https://rustup.rs/))
- **Node.js**: 16+ ([下载地址](https://nodejs.org/))
- **protoc**: Protocol Buffer 编译器
  ```bash
  # Ubuntu/Debian
  sudo apt install protobuf-compiler
  
  # macOS
  brew install protobuf
  ```

### 一键演示

```bash
# 1. 设置项目（安装依赖、构建）
./run_demo.sh setup

# 2. 运行完整演示
./run_demo.sh demo
```

### 手动步骤

如果您希望逐步了解各个组件：

```bash
# 1. 启动信令服务器
./run_demo.sh start-signaling

# 2. 启动回声 Actor 演示
./run_demo.sh start-echo

# 3. 测试各种功能
./run_demo.sh test-connection   # 测试信令连接
./run_demo.sh test-discovery    # 测试 Actor 发现
./run_demo.sh test-echo         # 测试回声服务
./run_demo.sh test-relay        # 测试消息中继
./run_demo.sh test-load         # 负载测试

# 4. 查看服务状态
./run_demo.sh status

# 5. 停止所有服务
./run_demo.sh stop-all
```

## 📁 项目结构

```
actor-rtc/
├── docs/                          # 框架设计文档
├── proto/                         # Protobuf 协议定义
│   ├── webrtc.proto              # WebRTC 基础类型
│   ├── actor.proto               # Actor 核心定义
│   ├── signaling.proto           # 信令层协议
│   ├── echo.proto                # 回声服务协议
│   ├── media_streaming.proto     # 媒体流协议
│   └── file_transfer.proto       # 文件传输协议
├── actor-rtc-framework/          # 🔥 框架核心 crate
│   ├── src/
│   │   ├── lib.rs               # 框架入口和预导入
│   │   ├── actor.rs             # Actor 系统核心
│   │   ├── context.rs           # Actor 上下文
│   │   ├── messaging.rs         # 消息处理系统
│   │   ├── signaling.rs         # 信令适配器
│   │   ├── webrtc.rs            # WebRTC 集成
│   │   ├── routing.rs           # 双路径调度器
│   │   ├── lifecycle.rs         # 生命周期管理
│   │   └── error.rs             # 错误类型定义
│   └── Cargo.toml
├── shared-protocols/             # 共享协议定义 crate
├── signaling-server/             # Node.js 信令服务器
├── examples/                     # 🎯 示例程序（使用框架）
│   ├── echo-demo/               # 回声服务示例
│   ├── media-demo/              # 媒体流示例 (规划中)
│   └── file-transfer-demo/      # 文件传输示例 (规划中)
├── test-utils/                   # 测试工具
└── run_demo.sh                   # 自动化脚本
```

## 🎯 示例程序详解

### 1. 回声服务示例 (Echo Demo)

**功能**: 展示框架的基础使用方法，实现请求-响应消息处理

**特性**:
- 使用 `actor-rtc-framework` 构建
- 完整的生命周期管理
- 单次和批量回声请求处理  
- 自动对等发现和连接
- 消息计数和时间戳

**核心代码**:
```rust
use actor_rtc_framework::prelude::*;

// 定义 Actor
pub struct EchoActor { /* ... */ }

// 实现生命周期
#[async_trait]
impl ILifecycle for EchoActor { /* ... */ }

// 实现消息处理
#[async_trait]
impl MessageHandler<EchoRequest> for EchoActor {
    type Response = EchoResponse;
    async fn handle(&self, request: EchoRequest, ctx: Arc<Context>) -> ActorResult<Self::Response> {
        // 处理回声请求
    }
}

// 启动系统
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let actor = Arc::new(EchoActor::new("demo".to_string()));
    let actor_id = ActorId::new(1001, ActorTypeCode::Authenticated, "echo_demo".to_string());
    let signaling = Box::new(WebSocketSignaling::new("ws://localhost:8080")?);
    
    ActorSystem::new(actor_id)
        .with_signaling(signaling)
        .attach(actor)
        .start()
        .await?;
        
    Ok(())
}
```

**使用方法**:
```bash
# 启动回声示例
SIGNALING_URL="ws://localhost:8080" ACTOR_ID="1001" \
  ./target/release/echo-demo "MyEchoDemo"

# 测试回声功能
./target/release/test-client echo \
  --message "Hello, Framework!" --target-id 1001
```

### 2. 媒体流 Actor (规划中)

**功能**: 演示音视频流的发布和订阅

**特性**:
- 音频/视频流发布
- 多种编码格式支持 (H.264, VP8, Opus)
- 质量自适应
- 流列表管理

### 3. 文件传输 Actor (规划中)

**功能**: 大文件的点对点传输

**特性**:
- 分块传输
- 断点续传
- 完整性校验
- 传输进度跟踪

## 🔧 开发指南

### 使用框架创建新的 Actor

使用 `actor-rtc-framework` 创建 Actor 非常简单，只需几个步骤：

#### 1. 添加依赖

在您的 `Cargo.toml` 中：
```toml
[dependencies]
actor-rtc-framework = "0.1.0"
shared-protocols = { path = "../shared-protocols" }  # 或从 crates.io
tokio = { version = "1.0", features = ["full"] }
anyhow = "1.0"
```

#### 2. 定义 Protobuf 服务契约

```protobuf
// proto/my_service.proto
syntax = "proto3";
package my_service;

message MyRequest { string data = 1; }
message MyResponse { string result = 1; }

service MyService {
  rpc ProcessData(MyRequest) returns (MyResponse);
}
```

#### 3. 实现 Actor

```rust
use actor_rtc_framework::prelude::*;
use shared_protocols::my_service::{MyRequest, MyResponse};

pub struct MyActor {
    name: String,
}

impl MyActor {
    pub fn new(name: String) -> Self {
        Self { name }
    }
}

// 实现生命周期管理
#[async_trait]
impl ILifecycle for MyActor {
    async fn on_start(&self, ctx: Arc<Context>) {
        ctx.log_info(&format!("Actor {} started!", self.name));
    }
    
    async fn on_stop(&self, ctx: Arc<Context>) {
        ctx.log_info(&format!("Actor {} stopped!", self.name));
    }
    
    async fn on_actor_discovered(&self, ctx: Arc<Context>, actor_id: &ActorId) -> bool {
        ctx.log_info(&format!("Discovered actor: {}", actor_id.serial_number));
        true  // 自动连接到新发现的 Actor
    }
}

// 实现消息处理
#[async_trait]
impl MessageHandler<MyRequest> for MyActor {
    type Response = MyResponse;
    
    async fn handle(&self, request: MyRequest, ctx: Arc<Context>) -> ActorResult<Self::Response> {
        ctx.log_info(&format!("Processing request: {}", request.data));
        
        // 处理业务逻辑
        let result = format!("Processed by {}: {}", self.name, request.data);
        
        Ok(MyResponse { result })
    }
}
```

#### 4. 启动 Actor 系统

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化日志
    tracing_subscriber::fmt::init();
    
    // 创建 Actor 实例
    let actor = Arc::new(MyActor::new("MyActor".to_string()));
    
    // 创建 Actor ID
    let actor_id = ActorId::new(1001, ActorTypeCode::Authenticated, "my_service".to_string());
    
    // 创建信令适配器
    let signaling = Box::new(WebSocketSignaling::new("ws://localhost:8080")?);
    
    // 创建并启动 Actor 系统
    ActorSystem::new(actor_id)
        .with_signaling(signaling)
        .attach(actor)
        .start()
        .await?;
        
    Ok(())
}
```

#### 5. 高级特性

**快车道处理 (流式数据)**:
```rust
#[async_trait]
impl StreamMessageHandler<MyStreamData> for MyActor {
    async fn handle_stream(&self, data: MyStreamData, ctx: Arc<Context>) -> ActorResult<()> {
        // 低延迟处理流式数据，绕过消息队列
        ctx.log_debug("Processing stream data");
        Ok(())
    }
}
```

**主动通信**:
```rust
// 在处理器中向其他 Actor 发送消息
async fn handle(&self, request: MyRequest, ctx: Arc<Context>) -> ActorResult<Self::Response> {
    // 发送单向消息
    let target_id = ActorId::new(2001, ActorTypeCode::Authenticated, "other_service".to_string());
    ctx.tell(&target_id, SomeMessage { data: "hello".to_string() }).await?;
    
    // 发送请求并等待响应
    let response: OtherResponse = ctx.call(&target_id, SomeRequest { query: "info".to_string() }).await?;
    
    Ok(MyResponse { result: "processed".to_string() })
}
```

### 测试 Actor

框架支持独立的单元测试：

```rust
#[tokio::test]
async fn test_my_actor() {
    let actor = MyActor::new();
    let ctx = create_test_context();
    
    let request = MyRequest { data: "test".to_string() };
    let response = actor.handle(request, ctx).await.unwrap();
    
    assert_eq!(response.result, "Processed: test");
}
```

## 📊 性能特性

### 双路径处理模型

- **状态路径**: 
  - 处理控制消息、RPC 调用
  - 保证消息顺序和可靠性
  - 支持高优先级和普通优先级队列

- **快车道**: 
  - 处理媒体流、大数据传输
  - 绕过消息队列，直接回调处理
  - 优化延迟和吞吐量

### 基准测试结果

```bash
# 运行负载测试
./run_demo.sh test-load

# 典型结果 (本地测试)
✅ Load test completed!
  10 successful connections  
  500 total messages sent
  2.34s elapsed time
  213.68 messages/second
```

## 🔒 安全特性

### ACL (访问控制列表)

信令服务器支持基于 Actor 类型的访问控制：

```javascript
// 允许 demo_echo 类型互相发现
server.addAclRule('demo_echo', 'demo_echo', 'ALLOW');

// 允许跨类型通信
server.addAclRule('demo_echo', 'demo_media', 'ALLOW');
```

### 身份感知路由

每个 Actor 都有结构化的身份标识：

```rust
let actor_id = ActorId {
    serial_number: 1001,
    type_: Some(ActorType {
        code: ActorTypeCode::Authenticated as i32,
        manufacturer: Some("demo".to_string()),
        name: "echo_service".to_string(),
    }),
};
```

## 🛠️ 工具和实用程序

### 测试客户端

```bash
# 连接并监听信令消息
./target/release/test-client listen

# 发现可用的 Actors
./target/release/test-client discover

# 测试回声服务
./target/release/test-client echo \
  --message "Hello!" --target-id 1001

# 基准测试
./target/release/test-client benchmark \
  --count 100 --target-id 1001
```

### 信令服务器测试

```bash
# 基础连接测试
./target/release/signaling-test connect

# 多客户端连接测试
./target/release/signaling-test multi-connect --count 10

# Actor 发现测试
./target/release/signaling-test discovery --actors 5

# 消息中继测试
./target/release/signaling-test message-relay

# 负载测试
./target/release/signaling-test load-test \
  --concurrent 20 --messages 100
```

## 🐛 故障排除

### 常见问题

1. **连接失败**:
   ```bash
   # 检查信令服务器是否运行
   ./run_demo.sh status
   
   # 查看日志
   ./run_demo.sh logs
   ```

2. **构建错误**:
   ```bash
   # 清理并重新构建
   ./run_demo.sh clean
   ./run_demo.sh setup
   ```

3. **端口冲突**:
   ```bash
   # 修改端口配置
   export SIGNALING_PORT=9090
   ./run_demo.sh start-signaling
   ```

### 调试模式

启用详细日志：

```bash
RUST_LOG=debug ./target/release/echo-actor
```

查看信令服务器调试信息：

```bash
DEBUG=* node signaling-server/src/server.js
```

## 📚 深入阅读

- [框架核心理念与架构](docs/1-Concepts-and-Architecture.zh.md)
- [开发者指南](docs/2-Developer-Guide.zh.md)
- [内部协议定义](docs/1.2-Framework-Internal-Protocols-zh.md)
- [信令机制解析](docs/3.4-Signaling.zh.md)

## 🤝 贡献指南

我们欢迎各种形式的贡献！

1. Fork 本项目
2. 创建特性分支 (`git checkout -b feature/AmazingFeature`)
3. 提交更改 (`git commit -m 'Add some AmazingFeature'`)
4. 推送到分支 (`git push origin feature/AmazingFeature`)
5. 开启 Pull Request

## 📄 许可证

本项目采用 MIT 许可证。

## 🙏 致谢

- WebRTC 社区提供的优秀协议和实现
- Rust 生态系统中的相关 crates
- Actor 模型的理论基础和实践经验

---

**注意**: 这是一个演示项目，展示框架的核心概念和能力。在生产环境中使用前，请进行充分的测试和安全评估。
