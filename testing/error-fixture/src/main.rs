//! Error-fixture server for binding error-mapping integration tests.
//!
//! Starts mock-actrix + a minimal echo service in-process, then writes
//! a single JSON line to stdout:
//!
//! ```json
//! {"port":PORT,"mfr_pubkey_b64":"BASE64","realm_id":1}
//! ```
//!
//! The calling test process reads that line, writes temp config files,
//! starts a binding client, and exercises the three error kinds:
//!
//! - `UnknownRoute`       — call any route the server does not handle
//! - `PermissionDenied`   — run with `--deny-all` to block every caller
//! - `TimedOut`           — call `fixture.Fixture.SlowEcho`; the handler
//!   never returns, so the caller times out
//!
//! The server blocks until stdin reaches EOF (parent process exited).

use std::io::Read;

use actr_config::ConfigParser;
use actr_framework::{Bytes, Context as RtContext, MessageDispatcher, Workload as RtWorkload};
use actr_hyper::Node;
use actr_mock_actrix::MockActrixServer;
use actr_protocol::{ActorResult, ActrError, RpcEnvelope};
use async_trait::async_trait;
use base64::Engine;
use clap::Parser;
use tracing::{info, warn};

// ── Workload ─────────────────────────────────────────────────────────────────

struct FixtureWorkload;

#[async_trait]
impl RtWorkload for FixtureWorkload {
    type Dispatcher = FixtureDispatcher;
}

struct FixtureDispatcher;

#[async_trait]
impl MessageDispatcher for FixtureDispatcher {
    type Workload = FixtureWorkload;

    async fn dispatch<C: RtContext>(
        _workload: &Self::Workload,
        envelope: RpcEnvelope,
        _ctx: &C,
    ) -> ActorResult<Bytes> {
        match envelope.route_key.as_str() {
            "fixture.Fixture.Echo" => {
                let payload = envelope.payload.unwrap_or_default();
                info!(route = "Echo", bytes = payload.len(), "fixture: echo");
                Ok(payload)
            }
            "fixture.Fixture.SlowEcho" => {
                // Never returns — caller will time out.
                info!("fixture: SlowEcho — blocking forever (caller timeout expected)");
                std::future::pending::<ActorResult<Bytes>>().await
            }
            route => {
                warn!(route, "fixture: unknown route");
                Err(ActrError::UnknownRoute(route.to_string()))
            }
        }
    }
}

// ── Args ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
#[command(about = "Fixture server for binding error-mapping tests")]
struct Args {
    /// Deny all inbound calls (ACL deny-all mode for PermissionDenied tests).
    #[arg(long)]
    deny_all: bool,
}

// ── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();

    // ── 1. Start mock-actrix + find a free port for the server's WS listener ───
    let mock = MockActrixServer::start().await?;
    let port = mock.port();
    let mfr_key = mock.mfr_signing_key();
    let mfr_pubkey_b64 =
        base64::engine::general_purpose::STANDARD.encode(mfr_key.verifying_key().as_bytes());

    // Allocate a free port for the fixture server's direct WebSocket listener.
    // Binding to port 0 lets the OS pick an unused port; we retrieve it then
    // drop the socket so the fixture server can bind the same port later.
    let ws_port = {
        let tmp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        tmp_listener.local_addr()?.port()
    };

    info!(port, ws_port, "mock-actrix ready");

    // ── 2. Write a temp manifest + runtime config ────────────────────────────
    let tmp = tempfile::tempdir()?;
    let manifest_path = tmp.path().join("manifest.toml");
    let runtime_path = tmp.path().join("actr.toml");

    // The ACL section: deny-all mode returns PermissionDenied for every call.
    //
    // `[acl]` with an empty `rules = []` array triggers the "empty rules list
    // — deny all" path in `check_acl_permission`.  Omitting the `[acl]`
    // section entirely falls back to "no ACL — allow all".
    let acl_section = if args.deny_all {
        "[acl]\nrules = []\n"
    } else {
        // Omit the `[acl]` section entirely so the runtime falls back to the
        // "no ACL configured — allow by default" path.
        ""
    };

    let manifest_toml = r#"edition = 1
exports = []

[package]
name = "ErrorFixture"
manufacturer = "test"
version = "0.2.0"
description = "Fixture service for error-mapping tests"
"#;

    std::fs::write(&manifest_path, manifest_toml)?;

    let runtime_toml = format!(
        r#"edition = 1

[package]
path = "{manifest_path}"

[signaling]
url = "ws://127.0.0.1:{port}/signaling/ws"

[ais_endpoint]
url = "http://127.0.0.1:{port}/ais"

[deployment]
realm_id = 1

[discovery]
visible = true

[observability]
filter_level = "warn"
tracing_enabled = false

[webrtc]
force_relay = false
stun_urls = []
turn_urls = []

[websocket]
listen_port = {ws_port}
advertised_host = "127.0.0.1"

{acl_section}
[[trust]]
kind = "static"
pubkey_b64 = "{mfr_pubkey_b64}"
"#,
        manifest_path = manifest_path.display(),
        port = port,
        ws_port = ws_port,
        acl_section = acl_section,
        mfr_pubkey_b64 = mfr_pubkey_b64,
    );

    std::fs::write(&runtime_path, &runtime_toml)?;

    // ── 3. Start the fixture actor ───────────────────────────────────────────
    let manifest = ConfigParser::from_manifest_file(&manifest_path)?;
    let init = Node::from_config_file(&runtime_path)
        .await?
        .with_actor_type(manifest.package.actr_type.clone());
    let ais_endpoint = init.runtime_config().ais_endpoint.to_string();

    let attached = init.link(FixtureWorkload).await?;
    let registered = attached.register(&ais_endpoint).await?;
    let actr_ref = registered.start().await?;

    let actor_id = actr_ref.actor_id();
    info!(?actor_id, "fixture actor ready");

    // ── 4. Emit readiness JSON ───────────────────────────────────────────────
    // Use println! so it goes to stdout; tracing goes to stderr.
    let ready = serde_json::json!({
        "port": port,
        "ws_port": ws_port,
        "mfr_pubkey_b64": mfr_pubkey_b64,
        "realm_id": 1,
        "actr_type": {
            "manufacturer": "test",
            "name": "ErrorFixture",
            "version": "0.2.0"
        }
    });
    println!("{}", ready);

    // ── 5. Block until stdin closes (parent exited) ──────────────────────────
    tokio::task::spawn_blocking(|| {
        let mut buf = [0u8; 64];
        loop {
            match std::io::stdin().read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    })
    .await?;

    info!("fixture: stdin closed, shutting down");
    actr_ref.shutdown();
    actr_ref.wait_for_shutdown().await;
    Ok(())
}
