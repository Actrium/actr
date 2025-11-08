//! 自动生成的代码 - 请勿手动编辑
//!
//! 由 actr-cli 的 protoc-gen-actrframework 插件生成

#![allow(dead_code, unused_imports)]

use async_trait::async_trait;
use bytes::Bytes;
use prost::Message as ProstMessage;

use actr_framework::{Context, MessageDispatcher, Workload};
use actr_protocol::{ActorResult, ActrType, RpcRequest, RpcEnvelope, PayloadType};

// 导入 protobuf 消息类型（由 prost 生成）
use super::echo::*;


/// RpcRequest trait implementation - associates Request and Response types
///
/// This enables type-safe RPC calls with automatic response type inference:
/// ```rust,ignore
/// let response: EchoResponse = ctx.call(&target, request).await?;
/// ```
impl RpcRequest for EchoRequest {
    type Response = EchoResponse;

    fn route_key() -> &'static str {
        "echo.EchoService.Echo"
    }

    fn payload_type() -> PayloadType {
        PayloadType::RpcReliable
    }
}

#[async_trait]
# [doc = r" 服务处理器 trait - 用户需要实现此 trait"] # [doc = r""] # [doc = r" # 示例"] # [doc = r""] # [doc = r" ```rust,ignore"] # [doc = r" pub struct MyService { /* ... */ }"] # [doc = r""] # [doc = r" #[async_trait]"] # [doc = r" impl #handler_trait_ident for MyService {"] # [doc = r"     async fn method_name(&self, req: Request, ctx: &Context) -> ActorResult<Response> {"] # [doc = r"         // 业务逻辑"] # [doc = r"         Ok(Response::default())"] # [doc = r"     }"] # [doc = r" }"] # [doc = r" ```"] pub trait EchoServiceHandler : Send + Sync + 'static { # [doc = r" RPC 方法：#method_name"] async fn echo < C : Context > (& self , req : EchoRequest , ctx : & C ,) -> ActorResult < EchoResponse > ; }

# [doc = r" Workload 包装类型"] # [doc = r""] # [doc = r" 包装用户的 Handler 实现，满足孤儿规则"] pub struct EchoServiceWorkload < T : EchoServiceHandler > (pub T) ; impl < T : EchoServiceHandler > EchoServiceWorkload < T > { # [doc = r" 创建新的 Workload 实例"] pub fn new (handler : T) -> Self { Self (handler) } }
# [doc = r" Message dispatcher - 零大小类型 (ZST)"] # [doc = r""] # [doc = r" 此路由器由代码生成器自动生成，将 route_key 静态路由到对应的处理方法。"] # [doc = r""] # [doc = r" # 性能特性"] # [doc = r""] # [doc = r" - 零内存开销（PhantomData）"] # [doc = r" - 静态 match 派发，约 5-10ns"] # [doc = r" - 编译器完全内联"] pub struct EchoServiceDispatcher < T : EchoServiceHandler > (std :: marker :: PhantomData < T >) ;
#[async_trait]
impl < T : EchoServiceHandler > MessageDispatcher for EchoServiceDispatcher < T > { type Workload = EchoServiceWorkload < T > ; async fn dispatch < C : Context > (workload : & Self :: Workload , envelope : RpcEnvelope , ctx : & C ,) -> ActorResult < Bytes > { match envelope . route_key . as_str () { "echo.EchoService.Echo" => { let payload = envelope . payload . as_ref () . ok_or_else (|| actr_protocol :: ProtocolError :: DecodeError ("Missing payload in RpcEnvelope" . to_string ())) ? ; let req = EchoRequest :: decode (& * * payload) . map_err (| e | actr_protocol :: ProtocolError :: Actr (actr_protocol :: ActrError :: DecodeFailure { message : format ! ("Failed to decode {}: {}" , stringify ! (EchoRequest) , e) })) ? ; let resp = workload . 0. echo (req , ctx) . await ? ; Ok (resp . encode_to_vec () . into ()) } , _ => Err (actr_protocol :: ProtocolError :: Actr (actr_protocol :: ActrError :: UnknownRoute { route_key : envelope . route_key . to_string () })) } } }

# [doc = r" Workload trait 实现"] # [doc = r""] # [doc = r" 为包装类型实现 Workload，使其可被 ActorSystem 识别和调度"] impl < T : EchoServiceHandler > Workload for EchoServiceWorkload < T > { type Dispatcher = EchoServiceDispatcher < T > ; fn actor_type (& self) -> ActrType { ActrType { manufacturer : "acme" . to_string () , name : "echo.EchoService" . to_string () , } } }

/*
## 使用示例

### 1. 实现业务逻辑

```rust
use actr_framework::{Context, ActorSystem};
use actr_protocol::ActorResult;

pub struct MyService {
    // 业务状态
}

#[async_trait]
impl EchoServiceHandler for MyService {

    async fn echo(&self, req: EchoRequest, ctx: &Context) -> ActorResult<EchoResponse> {
        // 实现业务逻辑
        Ok(EchoResponse::default())
    }
}
```

### 2. 启动服务

```rust
#[tokio::main]
async fn main() -> ActorResult<()> {
    let config = actr_config::Config::from_file("Actr.toml")?;
    let service = MyService { /* ... */ };

    ActorSystem::new(config)?
        .attach(service)  // ← 自动获得 Workload + Dispatcher
        .start()
        .await?
        .wait_for_shutdown()
        .await
}
```

## 架构说明

- **EchoServiceHandler**: 用户实现的业务逻辑接口
- **EchoServiceDispatcher**: zero-sized type static dispatcher（自动生成）
- **Workload**: 通过 blanket impl 自动获得（自动生成）

用户只需实现 EchoServiceHandler，框架会自动提供路由和工作负载能力。
*/
