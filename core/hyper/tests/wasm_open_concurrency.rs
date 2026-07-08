//! M5 — open same-instance wasm concurrency.
//!
//! These tests drive the 0.2.0 (async-world) fixture through the production
//! dispatch path — a conflict-key scheduler in front of the **interleaved**
//! runner (`WasmWorkloadV2::run_interleaved`, a resident `run_concurrent`
//! region) — and assert the M5 guarantees end-to-end:
//!
//! 1. distinct-key dispatches truly interleave inside ONE instance (MAX_SEEN>=2)
//! 2. same-key dispatches stay strictly serial (MAX_SEEN==1)
//! 3. concurrency never exceeds the scheduler budget C
//! 4. an in-flight guest trap fails ALL siblings together and rebuilds (5. the
//!    whole-region teardown is asserted explicitly there too)
//! 6. a per-dispatch timeout cleanly cancels the stuck dispatch (bounded, no
//!    hang), frees its key, and does not poison the store
//! 7. gate-off degrades to the serial M4 path (MAX_SEEN==1)
//! 9. the package compat matrix: a V1 (sync-world) guest stays serial even when
//!    Interleaved is requested; a V2 guest works in both modes
//!
//! Evidence is gathered without sleep-based coordination: the guest reports the
//! peak in-flight count it observed in its own linear memory (MAX_SEEN), and the
//! host bridge gates guest→host crossings on semaphores + entry channels.

#![cfg(all(feature = "wasm-engine", actr_wasm_fixture_available))]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use actr_hyper::test_support::{TestConcurrentDispatcher, instantiate_wasm_workload};
use actr_hyper::wasm::WasmHost;
use actr_hyper::workload::{HostAbiFn, HostOperation, HostOperationResult};
use actr_hyper::{ConflictKeySpec, KeySource};
use actr_protocol::{ActrId, ActrType, Realm};
use bytes::Bytes;
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinHandle;

#[path = "wasm_actor_fixture.rs"]
mod wasm_actor_fixture;

const PROBE: &str = "test/inflight-probe";
const BOOM: &str = "test/boom-after-await";
const ECHO: &str = "test/echo";

fn fixture_bytes() -> &'static [u8] {
    wasm_actor_fixture::WASM_ACTOR_FIXTURE
}

/// A genuine 0.1.0 sync-lift Component (frozen pre-M4 build of the same guest
/// source), used by the compat matrix to prove the V1-on-Interleaved fallback.
const V1_SYNCLIFT_GUEST: &[u8] = include_bytes!("fixtures/v1_synclift_guest.wasm");

fn caller(serial: u64) -> Option<ActrId> {
    Some(ActrId {
        realm: Realm { realm_id: 1 },
        serial_number: serial,
        r#type: ActrType {
            manufacturer: "test".to_string(),
            name: "fixture".to_string(),
            version: "0.2.0".to_string(),
        },
    })
}

fn probe_spec() -> ConflictKeySpec {
    ConflictKeySpec::builder()
        .method(PROBE, KeySource::Sender)
        .method(BOOM, KeySource::Sender)
        .method(ECHO, KeySource::Sender)
        .build()
        .expect("build conflict-key spec")
}

fn read_u32(b: &Bytes) -> u32 {
    assert!(
        b.len() >= 4,
        "reply must be a 4-byte LE u32, got {} bytes",
        b.len()
    );
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// Host bridge with two independently-gated host imports:
/// * `test/gate`      — the `test/inflight-probe` suspension point (probes park here)
/// * `test/double_impl` — the `test/boom-after-await` suspension point (boom parks here)
///
/// Each import signals an entry channel and then parks on its release
/// semaphore, so a test can hold N guest tasks suspended inside the ONE instance
/// at once and release them deterministically — no sleeps.
struct GateControls {
    gate_entered: mpsc::UnboundedReceiver<()>,
    gate_release: Arc<Semaphore>,
    impl_entered: mpsc::UnboundedReceiver<()>,
    impl_release: Arc<Semaphore>,
    #[allow(dead_code)]
    calls: Arc<AtomicU64>,
}

fn gate_bridge() -> (HostAbiFn, GateControls) {
    let (gate_tx, gate_rx) = mpsc::unbounded_channel();
    let (impl_tx, impl_rx) = mpsc::unbounded_channel();
    let gate_release = Arc::new(Semaphore::new(0));
    let impl_release = Arc::new(Semaphore::new(0));
    let calls = Arc::new(AtomicU64::new(0));

    let bridge: HostAbiFn = {
        let gate_release = gate_release.clone();
        let impl_release = impl_release.clone();
        let calls = calls.clone();
        Arc::new(move |op| {
            let gate_tx = gate_tx.clone();
            let impl_tx = impl_tx.clone();
            let gate_release = gate_release.clone();
            let impl_release = impl_release.clone();
            let calls = calls.clone();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                match op {
                    HostOperation::CallRaw(req) if req.route_key == "test/gate" => {
                        let _ = gate_tx.send(());
                        gate_release
                            .acquire()
                            .await
                            .expect("gate semaphore open")
                            .forget();
                        HostOperationResult::Done
                    }
                    HostOperation::CallRaw(req) if req.route_key == "test/double_impl" => {
                        let _ = impl_tx.send(());
                        impl_release
                            .acquire()
                            .await
                            .expect("impl semaphore open")
                            .forget();
                        if req.payload.len() < 4 {
                            return HostOperationResult::Error(-1);
                        }
                        let x = i32::from_le_bytes([
                            req.payload[0],
                            req.payload[1],
                            req.payload[2],
                            req.payload[3],
                        ]);
                        HostOperationResult::Bytes((x * 2).to_le_bytes().to_vec())
                    }
                    _ => HostOperationResult::Error(-1),
                }
            })
        })
    };
    (
        bridge,
        GateControls {
            gate_entered: gate_rx,
            gate_release,
            impl_entered: impl_rx,
            impl_release,
            calls,
        },
    )
}

/// Block (bounded by a watchdog) until `n` guest tasks have signalled entry on
/// `rx`. Receiving from a channel is an event wait, not sleep-coordination.
async fn wait_entered(rx: &mut mpsc::UnboundedReceiver<()>, n: usize) {
    for i in 0..n {
        tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .unwrap_or_else(|_| panic!("watchdog: only {i}/{n} guest entries arrived"))
            .expect("entry channel open");
    }
}

fn spawn_dispatch(
    dispatcher: &Arc<TestConcurrentDispatcher>,
    route: &str,
    payload: Vec<u8>,
    caller_id: Option<ActrId>,
    bridge: &HostAbiFn,
) -> JoinHandle<actr_protocol::ActorResult<Bytes>> {
    let dispatcher = dispatcher.clone();
    let bridge = bridge.clone();
    let route = route.to_string();
    tokio::spawn(async move {
        dispatcher
            .dispatch(&route, payload, caller_id, &bridge)
            .await
    })
}

// ── Facet 1 — distinct keys truly interleave (MAX_SEEN >= 2) ─────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn interleave_distinct_keys_reaches_max_seen_2() {
    let host = WasmHost::compile(fixture_bytes()).expect("compile v2 fixture");
    let wl = instantiate_wasm_workload(&host).await.expect("instantiate");
    let dispatcher = Arc::new(wl.into_concurrent_dispatcher(probe_spec(), 8, 256, None));
    let (bridge, mut ctl) = gate_bridge();

    // Two distinct callers -> distinct conflict keys -> eligible to run at once.
    let d1 = spawn_dispatch(&dispatcher, PROBE, vec![], caller(1), &bridge);
    let d2 = spawn_dispatch(&dispatcher, PROBE, vec![], caller(2), &bridge);

    // Both must be suspended inside the ONE instance before either is released.
    wait_entered(&mut ctl.gate_entered, 2).await;
    ctl.gate_release.add_permits(2);

    let m1 = read_u32(&d1.await.unwrap().expect("d1 dispatch ok"));
    let m2 = read_u32(&d2.await.unwrap().expect("d2 dispatch ok"));

    assert!(
        m1.max(m2) >= 2,
        "distinct-key dispatches must interleave inside one instance \
         (MAX_SEEN>=2), got {m1} and {m2}"
    );
}

// ── Facet 2 — same key stays strictly serial (MAX_SEEN == 1) ─────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn same_key_stays_serial_max_seen_1() {
    let host = WasmHost::compile(fixture_bytes()).expect("compile v2 fixture");
    let wl = instantiate_wasm_workload(&host).await.expect("instantiate");
    let dispatcher = Arc::new(wl.into_concurrent_dispatcher(probe_spec(), 8, 256, None));
    let (bridge, mut ctl) = gate_bridge();

    // SAME caller -> same conflict key -> the scheduler must serialize them.
    let a = spawn_dispatch(&dispatcher, PROBE, vec![], caller(7), &bridge);
    let b = spawn_dispatch(&dispatcher, PROBE, vec![], caller(7), &bridge);

    // Only ONE may be in the guest at a time. Release them one at a time; the
    // second cannot have entered until the first replies (its key was held).
    wait_entered(&mut ctl.gate_entered, 1).await;
    ctl.gate_release.add_permits(1);
    let first = read_u32(&a.await.unwrap().expect("first dispatch ok"));

    wait_entered(&mut ctl.gate_entered, 1).await;
    ctl.gate_release.add_permits(1);
    let second = read_u32(&b.await.unwrap().expect("second dispatch ok"));

    // Never overlapped => the shared in-flight counter never exceeded 1.
    assert_eq!(first, 1, "same-key dispatch A must never overlap a sibling");
    assert_eq!(
        second, 1,
        "same-key dispatch B must never overlap a sibling"
    );
}

// ── Facet 3 — concurrency is capped at the scheduler budget C ────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrency_capped_at_budget() {
    const C: usize = 3;
    let host = WasmHost::compile(fixture_bytes()).expect("compile v2 fixture");
    let wl = instantiate_wasm_workload(&host).await.expect("instantiate");
    // queue_cap high enough to admit all submissions; budget bounds in-flight.
    let dispatcher = Arc::new(wl.into_concurrent_dispatcher(probe_spec(), C, 256, None));
    let (bridge, mut ctl) = gate_bridge();

    // C+2 distinct keys all want to run; only C may be in flight at once.
    let mut handles = Vec::new();
    for i in 0..(C as u64 + 2) {
        handles.push(spawn_dispatch(
            &dispatcher,
            PROBE,
            vec![],
            caller(100 + i),
            &bridge,
        ));
    }

    // Exactly C reach the guest and park; the extra 2 wait for a budget slot.
    wait_entered(&mut ctl.gate_entered, C).await;
    // Release everything; as the first C drain, the extra 2 are admitted.
    ctl.gate_release.add_permits(C + 2);

    let mut peak = 0u32;
    for h in handles {
        peak = peak.max(read_u32(&h.await.unwrap().expect("dispatch ok")));
    }
    assert_eq!(
        peak, C as u32,
        "peak in-flight must equal the budget C={C} (never exceed it)"
    );
}

// ── Facets 4 & 5 — an in-flight trap fails ALL siblings and rebuilds ─────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inflight_trap_fails_all_siblings_then_rebuilds() {
    const SIBLINGS: u64 = 3;
    let host = WasmHost::compile(fixture_bytes()).expect("compile v2 fixture");
    let wl = instantiate_wasm_workload(&host).await.expect("instantiate");
    let dispatcher = Arc::new(wl.into_concurrent_dispatcher(probe_spec(), 8, 256, None));
    let (bridge, mut ctl) = gate_bridge();

    // N-1 probe siblings park at test/gate (held), and one boom dispatch parks
    // at test/double_impl. All distinct keys, all in flight in the ONE region.
    let mut siblings = Vec::new();
    for i in 0..SIBLINGS {
        siblings.push(spawn_dispatch(
            &dispatcher,
            PROBE,
            vec![],
            caller(200 + i),
            &bridge,
        ));
    }
    let boom = spawn_dispatch(
        &dispatcher,
        BOOM,
        1i32.to_le_bytes().to_vec(),
        caller(299),
        &bridge,
    );

    // Wait until every sibling AND boom are suspended inside the instance.
    wait_entered(&mut ctl.gate_entered, SIBLINGS as usize).await;
    wait_entered(&mut ctl.impl_entered, 1).await;

    // Release ONLY boom's host import: it returns, the guest panics after the
    // await, and the whole run_concurrent region collapses — taking every
    // in-flight sibling down with it (facet 5: whole-region teardown, not
    // per-task isolation).
    ctl.impl_release.add_permits(1);

    let boom_res = boom.await.unwrap();
    assert!(boom_res.is_err(), "the trapping dispatch itself must fail");
    for (i, h) in siblings.into_iter().enumerate() {
        let res = h.await.unwrap();
        assert!(
            res.is_err(),
            "sibling {i} must fail when a co-resident dispatch traps (whole-region teardown)"
        );
        let msg = format!("{:?}", res.unwrap_err()).to_lowercase();
        assert!(
            msg.contains("trap") || msg.contains("unavailable") || msg.contains("instance"),
            "sibling {i} must fail with a retryable trap-class error, got: {msg}"
        );
    }

    // The instance must rebuild: a fresh dispatch succeeds AND reports
    // MAX_SEEN==1, which can only be true on a fresh linear memory (the pre-trap
    // in-flight count was SIBLINGS and never decremented on the torn-down region).
    ctl.gate_release.add_permits(1); // let the recovery probe pass straight through
    let recovered = dispatcher
        .dispatch(PROBE, vec![], caller(777), &bridge)
        .await
        .expect("a dispatch after the trap must succeed on the rebuilt instance");
    assert_eq!(
        read_u32(&recovered),
        1,
        "post-trap probe must see MAX_SEEN==1 (fresh linear memory ⇒ rebuild + cleared invocations)"
    );
}

// ── Facet 6 — per-dispatch timeout: clean cancel, bounded, no poison ─────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dispatch_timeout_cancels_frees_key_and_survives() {
    let host = WasmHost::compile(fixture_bytes()).expect("compile v2 fixture");
    let wl = instantiate_wasm_workload(&host).await.expect("instantiate");
    // 300ms per-dispatch deadline, enforced inside the region.
    let dispatcher = Arc::new(wl.into_concurrent_dispatcher(
        probe_spec(),
        8,
        256,
        Some(Duration::from_millis(300)),
    ));
    let (bridge, mut ctl) = gate_bridge();

    // Two distinct-key dispatches both park at the gate forever (never
    // released). Each must independently hit its deadline even while the other
    // occupies the region (RA7: the in-region timer stays prompt under load).
    let a = spawn_dispatch(&dispatcher, PROBE, vec![], caller(1), &bridge);
    let b = spawn_dispatch(&dispatcher, PROBE, vec![], caller(2), &bridge);
    wait_entered(&mut ctl.gate_entered, 2).await;

    // Bounded resolution to TimedOut — a real hang would trip this watchdog.
    let ra = tokio::time::timeout(Duration::from_secs(5), a)
        .await
        .expect("dispatch A must resolve within the watchdog, not hang")
        .unwrap();
    let rb = tokio::time::timeout(Duration::from_secs(5), b)
        .await
        .expect("dispatch B must resolve within the watchdog, not hang")
        .unwrap();
    assert!(
        matches!(ra, Err(actr_protocol::ActrError::TimedOut)),
        "dispatch A must resolve TimedOut, got {ra:?}"
    );
    assert!(
        matches!(rb, Err(actr_protocol::ActrError::TimedOut)),
        "dispatch B must resolve TimedOut, got {rb:?}"
    );

    // Layer-2 landed: the timed-out dispatches were truly cancelled and their
    // keys freed, and the store was NOT poisoned. A NEW dispatch on the SAME
    // key as A must now complete promptly on the same instance — it can only be
    // admitted + run if the cancelled dispatch really left the region (else the
    // scheduler would never re-arm this key and this await would hang past the
    // watchdog). We use the un-gated `test/echo` route so the recovery does not
    // depend on the gate: a CLEAN cancel is a drop (not an unwind), so the
    // guest's own in-flight counter leaks and — more subtly — the cancelled
    // dispatch's guest-side host-import teardown lags slightly behind the prompt
    // reply/key-free, which would otherwise let a lingering waiter steal a fresh
    // gate permit. Echo has no host import, so it isolates the property under
    // test: same-key advance + a healthy (un-poisoned) store.
    let payload = b"post-timeout".to_vec();
    let recovered = tokio::time::timeout(
        Duration::from_secs(5),
        dispatcher.dispatch(ECHO, payload.clone(), caller(1), &bridge),
    )
    .await
    .expect("same-key dispatch after a timeout must not hang (key was freed)")
    .expect("same-key dispatch after a timeout must succeed (store not poisoned)");
    assert_eq!(
        recovered.as_ref(),
        payload.as_slice(),
        "the recovered dispatch must round-trip on the same (un-poisoned) instance"
    );
}

// ── Facet 7 — gate off degrades to the serial M4 path (MAX_SEEN == 1) ────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gate_off_is_serial_max_seen_1() {
    use actr_hyper::workload::InvocationContext;
    use actr_protocol::{Direction, RpcEnvelope, prost::Message as _};

    let host = WasmHost::compile(fixture_bytes()).expect("compile v2 fixture");
    let wl = instantiate_wasm_workload(&host).await.expect("instantiate");
    // Serial runner (gate off): the V2 kernel drives the per-dispatch region.
    let runner = Arc::new(wl.into_workload_runner());
    let (bridge, mut ctl) = gate_bridge();

    let spawn_serial = |serial: u64, bridge: HostAbiFn| {
        let runner = runner.clone();
        tokio::spawn(async move {
            let bytes = RpcEnvelope {
                route_key: PROBE.to_string(),
                payload: Some(Bytes::new()),
                request_id: format!("serial-{serial}"),
                direction: Some(Direction::Request as i32),
                ..Default::default()
            }
            .encode_to_vec();
            let inv = InvocationContext {
                self_id: ActrId::default(),
                caller_id: caller(serial),
                request_id: format!("serial-{serial}"),
            };
            runner.dispatch(&bytes, inv, &bridge).await
        })
    };

    // Submit two "concurrently" to the serial runner. It processes one at a
    // time, so the second cannot enter the guest until the first completes.
    let a = spawn_serial(1, bridge.clone());
    let b = spawn_serial(2, bridge.clone());

    wait_entered(&mut ctl.gate_entered, 1).await;
    ctl.gate_release.add_permits(1);
    let first = read_u32(&a.await.unwrap().expect("serial first ok"));

    wait_entered(&mut ctl.gate_entered, 1).await;
    ctl.gate_release.add_permits(1);
    let second = read_u32(&b.await.unwrap().expect("serial second ok"));

    assert_eq!(first, 1, "serial path must never overlap (A)");
    assert_eq!(second, 1, "serial path must never overlap (B)");

    runner.shutdown().await;
}

// ── Facet 9 — package compat matrix ──────────────────────────────────────────

/// A V1 (0.1.0 sync-world) guest must stay serial even when Interleaved is
/// requested: `WasmKernel::is_v2()` is false, so the executor routes it to the
/// serial `run_loop`. Dispatch must still work.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compat_v1_guest_on_interleaved_falls_back_to_serial() {
    let host = WasmHost::compile(V1_SYNCLIFT_GUEST).expect("compile v1 fixture");
    let wl = instantiate_wasm_workload(&host)
        .await
        .expect("instantiate v1");
    // Request Interleaved; a V1 kernel must transparently run serially.
    let runner = wl.into_interleaved_runner(None);

    use actr_protocol::{Direction, RpcEnvelope, prost::Message as _};
    let payload = b"v1-serial-fallback".to_vec();
    let bytes = RpcEnvelope {
        route_key: "test/echo".to_string(),
        payload: Some(Bytes::from(payload.clone())),
        request_id: "v1-compat".to_string(),
        direction: Some(Direction::Request as i32),
        ..Default::default()
    }
    .encode_to_vec();
    let inv = actr_hyper::workload::InvocationContext {
        self_id: ActrId::default(),
        caller_id: None,
        request_id: "v1-compat".to_string(),
    };
    let bridge: HostAbiFn = Arc::new(|_| Box::pin(async move { HostOperationResult::Error(-1) }));

    let reply = runner
        .dispatch(&bytes, inv, &bridge)
        .await
        .expect("V1 echo must dispatch on the serial fallback");
    assert_eq!(reply.as_ref(), payload.as_slice());
    runner.shutdown().await;
}

/// A V2 guest must work in BOTH modes: serial (facet 7 above) and interleaved
/// (facets 1-3 above). This is the positive control that the same 0.2.0 package
/// dispatches correctly through the interleaved runner for a lone message too.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compat_v2_guest_single_dispatch_on_interleaved() {
    let host = WasmHost::compile(fixture_bytes()).expect("compile v2 fixture");
    let wl = instantiate_wasm_workload(&host)
        .await
        .expect("instantiate v2");
    let dispatcher = Arc::new(wl.into_concurrent_dispatcher(probe_spec(), 8, 256, None));
    let (bridge, mut ctl) = gate_bridge();

    let d = spawn_dispatch(&dispatcher, PROBE, vec![], caller(1), &bridge);
    wait_entered(&mut ctl.gate_entered, 1).await;
    ctl.gate_release.add_permits(1);
    let reply = d.await.unwrap().expect("v2 single dispatch ok");
    assert_eq!(read_u32(&reply), 1, "a lone dispatch sees MAX_SEEN==1");
}
