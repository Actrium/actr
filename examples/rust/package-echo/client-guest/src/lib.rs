//! Package Echo Client Guest — cdylib workload for the client host.
//!
//! Implements the local `ClientService.SendMessage` RPC:
//! 1. Discover `actrium:EchoService:<version>` (cached after first success)
//! 2. Call `echo.EchoService.Echo` on the remote server
//! 3. Return the reply to the host
//! 4. On failure with a cached target, clear cache and retry once

pub mod echo {
    include!(concat!(env!("OUT_DIR"), "/echo.rs"));
}

pub mod client {
    include!(concat!(env!("OUT_DIR"), "/client.rs"));
}

use actr_framework::{Context, MessageDispatcher, Workload, entry};
use actr_protocol::{ActorResult, ActrError, ActrId, ActrType, RpcEnvelope, RpcRequest};
use async_trait::async_trait;
use bytes::Bytes;
use prost::Message as ProstMessage;

use client::{SendMessageRequest, SendMessageResponse};
use echo::{EchoRequest, EchoResponse};

const ECHO_SERVICE_MANUFACTURER: &str = "actrium";
const ECHO_SERVICE_NAME: &str = "EchoService";

fn echo_actr_version() -> String {
    std::env::var("ECHO_ACTR_VERSION").unwrap_or_else(|_| "0.2.1-beta".to_string())
}

impl RpcRequest for EchoRequest {
    type Response = EchoResponse;

    fn route_key() -> &'static str {
        "echo.EchoService.Echo"
    }

    fn payload_type() -> actr_protocol::PayloadType {
        actr_protocol::PayloadType::RpcReliable
    }
}

impl RpcRequest for SendMessageRequest {
    type Response = SendMessageResponse;

    fn route_key() -> &'static str {
        "client.ClientService.SendMessage"
    }

    fn payload_type() -> actr_protocol::PayloadType {
        actr_protocol::PayloadType::RpcReliable
    }
}

/// Client guest workload — holds cached server ActrId.
pub struct ClientGuestWorkload {
    cached_server_id: std::cell::Cell<Option<ActrId>>,
}

impl Default for ClientGuestWorkload {
    fn default() -> Self {
        Self {
            cached_server_id: std::cell::Cell::new(None),
        }
    }
}

// Safety: cdylib guest is single-threaded (host serializes actr_handle calls).
unsafe impl Send for ClientGuestWorkload {}
unsafe impl Sync for ClientGuestWorkload {}

pub struct ClientGuestDispatcher;

#[async_trait]
impl MessageDispatcher for ClientGuestDispatcher {
    type Workload = ClientGuestWorkload;

    async fn dispatch<C: Context>(
        workload: &Self::Workload,
        envelope: RpcEnvelope,
        ctx: &C,
    ) -> ActorResult<Bytes> {
        match envelope.route_key.as_str() {
            "client.ClientService.SendMessage" => {
                let payload = envelope.payload.unwrap_or_default();
                let req = SendMessageRequest::decode(payload.as_ref()).map_err(|e| {
                    ActrError::DecodeFailure(format!("decode SendMessageRequest: {e}"))
                })?;

                let reply = send_message(workload, ctx, &req.message).await?;

                let resp = SendMessageResponse { reply };
                Ok(Bytes::from(resp.encode_to_vec()))
            }
            _ => Err(ActrError::UnknownRoute(envelope.route_key)),
        }
    }
}

async fn send_message<C: Context>(
    workload: &ClientGuestWorkload,
    ctx: &C,
    message: &str,
) -> ActorResult<String> {
    let server_id = resolve_server(workload, ctx).await?;

    let echo_req = EchoRequest {
        message: message.to_string(),
    };

    match ctx
        .call(&actr_framework::Dest::Actor(server_id.clone()), echo_req)
        .await
    {
        Ok(resp) => Ok(resp.reply),
        Err(e) => {
            // Cache miss: clear cached id and retry once with fresh discovery
            workload.cached_server_id.set(None);
            let fresh_id = discover_server(ctx).await?;
            workload.cached_server_id.set(Some(fresh_id.clone()));

            let echo_req2 = EchoRequest {
                message: message.to_string(),
            };
            let resp = ctx
                .call(&actr_framework::Dest::Actor(fresh_id), echo_req2)
                .await
                .map_err(|e2| {
                    ActrError::Internal(format!(
                        "retry after cache clear failed: {e2} (original: {e})"
                    ))
                })?;
            Ok(resp.reply)
        }
    }
}

async fn resolve_server<C: Context>(
    workload: &ClientGuestWorkload,
    ctx: &C,
) -> ActorResult<ActrId> {
    if let Some(id) = workload.cached_server_id.take() {
        workload.cached_server_id.set(Some(id.clone()));
        return Ok(id);
    }
    let id = discover_server(ctx).await?;
    workload.cached_server_id.set(Some(id.clone()));
    Ok(id)
}

async fn discover_server<C: Context>(ctx: &C) -> ActorResult<ActrId> {
    let target_type = ActrType {
        manufacturer: ECHO_SERVICE_MANUFACTURER.to_string(),
        name: ECHO_SERVICE_NAME.to_string(),
        version: echo_actr_version(),
    };
    ctx.discover_route_candidate(&target_type).await
}

impl Workload for ClientGuestWorkload {
    type Dispatcher = ClientGuestDispatcher;
}

entry!(ClientGuestWorkload);
