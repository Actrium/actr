// Host: loads the guest Component, links host imports, invokes exports.
//
// Demonstrates:
//   - bidirectional flow (host calls guest, guest calls back into host)
//   - nested record + variant roundtrip
//   - per-call overhead microbenchmark
//   - instance reuse across many dispatches
//   - panic -> Trap propagation

use anyhow::Result;
use std::time::Instant;
use wasmtime::component::{bindgen, Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

bindgen!({
    world: "spike-guest",
    path: "../wit",
});

use actr::spike::host::Host as HostImports;
use actr::spike::types::{ActrId, ErrorCategory, ErrorEvent, PeerEvent, PeerInfo, RpcEnvelope, SpikeError};

struct HostState {
    wasi: WasiCtx,
    table: ResourceTable,
    call_count: u64,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl HostImports for HostState {
    fn call_raw(
        &mut self,
        target: ActrId,
        route_key: String,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, SpikeError> {
        self.call_count += 1;
        // Fake reply; real impl would dispatch via actr routing.
        let reply = format!(
            "fake-reply target={} route={} paylen={}",
            target.serial_number,
            route_key,
            payload.len()
        );
        Ok(reply.into_bytes())
    }

    fn log_message(&mut self, level: String, msg: String) {
        println!("[guest:{level}] {msg}");
    }
}

fn main() -> Result<()> {
    let guest_wasm = std::env::args()
        .nth(1)
        .unwrap_or_else(|| {
            "../guest/target/wasm32-wasip2/release/spike_guest.wasm".to_string()
        });

    println!("=== Phase 0 Component Model spike ===");
    println!("loading guest component: {guest_wasm}");

    let mut config = Config::new();
    config.wasm_component_model(true);
    let engine = Engine::new(&config)?;

    let component = Component::from_file(&engine, &guest_wasm)
        .map_err(|e| anyhow::anyhow!("loading guest .wasm as Component: {e}"))?;

    let mut linker: Linker<HostState> = Linker::new(&engine);
    // Wire host imports
    actr::spike::host::add_to_linker::<_, HasSelf<_>>(&mut linker, |state: &mut HostState| state)
        .map_err(|e| anyhow::anyhow!("linking actr:spike/host: {e}"))?;
    // WASI preview2 (needed because wit-bindgen 0.57 targeting wasip2 pulls wasi stdlib imports)
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;

    let mut store = Store::new(
        &engine,
        HostState {
            wasi: WasiCtxBuilder::new().inherit_stdio().build(),
            table: ResourceTable::new(),
            call_count: 0,
        },
    );

    let bindings = SpikeGuest::instantiate(&mut store, &component, &linker)
        .map_err(|e| anyhow::anyhow!("instantiating spike-guest: {e}"))?;

    // ---- 1. on-start
    println!("\n-- on_start --");
    bindings
        .actr_spike_workload()
        .call_on_start(&mut store)?
        .map_err(|e| anyhow::anyhow!("on_start: {e:?}"))?;

    // ---- 2. dispatch (basic echo)
    println!("\n-- dispatch basic --");
    let env = RpcEnvelope {
        route_key: "echo.Echo".to_string(),
        payload: b"hello".to_vec(),
        request_id: "req-1".to_string(),
    };
    let reply = bindings
        .actr_spike_workload()
        .call_dispatch(&mut store, &env)?
        .map_err(|e| anyhow::anyhow!("dispatch: {e:?}"))?;
    println!("dispatch reply: {:?}", String::from_utf8_lossy(&reply));
    assert_eq!(reply, b"echo: hello");

    // ---- 3. dispatch returning guest error
    println!("\n-- dispatch returning bad-payload --");
    let env_bad = RpcEnvelope {
        route_key: "bad.Payload".to_string(),
        payload: vec![],
        request_id: "req-2".to_string(),
    };
    match bindings
        .actr_spike_workload()
        .call_dispatch(&mut store, &env_bad)?
    {
        Ok(_) => anyhow::bail!("expected guest error"),
        Err(SpikeError::BadPayload(m)) => println!("guest returned expected BadPayload: {m}"),
        Err(other) => println!("guest returned other error: {other:?}"),
    }

    // ---- 4. Nested records through the boundary
    println!("\n-- on_peer_event (nested record) --");
    let peer_evt = PeerEvent {
        peer: PeerInfo {
            peer_id: "peer-alpha".to_string(),
            relayed: true,
        },
        timestamp_ms: 1_700_000_000_000,
    };
    bindings
        .actr_spike_workload()
        .call_on_peer_event(&mut store, &peer_evt)?
        .map_err(|e| anyhow::anyhow!("on_peer_event: {e:?}"))?;

    println!("\n-- report_error (variant-in-record) --");
    let err_evt = ErrorEvent {
        source: "spike.host".to_string(),
        category: ErrorCategory::Other("custom-k".to_string()),
        context: vec![
            ("trace_id".to_string(), "abc-123".to_string()),
            ("region".to_string(), "eu-west-1".to_string()),
        ],
    };
    bindings
        .actr_spike_workload()
        .call_report_error(&mut store, &err_evt)?
        .map_err(|e| anyhow::anyhow!("report_error: {e:?}"))?;

    // ---- 5. Per-call overhead microbenchmark (same instance reused)
    println!("\n-- benchmark: 1000 dispatches on the same instance --");
    let bench_env = RpcEnvelope {
        route_key: "bench.Echo".to_string(),
        payload: vec![0u8; 64],
        request_id: "bench".to_string(),
    };
    let iters: u64 = 1000;
    let t0 = Instant::now();
    for _ in 0..iters {
        let _ = bindings
            .actr_spike_workload()
            .call_dispatch(&mut store, &bench_env)?
            .map_err(|e| anyhow::anyhow!("bench dispatch: {e:?}"))?;
    }
    let elapsed = t0.elapsed();
    let per_call_us = elapsed.as_secs_f64() * 1_000_000.0 / iters as f64;
    println!(
        "{iters} dispatches: {:.3} ms total, ~{per_call_us:.2} us/call",
        elapsed.as_secs_f64() * 1000.0
    );

    println!(
        "\nhost.call_count (from guest on_start call-raw): {}",
        store.data().call_count
    );

    // ---- 6. Panic propagation
    println!("\n-- panic propagation test --");
    let panic_env = RpcEnvelope {
        route_key: "panic.Panic".to_string(),
        payload: vec![],
        request_id: "req-boom".to_string(),
    };
    match bindings
        .actr_spike_workload()
        .call_dispatch(&mut store, &panic_env)
    {
        Ok(_) => println!("WARN: panic did not propagate (unexpected)"),
        Err(e) => {
            let s = format!("{e:?}");
            let snippet: String = s.chars().take(240).collect();
            println!("panic surfaced as error (expected): {snippet}...");
        }
    }

    println!("\n=== spike OK ===");
    Ok(())
}
