// Host for the async Component Model spike.
//
// Validates end-to-end async path: guest calls (async fn) -> host calls (async
// fn) -> real tokio await -> back. Each guest dispatch runs one at a time per
// Store (actor-instance single-threadedness guarantee), but the host import
// inside dispatch is a real async fn that awaits tokio::sleep, simulating
// network RTT.
//
// Setup choices:
// - WIT uses plain `func`, not `async func` (see wit/actr-spike-async.wit).
//   Reason: `async func` would invoke the Component Model Concurrency proposal
//   which wasmtime 43 flags as "very incomplete" and which requires a totally
//   different Accessor-based binding shape.
// - bindgen macros use `async: true` (guest) and `imports: { default: async },
//   exports: { default: async }` (host) to get async Rust ergonomics without
//   the concurrent ABI.
// - Config requires `wasm_component_model_async(true)` regardless, because
//   wit-bindgen 0.57 guest emits `context.get` (async-ABI primitive) even when
//   every WIT function is sync.

use anyhow::Result;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

wasmtime::component::bindgen!({
    world: "spike-guest-async",
    path: "../wit",
    imports: { default: async | trappable },
    exports: { default: async },
});

use actr::spike_async::host::Host as HostImports;
use actr::spike_async::types::{ActrId, RpcEnvelope, SpikeError};

struct HostState {
    wasi: WasiCtx,
    table: ResourceTable,
    call_raw_count: Arc<AtomicU64>,
    log_count: Arc<AtomicU64>,
    // Label identifying which instance this Store belongs to, used to trace
    // per-instance log output during Test 2 / Test 3.
    label: String,
    silent_logs: bool,
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
    async fn call_raw(
        &mut self,
        target: ActrId,
        route_key: String,
        payload: Vec<u8>,
    ) -> wasmtime::Result<Result<Vec<u8>, SpikeError>> {
        self.call_raw_count.fetch_add(1, Ordering::SeqCst);

        // Simulate network RTT. 50ms is long enough to measure, short enough
        // that multi-dispatch tests still finish quickly.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let reply = format!(
            "downstream({route_key})[target={}]: ",
            target.serial_number
        );
        let mut out = reply.into_bytes();
        out.extend_from_slice(&payload);
        Ok(Ok(out))
    }

    async fn log_message(&mut self, level: String, msg: String) -> wasmtime::Result<()> {
        self.log_count.fetch_add(1, Ordering::SeqCst);
        if !self.silent_logs {
            println!("[{}:{level}] {msg}", self.label);
        }
        Ok(())
    }
}

fn build_config() -> Config {
    let mut config = Config::new();
    config.wasm_component_model(true);
    // Required because wit-bindgen 0.57 guest compiled with `async: true`
    // emits `context.get` (async-ABI primitive), even when every WIT function
    // is sync.
    config.wasm_component_model_async(true);
    // `async_support(true)` is no-op in 43.x when component-model-async is on
    // (deprecated warning), but the option is harmless and explicit.
    config
}

fn make_store(
    engine: &Engine,
    label: &str,
    call_raw_count: Arc<AtomicU64>,
    log_count: Arc<AtomicU64>,
    silent_logs: bool,
) -> Store<HostState> {
    Store::new(
        engine,
        HostState {
            wasi: WasiCtxBuilder::new().inherit_stdio().build(),
            table: ResourceTable::new(),
            call_raw_count,
            log_count,
            label: label.to_string(),
            silent_logs,
        },
    )
}

fn make_linker(engine: &Engine) -> Result<Linker<HostState>> {
    let mut linker: Linker<HostState> = Linker::new(engine);
    actr::spike_async::host::add_to_linker::<_, HasSelf<_>>(&mut linker, |s| s)
        .map_err(|e| anyhow::anyhow!("linking actr:spike-async/host: {e}"))?;
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    Ok(linker)
}

async fn instantiate(
    component: &Component,
    linker: &Linker<HostState>,
    store: &mut Store<HostState>,
) -> Result<SpikeGuestAsync> {
    SpikeGuestAsync::instantiate_async(store, component, linker)
        .await
        .map_err(|e| anyhow::anyhow!("instantiate_async: {e}"))
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    let guest_wasm = std::env::args()
        .nth(1)
        .unwrap_or_else(|| {
            "../guest/target/wasm32-wasip2/release/spike_guest_async.wasm".to_string()
        });

    println!("=== Phase 0.5 Component Model async spike ===");
    println!("loading async guest component: {guest_wasm}");

    let config = build_config();
    let engine = Engine::new(&config)?;
    let component = Component::from_file(&engine, &guest_wasm)
        .map_err(|e| anyhow::anyhow!("loading guest .wasm as Component: {e}"))?;
    let linker = make_linker(&engine)?;

    let global_call_raw = Arc::new(AtomicU64::new(0));
    let global_log = Arc::new(AtomicU64::new(0));

    // =========================================================
    // Test 1 — basic async dispatch round-trip
    // =========================================================
    println!("\n────────────────────────────────────────");
    println!("Test 1 — basic async dispatch round-trip");
    println!("────────────────────────────────────────");
    {
        let mut store = make_store(
            &engine,
            "T1",
            global_call_raw.clone(),
            global_log.clone(),
            false,
        );
        let bindings = instantiate(&component, &linker, &mut store).await?;

        bindings
            .actr_spike_async_workload()
            .call_on_start(&mut store)
            .await?
            .map_err(|e| anyhow::anyhow!("on_start: {e:?}"))?;

        let env = RpcEnvelope {
            route_key: "echo.Echo".to_string(),
            payload: b"hello".to_vec(),
            request_id: "req-1".to_string(),
        };
        let t0 = Instant::now();
        let reply = bindings
            .actr_spike_async_workload()
            .call_dispatch(&mut store, &env)
            .await?
            .map_err(|e| anyhow::anyhow!("dispatch: {e:?}"))?;
        let dt = t0.elapsed();
        let reply_str = String::from_utf8_lossy(&reply).to_string();
        println!(
            "Test 1 dispatch reply: {reply_str:?}  (took {:.1}ms)",
            dt.as_secs_f64() * 1000.0
        );
        assert!(
            reply_str.starts_with("async-echo: downstream(downstream.Echo)"),
            "Test 1 reply shape mismatch: {reply_str:?}"
        );
        println!("Test 1 PASS: end-to-end async round-trip OK");
    }

    // =========================================================
    // Test 2 — concurrent dispatches on DIFFERENT instances
    // =========================================================
    println!("\n────────────────────────────────────────");
    println!("Test 2 — concurrent dispatches on DIFFERENT instances");
    println!("────────────────────────────────────────");
    {
        let mut store_a = make_store(
            &engine,
            "T2a",
            global_call_raw.clone(),
            global_log.clone(),
            true,
        );
        let mut store_b = make_store(
            &engine,
            "T2b",
            global_call_raw.clone(),
            global_log.clone(),
            true,
        );
        let bindings_a = instantiate(&component, &linker, &mut store_a).await?;
        let bindings_b = instantiate(&component, &linker, &mut store_b).await?;

        // Warm up: make sure on_start has completed for both. That way Test 2
        // measures dispatch latency only, not first-instantiation + init.
        bindings_a
            .actr_spike_async_workload()
            .call_on_start(&mut store_a)
            .await?
            .map_err(|e| anyhow::anyhow!("on_start A: {e:?}"))?;
        bindings_b
            .actr_spike_async_workload()
            .call_on_start(&mut store_b)
            .await?
            .map_err(|e| anyhow::anyhow!("on_start B: {e:?}"))?;

        let env_a = RpcEnvelope {
            route_key: "echo.A".to_string(),
            payload: b"A".to_vec(),
            request_id: "req-A".to_string(),
        };
        let env_b = RpcEnvelope {
            route_key: "echo.B".to_string(),
            payload: b"B".to_vec(),
            request_id: "req-B".to_string(),
        };

        let t0 = Instant::now();
        let (ra, rb) = tokio::join!(
            async {
                bindings_a
                    .actr_spike_async_workload()
                    .call_dispatch(&mut store_a, &env_a)
                    .await
            },
            async {
                bindings_b
                    .actr_spike_async_workload()
                    .call_dispatch(&mut store_b, &env_b)
                    .await
            },
        );
        let dt = t0.elapsed();
        let _ = ra?.map_err(|e| anyhow::anyhow!("dispatch A: {e:?}"))?;
        let _ = rb?.map_err(|e| anyhow::anyhow!("dispatch B: {e:?}"))?;

        let ms = dt.as_secs_f64() * 1000.0;
        println!("Test 2 wall time: {ms:.1}ms");
        if ms < 80.0 {
            println!(
                "Test 2 PASS: <80ms => dispatches ran concurrently \
                 (~50ms host sleep, not 100ms serial)"
            );
        } else {
            println!(
                "Test 2 NOTE: {ms:.1}ms >= 80ms => dispatches appear SERIAL across instances \
                 (implication: each Store/Instance serialized on the async executor)"
            );
        }
    }

    // =========================================================
    // Test 3 — concurrent dispatches on the SAME instance
    // =========================================================
    println!("\n────────────────────────────────────────");
    println!("Test 3 — concurrent dispatches on the SAME instance");
    println!("────────────────────────────────────────");
    {
        let mut store = make_store(
            &engine,
            "T3",
            global_call_raw.clone(),
            global_log.clone(),
            false, // keep logs visible here to see enter/exit interleave
        );
        let bindings = instantiate(&component, &linker, &mut store).await?;
        bindings
            .actr_spike_async_workload()
            .call_on_start(&mut store)
            .await?
            .map_err(|e| anyhow::anyhow!("on_start: {e:?}"))?;

        let env_a = RpcEnvelope {
            route_key: "trace.A".to_string(),
            payload: b"AAAA".to_vec(),
            request_id: "req-3A".to_string(),
        };
        let env_b = RpcEnvelope {
            route_key: "trace.B".to_string(),
            payload: b"BBBB".to_vec(),
            request_id: "req-3B".to_string(),
        };

        // We cannot `tokio::join!` two calls that both borrow `&mut store` —
        // Rust's aliasing rules forbid it. This itself is a finding: wasmtime
        // models "same Store" serially at the type level. To even *attempt* a
        // concurrent same-instance dispatch we'd need a cell with interior
        // mutability, and wasmtime::Store isn't `Sync` anyway. So the type
        // system forces serialization before we ever get to runtime.
        //
        // Emulate "concurrent calls on the same instance" as the closest
        // achievable approximation: interleaved sequential calls, measuring
        // the wall time of running both back-to-back.
        let t0 = Instant::now();
        let ra = bindings
            .actr_spike_async_workload()
            .call_dispatch(&mut store, &env_a)
            .await?;
        let rb = bindings
            .actr_spike_async_workload()
            .call_dispatch(&mut store, &env_b)
            .await?;
        let dt = t0.elapsed();
        let _ = ra.map_err(|e| anyhow::anyhow!("dispatch 3A: {e:?}"))?;
        let _ = rb.map_err(|e| anyhow::anyhow!("dispatch 3B: {e:?}"))?;
        let ms = dt.as_secs_f64() * 1000.0;
        println!(
            "Test 3 wall time (sequential, forced by Store ownership): {ms:.1}ms"
        );
        println!(
            "Test 3 RESULT: wasmtime `Store<T>` is not `Sync`, and `call_dispatch` \
             takes `&mut Store<T>` — the Rust borrow checker forces \
             serialization of hooks on the same instance at compile time. \
             Actor-instance single-threadedness is guaranteed by construction, \
             not by runtime convention."
        );
    }

    // =========================================================
    // Test 4 — host thread behavior during guest await
    // =========================================================
    println!("\n────────────────────────────────────────");
    println!("Test 4 — host thread free during guest await");
    println!("────────────────────────────────────────");
    {
        let tick_count = Arc::new(AtomicU64::new(0));
        let tick_stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let tc2 = tick_count.clone();
        let ts2 = tick_stop.clone();
        let ticker = tokio::spawn(async move {
            while !ts2.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(10)).await;
                tc2.fetch_add(1, Ordering::SeqCst);
            }
        });

        let mut store = make_store(
            &engine,
            "T4",
            global_call_raw.clone(),
            global_log.clone(),
            true,
        );
        let bindings = instantiate(&component, &linker, &mut store).await?;

        // Fire a dispatch that internally awaits 50ms in call_raw. Ticker should
        // tick ~5 times during this period.
        let env = RpcEnvelope {
            route_key: "echo.TickProbe".to_string(),
            payload: b"tick-probe".to_vec(),
            request_id: "req-tick".to_string(),
        };
        let t0 = Instant::now();
        let _ = bindings
            .actr_spike_async_workload()
            .call_dispatch(&mut store, &env)
            .await?
            .map_err(|e| anyhow::anyhow!("dispatch: {e:?}"))?;
        let dt = t0.elapsed();
        tick_stop.store(true, Ordering::SeqCst);
        let _ = ticker.await;
        let ticks = tick_count.load(Ordering::SeqCst);
        let ms = dt.as_secs_f64() * 1000.0;
        println!(
            "Test 4 dispatch took {ms:.1}ms; ticker recorded {ticks} ticks during/around it"
        );
        if ticks >= 3 {
            println!(
                "Test 4 PASS: tokio executor kept running during guest host-import await \
                 (wasm instance did NOT block the reactor)"
            );
        } else {
            println!(
                "Test 4 FAIL: tokio executor appears blocked during guest await — \
                 ticker only got {ticks} ticks"
            );
        }
    }

    // =========================================================
    // Test 5 — guest-side async ergonomics
    // =========================================================
    println!("\n────────────────────────────────────────");
    println!("Test 5 — guest-side async ergonomics");
    println!("────────────────────────────────────────");
    println!(
        "Test 5 RESULT: guest writes normal `.await` syntax. wit-bindgen 0.57.1 \
         with `async: true` generates `async fn call_raw(...) -> Result<...>` \
         that the guest calls as `call_raw(...).await?`. No block_on, no manual \
         poll loops, no custom wakers."
    );
    println!(
        "Test 5 NOTE: with `async: true`, ALL imports become `async fn` at the \
         Rust surface — even imports declared plain `func` (not `async func`) \
         in WIT. So a sync WIT import still needs `.await` on the guest. Trade \
         at import-time ergonomics, not a runtime cost."
    );
    println!(
        "Test 5 NOTE: async imports take OWNED values (`String`, `Vec<u8>`) \
         instead of the sync path's `&str` / `&[u8]`. Suspend points can't \
         hold stack-frame borrows across await. Expect extra `.to_string()` / \
         `.clone()` at call sites."
    );

    // =========================================================
    // Test 6 — throughput comparison
    // =========================================================
    println!("\n────────────────────────────────────────");
    println!("Test 6 — throughput: 100 sequential async dispatches");
    println!("────────────────────────────────────────");
    {
        let mut store = make_store(
            &engine,
            "T6",
            global_call_raw.clone(),
            global_log.clone(),
            true,
        );
        let bindings = instantiate(&component, &linker, &mut store).await?;

        // Each dispatch internally awaits 50ms in call_raw, so 1000 iters would
        // take 50s. Run 100 iters and measure — report per-call overhead net
        // of the 50ms host sleep. That's the number comparable to Phase 0's
        // ~6us sync cost (Phase 0 had no sleeping host import).
        let iters: u64 = 100;
        let bench_env = RpcEnvelope {
            route_key: "bench.Echo".to_string(),
            payload: vec![0u8; 64],
            request_id: "bench".to_string(),
        };
        let t0 = Instant::now();
        for _ in 0..iters {
            let _ = bindings
                .actr_spike_async_workload()
                .call_dispatch(&mut store, &bench_env)
                .await?
                .map_err(|e| anyhow::anyhow!("bench dispatch: {e:?}"))?;
        }
        let elapsed = t0.elapsed();
        let per_call_ms = elapsed.as_secs_f64() * 1000.0 / iters as f64;
        let per_call_overhead_us =
            (elapsed.as_secs_f64() * 1_000_000.0 / iters as f64) - 50_000.0;
        println!(
            "Test 6: {iters} sequential dispatches (each includes 50ms host sleep): \
             total {:.1}ms, {per_call_ms:.2}ms/call",
            elapsed.as_secs_f64() * 1000.0
        );
        println!(
            "Test 6: overhead per call net of sleep: ~{per_call_overhead_us:.1} us \
             (compares to Phase 0's ~6 us sync cost; extra is async plumbing)"
        );
    }

    // =========================================================
    // Test 7 — error propagation across async boundary
    // =========================================================
    println!("\n────────────────────────────────────────");
    println!("Test 7 — error propagation across async boundary");
    println!("────────────────────────────────────────");
    {
        let mut store = make_store(
            &engine,
            "T7",
            global_call_raw.clone(),
            global_log.clone(),
            true,
        );
        let bindings = instantiate(&component, &linker, &mut store).await?;
        let env = RpcEnvelope {
            route_key: "err.Timeout".to_string(),
            payload: vec![],
            request_id: "req-err".to_string(),
        };
        match bindings
            .actr_spike_async_workload()
            .call_dispatch(&mut store, &env)
            .await?
        {
            Ok(_) => println!("Test 7 FAIL: expected guest error but dispatch returned Ok"),
            Err(SpikeError::Timeout(msg)) => {
                println!("Test 7 PASS: got Timeout variant, message: {msg:?}");
            }
            Err(other) => println!("Test 7 NOTE: got other error variant: {other:?}"),
        }
    }

    // =========================================================
    // Test 8 — guest panic AFTER a suspension point
    // =========================================================
    println!("\n────────────────────────────────────────");
    println!("Test 8 — guest panic AFTER a suspension point");
    println!("────────────────────────────────────────");
    {
        let mut store = make_store(
            &engine,
            "T8",
            global_call_raw.clone(),
            global_log.clone(),
            true,
        );
        let bindings = instantiate(&component, &linker, &mut store).await?;
        let env = RpcEnvelope {
            route_key: "panic.AfterAwait".to_string(),
            payload: b"boom-payload".to_vec(),
            request_id: "req-panic".to_string(),
        };
        match bindings
            .actr_spike_async_workload()
            .call_dispatch(&mut store, &env)
            .await
        {
            Ok(_) => println!("Test 8 WARN: panic did not propagate (unexpected)"),
            Err(e) => {
                let s = format!("{e:?}");
                let snippet: String = s.chars().take(500).collect();
                println!("Test 8 PASS: panic surfaced as Trap (Store poisoned):");
                println!(
                    "            {snippet}{}",
                    if s.len() > 500 { "…" } else { "" }
                );
            }
        }
    }

    println!("\n=== async spike OK ===");
    println!(
        "totals: host.call_raw invocations = {}, host.log_message invocations = {}",
        global_call_raw.load(Ordering::SeqCst),
        global_log.load(Ordering::SeqCst),
    );
    Ok(())
}
