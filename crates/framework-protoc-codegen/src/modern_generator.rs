//! 现代化代码生成器
//!
//! 基于 actr-framework 的实际架构生成代码：
//! - MessageDispatcher trait: zero-sized type static dispatcher
//! - Workload trait: 业务工作负载，associates Dispatcher type
//! - {Service}Handler trait: 用户实现的业务逻辑接口

use anyhow::Result;
use heck::ToSnakeCase;
use prost_types::MethodDescriptorProto;
use quote::{format_ident, quote};

use crate::payload_type_extractor::extract_payload_type_or_default;

/// 代码生成器角色
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratorRole {
    /// 为 exports 生成服务端代码
    ServerSide,
    /// 为 dependencies 生成客户端代码
    ClientSide,
}

/// 现代化代码生成器
pub struct ModernGenerator {
    package_name: String,
    service_name: String,
    role: GeneratorRole,
}

impl ModernGenerator {
    pub fn new(package_name: &str, service_name: &str, role: GeneratorRole) -> Self {
        Self {
            package_name: package_name.to_string(),
            service_name: service_name.to_string(),
            role,
        }
    }

    /// 生成完整代码
    pub fn generate(&self, methods: &[MethodDescriptorProto]) -> Result<String> {
        match self.role {
            GeneratorRole::ServerSide => self.generate_server_code(methods),
            GeneratorRole::ClientSide => self.generate_client_code(methods),
        }
    }

    /// 生成服务端代码（exports）
    fn generate_server_code(&self, methods: &[MethodDescriptorProto]) -> Result<String> {
        let sections = [
            // 1. 生成导入
            self.generate_imports(),
            // 2. 生成 RpcRequest trait 实现（类型安全的 Request → Response 关联）
            self.generate_message_impls(methods)?,
            // 3. 生成 Handler trait（用户实现的接口）
            self.generate_handler_trait(methods)?,
            // 4. Generate Dispatcher implementation（zero-sized type static dispatcher）
            self.generate_router_impl(methods)?,
            // 5. 生成 Workload blanket 实现
            self.generate_workload_blanket_impl(methods)?,
            // 6. 生成使用文档
            self.generate_usage_docs(methods)?,
        ];

        Ok(sections.join("\n\n"))
    }

    /// 生成客户端代码（dependencies）
    fn generate_client_code(&self, methods: &[MethodDescriptorProto]) -> Result<String> {
        let sections = [
            // 1. 生成导入
            self.generate_imports(),
            // 2. 生成 RpcRequest trait 实现（客户端也需要用于类型安全调用）
            self.generate_message_impls(methods)?,
            // 3. 生成 Context 扩展方法
            self.generate_context_extensions(methods)?,
            // 4. 生成使用文档
            self.generate_client_usage_docs(methods)?,
        ];

        Ok(sections.join("\n\n"))
    }

    /// 生成导入语句
    fn generate_imports(&self) -> String {
        // 生成protobuf消息导入
        // 假设消息类型在同级的 proto 模块中（由 prost 生成）
        let proto_module = self.package_name.replace('.', "_");

        format!(
            r#"//! 自动生成的代码 - 请勿手动编辑
//!
//! 由 actr-cli 的 protoc-gen-actrframework 插件生成

#![allow(dead_code, unused_imports)]

use async_trait::async_trait;
use bytes::Bytes;
use prost::Message as ProstMessage;

use actr_framework::{{Context, MessageDispatcher, Workload}};
use actr_protocol::{{ActorResult, RpcRequest, RpcEnvelope, PayloadType}};

// 导入 protobuf 消息类型（由 prost 生成）
use super::{proto_module}::*;
"#
        )
    }

    /// 生成 RpcRequest trait 实现
    ///
    /// 为每个 RPC 方法的 Request 类型生成 RpcRequest trait 实现，
    /// 关联其对应的 Response 类型。这使得客户端可以使用类型安全的 API：
    ///
    /// ```rust,ignore
    /// let response: EchoResponse = ctx.call(&target, request).await?;
    /// //              ^^^^^^^^^^^^ 从 EchoRequest::Response 推导
    /// ```
    fn generate_message_impls(&self, methods: &[MethodDescriptorProto]) -> Result<String> {
        let mut impls = Vec::new();

        for method in methods {
            let input_type = self.extract_message_type(method.input_type())?;
            let output_type = self.extract_message_type(method.output_type())?;

            // 生成路由键
            let route_key = format!(
                "{}.{}.{}",
                self.package_name,
                self.service_name,
                method.name()
            );

            // 提取 PayloadType
            let payload_type = extract_payload_type_or_default(method);

            // 生成 PayloadType 枚举路径（不能用 quote! 因为会加引号）
            let payload_type_code = payload_type.as_rust_variant();

            // 手动构造代码字符串避免 quote! 添加引号
            let impl_code = format!(
                r#"/// RpcRequest trait implementation - associates Request and Response types
///
/// This enables type-safe RPC calls with automatic response type inference:
/// ```rust,ignore
/// let response: {output_type} = ctx.call(&target, request).await?;
/// ```
impl RpcRequest for {input_type} {{
    type Response = {output_type};

    fn route_key() -> &'static str {{
        "{route_key}"
    }}

    fn payload_type() -> PayloadType {{
        {payload_type_code}
    }}
}}"#
            );

            impls.push(impl_code);
        }

        Ok(impls.join("\n\n"))
    }

    /// 生成 Handler trait
    fn generate_handler_trait(&self, methods: &[MethodDescriptorProto]) -> Result<String> {
        let handler_trait_name = format!("{}Handler", self.service_name);
        let handler_trait_ident = format_ident!("{}", handler_trait_name);

        let mut method_sigs = Vec::new();
        for method in methods {
            let method_name = method.name().to_snake_case();
            let method_ident = format_ident!("{}", method_name);
            let input_type = self.extract_message_type(method.input_type())?;
            let output_type = self.extract_message_type(method.output_type())?;
            let input_ident = format_ident!("{}", input_type);
            let output_ident = format_ident!("{}", output_type);

            method_sigs.push(quote! {
                /// RPC 方法：#method_name
                async fn #method_ident<C: Context>(
                    &self,
                    req: #input_ident,
                    ctx: &C,
                ) -> ActorResult<#output_ident>;
            });
        }

        let handler_trait_without_attr = quote! {
            /// 服务处理器 trait - 用户需要实现此 trait
            ///
            /// # 示例
            ///
            /// ```rust,ignore
            /// pub struct MyService { /* ... */ }
            ///
            /// #[async_trait]
            /// impl #handler_trait_ident for MyService {
            ///     async fn method_name(&self, req: Request, ctx: &Context) -> ActorResult<Response> {
            ///         // 业务逻辑
            ///         Ok(Response::default())
            ///     }
            /// }
            /// ```
            pub trait #handler_trait_ident: Send + Sync + 'static {
                #(#method_sigs)*
            }
        };

        // 手动添加 #[async_trait] 属性，避免 quote! 宏插入空格
        Ok(format!("#[async_trait]\n{handler_trait_without_attr}"))
    }

    /// Generate Dispatcher and Workload 包装类型
    fn generate_router_impl(&self, methods: &[MethodDescriptorProto]) -> Result<String> {
        let router_name = format!("{}Dispatcher", self.service_name);
        let router_ident = format_ident!("{}", router_name);
        let workload_name = format!("{}Workload", self.service_name);
        let workload_ident = format_ident!("{}", workload_name);
        let handler_trait = format!("{}Handler", self.service_name);
        let handler_trait_ident = format_ident!("{}", handler_trait);

        // 生成 match 分支
        let mut match_arms = Vec::new();
        for method in methods {
            let route_key = format!(
                "{}.{}.{}",
                self.package_name,
                self.service_name,
                method.name()
            );
            let method_name = method.name().to_snake_case();
            let method_ident = format_ident!("{}", method_name);
            let input_type = self.extract_message_type(method.input_type())?;
            let input_ident = format_ident!("{}", input_type);

            match_arms.push(quote! {
                #route_key => {
                    // Extract payload from envelope
                    let payload = envelope.payload.as_ref()
                        .ok_or_else(|| actr_protocol::ProtocolError::DecodeError(
                            "Missing payload in RpcEnvelope".to_string()
                        ))?;

                    // Deserialize request
                    let req = #input_ident::decode(&**payload)
                        .map_err(|e| actr_protocol::ProtocolError::Actr(
                            actr_protocol::ActrError::DecodeFailure {
                                message: format!("Failed to decode {}: {}", stringify!(#input_ident), e)
                            }
                        ))?;

                    // 调用业务逻辑
                    let resp = workload.0.#method_ident(req, ctx).await?;

                    // 序列化响应
                    Ok(resp.encode_to_vec().into())
                }
            });
        }

        // 分开生成各个部分以确保属性正确输出
        let workload_struct = quote! {
            /// Workload 包装类型
            ///
            /// 包装用户的 Handler 实现，满足孤儿规则
            pub struct #workload_ident<T: #handler_trait_ident>(pub T);

            impl<T: #handler_trait_ident> #workload_ident<T> {
                /// 创建新的 Workload 实例
                pub fn new(handler: T) -> Self {
                    Self(handler)
                }
            }
        };

        let router_struct = quote! {
            /// Message dispatcher - 零大小类型 (ZST)
            ///
            /// 此路由器由代码生成器自动生成，将 route_key 静态路由到对应的处理方法。
            ///
            /// # 性能特性
            ///
            /// - 零内存开销（PhantomData）
            /// - 静态 match 派发，约 5-10ns
            /// - 编译器完全内联
            pub struct #router_ident<T: #handler_trait_ident>(std::marker::PhantomData<T>);
        };

        let router_impl_without_attr = quote! {
            impl<T: #handler_trait_ident> MessageDispatcher for #router_ident<T> {
                type Workload = #workload_ident<T>;

                async fn dispatch<C: Context>(
                    workload: &Self::Workload,
                    envelope: RpcEnvelope,
                    ctx: &C,
                ) -> ActorResult<Bytes> {
                    match envelope.route_key.as_str() {
                        #(#match_arms,)*
                        _ => Err(actr_protocol::ProtocolError::Actr(
                            actr_protocol::ActrError::UnknownRoute {
                                route_key: envelope.route_key.to_string()
                            }
                        ))
                    }
                }
            }
        };

        // 手动添加 #[async_trait] 属性，避免 quote! 宏插入空格
        let router_impl = format!("#[async_trait]\n{router_impl_without_attr}");

        Ok(format!("{workload_struct}\n{router_struct}\n{router_impl}"))
    }

    /// 生成 Workload 实现
    fn generate_workload_blanket_impl(&self, _methods: &[MethodDescriptorProto]) -> Result<String> {
        let router_name = format!("{}Dispatcher", self.service_name);
        let router_ident = format_ident!("{}", router_name);
        let workload_name = format!("{}Workload", self.service_name);
        let workload_ident = format_ident!("{}", workload_name);
        let handler_trait = format!("{}Handler", self.service_name);
        let handler_trait_ident = format_ident!("{}", handler_trait);

        Ok(quote! {
            /// Workload trait 实现
            ///
            /// 为包装类型实现 Workload，使其可被 ActorSystem 识别和调度
            impl<T: #handler_trait_ident> Workload for #workload_ident<T> {
                type Dispatcher = #router_ident<T>;
            }
        }
        .to_string())
    }

    /// 生成 Context 扩展方法（客户端）
    fn generate_context_extensions(&self, methods: &[MethodDescriptorProto]) -> Result<String> {
        let client_struct_name = format!("{}Client", self.service_name);
        let client_ident = format_ident!("{}", client_struct_name);

        let mut client_methods = Vec::new();
        for method in methods {
            let method_name = method.name().to_snake_case();
            let method_ident = format_ident!("{}", method_name);
            let input_type = self.extract_message_type(method.input_type())?;
            let output_type = self.extract_message_type(method.output_type())?;
            let input_ident = format_ident!("{}", input_type);
            let output_ident = format_ident!("{}", output_type);

            let route_key = format!(
                "{}.{}.{}",
                self.package_name,
                self.service_name,
                method.name()
            );

            client_methods.push(quote! {
                /// 调用远程方法：#method_name
                pub async fn #method_ident(
                    &self,
                    req: #input_ident,
                ) -> ActorResult<#output_ident> {
                    self.ctx.call_remote(#route_key, req).await
                }
            });
        }

        // 生成 Context 扩展
        let extension_method_name = self.service_name.to_snake_case();
        let extension_method_ident = format_ident!("{}", extension_method_name);

        Ok(quote! {
            /// 客户端接口
            ///
            /// 提供类型安全的远程调用方法
            pub struct #client_ident<'a> {
                ctx: &'a Context,
            }

            impl<'a> #client_ident<'a> {
                #(#client_methods)*
            }

            /// Context 扩展 trait
            ///
            /// 为 Context 添加便捷的客户端方法
            pub trait ContextExt {
                fn #extension_method_ident(&self) -> #client_ident;
            }

            impl ContextExt for Context {
                fn #extension_method_ident(&self) -> #client_ident {
                    #client_ident { ctx: self }
                }
            }
        }
        .to_string())
    }

    /// 生成服务端使用文档
    fn generate_usage_docs(&self, methods: &[MethodDescriptorProto]) -> Result<String> {
        let handler_trait = format!("{}Handler", self.service_name);
        let first_method = methods.first();

        let example_method = if let Some(method) = first_method {
            let method_name = method.name().to_snake_case();
            let input_type = self.extract_message_type(method.input_type())?;
            let output_type = self.extract_message_type(method.output_type())?;
            format!(
                r#"
    async fn {method_name}(&self, req: {input_type}, ctx: &Context) -> ActorResult<{output_type}> {{
        // 实现业务逻辑
        Ok({output_type}::default())
    }}"#
            )
        } else {
            "    // 实现方法...".to_string()
        };

        Ok(format!(
            r#"/*
## 使用示例

### 1. 实现业务逻辑

```rust
use actr_framework::{{Context, ActorSystem}};
use actr_protocol::ActorResult;

pub struct MyService {{
    // 业务状态
}}

#[async_trait]
impl {handler_trait} for MyService {{
{example_method}
}}
```

### 2. 启动服务

```rust
#[tokio::main]
async fn main() -> ActorResult<()> {{
    let config = actr_config::Config::from_file("Actr.toml")?;
    let service = MyService {{ /* ... */ }};

    ActorSystem::new(config)?
        .attach(service)  // ← 自动获得 Workload + Dispatcher
        .start()
        .await?
        .wait_for_shutdown()
        .await
}}
```

## 架构说明

- **{handler_trait}**: 用户实现的业务逻辑接口
- **{}Dispatcher**: zero-sized type static dispatcher（自动生成）
- **Workload**: 通过 blanket impl 自动获得（自动生成）

用户只需实现 {handler_trait}，框架会自动提供路由和工作负载能力。
*/
"#,
            self.service_name
        ))
    }

    /// 生成客户端使用文档
    fn generate_client_usage_docs(&self, methods: &[MethodDescriptorProto]) -> Result<String> {
        let service_name_snake = self.service_name.to_snake_case();
        let method_name_snake = methods
            .first()
            .map(|m| m.name().to_snake_case())
            .unwrap_or("unknown_method".to_string());

        Ok(format!(
            r#"/*
## 客户端使用示例

```rust
use actr_framework::Context;
use actr_protocol::ActorResult;

async fn call_remote_service(ctx: &Context) -> ActorResult<()> {{
    use super::ContextExt;

    // 类型安全的远程调用
    let response = ctx.{service_name_snake}()
        .{method_name_snake}(request)
        .await?;

    Ok(())
}}
```

## 编译时路由

所有远程调用在编译时确定目标服务和方法，无需运行时查找。
*/
"#
        ))
    }

    /// 提取消息类型名称
    fn extract_message_type(&self, type_name: &str) -> Result<String> {
        let cleaned = type_name.trim_start_matches('.');
        if let Some(last_part) = cleaned.split('.').next_back() {
            Ok(last_part.to_string())
        } else {
            Ok(cleaned.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost_types::MethodDescriptorProto;

    #[test]
    fn test_extract_message_type() {
        let generator = ModernGenerator::new("test.v1", "TestService", GeneratorRole::ServerSide);

        assert_eq!(
            generator
                .extract_message_type(".test.v1.EchoRequest")
                .unwrap(),
            "EchoRequest"
        );
        assert_eq!(
            generator
                .extract_message_type("test.v1.EchoResponse")
                .unwrap(),
            "EchoResponse"
        );
        assert_eq!(
            generator.extract_message_type("SimpleMessage").unwrap(),
            "SimpleMessage"
        );
    }

    #[test]
    fn test_generate_message_impls_includes_payload_type() {
        let generator = ModernGenerator::new("test.v1", "TestService", GeneratorRole::ServerSide);

        let methods = vec![MethodDescriptorProto {
            name: Some("Echo".to_string()),
            input_type: Some(".test.v1.EchoRequest".to_string()),
            output_type: Some(".test.v1.EchoResponse".to_string()),
            options: None,
            ..Default::default()
        }];

        let result = generator.generate_message_impls(&methods).unwrap();

        // Debug: print generated code
        eprintln!("Generated code:\n{result}");

        // 验证生成的代码包含 payload_type() 方法
        assert!(
            result.contains("fn payload_type"),
            "Should contain 'fn payload_type'"
        );
        assert!(
            result.contains("PayloadType"),
            "Should contain 'PayloadType'"
        );
        // 验证默认值是 RpcReliable
        assert!(
            result.contains("RpcReliable"),
            "Should contain 'RpcReliable'"
        );
    }

    #[test]
    fn test_generate_imports_includes_payload_type() {
        let generator = ModernGenerator::new("test.v1", "TestService", GeneratorRole::ServerSide);
        let imports = generator.generate_imports();

        // 验证导入了 PayloadType
        assert!(imports.contains("PayloadType"));
        assert!(
            imports
                .contains("use actr_protocol::{ActorResult, RpcRequest, RpcEnvelope, PayloadType}")
        );
    }

    #[test]
    fn test_generate_client_code() {
        let generator = ModernGenerator::new("test.v1", "TestService", GeneratorRole::ClientSide);

        let methods = vec![MethodDescriptorProto {
            name: Some("Echo".to_string()),
            input_type: Some(".test.v1.EchoRequest".to_string()),
            output_type: Some(".test.v1.EchoResponse".to_string()),
            options: None,
            ..Default::default()
        }];

        let result = generator.generate(&methods);
        assert!(result.is_ok());

        let code = result.unwrap();
        // 客户端代码也应该包含 RpcRequest impl
        assert!(code.contains("impl RpcRequest for EchoRequest"));
        assert!(code.contains("fn payload_type() -> PayloadType"));
    }

    #[test]
    fn test_generate_server_code() {
        let generator = ModernGenerator::new("test.v1", "TestService", GeneratorRole::ServerSide);

        let methods = vec![MethodDescriptorProto {
            name: Some("Echo".to_string()),
            input_type: Some(".test.v1.EchoRequest".to_string()),
            output_type: Some(".test.v1.EchoResponse".to_string()),
            options: None,
            ..Default::default()
        }];

        let result = generator.generate(&methods);
        assert!(result.is_ok());

        let code = result.unwrap();
        // 验证生成了 Handler trait
        assert!(code.contains("pub trait TestServiceHandler"));
        // 验证生成了 Dispatcher
        assert!(code.contains("pub struct TestServiceDispatcher"));
        // 验证生成了 payload_type
        assert!(code.contains("fn payload_type() -> PayloadType"));
    }
}
