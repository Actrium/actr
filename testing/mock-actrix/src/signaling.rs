//! WebSocket signaling handling.
//!
//! This module implements the signaling-server side of the WebSocket
//! protocol defined in `actr-protocol`. It processes `SignalingEnvelope`
//! frames, handles registration / route discovery / ping-pong / relay
//! forwarding, and keeps an in-memory registry of connected actors.
//!
//! All low-level framing (accept/upgrade/split/ping) is delegated to axum's
//! `WebSocketUpgrade`; this module is purely protocol logic so the HTTP
//! server and the signaling server can share a single TCP listener.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{
    AIdCredential, ActrId, ActrIdExt, ActrRelay, RegisterRequest, RegisterResponse,
    RouteCandidatesResponse, SignalingEnvelope, SignalingToActr, TurnCredential, actr_relay,
    actr_to_signaling, peer_to_signaling, register_response, route_candidates_response,
    signaling_envelope, signaling_to_actr,
};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use bytes::Bytes;
use ed25519_dalek::Signer;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::state::{MockState, RegisteredActor};

/// Query parameters the real actrix gate accepts (actor_id, key_id, claims,
/// signature). The mock uses `actor_id` to tie incoming WebSocket connections
/// back to a previously HTTP-registered actor so that route discovery works
/// even when the actor never sends a `PeerToSignaling::RegisterRequest`.
#[derive(Debug, Default, Deserialize)]
#[allow(dead_code)]
pub struct WsQuery {
    actor_id: Option<String>,
    key_id: Option<String>,
    claims: Option<String>,
    signature: Option<String>,
}

/// Axum handler: upgrades the request to a WebSocket and hands it to the
/// signaling loop.
pub async fn ws_upgrade_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<WsQuery>,
    State(state): State<Arc<MockState>>,
) -> impl IntoResponse {
    let actor_id_param = params.actor_id.clone();
    ws.on_upgrade(move |socket| handle_connection(socket, state, actor_id_param))
}

async fn handle_connection(
    socket: WebSocket,
    state: Arc<MockState>,
    actor_id_param: Option<String>,
) {
    state.connection_count.fetch_add(1, Ordering::SeqCst);

    let (mut ws_tx, mut ws_rx) = socket.split();
    let (client_tx, mut client_rx) = mpsc::unbounded_channel::<Message>();
    let client_id = uuid::Uuid::new_v4().to_string();
    let cancel = state.cancel.clone();

    state
        .clients
        .write()
        .await
        .insert(client_id.clone(), client_tx);

    // If the peer connected with `?actor_id=...` (web/service-worker path
    // after HTTP register), bind its existing registry entry to this WS
    // client_id so route discovery can see it.
    if let Some(actor_id_str) = actor_id_param.as_deref() {
        if let Some(actr_id) = parse_actor_id(actor_id_str) {
            let mut registry = state.registry.write().await;
            let mut found = false;
            for entry in registry.iter_mut() {
                if entry.actr_id == actr_id {
                    entry.client_id = client_id.clone();
                    found = true;
                    break;
                }
            }
            drop(registry);
            if found {
                state
                    .client_to_actr_id
                    .write()
                    .await
                    .insert(client_id.clone(), actr_id);
                tracing::info!(
                    actor_id = %actor_id_str,
                    "mock-actrix: WS bound to HTTP-registered actor"
                );
            } else {
                tracing::warn!(
                    actor_id = %actor_id_str,
                    "mock-actrix: WS actor_id has no prior HTTP registration"
                );
            }
        }
    }

    loop {
        tokio::select! {
            // Server-wide shutdown: close the socket so the peer notices.
            _ = cancel.cancelled() => {
                let _ = ws_tx.send(Message::Close(None)).await;
                break;
            }
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        state.message_count.fetch_add(1, Ordering::Relaxed);
                        if let Ok(envelope) =
                            <SignalingEnvelope as ProstMessage>::decode(&data[..])
                        {
                            state.received_messages.lock().await.push(envelope.clone());
                            process_envelope(envelope, &client_id, &state).await;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(err)) => {
                        tracing::debug!(%err, "mock-actrix: ws recv error");
                        break;
                    }
                    _ => {}
                }
            }
            out = client_rx.recv() => {
                match out {
                    Some(msg) => {
                        if ws_tx.send(msg).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }

    // Cleanup: drop the WS sender and clear this client's binding on every
    // registry entry it owned. We keep the registry rows themselves so that
    // HTTP-registered actors remain discoverable across reconnects (their
    // `client_id` is simply reset to the empty string).
    state.clients.write().await.remove(&client_id);
    {
        let mut registry = state.registry.write().await;
        for entry in registry.iter_mut() {
            if entry.client_id == client_id {
                entry.client_id.clear();
            }
        }
    }
    state.client_to_actr_id.write().await.remove(&client_id);
    state.disconnection_count.fetch_add(1, Ordering::SeqCst);
}

async fn process_envelope(envelope: SignalingEnvelope, sender_id: &str, state: &Arc<MockState>) {
    let Some(flow) = envelope.flow.as_ref() else {
        return;
    };

    match flow {
        signaling_envelope::Flow::PeerToServer(peer_msg) => {
            handle_peer_to_server(&envelope, peer_msg, sender_id, state).await;
        }
        signaling_envelope::Flow::ActrToServer(actr_msg) => {
            handle_actr_to_server(&envelope, actr_msg, sender_id, state).await;
        }
        signaling_envelope::Flow::ActrRelay(relay) => {
            handle_actr_relay(&envelope, relay, sender_id, state).await;
        }
        _ => {}
    }
}

async fn handle_peer_to_server(
    envelope: &SignalingEnvelope,
    peer_msg: &actr_protocol::PeerToSignaling,
    sender_id: &str,
    state: &Arc<MockState>,
) {
    let Some(payload) = peer_msg.payload.as_ref() else {
        return;
    };

    match payload {
        peer_to_signaling::Payload::RegisterRequest(req) => {
            let register_ok = build_register_ok(req, state).await;

            // Store registration
            {
                let entry = RegisteredActor {
                    actr_id: register_ok.actr_id.clone(),
                    actr_type: req.actr_type.clone(),
                    client_id: sender_id.to_string(),
                    ws_address: req.ws_address.clone(),
                    service_spec: req.service_spec.clone(),
                };
                state.registry.write().await.push(entry);
                state
                    .client_to_actr_id
                    .write()
                    .await
                    .insert(sender_id.to_string(), register_ok.actr_id.clone());
            }

            let response = RegisterResponse {
                result: Some(register_response::Result::Success(register_ok.clone())),
            };

            let response_envelope = SignalingEnvelope {
                envelope_version: 1,
                envelope_id: uuid::Uuid::new_v4().to_string(),
                reply_for: Some(envelope.envelope_id.clone()),
                timestamp: now_timestamp(),
                flow: Some(signaling_envelope::Flow::ServerToActr(SignalingToActr {
                    target: register_ok.actr_id.clone(),
                    payload: Some(signaling_to_actr::Payload::RegisterResponse(response)),
                })),
                traceparent: None,
                tracestate: None,
            };

            send_to_client(sender_id, &response_envelope, state).await;

            tracing::info!(
                serial = register_ok.actr_id.serial_number,
                manufacturer = req.actr_type.manufacturer,
                name = req.actr_type.name,
                "mock-actrix: registered actor (ws)"
            );
        }
    }
}

/// Build a `RegisterOk` for the given `RegisterRequest`.
///
/// This is the shared code path used by both the WS signaling handler and the
/// `POST /register` HTTP handler (so `AisClient::register_with_manifest`
/// and the WebSocket bootstrap always see identical responses).
pub async fn build_register_ok(
    req: &RegisterRequest,
    state: &Arc<MockState>,
) -> register_response::RegisterOk {
    let serial = state.next_serial.fetch_add(1, Ordering::SeqCst);

    let actr_id = ActrId {
        realm: req.realm,
        serial_number: serial,
        r#type: req.actr_type.clone(),
    };

    let claims = actr_protocol::IdentityClaims {
        realm_id: req.realm.realm_id,
        actor_id: format!("{serial}"),
        expires_at: chrono::Utc::now().timestamp() as u64 + 86400,
    };
    let claims_bytes = claims.encode_to_vec();
    let signature = state.ais_signing_key().sign(&claims_bytes);
    let verifying_key = state.ais_signing_key().verifying_key();

    let credential = AIdCredential {
        key_id: state.ais_signing_key_id(),
        claims: claims_bytes.into(),
        signature: signature.to_bytes().to_vec().into(),
    };

    let turn_credential = TurnCredential {
        username: format!("{}:{}", chrono::Utc::now().timestamp() + 86400, serial),
        password: format!("mock-turn-{serial:016x}"),
        expires_at: chrono::Utc::now().timestamp() as u64 + 86400,
    };

    register_response::RegisterOk {
        actr_id,
        credential,
        turn_credential,
        credential_expires_at: Some(prost_types::Timestamp {
            seconds: chrono::Utc::now().timestamp() + 86400,
            nanos: 0,
        }),
        signaling_heartbeat_interval_secs: 30,
        signing_pubkey: verifying_key.to_bytes().to_vec().into(),
        signing_key_id: state.ais_signing_key_id(),
        psk: Some(format!("mock-psk-{serial:016x}").into_bytes().into()),
        psk_expires_at: Some(chrono::Utc::now().timestamp() + 30 * 86400),
    }
}

async fn handle_actr_to_server(
    envelope: &SignalingEnvelope,
    actr_msg: &actr_protocol::ActrToSignaling,
    sender_id: &str,
    state: &Arc<MockState>,
) {
    let Some(payload) = actr_msg.payload.as_ref() else {
        return;
    };

    match payload {
        actr_to_signaling::Payload::Ping(_ping) => {
            let pong = actr_protocol::Pong {
                seq: state.message_count.load(Ordering::Relaxed) as u64,
                suggest_interval_secs: Some(30),
                credential_warning: None,
            };

            send_response(
                sender_id,
                envelope,
                SignalingToActr {
                    target: actr_msg.source.clone(),
                    payload: Some(signaling_to_actr::Payload::Pong(pong)),
                },
                state,
            )
            .await;
        }

        actr_to_signaling::Payload::RouteCandidatesRequest(req) => {
            let registry = state.registry.read().await;

            let mut candidates = Vec::new();
            let mut ws_address_map = Vec::new();

            for entry in registry.iter() {
                if entry.actr_type == req.target_type {
                    if entry.actr_id.serial_number == actr_msg.source.serial_number {
                        continue;
                    }
                    candidates.push(entry.actr_id.clone());
                    if let Some(ws_addr) = &entry.ws_address {
                        ws_address_map.push(actr_protocol::WsAddressEntry {
                            candidate_id: entry.actr_id.clone(),
                            ws_address: Some(ws_addr.clone()),
                        });
                    }
                }
            }

            if let Some(criteria) = &req.criteria {
                let max = criteria.candidate_count as usize;
                candidates.truncate(max);
            }

            tracing::info!(
                count = candidates.len(),
                target_type = format!("{}.{}", req.target_type.manufacturer, req.target_type.name),
                "mock-actrix: route candidates response"
            );

            let ok = route_candidates_response::RouteCandidatesOk {
                candidates,
                ws_address_map,
            };

            let response = RouteCandidatesResponse {
                result: Some(route_candidates_response::Result::Success(ok)),
            };

            send_response(
                sender_id,
                envelope,
                SignalingToActr {
                    target: actr_msg.source.clone(),
                    payload: Some(signaling_to_actr::Payload::RouteCandidatesResponse(
                        response,
                    )),
                },
                state,
            )
            .await;
        }

        actr_to_signaling::Payload::UnregisterRequest(_) => {
            {
                let mut registry = state.registry.write().await;
                registry.retain(|a| a.client_id != sender_id);
            }

            send_response(
                sender_id,
                envelope,
                SignalingToActr {
                    target: actr_msg.source.clone(),
                    payload: Some(signaling_to_actr::Payload::UnregisterResponse(
                        actr_protocol::UnregisterResponse {
                            result: Some(actr_protocol::unregister_response::Result::Success(
                                actr_protocol::unregister_response::UnregisterOk {},
                            )),
                        },
                    )),
                },
                state,
            )
            .await;
        }

        actr_to_signaling::Payload::GetSigningKeyRequest(req) => {
            let verifying_key = state.ais_signing_key().verifying_key();
            send_response(
                sender_id,
                envelope,
                SignalingToActr {
                    target: actr_msg.source.clone(),
                    payload: Some(signaling_to_actr::Payload::GetSigningKeyResponse(
                        actr_protocol::GetSigningKeyResponse {
                            key_id: req.key_id,
                            pubkey: verifying_key.to_bytes().to_vec().into(),
                        },
                    )),
                },
                state,
            )
            .await;
        }

        actr_to_signaling::Payload::DiscoveryRequest(req) => {
            let registry = state.registry.read().await;

            let entries: Vec<_> = registry
                .iter()
                .filter(|e| {
                    req.manufacturer
                        .as_ref()
                        .is_none_or(|m| &e.actr_type.manufacturer == m)
                })
                .map(|e| {
                    let fingerprint = e
                        .service_spec
                        .as_ref()
                        .map_or(String::new(), |s| s.fingerprint.clone());
                    actr_protocol::discovery_response::TypeEntry {
                        actr_type: e.actr_type.clone(),
                        name: e
                            .service_spec
                            .as_ref()
                            .map_or(e.actr_type.name.clone(), |s| s.name.clone()),
                        description: e.service_spec.as_ref().and_then(|s| s.description.clone()),
                        service_fingerprint: fingerprint,
                        published_at: e.service_spec.as_ref().and_then(|s| s.published_at),
                        tags: e.service_spec.as_ref().map_or(vec![], |s| s.tags.clone()),
                    }
                })
                .collect();

            send_response(
                sender_id,
                envelope,
                SignalingToActr {
                    target: actr_msg.source.clone(),
                    payload: Some(signaling_to_actr::Payload::DiscoveryResponse(
                        actr_protocol::DiscoveryResponse {
                            result: Some(actr_protocol::discovery_response::Result::Success(
                                actr_protocol::discovery_response::DiscoveryOk { entries },
                            )),
                        },
                    )),
                },
                state,
            )
            .await;
        }

        actr_to_signaling::Payload::GetServiceSpecRequest(req) => {
            let registry = state.registry.read().await;

            let spec = registry.iter().find_map(|e| {
                if let Some(spec) = &e.service_spec {
                    if spec.name == req.name {
                        return Some(spec.clone());
                    }
                }
                None
            });

            let payload = match spec {
                Some(spec) => signaling_to_actr::Payload::GetServiceSpecResponse(
                    actr_protocol::GetServiceSpecResponse {
                        result: Some(actr_protocol::get_service_spec_response::Result::Success(
                            spec,
                        )),
                    },
                ),
                None => signaling_to_actr::Payload::GetServiceSpecResponse(
                    actr_protocol::GetServiceSpecResponse {
                        result: Some(actr_protocol::get_service_spec_response::Result::Error(
                            actr_protocol::ErrorResponse {
                                code: 404,
                                message: format!("service '{}' not found", req.name),
                            },
                        )),
                    },
                ),
            };

            send_response(
                sender_id,
                envelope,
                SignalingToActr {
                    target: actr_msg.source.clone(),
                    payload: Some(payload),
                },
                state,
            )
            .await;
        }

        _ => {
            tracing::debug!("mock-actrix: ignoring ActrToSignaling payload");
        }
    }
}

async fn handle_actr_relay(
    envelope: &SignalingEnvelope,
    relay: &ActrRelay,
    sender_id: &str,
    state: &Arc<MockState>,
) {
    if let Some(actr_relay::Payload::SessionDescription(sd)) = relay.payload.as_ref() {
        if sd.r#type == 3 {
            state.ice_restart_offer_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    if state.pause_forwarding.load(Ordering::Acquire) {
        return;
    }

    // Role negotiation: server decides offerer/answerer by serial ordering.
    if let Some(actr_relay::Payload::RoleNegotiation(role_neg)) = relay.payload.as_ref() {
        let from_is_offerer = role_neg.from.serial_number < role_neg.to.serial_number;

        let envelope_for_from = SignalingEnvelope {
            envelope_version: 1,
            envelope_id: uuid::Uuid::new_v4().to_string(),
            reply_for: None,
            timestamp: now_timestamp(),
            flow: Some(signaling_envelope::Flow::ActrRelay(ActrRelay {
                source: role_neg.to.clone(),
                credential: AIdCredential::default(),
                target: role_neg.from.clone(),
                payload: Some(actr_relay::Payload::RoleAssignment(
                    actr_protocol::RoleAssignment {
                        is_offerer: from_is_offerer,
                        remote_fixed: None,
                    },
                )),
            })),
            traceparent: None,
            tracestate: None,
        };

        let envelope_for_to = SignalingEnvelope {
            envelope_version: 1,
            envelope_id: uuid::Uuid::new_v4().to_string(),
            reply_for: None,
            timestamp: now_timestamp(),
            flow: Some(signaling_envelope::Flow::ActrRelay(ActrRelay {
                source: role_neg.from.clone(),
                credential: AIdCredential::default(),
                target: role_neg.to.clone(),
                payload: Some(actr_relay::Payload::RoleAssignment(
                    actr_protocol::RoleAssignment {
                        is_offerer: !from_is_offerer,
                        remote_fixed: None,
                    },
                )),
            })),
            traceparent: None,
            tracestate: None,
        };

        let clients = state.clients.read().await;
        for (cid, tx) in clients.iter() {
            if cid == sender_id {
                let encoded = envelope_for_from.encode_to_vec();
                let _ = tx.send(Message::Binary(Bytes::from(encoded)));
            } else {
                let encoded = envelope_for_to.encode_to_vec();
                let _ = tx.send(Message::Binary(Bytes::from(encoded)));
            }
        }
        return;
    }

    // Forward relay to target by ActrId lookup.
    let target_id = &relay.target;
    let client_map = state.client_to_actr_id.read().await;
    let clients = state.clients.read().await;

    let target_client_id = client_map.iter().find_map(|(cid, aid)| {
        if aid == target_id {
            Some(cid.clone())
        } else {
            None
        }
    });

    if let Some(target_cid) = target_client_id {
        if let Some(tx) = clients.get(&target_cid) {
            let encoded = envelope.encode_to_vec();
            let _ = tx.send(Message::Binary(Bytes::from(encoded)));
        }
    } else {
        // Fallback: broadcast to all other clients (legacy test compatibility).
        let encoded = envelope.encode_to_vec();
        for (cid, tx) in clients.iter() {
            if cid != sender_id {
                let _ = tx.send(Message::Binary(Bytes::from(encoded.clone())));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn send_response(
    client_id: &str,
    original_envelope: &SignalingEnvelope,
    payload: SignalingToActr,
    state: &Arc<MockState>,
) {
    let response_envelope = SignalingEnvelope {
        envelope_version: 1,
        envelope_id: uuid::Uuid::new_v4().to_string(),
        reply_for: Some(original_envelope.envelope_id.clone()),
        timestamp: now_timestamp(),
        flow: Some(signaling_envelope::Flow::ServerToActr(payload)),
        traceparent: None,
        tracestate: None,
    };
    send_to_client(client_id, &response_envelope, state).await;
}

async fn send_to_client(client_id: &str, envelope: &SignalingEnvelope, state: &Arc<MockState>) {
    let encoded = envelope.encode_to_vec();
    let clients = state.clients.read().await;
    if let Some(tx) = clients.get(client_id) {
        let _ = tx.send(Message::Binary(Bytes::from(encoded)));
    }
}

fn now_timestamp() -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: chrono::Utc::now().timestamp(),
        nanos: 0,
    }
}

/// Parse an `ActrId` from the `actor_id` query parameter sent by clients
/// after HTTP registration.
fn parse_actor_id(s: &str) -> Option<ActrId> {
    ActrId::from_string_repr(s).ok()
}
