use super::CancelTransferRequest;
use super::CancelTransferResponse;
use super::FileChunk;
use super::GetTransferStatusRequest;
use super::RetransmitRequest;
use super::RetransmitResponse;
use super::StartTransferRequest;
use super::StartTransferResponse;
use super::TransferProgress;
use super::TransferStatus;
/// Auto-generated LocalActor trait for #trait_ident
#[async_trait::async_trait]
pub trait IFileTransferService: actor_rtc_framework::local_actor::LocalActor {
    async fn start_transfer(
        &self,
        request: StartTransferRequest,
        context: std::sync::Arc<actor_rtc_framework::context::Context>,
    ) -> actor_rtc_framework::error::ActorResult<StartTransferResponse>;
    async fn send_chunk(
        &self,
        request: FileChunk,
        context: std::sync::Arc<actor_rtc_framework::context::Context>,
    ) -> actor_rtc_framework::error::ActorResult<TransferProgress>;
    async fn request_retransmit(
        &self,
        request: RetransmitRequest,
        context: std::sync::Arc<actor_rtc_framework::context::Context>,
    ) -> actor_rtc_framework::error::ActorResult<RetransmitResponse>;
    async fn get_transfer_status(
        &self,
        request: GetTransferStatusRequest,
        context: std::sync::Arc<actor_rtc_framework::context::Context>,
    ) -> actor_rtc_framework::error::ActorResult<TransferStatus>;
    async fn cancel_transfer(
        &self,
        request: CancelTransferRequest,
        context: std::sync::Arc<actor_rtc_framework::context::Context>,
    ) -> actor_rtc_framework::error::ActorResult<CancelTransferResponse>;
    async fn progress_stream(
        &self,
        _request: TransferProgress,
        _context: std::sync::Arc<actor_rtc_framework::context::Context>,
    ) -> actor_rtc_framework::error::ActorResult<TransferProgress> {
        Err(actor_rtc_framework::error::ActorError::Protocol(
            "Streaming method not yet implemented".to_string(),
        ))
    }
}
/// Auto-generated RemoteActor client for #client_ident
#[allow(dead_code)]
pub struct FileTransferServiceClient {
    remote_actor: actor_rtc_framework::remote_actor::RemoteActor,
}
#[allow(dead_code)]
impl FileTransferServiceClient {
    pub fn new(remote_actor: actor_rtc_framework::remote_actor::RemoteActor) -> Self {
        Self { remote_actor }
    }
    pub async fn connect(&self) -> actor_rtc_framework::error::ActorResult<()> {
        self.remote_actor.connect().await
    }
    pub async fn disconnect(&self) -> actor_rtc_framework::error::ActorResult<()> {
        self.remote_actor.disconnect().await
    }
    pub async fn start_transfer(
        &self,
        request: StartTransferRequest,
    ) -> actor_rtc_framework::error::ActorResult<StartTransferResponse> {
        self.remote_actor.call(request).await
    }
    pub async fn send_chunk(
        &self,
        request: FileChunk,
    ) -> actor_rtc_framework::error::ActorResult<TransferProgress> {
        self.remote_actor.call(request).await
    }
    pub async fn request_retransmit(
        &self,
        request: RetransmitRequest,
    ) -> actor_rtc_framework::error::ActorResult<RetransmitResponse> {
        self.remote_actor.call(request).await
    }
    pub async fn get_transfer_status(
        &self,
        request: GetTransferStatusRequest,
    ) -> actor_rtc_framework::error::ActorResult<TransferStatus> {
        self.remote_actor.call(request).await
    }
    pub async fn cancel_transfer(
        &self,
        request: CancelTransferRequest,
    ) -> actor_rtc_framework::error::ActorResult<CancelTransferResponse> {
        self.remote_actor.call(request).await
    }
    pub async fn progress_stream(
        &self,
        message: TransferProgress,
    ) -> actor_rtc_framework::error::ActorResult<()> {
        self.remote_actor.tell(message).await
    }
}
/// Auto-generated LocalActor adapter for #trait_ident
pub struct FileTransferServiceAdapter;
impl actor_rtc_framework::routing::RouteProvider<dyn IFileTransferService>
    for FileTransferServiceAdapter
{
    fn get_routes(
        actor: std::sync::Arc<dyn IFileTransferService>,
    ) -> Vec<actor_rtc_framework::routing::Route> {
        vec![
            actor_rtc_framework::routing::Route {
                method_name: "file.transfer.FileTransferService/StartTransfer".to_string(),
                handler: {
                    let actor_clone = actor.clone();
                    Box::new(
                        move |ctx: std::sync::Arc<actor_rtc_framework::context::Context>,
                              req_bytes: Vec<u8>| {
                            let actor_for_task = actor_clone.clone();
                            Box::pin(async move {
                                use prost::Message;
                                let request =
                                    StartTransferRequest::decode(&*req_bytes).map_err(|e| {
                                        actor_rtc_framework::error::ActorError::Protocol(format!(
                                            "Failed to decode {}: {}",
                                            "file_transfer.StartTransferRequest", e
                                        ))
                                    })?;
                                let response: StartTransferResponse = actor_for_task
                                    .start_transfer(request, ctx)
                                    .await
                                    .map_err(|e| {
                                        actor_rtc_framework::error::ActorError::Business(format!(
                                            "Method {} failed: {:?}",
                                            "StartTransfer", e
                                        ))
                                    })?;
                                let mut buf = Vec::new();
                                response.encode(&mut buf).map_err(|e| {
                                    actor_rtc_framework::error::ActorError::Protocol(format!(
                                        "Failed to encode {}: {}",
                                        "file_transfer.StartTransferResponse", e
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
                method_name: "file.transfer.FileTransferService/SendChunk".to_string(),
                handler: {
                    let actor_clone = actor.clone();
                    Box::new(
                        move |ctx: std::sync::Arc<actor_rtc_framework::context::Context>,
                              req_bytes: Vec<u8>| {
                            let actor_for_task = actor_clone.clone();
                            Box::pin(async move {
                                use prost::Message;
                                let request = FileChunk::decode(&*req_bytes).map_err(|e| {
                                    actor_rtc_framework::error::ActorError::Protocol(format!(
                                        "Failed to decode {}: {}",
                                        "file_transfer.FileChunk", e
                                    ))
                                })?;
                                let response: TransferProgress =
                                    actor_for_task.send_chunk(request, ctx).await.map_err(|e| {
                                        actor_rtc_framework::error::ActorError::Business(format!(
                                            "Method {} failed: {:?}",
                                            "SendChunk", e
                                        ))
                                    })?;
                                let mut buf = Vec::new();
                                response.encode(&mut buf).map_err(|e| {
                                    actor_rtc_framework::error::ActorError::Protocol(format!(
                                        "Failed to encode {}: {}",
                                        "file_transfer.TransferProgress", e
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
                method_name: "file.transfer.FileTransferService/RequestRetransmit".to_string(),
                handler: {
                    let actor_clone = actor.clone();
                    Box::new(
                        move |ctx: std::sync::Arc<actor_rtc_framework::context::Context>,
                              req_bytes: Vec<u8>| {
                            let actor_for_task = actor_clone.clone();
                            Box::pin(async move {
                                use prost::Message;
                                let request =
                                    RetransmitRequest::decode(&*req_bytes).map_err(|e| {
                                        actor_rtc_framework::error::ActorError::Protocol(format!(
                                            "Failed to decode {}: {}",
                                            "file_transfer.RetransmitRequest", e
                                        ))
                                    })?;
                                let response: RetransmitResponse = actor_for_task
                                    .request_retransmit(request, ctx)
                                    .await
                                    .map_err(|e| {
                                        actor_rtc_framework::error::ActorError::Business(format!(
                                            "Method {} failed: {:?}",
                                            "RequestRetransmit", e
                                        ))
                                    })?;
                                let mut buf = Vec::new();
                                response.encode(&mut buf).map_err(|e| {
                                    actor_rtc_framework::error::ActorError::Protocol(format!(
                                        "Failed to encode {}: {}",
                                        "file_transfer.RetransmitResponse", e
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
                method_name: "file.transfer.FileTransferService/GetTransferStatus".to_string(),
                handler: {
                    let actor_clone = actor.clone();
                    Box::new(
                        move |ctx: std::sync::Arc<actor_rtc_framework::context::Context>,
                              req_bytes: Vec<u8>| {
                            let actor_for_task = actor_clone.clone();
                            Box::pin(async move {
                                use prost::Message;
                                let request = GetTransferStatusRequest::decode(&*req_bytes)
                                    .map_err(|e| {
                                        actor_rtc_framework::error::ActorError::Protocol(format!(
                                            "Failed to decode {}: {}",
                                            "file_transfer.GetTransferStatusRequest", e
                                        ))
                                    })?;
                                let response: TransferStatus = actor_for_task
                                    .get_transfer_status(request, ctx)
                                    .await
                                    .map_err(|e| {
                                        actor_rtc_framework::error::ActorError::Business(format!(
                                            "Method {} failed: {:?}",
                                            "GetTransferStatus", e
                                        ))
                                    })?;
                                let mut buf = Vec::new();
                                response.encode(&mut buf).map_err(|e| {
                                    actor_rtc_framework::error::ActorError::Protocol(format!(
                                        "Failed to encode {}: {}",
                                        "file_transfer.TransferStatus", e
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
                method_name: "file.transfer.FileTransferService/CancelTransfer".to_string(),
                handler: {
                    let actor_clone = actor.clone();
                    Box::new(
                        move |ctx: std::sync::Arc<actor_rtc_framework::context::Context>,
                              req_bytes: Vec<u8>| {
                            let actor_for_task = actor_clone.clone();
                            Box::pin(async move {
                                use prost::Message;
                                let request =
                                    CancelTransferRequest::decode(&*req_bytes).map_err(|e| {
                                        actor_rtc_framework::error::ActorError::Protocol(format!(
                                            "Failed to decode {}: {}",
                                            "file_transfer.CancelTransferRequest", e
                                        ))
                                    })?;
                                let response: CancelTransferResponse = actor_for_task
                                    .cancel_transfer(request, ctx)
                                    .await
                                    .map_err(|e| {
                                        actor_rtc_framework::error::ActorError::Business(format!(
                                            "Method {} failed: {:?}",
                                            "CancelTransfer", e
                                        ))
                                    })?;
                                let mut buf = Vec::new();
                                response.encode(&mut buf).map_err(|e| {
                                    actor_rtc_framework::error::ActorError::Protocol(format!(
                                        "Failed to encode {}: {}",
                                        "file_transfer.CancelTransferResponse", e
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
                method_name: "file.transfer.FileTransferService/ProgressStream".to_string(),
                handler: {
                    let actor_clone = actor.clone();
                    Box::new(
                        move |ctx: std::sync::Arc<actor_rtc_framework::context::Context>,
                              req_bytes: Vec<u8>| {
                            let actor_for_task = actor_clone.clone();
                            Box::pin(async move {
                                use prost::Message;
                                let message =
                                    TransferProgress::decode(&*req_bytes).map_err(|e| {
                                        actor_rtc_framework::error::ActorError::Protocol(format!(
                                            "Failed to decode {}: {}",
                                            "file_transfer.TransferProgress", e
                                        ))
                                    })?;
                                actor_for_task.progress_stream(message, ctx).await.map_err(
                                    |e| {
                                        actor_rtc_framework::error::ActorError::Business(format!(
                                            "Streaming method {} failed: {:?}",
                                            "ProgressStream", e
                                        ))
                                    },
                                )?;
                                Ok(Vec::new())
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
impl<T> actor_rtc_framework::routing::RouteProvider<T> for FileTransferServiceAdapter
where
    T: IFileTransferService + Send + Sync + 'static,
{
    fn get_routes(actor: std::sync::Arc<T>) -> Vec<actor_rtc_framework::routing::Route> {
        let trait_obj: std::sync::Arc<dyn IFileTransferService> = actor;
        Self::get_routes(trait_obj)
    }
}
/// Auto-generated RemoteActor client manager for #manager_ident
#[allow(dead_code)]
pub struct FileTransferServiceClientManager {
    manager:
        std::sync::Arc<tokio::sync::RwLock<actor_rtc_framework::remote_actor::RemoteActorManager>>,
}
#[allow(dead_code)]
impl FileTransferServiceClientManager {
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
            stringify!(FileTransferServiceClientManager).to_string(),
            service_address,
        )
    }
    pub async fn start_transfer(
        &self,
        actor_id: &shared_protocols::actor::ActorId,
        request: StartTransferRequest,
    ) -> actor_rtc_framework::error::ActorResult<StartTransferResponse> {
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
    pub async fn send_chunk(
        &self,
        actor_id: &shared_protocols::actor::ActorId,
        request: FileChunk,
    ) -> actor_rtc_framework::error::ActorResult<TransferProgress> {
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
    pub async fn request_retransmit(
        &self,
        actor_id: &shared_protocols::actor::ActorId,
        request: RetransmitRequest,
    ) -> actor_rtc_framework::error::ActorResult<RetransmitResponse> {
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
    pub async fn get_transfer_status(
        &self,
        actor_id: &shared_protocols::actor::ActorId,
        request: GetTransferStatusRequest,
    ) -> actor_rtc_framework::error::ActorResult<TransferStatus> {
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
    pub async fn cancel_transfer(
        &self,
        actor_id: &shared_protocols::actor::ActorId,
        request: CancelTransferRequest,
    ) -> actor_rtc_framework::error::ActorResult<CancelTransferResponse> {
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
    pub async fn progress_stream(
        &self,
        actor_id: &shared_protocols::actor::ActorId,
        message: TransferProgress,
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
}
