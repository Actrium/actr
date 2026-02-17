use actr_protocol::acl_rule::{Permission, Principal};
use actr_protocol::{
    Acl, AclRule, ActrRelay, ActrType, Realm, RegisterRequest, RegisterResponse, RoleNegotiation,
    actr_relay, peer_to_signaling, register_response, route_candidates_response,
    signaling_envelope, signaling_to_actr,
};
use actrix_common::aid::credential::validator::AIdCredentialValidator;
use futures::{SinkExt, StreamExt};
use prost::Message;
use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use tokio::time::sleep;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message as WsMessage,
};
use uuid::Uuid;

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
type WsWrite = futures::stream::SplitSink<WsStream, WsMessage>;
type WsRead = futures::stream::SplitStream<WsStream>;

const START_TIMEOUT: Duration = Duration::from_secs(20);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const ACTRIX_SHARED_KEY: &str = "0123456789abcdef0123456789abcdef";
const DEFAULT_TOKEN_TTL: u64 = 3600;

#[cfg(test)]
use serial_test::serial;

fn choose_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral")
        .local_addr()
        .unwrap()
        .port()
}

fn write_fullstack_config(dir: &PathBuf, port: u16, token_ttl_secs: u64) -> PathBuf {
    let data_dir = dir.join("data");
    fs::create_dir_all(&data_dir).expect("create data dir");
    let config_path = dir.join("config.toml");
    let mut f = fs::File::create(&config_path).expect("create config file");
    writeln!(
        f,
        r#"
name = "actrix-fullstack-test"
enable = 25  # ENABLE_SIGNALING | ENABLE_AIS | ENABLE_KS
env = "dev"
sqlite_path = "{sqlite}"
actrix_shared_key = "{shared}"
location_tag = "local,test,fullstack"

[bind]
[bind.http]
domain_name = "localhost"
advertised_ip = "127.0.0.1"
ip = "127.0.0.1"
port = {port}

[bind.ice]
domain_name = "localhost"
advertised_ip = "127.0.0.1"
ip = "127.0.0.1"
port = 0

[turn]
advertised_ip = "127.0.0.1"
advertised_port = 3478
relay_port_range = "49152-65535"
realm = "actor-rtc.local"

[services.ks]
[services.ks.storage]
backend = "sqlite"
key_ttl_seconds = 3600
[services.ks.storage.sqlite]
path = "ks.db"

[services.ais]
[services.ais.server]
token_ttl_secs = {token_ttl}

[services.signaling]
[services.signaling.server]
ws_path = "/signaling"

[observability.log]
output = "console"
level = "info"

[process]
pid = "{pid}"
"#,
        sqlite = data_dir.display(),
        shared = ACTRIX_SHARED_KEY,
        port = port,
        token_ttl = token_ttl_secs,
        pid = dir.join("actrix.pid").display()
    )
    .expect("write config");
    config_path
}

fn spawn_actrix(config: &PathBuf, log_path: &PathBuf) -> Child {
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_actrix"));
    let log_file = fs::File::create(log_path).expect("create log file");
    Command::new(bin)
        .arg("--config")
        .arg(config)
        .stdout(Stdio::from(log_file.try_clone().expect("dup log")))
        .stderr(Stdio::from(log_file))
        .spawn()
        .expect("spawn actrix")
}

async fn ensure_realm(sqlite_dir: &PathBuf, realm_id: u32) {
    let db = actrix_common::storage::db::Database::new(sqlite_dir)
        .await
        .expect("init db");
    db.execute(&format!(
        "INSERT OR IGNORE INTO realm (realm_id, name, status, expires_at)
         VALUES ({realm_id}, 'test-realm', 0, NULL)"
    ))
    .await
    .expect("insert realm");
}

async fn connect_ws(port: u16) -> (WsWrite, WsRead) {
    let ws_url = format!("ws://127.0.0.1:{}/signaling/ws", port);
    let (ws_stream, _) = connect_async(&ws_url).await.expect("ws connect");
    ws_stream.split()
}

fn make_envelope(flow: signaling_envelope::Flow) -> actr_protocol::SignalingEnvelope {
    actr_protocol::SignalingEnvelope {
        envelope_version: 1,
        envelope_id: Uuid::new_v4().to_string(),
        timestamp: prost_types::Timestamp {
            seconds: chrono::Utc::now().timestamp(),
            nanos: 0,
        },
        reply_for: None,
        traceparent: None,
        tracestate: None,
        flow: Some(flow),
    }
}

async fn send_envelope(write: &mut WsWrite, env: actr_protocol::SignalingEnvelope) {
    let mut buf = Vec::new();
    env.encode(&mut buf).expect("encode envelope");
    write
        .send(WsMessage::Binary(buf.into()))
        .await
        .expect("send envelope");
}

async fn recv_envelope(read: &mut WsRead) -> actr_protocol::SignalingEnvelope {
    let resp = read.next().await.expect("ws response").expect("ws msg");
    match resp {
        WsMessage::Binary(data) => {
            actr_protocol::SignalingEnvelope::decode(&data[..]).expect("decode signaling resp")
        }
        other => panic!("expected binary ws message, got {other:?}"),
    }
}

async fn ws_register(
    port: u16,
    manufacturer: &str,
    name: &str,
    acl: Option<Acl>,
) -> (WsWrite, WsRead, register_response::RegisterOk) {
    let (mut write, mut read) = connect_ws(port).await;

    let register_req = RegisterRequest {
        actr_type: ActrType {
            manufacturer: manufacturer.to_string(),
            name: name.to_string(),
        },
        realm: Realm { realm_id: 1001 },
        service_spec: None,
        acl,
    };

    let env = make_envelope(signaling_envelope::Flow::PeerToServer(
        actr_protocol::PeerToSignaling {
            payload: Some(peer_to_signaling::Payload::RegisterRequest(register_req)),
        },
    ));
    send_envelope(&mut write, env).await;
    let resp = recv_envelope(&mut read).await;
    let register_ok = match resp.flow {
        Some(signaling_envelope::Flow::ServerToActr(server_msg)) => match server_msg.payload {
            Some(signaling_to_actr::Payload::RegisterResponse(RegisterResponse {
                result: Some(register_response::Result::Success(ok)),
            })) => ok,
            other => panic!("expected register success, got {other:?}"),
        },
        other => panic!("unexpected flow: {other:?}"),
    };

    (write, read, register_ok)
}

async fn wait_for_health(url: &str, child: &mut Child, log_path: &PathBuf) {
    let client = reqwest::Client::new();
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap_or(None) {
            let log = fs::read_to_string(log_path).unwrap_or_default();
            panic!("actrix exited early: status={status:?}\nlogs:\n{log}");
        }

        if let Ok(resp) = client.get(url).send().await {
            if resp.status().is_success() {
                return;
            }
        }
        if start.elapsed() > START_TIMEOUT {
            let log = fs::read_to_string(log_path).unwrap_or_default();
            panic!("health check not ready at {}\nlogs:\n{}", url, log);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn graceful_shutdown(mut child: Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(child.id() as i32, libc::SIGINT);
    }
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => return,
            Ok(None) => {
                if start.elapsed() > SHUTDOWN_TIMEOUT {
                    let _ = child.kill();
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(_) => return,
        }
    }
}

#[tokio::test]
#[serial]
async fn actrix_end_to_end_register_and_health() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let port = choose_port();
    let config_path = write_fullstack_config(&tmp.path().to_path_buf(), port, DEFAULT_TOKEN_TTL);
    let log_path = tmp.path().join("actrix_fullstack.log");
    ensure_realm(&tmp.path().join("data"), 1001).await;
    let mut child = spawn_actrix(&config_path, &log_path);

    let base = format!("http://127.0.0.1:{port}");
    let ks_health = format!("{base}/ks/health");
    let ais_health = format!("{base}/ais/health");
    let signaling_health = format!("{base}/signaling/health");

    wait_for_health(&ks_health, &mut child, &log_path).await;
    wait_for_health(&ais_health, &mut child, &log_path).await;
    wait_for_health(&signaling_health, &mut child, &log_path).await;
    ensure_realm(&tmp.path().join("data"), 1001).await;

    let client = reqwest::Client::new();

    // KS health JSON
    let ks_resp = client.get(&ks_health).send().await.expect("ks health");
    assert!(ks_resp.status().is_success());
    let ks_body: Value = ks_resp.json().await.expect("ks health json");
    assert_eq!(ks_body["status"], "healthy");

    // AIS health JSON
    let ais_resp = client.get(&ais_health).send().await.expect("ais health");
    assert!(ais_resp.status().is_success());
    let ais_body: Value = ais_resp.json().await.expect("ais health json");
    assert_eq!(ais_body["status"], "healthy");

    // Signaling health plain text
    let sig_resp = client
        .get(&signaling_health)
        .send()
        .await
        .expect("sig health");
    assert!(sig_resp.status().is_success());
    let sig_text = sig_resp.text().await.expect("sig text");
    assert!(
        sig_text.to_lowercase().contains("healthy"),
        "signaling health text: {sig_text}"
    );

    // Register an actor via AIS HTTP (protobuf body)
    let register_req = RegisterRequest {
        actr_type: ActrType {
            manufacturer: "test-mfg".to_string(),
            name: "device".to_string(),
        },
        realm: Realm { realm_id: 1001 },
        service_spec: None,
        acl: None,
    };
    let body = register_req.encode_to_vec();
    let register_url = format!("{base}/ais/register");
    let rsp_bytes = client
        .post(&register_url)
        .body(body)
        .send()
        .await
        .expect("register call")
        .bytes()
        .await
        .expect("register bytes")
        .to_vec();
    let register_rsp =
        actr_protocol::RegisterResponse::decode(&*rsp_bytes).expect("decode register response");
    let ok = match register_rsp.result.expect("result missing") {
        register_response::Result::Success(ok) => ok,
        register_response::Result::Error(err) => {
            panic!("register failed: {:?}", err);
        }
    };
    assert_eq!(ok.actr_id.realm.realm_id, 1001);

    // Validate credential through AIdCredentialValidator (fetches key via KS gRPC)
    let ks_client_cfg = actrix_common::config::ks::KsClientConfig {
        endpoint: "http://127.0.0.1:50052".to_string(),
        timeout_seconds: 5,
        enable_tls: false,
        tls_domain: None,
        ca_cert: None,
        client_cert: None,
        client_key: None,
    };
    AIdCredentialValidator::init(&ks_client_cfg, ACTRIX_SHARED_KEY, tmp.path())
        .await
        .expect("validator init");
    let (claims, _) = AIdCredentialValidator::check(&ok.credential, 1001)
        .await
        .expect("validate credential");
    assert_eq!(claims.realm_id, 1001);

    // WebSocket signaling ping/pong with valid credential
    let ws_url = format!("ws://127.0.0.1:{}/signaling/ws", port);
    let (ws_stream, _) = connect_async(&ws_url).await.expect("ws connect");
    let (mut write, mut read) = ws_stream.split();

    let ping_msg = actr_protocol::ActrToSignaling {
        source: ok.actr_id.clone(),
        credential: ok.credential.clone(),
        payload: Some(actr_protocol::actr_to_signaling::Payload::Ping(
            actr_protocol::Ping {
                availability: 100,
                mailbox_backlog: 0.0,
                power_reserve: 80.0,
                ..Default::default()
            },
        )),
    };
    let envelope = actr_protocol::SignalingEnvelope {
        envelope_version: 1,
        envelope_id: Uuid::new_v4().to_string(),
        timestamp: prost_types::Timestamp {
            seconds: chrono::Utc::now().timestamp(),
            nanos: 0,
        },
        reply_for: None,
        traceparent: None,
        tracestate: None,
        flow: Some(actr_protocol::signaling_envelope::Flow::ActrToServer(
            ping_msg,
        )),
    };
    let mut buf = Vec::new();
    envelope.encode(&mut buf).expect("encode envelope");
    write
        .send(WsMessage::Binary(buf.into()))
        .await
        .expect("send ping");

    let resp = read.next().await.expect("ws response").expect("ws msg");
    let pong_env = match resp {
        WsMessage::Binary(data) => {
            actr_protocol::SignalingEnvelope::decode(&data[..]).expect("decode signaling resp")
        }
        other => panic!("expected binary ws message, got {other:?}"),
    };
    match pong_env.flow {
        Some(actr_protocol::signaling_envelope::Flow::ServerToActr(server_msg)) => {
            match server_msg.payload {
                Some(actr_protocol::signaling_to_actr::Payload::Pong(_)) => {}
                other => panic!("expected Pong, got {other:?}"),
            }
        }
        other => panic!("unexpected flow: {other:?}"),
    }

    // Tamper credential to ensure 401 is returned
    let mut bad_cred = ok.credential.clone();
    if !bad_cred.encrypted_token.is_empty() {
        let mut tampered = bad_cred.encrypted_token.to_vec();
        tampered[0] ^= 0xFF;
        bad_cred.encrypted_token = tampered.into();
    }
    let bad_msg = actr_protocol::ActrToSignaling {
        source: ok.actr_id.clone(),
        credential: bad_cred,
        payload: Some(actr_protocol::actr_to_signaling::Payload::Ping(
            actr_protocol::Ping {
                availability: 50,
                mailbox_backlog: 1.0,
                power_reserve: 50.0,
                ..Default::default()
            },
        )),
    };
    let mut buf = Vec::new();
    actr_protocol::SignalingEnvelope {
        envelope_version: 1,
        envelope_id: Uuid::new_v4().to_string(),
        timestamp: prost_types::Timestamp {
            seconds: chrono::Utc::now().timestamp(),
            nanos: 0,
        },
        reply_for: None,
        traceparent: None,
        tracestate: None,
        flow: Some(actr_protocol::signaling_envelope::Flow::ActrToServer(
            bad_msg,
        )),
    }
    .encode(&mut buf)
    .expect("encode bad envelope");
    write
        .send(WsMessage::Binary(buf.into()))
        .await
        .expect("send bad ping");

    let resp = read.next().await.expect("ws response").expect("ws msg");
    let err_env = match resp {
        WsMessage::Binary(data) => {
            actr_protocol::SignalingEnvelope::decode(&data[..]).expect("decode signaling resp")
        }
        other => panic!("expected binary ws message, got {other:?}"),
    };
    match err_env.flow {
        Some(actr_protocol::signaling_envelope::Flow::ServerToActr(server_msg)) => {
            match server_msg.payload {
                Some(actr_protocol::signaling_to_actr::Payload::Error(err)) => {
                    assert_eq!(err.code, 401, "expected 401 for bad credential");
                }
                other => panic!("expected Error, got {other:?}"),
            }
        }
        other => panic!("unexpected flow: {other:?}"),
    }

    graceful_shutdown(child);
}

#[tokio::test]
#[serial]
async fn signaling_register_and_discovery_acl_allow() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let port = choose_port();
    let config_path = write_fullstack_config(&tmp.path().to_path_buf(), port, DEFAULT_TOKEN_TTL);
    let log_path = tmp.path().join("actrix_fullstack.log");
    ensure_realm(&tmp.path().join("data"), 1001).await;
    let mut child = spawn_actrix(&config_path, &log_path);

    let base = format!("http://127.0.0.1:{port}");
    wait_for_health(&format!("{base}/ks/health"), &mut child, &log_path).await;
    wait_for_health(&format!("{base}/ais/health"), &mut child, &log_path).await;
    wait_for_health(&format!("{base}/signaling/health"), &mut child, &log_path).await;
    ensure_realm(&tmp.path().join("data"), 1001).await;

    // Service registers with ACL allowing client:* to discover
    let acl = Acl {
        rules: vec![AclRule {
            principals: vec![Principal {
                realm: Some(Realm { realm_id: 1001 }),
                actr_type: Some(ActrType {
                    manufacturer: "mfg".into(),
                    name: "client".into(),
                }),
            }],
            permission: Permission::Allow as i32,
        }],
    };
    let (_ws_service_write, _ws_service_read, _service_ok) =
        ws_register(port, "mfg", "svc", Some(acl)).await;

    // Client registers (no ACL needed)
    let (mut client_write, mut client_read, client_ok) =
        ws_register(port, "mfg", "client", None).await;

    // Discovery should return the service type because ACL allows it
    let discover = actr_protocol::ActrToSignaling {
        source: client_ok.actr_id.clone(),
        credential: client_ok.credential.clone(),
        payload: Some(actr_protocol::actr_to_signaling::Payload::DiscoveryRequest(
            actr_protocol::DiscoveryRequest {
                manufacturer: Some("mfg".into()),
                limit: Some(10),
            },
        )),
    };
    let env = make_envelope(signaling_envelope::Flow::ActrToServer(discover));
    send_envelope(&mut client_write, env).await;
    let resp = recv_envelope(&mut client_read).await;
    match resp.flow {
        Some(signaling_envelope::Flow::ServerToActr(server_msg)) => match server_msg.payload {
            Some(signaling_to_actr::Payload::DiscoveryResponse(rsp)) => match rsp.result {
                Some(actr_protocol::discovery_response::Result::Success(ok)) => {
                    assert!(
                        !ok.entries.is_empty(),
                        "expected at least one service entry"
                    );
                    assert_eq!(ok.entries[0].actr_type.name, "svc");
                    assert_eq!(ok.entries[0].actr_type.manufacturer, "mfg");
                }
                other => panic!("unexpected discovery result {other:?}"),
            },
            other => panic!("unexpected payload {other:?}"),
        },
        other => panic!("unexpected flow {other:?}"),
    }

    graceful_shutdown(child);
}

#[tokio::test]
#[serial]
async fn signaling_discovery_acl_denied() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let port = choose_port();
    let config_path = write_fullstack_config(&tmp.path().to_path_buf(), port, DEFAULT_TOKEN_TTL);
    let log_path = tmp.path().join("actrix_fullstack.log");
    ensure_realm(&tmp.path().join("data"), 1001).await;
    let mut child = spawn_actrix(&config_path, &log_path);

    let base = format!("http://127.0.0.1:{port}");
    wait_for_health(&format!("{base}/ks/health"), &mut child, &log_path).await;
    wait_for_health(&format!("{base}/ais/health"), &mut child, &log_path).await;
    wait_for_health(&format!("{base}/signaling/health"), &mut child, &log_path).await;
    ensure_realm(&tmp.path().join("data"), 1001).await;

    // Service registers without ACL (default deny)
    let (_ws_service_write, _ws_service_read, _service_ok) =
        ws_register(port, "mfg", "svc-deny", None).await;

    // Client registers
    let (mut client_write, mut client_read, client_ok) =
        ws_register(port, "mfg", "client", None).await;

    let discover = actr_protocol::ActrToSignaling {
        source: client_ok.actr_id.clone(),
        credential: client_ok.credential.clone(),
        payload: Some(actr_protocol::actr_to_signaling::Payload::DiscoveryRequest(
            actr_protocol::DiscoveryRequest {
                manufacturer: Some("mfg".into()),
                limit: Some(10),
            },
        )),
    };
    let env = make_envelope(signaling_envelope::Flow::ActrToServer(discover));
    send_envelope(&mut client_write, env).await;
    let resp = recv_envelope(&mut client_read).await;
    match resp.flow {
        Some(signaling_envelope::Flow::ServerToActr(server_msg)) => match server_msg.payload {
            Some(signaling_to_actr::Payload::DiscoveryResponse(rsp)) => match rsp.result {
                Some(actr_protocol::discovery_response::Result::Success(ok)) => {
                    assert!(
                        ok.entries.is_empty(),
                        "ACL default deny should yield empty discovery list"
                    );
                }
                other => panic!("unexpected discovery result {other:?}"),
            },
            other => panic!("unexpected payload {other:?}"),
        },
        other => panic!("unexpected flow {other:?}"),
    }

    graceful_shutdown(child);
}

#[tokio::test]
#[serial]
async fn signaling_rejects_expired_credential() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let port = choose_port();
    let config_path = write_fullstack_config(&tmp.path().to_path_buf(), port, 1);
    let log_path = tmp.path().join("actrix_fullstack.log");
    ensure_realm(&tmp.path().join("data"), 1001).await;
    let mut child = spawn_actrix(&config_path, &log_path);

    let base = format!("http://127.0.0.1:{port}");
    wait_for_health(&format!("{base}/ks/health"), &mut child, &log_path).await;
    wait_for_health(&format!("{base}/ais/health"), &mut child, &log_path).await;
    wait_for_health(&format!("{base}/signaling/health"), &mut child, &log_path).await;
    ensure_realm(&tmp.path().join("data"), 1001).await;

    // Register actor
    let (mut write, mut read, ok) = ws_register(port, "mfg", "shortlived", None).await;

    // Wait for credential to expire
    sleep(Duration::from_secs(2)).await;

    // Send ping with expired credential -> expect 401 error
    let ping_msg = actr_protocol::ActrToSignaling {
        source: ok.actr_id.clone(),
        credential: ok.credential.clone(),
        payload: Some(actr_protocol::actr_to_signaling::Payload::Ping(
            actr_protocol::Ping {
                availability: 50,
                mailbox_backlog: 1.0,
                power_reserve: 50.0,
                ..Default::default()
            },
        )),
    };
    let env = make_envelope(signaling_envelope::Flow::ActrToServer(ping_msg));
    send_envelope(&mut write, env).await;
    let resp = recv_envelope(&mut read).await;
    match resp.flow {
        Some(signaling_envelope::Flow::ServerToActr(server_msg)) => match server_msg.payload {
            Some(signaling_to_actr::Payload::Error(err)) => {
                assert_eq!(err.code, 401, "expired credential should be rejected");
            }
            other => panic!("expected error for expired credential, got {other:?}"),
        },
        other => panic!("unexpected flow {other:?}"),
    }

    graceful_shutdown(child);
}

#[tokio::test]
#[serial]
async fn signaling_route_candidates_with_acl() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let port = choose_port();
    let config_path = write_fullstack_config(&tmp.path().to_path_buf(), port, DEFAULT_TOKEN_TTL);
    let log_path = tmp.path().join("actrix_fullstack.log");
    ensure_realm(&tmp.path().join("data"), 1001).await;
    let mut child = spawn_actrix(&config_path, &log_path);

    let base = format!("http://127.0.0.1:{port}");
    wait_for_health(&format!("{base}/ks/health"), &mut child, &log_path).await;
    wait_for_health(&format!("{base}/ais/health"), &mut child, &log_path).await;
    wait_for_health(&format!("{base}/signaling/health"), &mut child, &log_path).await;
    ensure_realm(&tmp.path().join("data"), 1001).await;

    // Service registers with ACL allowing client:sdp to discover/route
    let acl = Acl {
        rules: vec![AclRule {
            principals: vec![Principal {
                realm: Some(Realm { realm_id: 1001 }),
                actr_type: Some(ActrType {
                    manufacturer: "mfg".into(),
                    name: "client-sdp".into(),
                }),
            }],
            permission: Permission::Allow as i32,
        }],
    };
    let (_svc_w, _svc_r, svc_ok) = ws_register(port, "mfg", "svc-rtp", Some(acl)).await;

    // Client registers
    let (mut cli_w, mut cli_r, cli_ok) = ws_register(port, "mfg", "client-sdp", None).await;

    // RouteCandidates should return the service because ACL allows
    let route_req = actr_protocol::ActrToSignaling {
        source: cli_ok.actr_id.clone(),
        credential: cli_ok.credential.clone(),
        payload: Some(
            actr_protocol::actr_to_signaling::Payload::RouteCandidatesRequest(
                actr_protocol::RouteCandidatesRequest {
                    target_type: ActrType {
                        manufacturer: "mfg".into(),
                        name: "svc-rtp".into(),
                    },
                    client_fingerprint: "".into(),
                    criteria: Some(
                        actr_protocol::route_candidates_request::NodeSelectionCriteria {
                            candidate_count: 5,
                            ranking_factors: vec![],
                            minimal_dependency_requirement: None,
                            minimal_health_requirement: None,
                        },
                    ),
                    client_location: None,
                },
            ),
        ),
    };
    send_envelope(
        &mut cli_w,
        make_envelope(signaling_envelope::Flow::ActrToServer(route_req)),
    )
    .await;
    let resp = recv_envelope(&mut cli_r).await;
    match resp.flow {
        Some(signaling_envelope::Flow::ServerToActr(server_msg)) => match server_msg.payload {
            Some(signaling_to_actr::Payload::RouteCandidatesResponse(rsp)) => match rsp.result {
                Some(route_candidates_response::Result::Success(ok)) => {
                    assert!(
                        ok.candidates
                            .iter()
                            .any(|id| id.serial_number == svc_ok.actr_id.serial_number),
                        "expected routed candidate"
                    );
                }
                other => panic!("unexpected route result {other:?}"),
            },
            other => panic!("unexpected payload {other:?}"),
        },
        other => panic!("unexpected flow {other:?}"),
    }

    graceful_shutdown(child);
}

#[tokio::test]
#[serial]
async fn signaling_route_candidates_acl_denied() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let port = choose_port();
    let config_path = write_fullstack_config(&tmp.path().to_path_buf(), port, DEFAULT_TOKEN_TTL);
    let log_path = tmp.path().join("actrix_fullstack.log");
    ensure_realm(&tmp.path().join("data"), 1001).await;
    let mut child = spawn_actrix(&config_path, &log_path);

    let base = format!("http://127.0.0.1:{port}");
    wait_for_health(&format!("{base}/ks/health"), &mut child, &log_path).await;
    wait_for_health(&format!("{base}/ais/health"), &mut child, &log_path).await;
    wait_for_health(&format!("{base}/signaling/health"), &mut child, &log_path).await;
    ensure_realm(&tmp.path().join("data"), 1001).await;

    // Service registers without ACL (default deny)
    let (_svc_w, _svc_r, _svc_ok) = ws_register(port, "mfg", "svc-deny-route", None).await;

    // Client registers
    let (mut cli_w, mut cli_r, cli_ok) = ws_register(port, "mfg", "client-sdp", None).await;

    let route_req = actr_protocol::ActrToSignaling {
        source: cli_ok.actr_id.clone(),
        credential: cli_ok.credential.clone(),
        payload: Some(
            actr_protocol::actr_to_signaling::Payload::RouteCandidatesRequest(
                actr_protocol::RouteCandidatesRequest {
                    target_type: ActrType {
                        manufacturer: "mfg".into(),
                        name: "svc-deny-route".into(),
                    },
                    client_fingerprint: "".into(),
                    criteria: Some(
                        actr_protocol::route_candidates_request::NodeSelectionCriteria {
                            candidate_count: 5,
                            ranking_factors: vec![],
                            minimal_dependency_requirement: None,
                            minimal_health_requirement: None,
                        },
                    ),
                    client_location: None,
                },
            ),
        ),
    };
    send_envelope(
        &mut cli_w,
        make_envelope(signaling_envelope::Flow::ActrToServer(route_req)),
    )
    .await;
    let resp = recv_envelope(&mut cli_r).await;
    match resp.flow {
        Some(signaling_envelope::Flow::ServerToActr(server_msg)) => match server_msg.payload {
            Some(signaling_to_actr::Payload::RouteCandidatesResponse(rsp)) => match rsp.result {
                Some(route_candidates_response::Result::Success(ok)) => {
                    assert!(
                        ok.candidates.is_empty(),
                        "ACL deny should yield empty route candidates"
                    );
                }
                other => panic!("unexpected route result {other:?}"),
            },
            other => panic!("unexpected payload {other:?}"),
        },
        other => panic!("unexpected flow {other:?}"),
    }

    graceful_shutdown(child);
}

#[tokio::test]
#[serial]
async fn signaling_actr_relay_role_assignment() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let port = choose_port();
    let config_path = write_fullstack_config(&tmp.path().to_path_buf(), port, DEFAULT_TOKEN_TTL);
    let log_path = tmp.path().join("actrix_fullstack.log");
    ensure_realm(&tmp.path().join("data"), 1001).await;
    let mut child = spawn_actrix(&config_path, &log_path);

    let base = format!("http://127.0.0.1:{port}");
    wait_for_health(&format!("{base}/ks/health"), &mut child, &log_path).await;
    wait_for_health(&format!("{base}/ais/health"), &mut child, &log_path).await;
    wait_for_health(&format!("{base}/signaling/health"), &mut child, &log_path).await;
    ensure_realm(&tmp.path().join("data"), 1001).await;

    // Service registers with ACL allowing client-offer
    let acl = Acl {
        rules: vec![AclRule {
            principals: vec![Principal {
                realm: Some(Realm { realm_id: 1001 }),
                actr_type: Some(ActrType {
                    manufacturer: "mfg".into(),
                    name: "client-offer".into(),
                }),
            }],
            permission: Permission::Allow as i32,
        }],
    };
    let (mut svc_w, mut svc_r, svc_ok) = ws_register(port, "mfg", "svc-relay", Some(acl)).await;

    // Client registers
    let (mut cli_w, mut cli_r, cli_ok) = ws_register(port, "mfg", "client-offer", None).await;

    // Send role negotiation relay
    let relay = ActrRelay {
        source: cli_ok.actr_id.clone(),
        credential: cli_ok.credential.clone(),
        target: svc_ok.actr_id.clone(),
        payload: Some(actr_relay::Payload::RoleNegotiation(RoleNegotiation {
            from: cli_ok.actr_id.clone(),
            to: svc_ok.actr_id.clone(),
            realm_id: 1001,
        })),
    };
    send_envelope(
        &mut cli_w,
        make_envelope(signaling_envelope::Flow::ActrRelay(relay)),
    )
    .await;

    // Client should receive RoleAssignment
    let client_resp = recv_envelope(&mut cli_r).await;
    let mut got_client_assignment = false;
    if let Some(signaling_envelope::Flow::ActrRelay(relay_msg)) = client_resp.flow {
        if let Some(actr_relay::Payload::RoleAssignment(assign)) = relay_msg.payload {
            got_client_assignment = true;
            assert!(assign.remote_fixed.is_some());
        }
    }
    assert!(got_client_assignment, "client should get role assignment");

    // Service should receive RoleAssignment
    let service_resp = recv_envelope(&mut svc_r).await;
    let mut got_service_assignment = false;
    if let Some(signaling_envelope::Flow::ActrRelay(relay_msg)) = service_resp.flow {
        if let Some(actr_relay::Payload::RoleAssignment(assign)) = relay_msg.payload {
            got_service_assignment = true;
            assert!(assign.remote_fixed.is_some());
        }
    }
    assert!(got_service_assignment, "service should get role assignment");

    // Cleanup
    let _ = cli_w.send(WsMessage::Close(None)).await;
    let _ = svc_w.send(WsMessage::Close(None)).await;
    graceful_shutdown(child);
}
