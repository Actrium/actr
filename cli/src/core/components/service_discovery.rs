use crate::core::{
    AvailabilityStatus, HealthStatus, ProtoFile, ServiceDetails, ServiceDiscovery, ServiceFilter,
    ServiceInfo,
};
use actr_config::Config;
use actr_hyper::AisClient;
use actr_protocol::{
    AIdCredential, ActrId, ActrToSignaling, ActrType, DiscoveryRequest, ErrorResponse,
    GetServiceSpecRequest, RegisterRequest, SignalingEnvelope, actr_to_signaling,
    discovery_response, get_service_spec_response, register_response, signaling_envelope,
    signaling_to_actr,
};
use actr_protocol::{ActrIdExt, ActrTypeExt};
use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use prost::Message;
use std::path::PathBuf;
use std::time::SystemTime;
use tokio::{
    sync::Mutex,
    time::{Duration, sleep},
};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use url::Url;

type SignalingSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct SignalingState {
    socket: SignalingSocket,
    actr_id: ActrId,
    credential: AIdCredential,
}

pub struct NetworkServiceDiscovery {
    config: Config,
    state: Mutex<Option<SignalingState>>,
}

impl NetworkServiceDiscovery {
    const LOOKUP_RETRY_ATTEMPTS: usize = 45;
    const LOOKUP_RETRY_DELAY: Duration = Duration::from_secs(2);

    pub fn new(config: Config) -> Self {
        Self {
            config,
            state: Mutex::new(None),
        }
    }

    fn format_actr_type(actr_type: &ActrType) -> String {
        actr_type.to_string_repr()
    }

    async fn ensure_connected(&self) -> Result<()> {
        let mut state_guard = self.state.lock().await;
        if state_guard.is_some() {
            return Ok(());
        }

        let state = self.connect_and_register().await?;
        *state_guard = Some(state);
        Ok(())
    }

    // TODO: add filter support
    async fn discover_entries(
        &self,
        _filter: Option<&ServiceFilter>,
    ) -> Result<Vec<discovery_response::TypeEntry>> {
        self.ensure_connected().await?;
        let mut state_guard = self.state.lock().await;
        let state = state_guard
            .as_mut()
            .context("Signaling state not initialized")?;

        // TODO: add filter support
        let request = DiscoveryRequest {
            manufacturer: None,
            limit: None,
        };
        let payload = actr_to_signaling::Payload::DiscoveryRequest(request);
        let envelope =
            Self::build_envelope(signaling_envelope::Flow::ActrToServer(ActrToSignaling {
                source: state.actr_id.clone(),
                credential: state.credential.clone(),
                payload: Some(payload),
            }))?;

        let result = match Self::send_envelope(&mut state.socket, envelope).await {
            Ok(()) => loop {
                let envelope = Self::read_envelope(&mut state.socket).await?;
                match envelope.flow {
                    Some(signaling_envelope::Flow::ServerToActr(server)) => match server.payload {
                        Some(signaling_to_actr::Payload::DiscoveryResponse(response)) => {
                            break Self::handle_discovery_response(response);
                        }
                        Some(signaling_to_actr::Payload::Error(error)) => {
                            break Err(Self::as_error("Discovery failed", &error));
                        }
                        _ => {}
                    },
                    Some(signaling_envelope::Flow::EnvelopeError(error)) => {
                        break Err(Self::as_error("Discovery failed", &error));
                    }
                    _ => {}
                }
            },
            Err(err) => Err(err),
        };
        if result.is_err() {
            *state_guard = None;
        }
        result
    }

    fn handle_discovery_response(
        response: actr_protocol::DiscoveryResponse,
    ) -> Result<Vec<discovery_response::TypeEntry>> {
        match response.result {
            Some(discovery_response::Result::Success(success)) => Ok(success.entries),
            Some(discovery_response::Result::Error(error)) => {
                Err(Self::as_error("Discovery failed", &error))
            }
            None => Err(anyhow!("Discovery response is missing result")),
        }
    }

    async fn connect_and_register(&self) -> Result<SignalingState> {
        let register_request = RegisterRequest {
            actr_type: self.config.package.actr_type.clone(),
            realm: self.config.realm.clone(),
            service_spec: None,
            service: None,
            acl: None,
            ws_address: None,
            ..Default::default()
        };

        let mut ais_client =
            AisClient::new(self.config.ais_endpoint.as_deref().unwrap_or_default());
        if let Some(realm_secret) = &self.config.realm_secret {
            ais_client = ais_client.with_realm_secret(realm_secret.clone());
        }

        let register_response = ais_client
            .register_with_manifest(register_request)
            .await
            .map_err(|err| anyhow!("AIS HTTP registration failed: {err}"))?;

        let (actr_id, credential) = match register_response.result {
            Some(register_response::Result::Success(success)) => {
                (success.actr_id, success.credential)
            }
            Some(register_response::Result::Error(error)) => {
                return Err(Self::as_error("AIS registration failed", &error));
            }
            None => return Err(anyhow!("AIS registration response is missing result")),
        };

        let signaling_url = Self::build_signaling_url_with_identity(
            &self.config.signaling_url,
            &actr_id,
            &credential,
        );
        let (socket, _) = connect_async(signaling_url.as_str())
            .await
            .with_context(|| format!("Failed to connect to signaling: {signaling_url}"))?;

        Ok(SignalingState {
            socket,
            actr_id,
            credential,
        })
    }

    fn build_signaling_url_with_identity(
        signaling_url: &Url,
        actr_id: &ActrId,
        credential: &AIdCredential,
    ) -> Url {
        let mut url = signaling_url.clone();
        let claims_b64 = base64::engine::general_purpose::STANDARD.encode(&credential.claims);
        let signature_b64 = base64::engine::general_purpose::STANDARD.encode(&credential.signature);

        url.query_pairs_mut()
            .append_pair("actor_id", &actr_id.to_string_repr())
            .append_pair("key_id", &credential.key_id.to_string())
            .append_pair("claims", &claims_b64)
            .append_pair("signature", &signature_b64);

        url
    }

    fn as_error(context: &str, error: &ErrorResponse) -> anyhow::Error {
        anyhow!("{context}: {} ({})", error.message, error.code)
    }

    async fn retry_lookup<T, F, Fut>(&self, context: &str, mut lookup: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<Option<T>>>,
    {
        let mut last_error = None;

        for attempt in 0..Self::LOOKUP_RETRY_ATTEMPTS {
            match lookup().await {
                Ok(Some(value)) => return Ok(value),
                Ok(None) => last_error = Some(anyhow!("{context}")),
                Err(err) => last_error = Some(err),
            }

            if attempt + 1 < Self::LOOKUP_RETRY_ATTEMPTS {
                sleep(Self::LOOKUP_RETRY_DELAY).await;
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow!("{context}")))
    }

    async fn send_envelope(
        socket: &mut SignalingSocket,
        envelope: SignalingEnvelope,
    ) -> Result<()> {
        let mut buf = Vec::new();
        envelope
            .encode(&mut buf)
            .context("Failed to encode signaling envelope")?;
        socket
            .send(WsMessage::Binary(buf.into()))
            .await
            .context("Failed to send signaling envelope")?;
        Ok(())
    }

    async fn read_envelope(socket: &mut SignalingSocket) -> Result<SignalingEnvelope> {
        while let Some(message) = socket.next().await {
            match message.context("Failed to read signaling response")? {
                WsMessage::Binary(bytes) => {
                    return SignalingEnvelope::decode(bytes)
                        .context("Failed to decode signaling envelope");
                }
                WsMessage::Close(_) => {
                    return Err(anyhow!("Signaling connection closed"));
                }
                WsMessage::Ping(_) | WsMessage::Pong(_) => {}
                WsMessage::Text(text) => {
                    return Err(anyhow!("Unexpected text message from signaling: {text}"));
                }
                WsMessage::Frame(_) => {}
            }
        }

        Err(anyhow!("Signaling connection closed"))
    }

    fn build_envelope(flow: signaling_envelope::Flow) -> Result<SignalingEnvelope> {
        Ok(SignalingEnvelope {
            envelope_version: 1,
            envelope_id: uuid::Uuid::new_v4().to_string(),
            reply_for: None,
            timestamp: prost_types::Timestamp {
                seconds: chrono::Utc::now().timestamp(),
                nanos: 0,
            },
            traceparent: None,
            tracestate: None,
            flow: Some(flow),
        })
    }

    fn select_version(entry: &discovery_response::TypeEntry) -> String {
        entry
            .tags
            .iter()
            .find(|tag| tag.as_str() == "latest")
            .cloned()
            .or_else(|| entry.tags.first().cloned())
            .unwrap_or_else(|| "unknown".to_string())
    }

    fn matches_filter(entry: &discovery_response::TypeEntry, filter: &ServiceFilter) -> bool {
        if let Some(pattern) = &filter.name_pattern {
            let full_name = Self::format_actr_type(&entry.actr_type);
            let matches = Self::matches_pattern(&entry.name, pattern)
                || Self::matches_pattern(&full_name, pattern);
            if !matches {
                return false;
            }
        }

        if let Some(version_range) = &filter.version_range
            && Self::select_version(entry) != *version_range
            && !entry.tags.iter().any(|tag| tag == version_range)
        {
            return false;
        }

        if let Some(tags) = &filter.tags {
            let has_all = tags.iter().all(|tag| entry.tags.iter().any(|t| t == tag));
            if !has_all {
                return false;
            }
        }

        true
    }

    fn matches_pattern(value: &str, pattern: &str) -> bool {
        if pattern == "*" {
            return true;
        }

        let segments: Vec<&str> = pattern.split('*').collect();
        if segments.len() == 1 {
            return value == pattern;
        }

        if !pattern.starts_with('*')
            && let Some(first) = segments.first()
            && !value.starts_with(first)
        {
            return false;
        }

        if !pattern.ends_with('*')
            && let Some(last) = segments.last()
            && !value.ends_with(last)
        {
            return false;
        }

        let mut search_start = 0;
        let end_limit = if !pattern.ends_with('*') {
            value
                .len()
                .saturating_sub(segments.last().unwrap_or(&"").len())
        } else {
            value.len()
        };

        for (index, segment) in segments.iter().enumerate() {
            if segment.is_empty() {
                continue;
            }
            if index == 0 && !pattern.starts_with('*') {
                search_start = segment.len();
                continue;
            }
            if index == segments.len() - 1 && !pattern.ends_with('*') {
                continue;
            }
            if let Some(found) = value[search_start..end_limit].find(segment) {
                search_start += found + segment.len();
            } else {
                return false;
            }
        }

        true
    }

    fn matches_lookup_name(entry: &discovery_response::TypeEntry, name: &str) -> bool {
        if entry.name == name || Self::format_actr_type(&entry.actr_type) == name {
            return true;
        }

        let Ok(lookup_type) = ActrType::from_string_repr(name) else {
            return false;
        };

        entry.actr_type == lookup_type
    }
}

#[async_trait]
impl ServiceDiscovery for NetworkServiceDiscovery {
    async fn discover_services(&self, filter: Option<&ServiceFilter>) -> Result<Vec<ServiceInfo>> {
        let entries = self.discover_entries(filter).await?;
        let services = entries
            .into_iter()
            .filter(|entry| match filter {
                Some(filter) => Self::matches_filter(entry, filter),
                None => true,
            })
            .map(ServiceInfo::from)
            .collect();
        Ok(services)
    }

    async fn get_service_details(&self, name: &str) -> Result<ServiceDetails> {
        let entry = self
            .retry_lookup(&format!("Service not found: {name}"), || async {
                let entries = self.discover_entries(None).await?;
                Ok(entries
                    .into_iter()
                    .find(|entry| Self::matches_lookup_name(entry, name)))
            })
            .await?;
        let info = ServiceInfo::from(entry.clone());

        // Try to get ServiceSpec with proto files
        let proto_files = match self.get_service_proto(&entry.name).await {
            Ok(proto_files) => proto_files,
            Err(e) => {
                tracing::warn!("Failed to get ServiceSpec for {name}: {e}");
                Vec::new()
            }
        };

        Ok(ServiceDetails {
            info,
            proto_files,
            dependencies: Vec::new(),
        })
    }

    // TODO: improve the performance of this method
    async fn check_service_availability(&self, name: &str) -> Result<AvailabilityStatus> {
        let available = self
            .retry_lookup(&format!("Service not found: {name}"), || async {
                let entries = self.discover_entries(None).await?;
                Ok(entries
                    .into_iter()
                    .any(|entry| Self::matches_lookup_name(&entry, name))
                    .then_some(true))
            })
            .await
            .unwrap_or(false);

        Ok(AvailabilityStatus {
            is_available: available,
            last_seen: available.then(SystemTime::now),
            health: if available {
                HealthStatus::Healthy
            } else {
                HealthStatus::Unknown
            },
        })
    }

    async fn get_service_proto(&self, name: &str) -> Result<Vec<ProtoFile>> {
        self.retry_lookup(&format!("Get service spec failed: {name}"), || async {
            self.ensure_connected().await?;
            let mut state_guard = self.state.lock().await;
            let state = state_guard
                .as_mut()
                .context("Signaling state not initialized")?;

            let request = GetServiceSpecRequest {
                name: name.to_string(),
            };
            let payload = actr_to_signaling::Payload::GetServiceSpecRequest(request);
            let envelope =
                Self::build_envelope(signaling_envelope::Flow::ActrToServer(ActrToSignaling {
                    source: state.actr_id.clone(),
                    credential: state.credential.clone(),
                    payload: Some(payload),
                }))?;

            let result = match Self::send_envelope(&mut state.socket, envelope).await {
                Ok(()) => loop {
                    let envelope = Self::read_envelope(&mut state.socket).await?;
                    match envelope.flow {
                        Some(signaling_envelope::Flow::ServerToActr(server)) => {
                            match server.payload {
                                Some(signaling_to_actr::Payload::GetServiceSpecResponse(
                                    response,
                                )) => {
                                    let proto_files = match response.result {
                                        Some(get_service_spec_response::Result::Success(
                                            success,
                                        )) => success
                                            .protobufs
                                            .into_iter()
                                            .map(|p| ProtoFile {
                                                name: format!("{}.proto", p.package),
                                                path: PathBuf::new(),
                                                content: p.content,
                                                services: Vec::new(),
                                            })
                                            .collect::<Vec<_>>(),
                                        Some(get_service_spec_response::Result::Error(error)) => {
                                            break Err(Self::as_error(
                                                "Get service spec failed",
                                                &error,
                                            ));
                                        }
                                        None => {
                                            break Err(anyhow!(
                                                "Get service spec response is missing result"
                                            ));
                                        }
                                    };
                                    break Ok(Some(proto_files));
                                }
                                Some(signaling_to_actr::Payload::Error(error)) => {
                                    break Err(Self::as_error("Get service spec failed", &error));
                                }
                                _ => {}
                            }
                        }
                        Some(signaling_envelope::Flow::EnvelopeError(error)) => {
                            break Err(Self::as_error("Get service spec failed", &error));
                        }
                        _ => {}
                    }
                },
                Err(err) => Err(err),
            };

            if result.is_err() {
                *state_guard = None;
            }

            result
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actr_protocol::Realm;

    fn sample_actor_id() -> ActrId {
        ActrId {
            serial_number: 42,
            r#type: ActrType {
                manufacturer: "acme".to_string(),
                name: "echo".to_string(),
                version: "1.0.0".to_string(),
            },
            realm: Realm { realm_id: 1001 },
        }
    }

    fn sample_credential() -> AIdCredential {
        AIdCredential {
            key_id: 7,
            claims: vec![1, 2, 3, 4].into(),
            signature: vec![5, 6, 7, 8].into(),
        }
    }

    #[test]
    fn build_signaling_url_with_identity_appends_auth_query() {
        let signaling_url = Url::parse("ws://localhost:8081/signaling/ws?existing=1").unwrap();
        let actor_id = sample_actor_id();
        let credential = sample_credential();

        let authenticated_url = NetworkServiceDiscovery::build_signaling_url_with_identity(
            &signaling_url,
            &actor_id,
            &credential,
        );
        let query_pairs: std::collections::HashMap<_, _> =
            authenticated_url.query_pairs().into_owned().collect();

        assert_eq!(query_pairs.get("existing"), Some(&"1".to_string()));
        assert_eq!(
            query_pairs.get("actor_id"),
            Some(&actor_id.to_string_repr())
        );
        assert_eq!(query_pairs.get("key_id"), Some(&"7".to_string()));
        assert_eq!(
            query_pairs.get("claims"),
            Some(&base64::engine::general_purpose::STANDARD.encode([1, 2, 3, 4]))
        );
        assert_eq!(
            query_pairs.get("signature"),
            Some(&base64::engine::general_purpose::STANDARD.encode([5, 6, 7, 8]))
        );
    }
}
