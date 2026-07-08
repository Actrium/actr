//! Multi-peer WebRTC concurrency integration tests.
//!
//! The concurrency in this file is intentional and contained within one Tokio
//! runtime. This models one actr accepting independent PeerConnections from
//! multiple client actrs without relying on libtest to run unrelated harnesses
//! concurrently.

use actr_hyper::outbound::PeerGate;
use actr_hyper::test_support::{
    TestHarness, create_peer_with_websocket, make_actor_id, spawn_response_receiver,
};
use actr_hyper::transport::{DefaultWireBuilder, DefaultWireBuilderConfig, PeerTransport};
use actr_protocol::{Direction, RpcEnvelope};
use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio::sync::Barrier;
use tokio::task::JoinSet;

const SERVER: u64 = 100;
const CONCURRENT_CLIENT_COUNTS: [usize; 3] = [20, 40, 80];
const RPC_TIMEOUT: Duration = Duration::from_secs(60);
const WAVE_TIMEOUT: Duration = Duration::from_secs(120);
const SEQUENTIAL_CLIENT_COUNT: usize = 100;
const SEQUENTIAL_CONNECT_INTERVAL: Duration = Duration::from_secs(1);
const SEQUENTIAL_CASE_TIMEOUT: Duration = Duration::from_secs(15);
const CHILD_START_TIMEOUT: Duration = Duration::from_secs(60);
const CHILD_EXIT_TIMEOUT: Duration = Duration::from_secs(10);
const CHILD_PROCESS_TEST: &str = "sequential_churn_client_process";
const CHILD_SERVER_URL_ENV: &str = "ACTR_CHURN_SERVER_URL";
const CHILD_SERIAL_ENV: &str = "ACTR_CHURN_CLIENT_SERIAL";
const CHILD_READY_ADDR_ENV: &str = "ACTR_CHURN_READY_ADDR";
const CHILD_READY_PREFIX: &str = "ACTR_CHURN_CLIENT_READY";

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_file(true)
        .with_line_number(true)
        .with_test_writer()
        .try_init()
        .ok();
}

fn client_serials(start: u64, count: usize) -> Vec<u64> {
    (0..count).map(|offset| start + offset as u64).collect()
}

async fn run_concurrent_rpc_wave(harness: &TestHarness, client_serials: &[u64], wave: &str) {
    let barrier = Arc::new(Barrier::new(client_serials.len() + 1));
    let server_id = harness.peer(SERVER).id.clone();
    let mut requests = JoinSet::new();

    for &client_serial in client_serials {
        let barrier = barrier.clone();
        let gate = harness.peer(client_serial).gate.clone();
        let target = server_id.clone();
        let request_id = format!("multi-peer-{wave}-{client_serial}");

        requests.spawn(async move {
            barrier.wait().await;
            let envelope = RpcEnvelope {
                request_id: request_id.clone(),
                route_key: "test.multi-peer".to_string(),
                payload: Some(bytes::Bytes::from(client_serial.to_string())),
                direction: Some(Direction::Request as i32),
                timeout_ms: RPC_TIMEOUT.as_millis() as i64,
                ..Default::default()
            };
            let result = gate.send_request(&target, envelope).await;
            (client_serial, request_id, result)
        });
    }

    // Release every client in the same scheduler turn so the first wave also
    // exercises concurrent lazy transport creation and WebRTC negotiation.
    barrier.wait().await;

    tokio::time::timeout(WAVE_TIMEOUT, async {
        while let Some(joined) = requests.join_next().await {
            let (client_serial, request_id, result) =
                joined.expect("concurrent RPC task should not panic");
            let response = result.unwrap_or_else(|error| {
                panic!("client {client_serial} request {request_id} failed during {wave}: {error}")
            });
            assert_eq!(
                response.as_ref(),
                b"pong",
                "client {client_serial} received an unexpected response for {request_id}"
            );
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{wave} RPC wave did not finish within {WAVE_TIMEOUT:?}"));
}

async fn assert_ready_session(harness: &TestHarness, client_serial: u64) {
    let server = harness.peer(SERVER);
    let client = harness.peer(client_serial);

    assert!(
        server
            .coordinator
            .get_peer_session_id(&client.id)
            .await
            .is_some(),
        "server has no session for client {client_serial}"
    );
    assert!(
        server
            .coordinator
            .has_open_data_channel_for_test(&client.id)
            .await
            .expect("server DataChannel state should be readable"),
        "server DataChannel is not open for client {client_serial}"
    );
    assert!(
        client
            .coordinator
            .get_peer_session_id(&server.id)
            .await
            .is_some(),
        "client {client_serial} has no session for the server"
    );
    assert!(
        client
            .coordinator
            .has_open_data_channel_for_test(&server.id)
            .await
            .expect("client DataChannel state should be readable"),
        "client {client_serial} DataChannel is not open"
    );
}

async fn server_sessions(harness: &TestHarness, client_serials: &[u64]) -> HashMap<u64, u64> {
    let server = harness.peer(SERVER);
    let mut sessions = HashMap::new();

    for &client_serial in client_serials {
        let client_id = &harness.peer(client_serial).id;
        let session_id = server
            .coordinator
            .get_peer_session_id(client_id)
            .await
            .unwrap_or_else(|| panic!("server has no session for client {client_serial}"));
        assert!(
            server
                .coordinator
                .has_open_data_channel_for_test(client_id)
                .await
                .expect("server DataChannel state should be readable"),
            "server DataChannel is not open for client {client_serial}"
        );
        sessions.insert(client_serial, session_id);
    }

    sessions
}

async fn assert_server_ready_for_external_client(harness: &TestHarness, client_serial: u64) {
    let client_id = make_actor_id(client_serial);
    let server = harness.peer(SERVER);

    assert!(
        server
            .coordinator
            .get_peer_session_id(&client_id)
            .await
            .is_some(),
        "server has no session for external client {client_serial}"
    );
    assert!(
        server
            .coordinator
            .has_open_data_channel_for_test(&client_id)
            .await
            .expect("server DataChannel state should be readable"),
        "server DataChannel is not open for external client {client_serial}"
    );
}

struct ChurnClientProcess {
    child: Child,
    stderr_task: tokio::task::JoinHandle<String>,
}

impl ChurnClientProcess {
    async fn kill(mut self, client_serial: u64) {
        self.child
            .start_kill()
            .unwrap_or_else(|error| panic!("failed to kill client {client_serial}: {error}"));

        let status = tokio::time::timeout(CHILD_EXIT_TIMEOUT, self.child.wait())
            .await
            .unwrap_or_else(|_| {
                panic!("killed client {client_serial} did not exit within {CHILD_EXIT_TIMEOUT:?}")
            })
            .unwrap_or_else(|error| {
                panic!("failed waiting for killed client {client_serial}: {error}")
            });
        let stderr = self
            .stderr_task
            .await
            .expect("client stderr reader task should not panic");

        assert!(
            !status.success(),
            "client {client_serial} unexpectedly exited successfully instead of being killed; stderr: {stderr}"
        );
    }
}

async fn spawn_churn_client(server_url: &str, client_serial: u64) -> ChurnClientProcess {
    let ready_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("client readiness listener should bind");
    let ready_addr = ready_listener
        .local_addr()
        .expect("client readiness listener address should be available");
    let mut child =
        Command::new(std::env::current_exe().expect("test executable path should exist"));
    child
        .arg(CHILD_PROCESS_TEST)
        .arg("--ignored")
        .arg("--exact")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(CHILD_SERVER_URL_ENV, server_url)
        .env(CHILD_SERIAL_ENV, client_serial.to_string())
        .env(CHILD_READY_ADDR_ENV, ready_addr.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = child
        .spawn()
        .unwrap_or_else(|error| panic!("failed to spawn client {client_serial}: {error}"));
    let stderr = child
        .stderr
        .take()
        .expect("spawned client stderr should be piped");
    let stderr_task = tokio::spawn(async move {
        let mut output = String::new();
        BufReader::new(stderr)
            .read_to_string(&mut output)
            .await
            .expect("client stderr should be readable");
        output
    });

    let ready_marker = format!("{CHILD_READY_PREFIX} {client_serial}");
    let saw_ready = tokio::time::timeout(CHILD_START_TIMEOUT, async {
        let (mut ready_stream, _) = ready_listener
            .accept()
            .await
            .expect("client readiness connection should be accepted");
        let mut marker = String::new();
        ready_stream
            .read_to_string(&mut marker)
            .await
            .expect("client readiness marker should be readable");
        marker.trim() == ready_marker
    })
    .await
    .unwrap_or(false);

    if !saw_ready {
        let status_before_kill = child
            .try_wait()
            .expect("client exit status should be readable");
        let _ = child.start_kill();
        let status = child.wait().await.ok().or(status_before_kill);
        let stderr = stderr_task
            .await
            .expect("client stderr reader task should not panic");
        panic!(
            "client {client_serial} did not report RPC readiness within {CHILD_START_TIMEOUT:?}; status: {status:?}; stderr: {stderr}"
        );
    }

    ChurnClientProcess { child, stderr_task }
}

async fn run_multi_peer_concurrency_case(client_count: usize) {
    let mut harness = TestHarness::new().await;
    harness.add_peer(SERVER).await;
    let clients = client_serials(200, client_count);
    for &client_serial in &clients {
        harness.add_peer(client_serial).await;
    }

    // Each endpoint has exactly one consumer of coordinator.receive_message():
    // the server consumes requests and every client consumes responses.
    let mut background_tasks = vec![
        harness
            .peer(SERVER)
            .start_echo_responder(&format!("multi_peer_{client_count}_server")),
    ];
    for &client_serial in &clients {
        background_tasks.push(
            harness
                .peer(client_serial)
                .start_response_receiver(&format!(
                    "multi_peer_{client_count}_client_{client_serial}"
                )),
        );
    }

    run_concurrent_rpc_wave(&harness, &clients, &format!("{client_count}-initial")).await;

    let initial_server_sessions = server_sessions(&harness, &clients).await;
    let unique_sessions: HashSet<_> = initial_server_sessions.values().copied().collect();
    assert_eq!(
        unique_sessions.len(),
        clients.len(),
        "each client must use an independent server-side PeerConnection session"
    );

    for &client_serial in &clients {
        assert_ready_session(&harness, client_serial).await;
    }

    run_concurrent_rpc_wave(&harness, &clients, &format!("{client_count}-reuse")).await;
    assert_eq!(
        server_sessions(&harness, &clients).await,
        initial_server_sessions,
        "the second concurrent wave should reuse every active PeerConnection"
    );

    for &client_serial in &clients {
        assert_eq!(
            harness.peer(client_serial).pending_count().await,
            0,
            "client {client_serial} leaked a pending RPC"
        );
    }

    for &client_serial in &clients {
        let client = harness.peer(client_serial);
        client
            .coordinator
            .close_all_peers()
            .await
            .expect("client WebRTC cleanup should succeed");
        client
            .signaling_client
            .disconnect()
            .await
            .expect("client signaling disconnect should succeed");
    }
    harness
        .peer(SERVER)
        .coordinator
        .close_all_peers()
        .await
        .expect("server WebRTC cleanup should succeed");
    harness
        .peer(SERVER)
        .signaling_client
        .disconnect()
        .await
        .expect("server signaling disconnect should succeed");

    for task in background_tasks {
        task.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_server_handles_20_40_80_concurrent_webrtc_clients() {
    init_tracing();

    for client_count in CONCURRENT_CLIENT_COUNTS {
        run_multi_peer_concurrency_case(client_count).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "100 killed client processes with 1s spacing is a soak test; run explicitly when validating churn"]
async fn one_server_accepts_100_sequential_clients_with_client_kill_between_each() {
    init_tracing();

    let mut harness = TestHarness::new().await;
    harness.add_peer(SERVER).await;
    let server_url = harness.server.url();

    let server_task = harness
        .peer(SERVER)
        .start_echo_responder("sequential_churn_server");

    let sequential_clients = client_serials(1_000, SEQUENTIAL_CLIENT_COUNT);
    for (index, client_serial) in sequential_clients.iter().copied().enumerate() {
        let client_process = spawn_churn_client(&server_url, client_serial).await;
        assert_server_ready_for_external_client(&harness, client_serial).await;

        // The parent terminates the whole client process. No WebRTC or
        // signaling cleanup API runs on the client before the next connection.
        client_process.kill(client_serial).await;

        if index + 1 < sequential_clients.len() {
            tokio::time::sleep(SEQUENTIAL_CONNECT_INTERVAL).await;
        }
    }

    server_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "helper entrypoint spawned by the sequential client kill soak test"]
async fn sequential_churn_client_process() {
    let Ok(server_url) = std::env::var(CHILD_SERVER_URL_ENV) else {
        return;
    };
    let client_serial: u64 = std::env::var(CHILD_SERIAL_ENV)
        .expect("churn client serial should be set")
        .parse()
        .expect("churn client serial should be an integer");
    let ready_addr =
        std::env::var(CHILD_READY_ADDR_ENV).expect("churn client readiness address should be set");
    let client_id = make_actor_id(client_serial);
    let server_id = make_actor_id(SERVER);

    let (coordinator, _signaling_client) =
        create_peer_with_websocket(client_id.clone(), &server_url)
            .await
            .expect("churn client should connect to signaling");
    let wire_builder = Arc::new(DefaultWireBuilder::new(
        Some(coordinator.clone()),
        DefaultWireBuilderConfig::default(),
    ));
    let transport_manager = Arc::new(PeerTransport::new(client_id, wire_builder));
    let gate = Arc::new(PeerGate::new(transport_manager, Some(coordinator.clone())));
    let _response_receiver = spawn_response_receiver(
        coordinator.clone(),
        gate.clone(),
        &format!("sequential_churn_client_{client_serial}"),
    );

    let request_id = format!("sequential-churn-{client_serial}");
    let envelope = RpcEnvelope {
        request_id: request_id.clone(),
        route_key: "test.multi-peer".to_string(),
        payload: Some(bytes::Bytes::from(client_serial.to_string())),
        direction: Some(Direction::Request as i32),
        timeout_ms: RPC_TIMEOUT.as_millis() as i64,
        ..Default::default()
    };
    let response = tokio::time::timeout(
        SEQUENTIAL_CASE_TIMEOUT,
        gate.send_request(&server_id, envelope),
    )
    .await
    .unwrap_or_else(|_| panic!("client {client_serial} request {request_id} timed out"))
    .unwrap_or_else(|error| panic!("client {client_serial} request {request_id} failed: {error}"));
    assert_eq!(response.as_ref(), b"pong");
    assert!(
        coordinator
            .has_open_data_channel_for_test(&server_id)
            .await
            .expect("client DataChannel state should be readable"),
        "client {client_serial} DataChannel is not open"
    );

    let mut ready_stream = TcpStream::connect(&ready_addr)
        .await
        .expect("client should connect to the parent readiness listener");
    ready_stream
        .write_all(format!("{CHILD_READY_PREFIX} {client_serial}").as_bytes())
        .await
        .expect("client readiness marker should be writable");
    ready_stream
        .flush()
        .await
        .expect("client readiness marker should flush");
    ready_stream
        .shutdown()
        .await
        .expect("client readiness stream should shut down");

    std::future::pending::<()>().await;
}
