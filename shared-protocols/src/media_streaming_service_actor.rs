use super::ListStreamsRequest;
use super::ListStreamsResponse;
use super::PublishStreamRequest;
use super::PublishStreamResponse;
use super::StopStreamRequest;
use super::StopStreamResponse;
use super::SubscribeStreamRequest;
use super::SubscribeStreamResponse;
/// Auto-generated LocalActor trait for #trait_ident
#[async_trait::async_trait]
pub trait IMediaStreamingService: actor_rtc_framework::local_actor::LocalActor {
    async fn publish_stream(
        &self,
        request: PublishStreamRequest,
        context: std::sync::Arc<actor_rtc_framework::context::Context>,
    ) -> actor_rtc_framework::error::ActorResult<PublishStreamResponse>;
    async fn subscribe_stream(
        &self,
        request: SubscribeStreamRequest,
        context: std::sync::Arc<actor_rtc_framework::context::Context>,
    ) -> actor_rtc_framework::error::ActorResult<SubscribeStreamResponse>;
    async fn stop_publishing(
        &self,
        request: StopStreamRequest,
        context: std::sync::Arc<actor_rtc_framework::context::Context>,
    ) -> actor_rtc_framework::error::ActorResult<StopStreamResponse>;
    async fn stop_subscribing(
        &self,
        request: StopStreamRequest,
        context: std::sync::Arc<actor_rtc_framework::context::Context>,
    ) -> actor_rtc_framework::error::ActorResult<StopStreamResponse>;
    async fn list_streams(
        &self,
        request: ListStreamsRequest,
        context: std::sync::Arc<actor_rtc_framework::context::Context>,
    ) -> actor_rtc_framework::error::ActorResult<ListStreamsResponse>;
}
/// Auto-generated RemoteActor client for #client_ident
#[allow(dead_code)]
pub struct MediaStreamingServiceClient {
    remote_actor: actor_rtc_framework::remote_actor::RemoteActor,
}
#[allow(dead_code)]
impl MediaStreamingServiceClient {
    pub fn new(remote_actor: actor_rtc_framework::remote_actor::RemoteActor) -> Self {
        Self { remote_actor }
    }
    pub async fn connect(&self) -> actor_rtc_framework::error::ActorResult<()> {
        self.remote_actor.connect().await
    }
    pub async fn disconnect(&self) -> actor_rtc_framework::error::ActorResult<()> {
        self.remote_actor.disconnect().await
    }
    pub async fn publish_stream(
        &self,
        request: PublishStreamRequest,
    ) -> actor_rtc_framework::error::ActorResult<PublishStreamResponse> {
        self.remote_actor.call(request).await
    }
    pub async fn subscribe_stream(
        &self,
        request: SubscribeStreamRequest,
    ) -> actor_rtc_framework::error::ActorResult<SubscribeStreamResponse> {
        self.remote_actor.call(request).await
    }
    pub async fn stop_publishing(
        &self,
        request: StopStreamRequest,
    ) -> actor_rtc_framework::error::ActorResult<StopStreamResponse> {
        self.remote_actor.call(request).await
    }
    pub async fn stop_subscribing(
        &self,
        request: StopStreamRequest,
    ) -> actor_rtc_framework::error::ActorResult<StopStreamResponse> {
        self.remote_actor.call(request).await
    }
    pub async fn list_streams(
        &self,
        request: ListStreamsRequest,
    ) -> actor_rtc_framework::error::ActorResult<ListStreamsResponse> {
        self.remote_actor.call(request).await
    }
}
/// Auto-generated LocalActor adapter for #trait_ident
pub struct MediaStreamingServiceAdapter;
impl actor_rtc_framework::routing::RouteProvider<dyn IMediaStreamingService>
    for MediaStreamingServiceAdapter
{
    fn get_routes(
        actor: std::sync::Arc<dyn IMediaStreamingService>,
    ) -> Vec<actor_rtc_framework::routing::Route> {
        vec![
            actor_rtc_framework::routing::Route {
                method_name: "media.streaming.MediaStreamingService/PublishStream".to_string(),
                handler: {
                    let actor_clone = actor.clone();
                    Box::new(
                        move |ctx: std::sync::Arc<actor_rtc_framework::context::Context>,
                              req_bytes: Vec<u8>| {
                            let actor_for_task = actor_clone.clone();
                            Box::pin(async move {
                                use prost::Message;
                                let request =
                                    PublishStreamRequest::decode(&*req_bytes).map_err(|e| {
                                        actor_rtc_framework::error::ActorError::Protocol(format!(
                                            "Failed to decode {}: {}",
                                            "media_streaming.PublishStreamRequest", e
                                        ))
                                    })?;
                                let response: PublishStreamResponse = actor_for_task
                                    .publish_stream(request, ctx)
                                    .await
                                    .map_err(|e| {
                                        actor_rtc_framework::error::ActorError::Business(format!(
                                            "Method {} failed: {:?}",
                                            "PublishStream", e
                                        ))
                                    })?;
                                let mut buf = Vec::new();
                                response.encode(&mut buf).map_err(|e| {
                                    actor_rtc_framework::error::ActorError::Protocol(format!(
                                        "Failed to encode {}: {}",
                                        "media_streaming.PublishStreamResponse", e
                                    ))
                                })?;
                                Ok(buf)
                            })
                                as std::pin::Pin<
                                    Box<
                                        dyn std::future::Future<
                                                Output = actor_rtc_framework::error::ActorResult<
                                                    Vec<u8>,
                                                >,
                                            > + Send,
                                    >,
                                >
                        },
                    )
                        as Box<
                            dyn Fn(
                                    std::sync::Arc<actor_rtc_framework::context::Context>,
                                    Vec<u8>,
                                ) -> std::pin::Pin<
                                    Box<
                                        dyn std::future::Future<
                                                Output = actor_rtc_framework::error::ActorResult<
                                                    Vec<u8>,
                                                >,
                                            > + Send,
                                    >,
                                > + Send
                                + Sync,
                        >
                },
            },
            actor_rtc_framework::routing::Route {
                method_name: "media.streaming.MediaStreamingService/SubscribeStream".to_string(),
                handler: {
                    let actor_clone = actor.clone();
                    Box::new(
                        move |ctx: std::sync::Arc<actor_rtc_framework::context::Context>,
                              req_bytes: Vec<u8>| {
                            let actor_for_task = actor_clone.clone();
                            Box::pin(async move {
                                use prost::Message;
                                let request =
                                    SubscribeStreamRequest::decode(&*req_bytes).map_err(|e| {
                                        actor_rtc_framework::error::ActorError::Protocol(format!(
                                            "Failed to decode {}: {}",
                                            "media_streaming.SubscribeStreamRequest", e
                                        ))
                                    })?;
                                let response: SubscribeStreamResponse = actor_for_task
                                    .subscribe_stream(request, ctx)
                                    .await
                                    .map_err(|e| {
                                        actor_rtc_framework::error::ActorError::Business(format!(
                                            "Method {} failed: {:?}",
                                            "SubscribeStream", e
                                        ))
                                    })?;
                                let mut buf = Vec::new();
                                response.encode(&mut buf).map_err(|e| {
                                    actor_rtc_framework::error::ActorError::Protocol(format!(
                                        "Failed to encode {}: {}",
                                        "media_streaming.SubscribeStreamResponse", e
                                    ))
                                })?;
                                Ok(buf)
                            })
                                as std::pin::Pin<
                                    Box<
                                        dyn std::future::Future<
                                                Output = actor_rtc_framework::error::ActorResult<
                                                    Vec<u8>,
                                                >,
                                            > + Send,
                                    >,
                                >
                        },
                    )
                        as Box<
                            dyn Fn(
                                    std::sync::Arc<actor_rtc_framework::context::Context>,
                                    Vec<u8>,
                                ) -> std::pin::Pin<
                                    Box<
                                        dyn std::future::Future<
                                                Output = actor_rtc_framework::error::ActorResult<
                                                    Vec<u8>,
                                                >,
                                            > + Send,
                                    >,
                                > + Send
                                + Sync,
                        >
                },
            },
            actor_rtc_framework::routing::Route {
                method_name: "media.streaming.MediaStreamingService/StopPublishing".to_string(),
                handler: {
                    let actor_clone = actor.clone();
                    Box::new(
                        move |ctx: std::sync::Arc<actor_rtc_framework::context::Context>,
                              req_bytes: Vec<u8>| {
                            let actor_for_task = actor_clone.clone();
                            Box::pin(async move {
                                use prost::Message;
                                let request =
                                    StopStreamRequest::decode(&*req_bytes).map_err(|e| {
                                        actor_rtc_framework::error::ActorError::Protocol(format!(
                                            "Failed to decode {}: {}",
                                            "media_streaming.StopStreamRequest", e
                                        ))
                                    })?;
                                let response: StopStreamResponse = actor_for_task
                                    .stop_publishing(request, ctx)
                                    .await
                                    .map_err(|e| {
                                        actor_rtc_framework::error::ActorError::Business(format!(
                                            "Method {} failed: {:?}",
                                            "StopPublishing", e
                                        ))
                                    })?;
                                let mut buf = Vec::new();
                                response.encode(&mut buf).map_err(|e| {
                                    actor_rtc_framework::error::ActorError::Protocol(format!(
                                        "Failed to encode {}: {}",
                                        "media_streaming.StopStreamResponse", e
                                    ))
                                })?;
                                Ok(buf)
                            })
                                as std::pin::Pin<
                                    Box<
                                        dyn std::future::Future<
                                                Output = actor_rtc_framework::error::ActorResult<
                                                    Vec<u8>,
                                                >,
                                            > + Send,
                                    >,
                                >
                        },
                    )
                        as Box<
                            dyn Fn(
                                    std::sync::Arc<actor_rtc_framework::context::Context>,
                                    Vec<u8>,
                                ) -> std::pin::Pin<
                                    Box<
                                        dyn std::future::Future<
                                                Output = actor_rtc_framework::error::ActorResult<
                                                    Vec<u8>,
                                                >,
                                            > + Send,
                                    >,
                                > + Send
                                + Sync,
                        >
                },
            },
            actor_rtc_framework::routing::Route {
                method_name: "media.streaming.MediaStreamingService/StopSubscribing".to_string(),
                handler: {
                    let actor_clone = actor.clone();
                    Box::new(
                        move |ctx: std::sync::Arc<actor_rtc_framework::context::Context>,
                              req_bytes: Vec<u8>| {
                            let actor_for_task = actor_clone.clone();
                            Box::pin(async move {
                                use prost::Message;
                                let request =
                                    StopStreamRequest::decode(&*req_bytes).map_err(|e| {
                                        actor_rtc_framework::error::ActorError::Protocol(format!(
                                            "Failed to decode {}: {}",
                                            "media_streaming.StopStreamRequest", e
                                        ))
                                    })?;
                                let response: StopStreamResponse = actor_for_task
                                    .stop_subscribing(request, ctx)
                                    .await
                                    .map_err(|e| {
                                        actor_rtc_framework::error::ActorError::Business(format!(
                                            "Method {} failed: {:?}",
                                            "StopSubscribing", e
                                        ))
                                    })?;
                                let mut buf = Vec::new();
                                response.encode(&mut buf).map_err(|e| {
                                    actor_rtc_framework::error::ActorError::Protocol(format!(
                                        "Failed to encode {}: {}",
                                        "media_streaming.StopStreamResponse", e
                                    ))
                                })?;
                                Ok(buf)
                            })
                                as std::pin::Pin<
                                    Box<
                                        dyn std::future::Future<
                                                Output = actor_rtc_framework::error::ActorResult<
                                                    Vec<u8>,
                                                >,
                                            > + Send,
                                    >,
                                >
                        },
                    )
                        as Box<
                            dyn Fn(
                                    std::sync::Arc<actor_rtc_framework::context::Context>,
                                    Vec<u8>,
                                ) -> std::pin::Pin<
                                    Box<
                                        dyn std::future::Future<
                                                Output = actor_rtc_framework::error::ActorResult<
                                                    Vec<u8>,
                                                >,
                                            > + Send,
                                    >,
                                > + Send
                                + Sync,
                        >
                },
            },
            actor_rtc_framework::routing::Route {
                method_name: "media.streaming.MediaStreamingService/ListStreams".to_string(),
                handler: {
                    let actor_clone = actor.clone();
                    Box::new(
                        move |ctx: std::sync::Arc<actor_rtc_framework::context::Context>,
                              req_bytes: Vec<u8>| {
                            let actor_for_task = actor_clone.clone();
                            Box::pin(async move {
                                use prost::Message;
                                let request =
                                    ListStreamsRequest::decode(&*req_bytes).map_err(|e| {
                                        actor_rtc_framework::error::ActorError::Protocol(format!(
                                            "Failed to decode {}: {}",
                                            "media_streaming.ListStreamsRequest", e
                                        ))
                                    })?;
                                let response: ListStreamsResponse =
                                    actor_for_task.list_streams(request, ctx).await.map_err(
                                        |e| {
                                            actor_rtc_framework::error::ActorError::Business(
                                                format!("Method {} failed: {:?}", "ListStreams", e),
                                            )
                                        },
                                    )?;
                                let mut buf = Vec::new();
                                response.encode(&mut buf).map_err(|e| {
                                    actor_rtc_framework::error::ActorError::Protocol(format!(
                                        "Failed to encode {}: {}",
                                        "media_streaming.ListStreamsResponse", e
                                    ))
                                })?;
                                Ok(buf)
                            })
                                as std::pin::Pin<
                                    Box<
                                        dyn std::future::Future<
                                                Output = actor_rtc_framework::error::ActorResult<
                                                    Vec<u8>,
                                                >,
                                            > + Send,
                                    >,
                                >
                        },
                    )
                        as Box<
                            dyn Fn(
                                    std::sync::Arc<actor_rtc_framework::context::Context>,
                                    Vec<u8>,
                                ) -> std::pin::Pin<
                                    Box<
                                        dyn std::future::Future<
                                                Output = actor_rtc_framework::error::ActorResult<
                                                    Vec<u8>,
                                                >,
                                            > + Send,
                                    >,
                                > + Send
                                + Sync,
                        >
                },
            },
        ]
    }
}
/// Blanket RouteProvider implementation for concrete types that implement the trait
impl<T> actor_rtc_framework::routing::RouteProvider<T> for MediaStreamingServiceAdapter
where
    T: IMediaStreamingService + Send + Sync + 'static,
{
    fn get_routes(actor: std::sync::Arc<T>) -> Vec<actor_rtc_framework::routing::Route> {
        let trait_obj: std::sync::Arc<dyn IMediaStreamingService> = actor;
        Self::get_routes(trait_obj)
    }
}
/// Auto-generated RemoteActor client manager for #manager_ident
#[allow(dead_code)]
pub struct MediaStreamingServiceClientManager {
    manager:
        std::sync::Arc<tokio::sync::RwLock<actor_rtc_framework::remote_actor::RemoteActorManager>>,
}
#[allow(dead_code)]
impl MediaStreamingServiceClientManager {
    pub fn new(context: std::sync::Arc<actor_rtc_framework::context::Context>) -> Self {
        Self {
            manager: std::sync::Arc::new(tokio::sync::RwLock::new(
                actor_rtc_framework::remote_actor::RemoteActorManager::new(context),
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
            stringify!(MediaStreamingServiceClientManager).to_string(),
            service_address,
        )
    }
    pub async fn publish_stream(
        &self,
        actor_id: &shared_protocols::actor::ActorId,
        request: PublishStreamRequest,
    ) -> actor_rtc_framework::error::ActorResult<PublishStreamResponse> {
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
    pub async fn subscribe_stream(
        &self,
        actor_id: &shared_protocols::actor::ActorId,
        request: SubscribeStreamRequest,
    ) -> actor_rtc_framework::error::ActorResult<SubscribeStreamResponse> {
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
    pub async fn stop_publishing(
        &self,
        actor_id: &shared_protocols::actor::ActorId,
        request: StopStreamRequest,
    ) -> actor_rtc_framework::error::ActorResult<StopStreamResponse> {
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
    pub async fn stop_subscribing(
        &self,
        actor_id: &shared_protocols::actor::ActorId,
        request: StopStreamRequest,
    ) -> actor_rtc_framework::error::ActorResult<StopStreamResponse> {
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
    pub async fn list_streams(
        &self,
        actor_id: &shared_protocols::actor::ActorId,
        request: ListStreamsRequest,
    ) -> actor_rtc_framework::error::ActorResult<ListStreamsResponse> {
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
}
