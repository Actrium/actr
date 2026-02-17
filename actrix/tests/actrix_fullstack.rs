use actr_protocol::{ActrType, Realm, RegisterRequest, register_response};
use actrix_common::aid::credential::validator::AIdCredentialValidator;
use actrix_common::realm::Realm as DbRealm;
use actrix_common::storage::db;
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
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use uuid::Uuid;

const START_TIMEOUT: Duration = Duration::from_secs(20);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const ACTRIX_SHARED_KEY: &str = "0123456789abcdef0123456789abcdef";

#[cfg(test)]
use serial_test::serial;

fn choose_port() -> u16 {
    if let Some(p) = std::env::var("ACTRIX_TEST_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        return p;
    }
    49080 + (std::process::id() as u16 % 1000)
}

fn write_fullstack_config(dir: &PathBuf, port: u16) -> PathBuf {
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
    if !db::is_database_initialized() {
        db::set_db_path(sqlite_dir).await.expect("init db path");
    }
    if !DbRealm::exists_by_realm_id(realm_id).await {
        let mut realm = DbRealm::new(realm_id, "test-realm".to_string());
        realm
            .save()
            .await
            .expect("create realm for signaling validation");
    }
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
    let config_path = write_fullstack_config(&tmp.path().to_path_buf(), port);
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
