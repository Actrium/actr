use anyhow::Result;
use heck::ToSnakeCase;
use proc_macro2::{Ident, Span};
use prost_types::{FileDescriptorProto, MethodDescriptorProto};
use quote::{format_ident, quote};
use std::collections::HashSet;

/// File role determination based on proto file characteristics
#[derive(Debug, Clone, PartialEq)]
pub enum FileRole {
    /// Local service implementation - generates LocalActor trait and implementation
    LocalService,
    /// Remote service client - generates RemoteActor proxy and client code
    RemoteClient,
    /// Message types only - generates shared message types
    MessageTypes,
    /// Mixed service - generates both local and remote interfaces
    Mixed,
}

impl FileRole {
    /// Determine file role based on proto file structure and naming conventions
    pub fn from_proto_file(file: &FileDescriptorProto) -> Self {
        let file_name = file.name();
        let package_name = file.package();

        // File naming convention based role determination
        if file_name.contains("_service.proto") || file_name.ends_with("service.proto") {
            // Service definition files generate local implementations
            FileRole::LocalService
        } else if file_name.contains("_client.proto") || file_name.ends_with("client.proto") {
            // Client definition files generate remote proxies
            FileRole::RemoteClient
        } else if file.service.is_empty() {
            // Files with no services are message-only
            FileRole::MessageTypes
        } else if package_name.contains("local") || package_name.contains("service") {
            // Package naming convention for local services
            FileRole::LocalService
        } else if package_name.contains("client") || package_name.contains("remote") {
            // Package naming convention for remote clients
            FileRole::RemoteClient
        } else {
            // Default to mixed mode for backward compatibility
            FileRole::Mixed
        }
    }

    /// Check if this role should generate LocalActor implementations
    #[allow(dead_code)]
    pub fn generates_local_actor(&self) -> bool {
        matches!(self, FileRole::LocalService | FileRole::Mixed)
    }

    /// Check if this role should generate RemoteActor proxies
    #[allow(dead_code)]
    pub fn generates_remote_actor(&self) -> bool {
        matches!(self, FileRole::RemoteClient | FileRole::Mixed)
    }
}

/// Stream method classification for different generation strategies
#[derive(Debug, Clone, PartialEq)]
pub enum StreamType {
    /// Standard unary RPC: Request -> Response
    Unary,
    /// Client streaming: stream Request -> Response  
    ClientStream,
    /// Server streaming: Request -> stream Response
    ServerStream,
    /// Bidirectional streaming: stream Request -> stream Response
    BiDirectional,
}

impl StreamType {
    pub fn from_method(method: &MethodDescriptorProto) -> Self {
        let client_streaming = method.client_streaming.unwrap_or(false);
        let server_streaming = method.server_streaming.unwrap_or(false);

        match (client_streaming, server_streaming) {
            (false, false) => StreamType::Unary,
            (true, false) => StreamType::ClientStream,
            (false, true) => StreamType::ServerStream,
            (true, true) => StreamType::BiDirectional,
        }
    }

    #[allow(dead_code)]
    pub fn is_streaming(&self) -> bool {
        !matches!(self, StreamType::Unary)
    }
}

pub struct ActorTraitGenerator {
    _package_name: String,
    service_name: String,
    file_role: FileRole,
}

impl ActorTraitGenerator {
    pub fn new(package_name: &str, service_name: &str, file_role: FileRole) -> Self {
        Self {
            _package_name: package_name.to_string(),
            service_name: service_name.to_string(),
            file_role,
        }
    }

    pub fn generate(&self, methods: &[MethodDescriptorProto]) -> Result<String> {
        // Generate different code based on file role
        match self.file_role {
            FileRole::LocalService => self.generate_local_service_trait(methods),
            FileRole::RemoteClient => self.generate_remote_client_proxy(methods),
            FileRole::Mixed => {
                self.generate_mixed_file(methods)
            }
            FileRole::MessageTypes => {
                // For message-only files, generate minimal code
                Ok("// Message types only - no services defined".to_string())
            }
        }
    }

    fn generate_mixed_file(&self, methods: &[MethodDescriptorProto]) -> Result<String> {
        // Generate imports only once at the top
        let imports = self.generate_imports(methods);
        
        // Generate local trait content without imports
        let local_trait_content = self.generate_local_service_trait_content(methods)?;
        
        // Generate remote client content without imports  
        let remote_client_content = self.generate_remote_client_proxy_content(methods)?;
        
        // Generate adapter content without imports using content-only methods
        let adapter_generator = ActorAdapterGenerator::new(&self._package_name, &self.service_name, FileRole::Mixed);
        let adapter_content = adapter_generator.generate_local_service_adapter_content(methods)?;
        let manager_content = adapter_generator.generate_remote_client_manager_content(methods)?;
        
        let generated = quote! {
            #(#imports)*

            #local_trait_content

            #remote_client_content

            #adapter_content

            #manager_content
        };

        Ok(generated.to_string())
    }

    fn generate_local_service_trait_content(&self, methods: &[MethodDescriptorProto]) -> Result<proc_macro2::TokenStream> {
        let trait_name = format!("I{}", self.service_name);
        let trait_ident = Ident::new(&trait_name, Span::call_site());

        let mut method_tokens = Vec::new();
        let mut associated_types = Vec::new();

        for method in methods {
            let method_name = method.name().to_snake_case();
            let method_ident = Ident::new(&method_name, Span::call_site());
            let stream_type = StreamType::from_method(method);

            // 解析输入和输出类型
            let input_type = self.extract_message_type(method.input_type())?;
            let output_type = self.extract_message_type(method.output_type())?;

            let input_ident = Ident::new(&input_type, Span::call_site());
            let output_ident = Ident::new(&output_type, Span::call_site());

            // Generate LocalActor-compatible method signatures
            match stream_type {
                StreamType::Unary => {
                    method_tokens.push(quote! {
                        async fn #method_ident(
                            &self,
                            request: #input_ident,
                            context: std::sync::Arc<actor_rtc_framework::context::Context>,
                        ) -> actor_rtc_framework::error::ActorResult<#output_ident>;
                    });
                }
                StreamType::ServerStream => {
                    let stream_type_name = format!("{}Stream", method.name());
                    let stream_type_ident = Ident::new(&stream_type_name, Span::call_site());

                    associated_types.push(quote! {
                        type #stream_type_ident: futures_util::Stream<Item = actor_rtc_framework::error::ActorResult<#output_ident>> + Send + 'static;
                    });

                    method_tokens.push(quote! {
                        async fn #method_ident(
                            &self,
                            request: #input_ident,
                            context: std::sync::Arc<actor_rtc_framework::context::Context>,
                        ) -> actor_rtc_framework::error::ActorResult<Self::#stream_type_ident>;
                    });
                }
                _ => {
                    // For other stream types, generate placeholder methods
                    method_tokens.push(quote! {
                        async fn #method_ident(
                            &self,
                            _request: #input_ident,
                            _context: std::sync::Arc<actor_rtc_framework::context::Context>,
                        ) -> actor_rtc_framework::error::ActorResult<#output_ident> {
                            Err(actor_rtc_framework::error::ActorError::Protocol(
                                "Streaming method not yet implemented".to_string()
                            ))
                        }
                    });
                }
            }
        }

        let _imports = self.generate_imports(methods);

        let adapter_name = format!("{}Adapter", self.service_name);
        let _adapter_ident = Ident::new(&adapter_name, Span::call_site());

        let trait_content = quote! {
            /// Auto-generated LocalActor trait for #trait_ident
            #[async_trait::async_trait]
            pub trait #trait_ident: actor_rtc_framework::local_actor::LocalActor {
                #(#associated_types)*

                #(#method_tokens)*
            }
        };

        Ok(trait_content)
    }

    fn generate_local_service_trait(&self, methods: &[MethodDescriptorProto]) -> Result<String> {
        let imports = self.generate_imports(methods);
        let trait_content = self.generate_local_service_trait_content(methods)?;
        
        let generated = quote! {
            #(#imports)*

            #trait_content
        };

        Ok(generated.to_string())
    }

    fn generate_remote_client_proxy_content(&self, methods: &[MethodDescriptorProto]) -> Result<proc_macro2::TokenStream> {
        let client_name = format!("{}Client", self.service_name);
        let client_ident = Ident::new(&client_name, Span::call_site());

        let mut method_tokens = Vec::new();

        for method in methods {
            let method_name = method.name().to_snake_case();
            let method_ident = Ident::new(&method_name, Span::call_site());
            let stream_type = StreamType::from_method(method);

            let input_type = self.extract_message_type(method.input_type())?;
            let output_type = self.extract_message_type(method.output_type())?;

            let input_ident = Ident::new(&input_type, Span::call_site());
            let output_ident = Ident::new(&output_type, Span::call_site());

            // Generate RemoteActor-compatible methods
            match stream_type {
                StreamType::Unary => {
                    method_tokens.push(quote! {
                        pub async fn #method_ident(
                            &self,
                            request: #input_ident,
                        ) -> actor_rtc_framework::error::ActorResult<#output_ident> {
                            self.remote_actor.call(request).await
                        }
                    });
                }
                _ => {
                    // For streaming methods, use tell/notify pattern
                    method_tokens.push(quote! {
                        pub async fn #method_ident(
                            &self,
                            message: #input_ident,
                        ) -> actor_rtc_framework::error::ActorResult<()> {
                            self.remote_actor.tell(message).await
                        }
                    });
                }
            }
        }

        let client_content = quote! {
            /// Auto-generated RemoteActor client for #client_ident
            #[allow(dead_code)]
            pub struct #client_ident {
                remote_actor: actor_rtc_framework::remote_actor::RemoteActor,
            }

            #[allow(dead_code)]
            impl #client_ident {
                pub fn new(remote_actor: actor_rtc_framework::remote_actor::RemoteActor) -> Self {
                    Self { remote_actor }
                }

                pub async fn connect(&self) -> actor_rtc_framework::error::ActorResult<()> {
                    self.remote_actor.connect().await
                }

                pub async fn disconnect(&self) -> actor_rtc_framework::error::ActorResult<()> {
                    self.remote_actor.disconnect().await
                }

                #(#method_tokens)*
            }
        };

        Ok(client_content)
    }

    fn generate_remote_client_proxy(&self, methods: &[MethodDescriptorProto]) -> Result<String> {
        let imports = self.generate_imports(methods);
        let client_content = self.generate_remote_client_proxy_content(methods)?;
        
        let generated = quote! {
            #(#imports)*

            #client_content
        };

        Ok(generated.to_string())
    }

    fn generate_imports(&self, methods: &[MethodDescriptorProto]) -> Vec<proc_macro2::TokenStream> {
        let mut imports = Vec::new();

        // Collect all unique message types from methods
        let mut message_types = HashSet::new();
        for method in methods {
            let input_type = method.input_type().trim_start_matches('.');
            let output_type = method.output_type().trim_start_matches('.');
            message_types.insert(input_type);
            message_types.insert(output_type);
        }

        // Generate use statements for message types
        for msg_type in message_types {
            if let Some(type_name) = msg_type.split('.').last() {
                let type_ident = format_ident!("{}", type_name);
                imports.push(quote! {
                    use super::#type_ident;
                });
            }
        }

        imports
    }

    fn extract_message_type(&self, type_name: &str) -> Result<String> {
        // 移除包前缀，只保留类型名
        let cleaned = type_name.trim_start_matches('.');
        if let Some(last_part) = cleaned.split('.').last() {
            Ok(last_part.to_string())
        } else {
            Ok(cleaned.to_string())
        }
    }
}

pub struct ActorAdapterGenerator {
    _package_name: String,
    service_name: String,
    file_role: FileRole,
}

impl ActorAdapterGenerator {
    pub fn new(package_name: &str, service_name: &str, file_role: FileRole) -> Self {
        Self {
            _package_name: package_name.to_string(),
            service_name: service_name.to_string(),
            file_role,
        }
    }

    pub fn generate(&self, methods: &[MethodDescriptorProto]) -> Result<String> {
        match self.file_role {
            FileRole::LocalService => self.generate_local_service_adapter(methods),
            FileRole::RemoteClient => self.generate_remote_client_manager(methods),
            FileRole::Mixed => {
                // For Mixed mode, return empty string to avoid duplication
                // The mixed generation is handled by ActorServiceGenerator::generate_mixed_file
                Ok(String::new())
            }
            FileRole::MessageTypes => {
                // No adapters needed for message-only files
                Ok("// No adapters generated for message-only files".to_string())
            }
        }
    }

    pub fn generate_local_service_adapter_content(&self, methods: &[MethodDescriptorProto]) -> Result<proc_macro2::TokenStream> {
        let trait_name = format!("I{}", self.service_name);
        let adapter_name = format!("{}Adapter", self.service_name);

        let trait_ident = Ident::new(&trait_name, Span::call_site());
        let adapter_ident = Ident::new(&adapter_name, Span::call_site());

        // Check if there are any ServerStream methods (which generate associated types)
        let _has_associated_types = methods.iter().any(|method| {
            StreamType::from_method(method) == StreamType::ServerStream
        });

        // Generate route creation for each method
        let mut route_creation_tokens = Vec::new();
        
        for method in methods {
            let method_name = method.name();
            let method_ident = Ident::new(&method_name.to_snake_case(), Span::call_site());
            
            let input_type = method.input_type().trim_start_matches('.');
            let input_ident = Ident::new(&input_type.split('.').last().unwrap(), Span::call_site());
            
            let output_type = method.output_type().trim_start_matches('.');
            let output_ident = Ident::new(&output_type.split('.').last().unwrap(), Span::call_site());
            
            let full_method_name = format!("{}.{}/{}", 
                self._package_name.replace("_", "."), 
                self.service_name, 
                method_name);
                
            let stream_type = StreamType::from_method(method);
            
            match stream_type {
                StreamType::Unary => {
                    route_creation_tokens.push(quote! {
                        actor_rtc_framework::routing::Route {
                            method_name: #full_method_name.to_string(),
                            handler: {
                                let actor_clone = actor.clone();
                                Box::new(move |ctx: std::sync::Arc<actor_rtc_framework::context::Context>, req_bytes: Vec<u8>| {
                                    let actor_for_task = actor_clone.clone();
                                    Box::pin(async move {
                                        use prost::Message;
                                        let request = #input_ident::decode(&*req_bytes)
                                            .map_err(|e| actor_rtc_framework::error::ActorError::Protocol(
                                                format!("Failed to decode {}: {}", #input_type, e)
                                            ))?;
                                        let response: #output_ident = actor_for_task.#method_ident(request, ctx).await
                                            .map_err(|e| actor_rtc_framework::error::ActorError::Business(
                                                format!("Method {} failed: {:?}", #method_name, e)
                                            ))?;
                                        let mut buf = Vec::new();
                                        response.encode(&mut buf)
                                            .map_err(|e| actor_rtc_framework::error::ActorError::Protocol(
                                                format!("Failed to encode {}: {}", #output_type, e)
                                            ))?;
                                        Ok(buf)
                                    }) as std::pin::Pin<Box<dyn std::future::Future<Output = actor_rtc_framework::error::ActorResult<Vec<u8>>> + Send>>
                                }) as Box<dyn Fn(std::sync::Arc<actor_rtc_framework::context::Context>, Vec<u8>) -> std::pin::Pin<Box<dyn std::future::Future<Output = actor_rtc_framework::error::ActorResult<Vec<u8>>> + Send>> + Send + Sync>
                            },
                        }
                    });
                }
                StreamType::ClientStream => {
                    route_creation_tokens.push(quote! {
                        actor_rtc_framework::routing::Route {
                            method_name: #full_method_name.to_string(),
                            handler: {
                                let actor_clone = actor.clone();
                                Box::new(move |ctx: std::sync::Arc<actor_rtc_framework::context::Context>, req_bytes: Vec<u8>| {
                                    let actor_for_task = actor_clone.clone();
                                    Box::pin(async move {
                                        use prost::Message;
                                        let message = #input_ident::decode(&*req_bytes)
                                            .map_err(|e| actor_rtc_framework::error::ActorError::Protocol(
                                                format!("Failed to decode {}: {}", #input_type, e)
                                            ))?;
                                        actor_for_task.#method_ident(message, ctx).await
                                            .map_err(|e| actor_rtc_framework::error::ActorError::Business(
                                                format!("Streaming method {} failed: {:?}", #method_name, e)
                                            ))?;
                                        Ok(Vec::new())
                                    }) as std::pin::Pin<Box<dyn std::future::Future<Output = actor_rtc_framework::error::ActorResult<Vec<u8>>> + Send>>
                                }) as Box<dyn Fn(std::sync::Arc<actor_rtc_framework::context::Context>, Vec<u8>) -> std::pin::Pin<Box<dyn std::future::Future<Output = actor_rtc_framework::error::ActorResult<Vec<u8>>> + Send>> + Send + Sync>
                            },
                        }
                    });
                }
                _ => {
                    // Handle ServerStream, BidiStream similarly to ClientStream for now
                    route_creation_tokens.push(quote! {
                        actor_rtc_framework::routing::Route {
                            method_name: #full_method_name.to_string(),
                            handler: {
                                let actor_clone = actor.clone();
                                Box::new(move |ctx: std::sync::Arc<actor_rtc_framework::context::Context>, req_bytes: Vec<u8>| {
                                    let actor_for_task = actor_clone.clone();
                                    Box::pin(async move {
                                        use prost::Message;
                                        let message = #input_ident::decode(&*req_bytes)
                                            .map_err(|e| actor_rtc_framework::error::ActorError::Protocol(
                                                format!("Failed to decode {}: {}", #input_type, e)
                                            ))?;
                                        actor_for_task.#method_ident(message, ctx).await
                                            .map_err(|e| actor_rtc_framework::error::ActorError::Business(
                                                format!("Streaming method {} failed: {:?}", #method_name, e)
                                            ))?;
                                        Ok(Vec::new())
                                    }) as std::pin::Pin<Box<dyn std::future::Future<Output = actor_rtc_framework::error::ActorResult<Vec<u8>>> + Send>>
                                }) as Box<dyn Fn(std::sync::Arc<actor_rtc_framework::context::Context>, Vec<u8>) -> std::pin::Pin<Box<dyn std::future::Future<Output = actor_rtc_framework::error::ActorResult<Vec<u8>>> + Send>> + Send + Sync>
                            },
                        }
                    });
                }
            }
        }

        let content = quote! {
            /// Auto-generated LocalActor adapter for #trait_ident
            pub struct #adapter_ident;

            impl actor_rtc_framework::routing::RouteProvider<dyn #trait_ident> for #adapter_ident {
                fn get_routes(actor: std::sync::Arc<dyn #trait_ident>) -> Vec<actor_rtc_framework::routing::Route> {
                    vec![
                        #(#route_creation_tokens,)*
                    ]
                }
            }

            /// Blanket RouteProvider implementation for concrete types that implement the trait
            impl<T> actor_rtc_framework::routing::RouteProvider<T> for #adapter_ident 
            where 
                T: #trait_ident + Send + Sync + 'static,
            {
                fn get_routes(actor: std::sync::Arc<T>) -> Vec<actor_rtc_framework::routing::Route> {
                    let trait_obj: std::sync::Arc<dyn #trait_ident> = actor;
                    Self::get_routes(trait_obj)
                }
            }
        };

        Ok(content)
    }

    pub fn generate_remote_client_manager_content(&self, methods: &[MethodDescriptorProto]) -> Result<proc_macro2::TokenStream> {
        let manager_name = format!("{}ClientManager", self.service_name);
        let manager_ident = Ident::new(&manager_name, Span::call_site());

        // Generate method implementations for client manager
        let mut method_tokens = Vec::new();
        
        for method in methods {
            let method_name = method.name();
            let method_ident = Ident::new(&method_name.to_snake_case(), Span::call_site());
            
            let input_type = method.input_type().trim_start_matches('.');
            let input_ident = Ident::new(&input_type.split('.').last().unwrap(), Span::call_site());
            
            let output_type = method.output_type().trim_start_matches('.');
            let output_ident = Ident::new(&output_type.split('.').last().unwrap(), Span::call_site());
            
            let stream_type = StreamType::from_method(method);
            
            match stream_type {
                StreamType::Unary => {
                    method_tokens.push(quote! {
                        pub async fn #method_ident(
                            &self,
                            actor_id: &shared_protocols::actor::ActorId,
                            request: #input_ident,
                        ) -> actor_rtc_framework::error::ActorResult<#output_ident> {
                            if let Some(remote_actor) = {
                                let manager_guard = self.manager.read().await;
                                manager_guard.get_remote_actor(actor_id).cloned()
                            } {
                                remote_actor.call(request).await
                            } else {
                                Err(actor_rtc_framework::error::ActorError::ActorNotFound {
                                    actor_id: format!("{}", actor_id.serial_number),
                                })
                            }
                        }
                    });
                }
                StreamType::ClientStream => {
                    method_tokens.push(quote! {
                        pub async fn #method_ident(
                            &self,
                            actor_id: &shared_protocols::actor::ActorId,
                            message: #input_ident,
                        ) -> actor_rtc_framework::error::ActorResult<()> {
                            if let Some(remote_actor) = {
                                let manager_guard = self.manager.read().await;
                                manager_guard.get_remote_actor(actor_id).cloned()
                            } {
                                remote_actor.tell(message).await
                            } else {
                                Err(actor_rtc_framework::error::ActorError::ActorNotFound {
                                    actor_id: format!("{}", actor_id.serial_number),
                                })
                            }
                        }
                    });
                }
                _ => {
                    // Handle ServerStream, BidiStream similarly to ClientStream
                    method_tokens.push(quote! {
                        pub async fn #method_ident(
                            &self,
                            actor_id: &shared_protocols::actor::ActorId,
                            message: #input_ident,
                        ) -> actor_rtc_framework::error::ActorResult<()> {
                            if let Some(remote_actor) = {
                                let manager_guard = self.manager.read().await;
                                manager_guard.get_remote_actor(actor_id).cloned()
                            } {
                                remote_actor.tell(message).await
                            } else {
                                Err(actor_rtc_framework::error::ActorError::ActorNotFound {
                                    actor_id: format!("{}", actor_id.serial_number),
                                })
                            }
                        }
                    });
                }
            }
        }

        let content = quote! {
            /// Auto-generated RemoteActor client manager for #manager_ident
            #[allow(dead_code)]
            pub struct #manager_ident {
                manager: std::sync::Arc<tokio::sync::RwLock<actor_rtc_framework::remote_actor::RemoteActorManager>>,
            }

            #[allow(dead_code)]
            impl #manager_ident {
                pub fn new(context: std::sync::Arc<actor_rtc_framework::context::Context>) -> Self {
                    Self {
                        manager: std::sync::Arc::new(tokio::sync::RwLock::new(
                            actor_rtc_framework::remote_actor::RemoteActorManager::new(context)
                        )),
                    }
                }

                pub async fn register_remote_actor(
                    &self,
                    actor_id: shared_protocols::actor::ActorId,
                    service_address: Option<String>,
                ) -> actor_rtc_framework::error::ActorResult<()> {
                    let mut manager = self.manager.write().await;
                    manager.register_remote_actor(
                        actor_id,
                        stringify!(#manager_ident).to_string(),
                        service_address,
                    )
                }

                #(#method_tokens)*
            }
        };

        Ok(content)
    }

    fn generate_local_service_adapter(&self, methods: &[MethodDescriptorProto]) -> Result<String> {
        let trait_name = format!("I{}", self.service_name);
        let adapter_name = format!("{}Adapter", self.service_name);

        let trait_ident = Ident::new(&trait_name, Span::call_site());
        let adapter_ident = Ident::new(&adapter_name, Span::call_site());

        // Check if there are any ServerStream methods (which generate associated types)
        let has_associated_types = methods.iter().any(|method| {
            StreamType::from_method(method) == StreamType::ServerStream
        });

        // Generate route creation for each method
        let mut route_creation_tokens = Vec::new();

        for method in methods {
            let method_name = method.name().to_snake_case();
            let method_ident = Ident::new(&method_name, Span::call_site());
            let stream_type = StreamType::from_method(method);
            
            // Create the full method name for routing (e.g., "echo.EchoService/SendEcho")
            let full_method_name = format!("{}.{}/{}", self._package_name, self.service_name, method.name());

            let input_type = self.extract_message_type(method.input_type())?;
            let output_type = self.extract_message_type(method.output_type())?;
            
            let input_ident = Ident::new(&input_type, Span::call_site());
            let output_ident = Ident::new(&output_type, Span::call_site());

            // Generate route handler for each method
            match stream_type {
                StreamType::Unary => {
                    route_creation_tokens.push(quote! {
                        actor_rtc_framework::routing::Route {
                            method_name: #full_method_name.to_string(),
                            handler: {
                                let actor_clone = actor.clone();
                                Box::new(move |ctx: std::sync::Arc<actor_rtc_framework::context::Context>, 
                                               req_bytes: Vec<u8>| {
                                    let actor_for_task = actor_clone.clone();
                                    Box::pin(async move {
                                        use prost::Message;
                                        
                                        // Deserialize request
                                        let request = #input_ident::decode(&*req_bytes)
                                            .map_err(|e| actor_rtc_framework::error::ActorError::Protocol(
                                                format!("Failed to decode {}: {}", #input_type, e)
                                            ))?;

                                        // Call the trait method
                                        let response: #output_ident = actor_for_task.#method_ident(request, ctx).await
                                            .map_err(|e| actor_rtc_framework::error::ActorError::Business(
                                                format!("Method {} failed: {:?}", #method_name, e)
                                            ))?;

                                        // Serialize response
                                        let mut buf = Vec::new();
                                        response.encode(&mut buf)
                                            .map_err(|e| actor_rtc_framework::error::ActorError::Protocol(
                                                format!("Failed to encode {}: {}", #output_type, e)
                                            ))?;
                                        
                                        Ok(buf)
                                    }) as std::pin::Pin<Box<dyn std::future::Future<Output = actor_rtc_framework::error::ActorResult<Vec<u8>>> + Send>>
                                }) as Box<dyn Fn(std::sync::Arc<actor_rtc_framework::context::Context>, Vec<u8>) 
                                    -> std::pin::Pin<Box<dyn std::future::Future<Output = actor_rtc_framework::error::ActorResult<Vec<u8>>> + Send>> + Send + Sync>
                            },
                        }
                    });
                }
                _ => {
                    // For streaming methods, generate tell-based handlers
                    route_creation_tokens.push(quote! {
                        actor_rtc_framework::routing::Route {
                            method_name: #full_method_name.to_string(),
                            handler: {
                                let actor_clone = actor.clone();
                                Box::new(move |ctx: std::sync::Arc<actor_rtc_framework::context::Context>, 
                                               req_bytes: Vec<u8>| {
                                    let actor_for_task = actor_clone.clone();
                                    Box::pin(async move {
                                        use prost::Message;
                                        
                                        // Deserialize message
                                        let message = #input_ident::decode(&*req_bytes)
                                            .map_err(|e| actor_rtc_framework::error::ActorError::Protocol(
                                                format!("Failed to decode {}: {}", #input_type, e)
                                            ))?;

                                        // Call the trait method (streaming methods typically return Result<(), Status>)
                                        actor_for_task.#method_ident(message, ctx).await
                                            .map_err(|e| actor_rtc_framework::error::ActorError::Business(
                                                format!("Streaming method {} failed: {:?}", #method_name, e)
                                            ))?;

                                        // Return empty response for streaming/tell operations
                                        Ok(Vec::new())
                                    }) as std::pin::Pin<Box<dyn std::future::Future<Output = actor_rtc_framework::error::ActorResult<Vec<u8>>> + Send>>
                                }) as Box<dyn Fn(std::sync::Arc<actor_rtc_framework::context::Context>, Vec<u8>) 
                                    -> std::pin::Pin<Box<dyn std::future::Future<Output = actor_rtc_framework::error::ActorResult<Vec<u8>>> + Send>> + Send + Sync>
                            },
                        }
                    });
                }
            }
        }

        // Generate RouteProvider and AttachableActor implementations
        let generated = if has_associated_types {
            // For traits with associated types, use a generic implementation
            quote! {
                /// Auto-generated LocalActor adapter for #trait_ident
                pub struct #adapter_ident;

                impl<T> actor_rtc_framework::routing::RouteProvider<T> for #adapter_ident 
                where 
                    T: #trait_ident + Send + Sync + 'static 
                {
                    fn get_routes(actor: std::sync::Arc<T>) -> Vec<actor_rtc_framework::routing::Route> {
                        vec![
                            #(#route_creation_tokens),*
                        ]
                    }
                }

            }
        } else {
            // For traits without associated types, use trait object
            quote! {
                /// Auto-generated LocalActor adapter for #trait_ident
                pub struct #adapter_ident;

                impl actor_rtc_framework::routing::RouteProvider<dyn #trait_ident> for #adapter_ident {
                    fn get_routes(actor: std::sync::Arc<dyn #trait_ident>) -> Vec<actor_rtc_framework::routing::Route> {
                        vec![
                            #(#route_creation_tokens),*
                        ]
                    }
                }

            }
        };

        Ok(generated.to_string())
    }

    fn generate_remote_client_manager(&self, methods: &[MethodDescriptorProto]) -> Result<String> {
        let manager_name = format!("{}ClientManager", self.service_name);
        let manager_ident = Ident::new(&manager_name, Span::call_site());

        let mut method_tokens = Vec::new();

        for method in methods {
            let method_name = method.name().to_snake_case();
            let method_ident = Ident::new(&method_name, Span::call_site());
            let stream_type = StreamType::from_method(method);

            let input_type = self.extract_message_type(method.input_type())?;
            let output_type = self.extract_message_type(method.output_type())?;

            let input_ident = Ident::new(&input_type, Span::call_site());
            let output_ident = Ident::new(&output_type, Span::call_site());

            // Generate methods that work with RemoteActor
            match stream_type {
                StreamType::Unary => {
                    method_tokens.push(quote! {
                        pub async fn #method_ident(
                            &self,
                            actor_id: &shared_protocols::actor::ActorId,
                            request: #input_ident,
                        ) -> actor_rtc_framework::error::ActorResult<#output_ident> {
                            if let Some(remote_actor) = {
                                let manager_guard = self.manager.read().await;
                                manager_guard.get_remote_actor(actor_id).cloned()
                            } {
                                remote_actor.call(request).await
                            } else {
                                Err(actor_rtc_framework::error::ActorError::ActorNotFound {
                                    actor_id: format!("{}", actor_id.serial_number),
                                })
                            }
                        }
                    });
                }
                _ => {
                    method_tokens.push(quote! {
                        pub async fn #method_ident(
                            &self,
                            actor_id: &shared_protocols::actor::ActorId,
                            message: #input_ident,
                        ) -> actor_rtc_framework::error::ActorResult<()> {
                            if let Some(remote_actor) = {
                                let manager_guard = self.manager.read().await;
                                manager_guard.get_remote_actor(actor_id).cloned()
                            } {
                                remote_actor.tell(message).await
                            } else {
                                Err(actor_rtc_framework::error::ActorError::ActorNotFound {
                                    actor_id: format!("{}", actor_id.serial_number),
                                })
                            }
                        }
                    });
                }
            }
        }

        let generated = quote! {
            /// Auto-generated RemoteActor client manager for #manager_ident
            #[allow(dead_code)]
            pub struct #manager_ident {
                manager: std::sync::Arc<tokio::sync::RwLock<actor_rtc_framework::remote_actor::RemoteActorManager>>,
            }

            #[allow(dead_code)]
            impl #manager_ident {
                pub fn new(context: std::sync::Arc<actor_rtc_framework::context::Context>) -> Self {
                    Self {
                        manager: std::sync::Arc::new(tokio::sync::RwLock::new(
                            actor_rtc_framework::remote_actor::RemoteActorManager::new(context)
                        )),
                    }
                }

                pub async fn register_remote_actor(
                    &self,
                    actor_id: shared_protocols::actor::ActorId,
                    service_address: Option<String>,
                ) -> actor_rtc_framework::error::ActorResult<()> {
                    let mut manager = self.manager.write().await;
                    manager.register_remote_actor(
                        actor_id,
                        stringify!(#manager_ident).to_string(),
                        service_address,
                    )
                }

                #(#method_tokens)*
            }
        };

        Ok(generated.to_string())
    }


    fn extract_message_type(&self, type_name: &str) -> Result<String> {
        // 移除包前缀，只保留类型名
        let cleaned = type_name.trim_start_matches('.');
        if let Some(last_part) = cleaned.split('.').last() {
            Ok(last_part.to_string())
        } else {
            Ok(cleaned.to_string())
        }
    }
}
