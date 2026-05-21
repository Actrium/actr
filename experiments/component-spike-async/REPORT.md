# Phase 0.5 Component Model Async Spike - Report

**Date:** 2026-04-18
**Repo HEAD at spike start:** `5842b85c`
**Host:** Linux x86_64, Rust 1.91.1

## Executive summary

The async end-to-end path works. wasmtime's `Component::instantiate_async` +
wit-bindgen's async-aware bindings deliver the ergonomics we need (guest
writes plain `.await`, host writes `async fn` trait methods) and the runtime
behaves correctly: tokio executor stays free during guest awaits, multi-
instance parallelism is real, guest panics surface as Traps, error variants
round-trip.

All 8 tests pass. **No blockers for Phase 1.** The handwritten cooperative-
suspend / asyncify machinery in `core/hyper/src/wasm/host.rs` can be fully
deleted in Phase 1. `core/hyper/src/wasm/abi.rs` goes away entirely.

Three non-blocking gotchas worth planning for:

1. Rust 1.91's bundled `wasm-component-ld 0.5.17` can't parse the async
   component custom sections wit-bindgen 0.57 emits. Install 0.5.22+ and
   point `-Clinker=` at it.
2. Per-call overhead is ~1.1 ms/call vs. Phase 0's ~6 us/call sync baseline.
3. WIT `async func` (Component Model Concurrency proposal) is a **different**
   feature from wit-bindgen `async: true`. We chose the latter.

**Recommendation:** proceed to Phase 1 as planned.

## Tooling

### Versions used

| Tool                      | Version   | Notes                                  |
|---------------------------|-----------|----------------------------------------|
| rustc                     | 1.91.1    | same as Phase 0                        |
| wasm32-wasip2             | -         | `rustup target add wasm32-wasip2`      |
| wit-bindgen (crate)       | =0.57.1   | `async` feature on by default          |
| wasmtime                  | =43.0.1   | added `component-model-async` + `async` features |
| wasmtime-wasi             | =43.0.1   | use `p2::add_to_linker_async`          |
| wasm-tools (CLI)          | 1.247.0   | validate async components with `--features=all` |
| wasm-component-ld         | 0.5.22    | new: bundled 0.5.17 rejects async sections |
| tokio                     | 1         | full features, multi-thread runtime    |

### Wasmtime feature changes from Phase 0

Host `Cargo.toml` adds `component-model-async` and `async` features. Config
needs `wasm_component_model_async(true)` or the component fails to load.

Even when WIT is declared with only sync `func`, wit-bindgen 0.57 with
`async: true` emits `context.get` (async-ABI primitive) in the guest core
wasm, so the host must opt into async features to validate.

### Toolchain gotcha - wasm-component-ld

Rust 1.91 ships `wasm-component-ld 0.5.17`. wit-bindgen 0.57 with `async: true`
produces a component-type custom section 0.5.17 cannot parse:

    error: failed to parse core wasm for componentization
    Caused by:
      0: decoding custom section component-type:wit-bindgen:0.57.1:...
      1: invalid leading byte (0x43) for component defined type (at offset 0x132)

Fix:

    cargo install wasm-component-ld --version 0.5.22
    RUSTFLAGS="-Clinker=$HOME/.cargo/bin/wasm-component-ld" \
        cargo build --release --target wasm32-wasip2

### Build times (cold)

| Phase                                     | Wall time |
|-------------------------------------------|-----------|
| Guest build (wasip2, release, async)      | ~12 s     |
| Host build (wasmtime 43 + async + wasi)   | ~20 s (with Phase-0 cache) |

### Binary size

| Artifact                                      | Bytes   |
|-----------------------------------------------|---------|
| `spike_guest_async.wasm` (release)            | 310,453 |
| `spike_guest_async.stripped.wasm`             | 100,355 |

Async plumbing adds ~30 KB stripped vs. Phase 0's 69,410.

## Test results

### Test 1 - basic async dispatch round-trip

Dispatch took 51.2 ms; reply was
`"async-echo: downstream(downstream.Echo)[target=42]: hello"`. End-to-end
async round-trip through host-imported async `call_raw` works.

### Test 2 - concurrent dispatches on DIFFERENT instances

Two separate stores dispatched via `tokio::join!` — wall time 51.2 ms
(not 100 ms). Multi-actor parallelism is real.

### Test 3 - concurrent dispatches on the SAME instance

Cannot compile. `call_dispatch(&mut Store<T>, ...)` requires exclusive
`&mut store`; Rust's borrow checker rejects `tokio::join!` of two such
futures. `Store<T>` is not `Sync`. **Actor-instance single-threadedness
is enforced by the type system, not by runtime convention.** Fell back
to sequential: 102.4 ms (2 × 50 ms).

### Test 4 - host thread free during guest await

Ticker recorded 5 ticks during the 50 ms dispatch — tokio executor kept
running. Wasm instance does not block the reactor. This is the critical
property that makes cooperative-suspend/asyncify machinery obsolete.

### Test 5 - guest-side async ergonomics

Guest writes `call_raw(...).await?` directly. No block_on, no poll loops,
no custom wakers.

Gotchas:
1. With `async: true`, ALL imports become `async fn` at the Rust surface —
   even sync WIT imports like `log_message` need `.await` on the guest.
2. Async imports take owned `String` / `Vec<u8>` instead of `&str` / `&[u8]`.
   Futures can't hold stack-frame borrows across `await`.

### Test 6 - throughput

100 sequential dispatches (each with 50 ms host sleep): total 5113 ms,
51.13 ms/call. Overhead net of sleep: ~1130 us/call.

Comparison: Phase 0 sync was ~6 us/call. Async adds ~180x overhead.
Still fine for RPC-rate workloads, but worth re-measuring against the
real 16-hook contract in Phase 1.

### Test 7 - error propagation

Guest returned `Err(SpikeError::Timeout("guest reported timeout"))`; host
pattern-matched and received the variant + message intact.

### Test 8 - guest panic AFTER suspension point

Guest awaited `call_raw` then panicked. Surfaces as a wasmtime Trap with
full wasm backtrace. Store poisoned (unreusable). No memory / instance
corruption. Same panic semantics as Phase 0 sync path.

## WIT async syntax - canonical form

Two orthogonal mechanisms:

### Mechanism 1: WIT `async func`

Declaring `call-raw: async func(...) -> ...` invokes the Component Model
Concurrency proposal — semantically "this can be in flight concurrently
with OTHER calls on the same instance." wasmtime 43 generates
**Accessor-based** bindings (static methods on `HostWithStore: HasData`
taking `&Accessor<T, Self>`; drive with `store.run_concurrent(async |accessor| ...)`).
wasmtime config docs mark this "very incomplete."

### Mechanism 2 (chosen): plain WIT `func` + bindgen `async: true`

WIT functions declared sync. Guest:

    wit_bindgen::generate!({ world: "...", path: "...", async: true, generate_all });

Host:

    wasmtime::component::bindgen!({
        world: "...", path: "...",
        imports: { default: async | trappable },
        exports: { default: async },
    });

Generates `async fn call_raw(&mut self, ...) -> wasmtime::Result<...>` —
ordinary `&mut self`. `add_to_linker::<_, HasSelf<_>>` still works. Call
path still `call_dispatch(&mut store, &env).await`. Minimal shape change
from Phase 0.

Both mechanisms still require `config.wasm_component_model_async(true)`
because the guest emits `context.get` either way.

Decision: Mechanism 2 for Phase 1. Matches actr's single-threaded-actor
model.

## Comparison to actr's cooperative-suspend machinery

| Current                                                       | Phase 1 replacement                   |
|---------------------------------------------------------------|---------------------------------------|
| `Module::new(&engine, bytes)`                                 | `Component::from_binary(&engine, bytes)` |
| `Linker::<HostData>::new()` + 20 `func_wrap` calls            | `actr::workload::host::add_to_linker::<_, HasSelf<_>>(&mut linker, |s| s)?` |
| `wasi_snapshot_preview1` manual stubs                         | `wasmtime_wasi::p2::add_to_linker_async(&mut linker)?` |
| `actr_host_invoke` import + unwind/rewind state machine       | `async fn call_raw(&mut self, ...) -> ...` trait method |
| `AsyncifyMode` enum + asyncify buffer at 0x8000               | gone                                  |
| `actr_alloc` / `actr_free` guest imports                      | gone — WIT canonical ABI              |
| `InitPayloadV1` / `AbiFrame` / `AbiReply` prost encoding      | gone — WIT records/variants           |
| `write_legacy_context_bytes`, `write_to_wasm`, etc. helpers   | gone                                  |
| `asyncify_stop_unwind`, `asyncify_start_rewind` typed funcs   | gone                                  |
| Loop in `handle()` re-calling `actr_handle` during rewind     | gone — single `.await`                |

**Can `core/hyper/src/wasm/abi.rs` be fully deleted?** Yes. Entire file is
manual ptr/len ABI docs + error-code helper; both replaced by WIT canonical
ABI and WIT `variant`s.

**Can `core/hyper/src/wasm/host.rs` shrink from 789 LOC?** Estimate ~150
LOC: register_host_imports 180→30, asyncify+drive-loop 200→0, memory
helpers 100→0, `handle()` 140→30, engine plumbing 60→40.

`actr-framework/guest/abi.rs` also becomes redundant (generated by
wit-bindgen on the guest). `bindings/web/crates/runtime-sw/src/guest_bridge.rs`
needs a full rewrite against wit-bindgen exports, but the shape is 1:1
with the WIT contract.

## Blockers / unresolved

**Blockers:** none.

**Unresolved:**

1. Re-measure the ~1.1 ms/call overhead with the real 16-hook contract in
   Phase 1; should be per-call, not per-hook-count multiplicative.
2. `wasm-component-ld 0.5.22` needs a CI install step until rustc ships it.
3. Decide Phase 1 policy on `async: true` globally vs. per-function
   (`async: [ ... ]` filter) for sync imports like `log_message`.
4. Component Model Concurrency (Mechanism 1) is still in flux upstream;
   revisit only if actr ever needs true in-instance concurrency.

## Phase 1 planning adjustments

1. Use Mechanism 2 (plain WIT `func` + bindgen `async: true`); document
   this choice.
2. Add `cargo install wasm-component-ld --version 0.5.22` to CI.
3. Budget 2-3 days to rewrite
   `bindings/web/crates/runtime-sw/src/guest_bridge.rs` against wit-bindgen
   exports.
4. Port these 8 tests to the full 16-hook WIT as a Phase-1 regression
   suite before flipping the switch.
5. Update `core/hyper/src/wasm/host.rs`'s "Not `Sync`: caller is
   responsible for concurrency protection" docstring — the new invariant
   is compile-time-enforced via `&mut Store<T>`.
6. Re-bench overhead against the real 16-hook contract during Phase 1
   kickoff.

## Recommendation

**Proceed to Phase 1.** No pivot needed. One more measurement pass against
the real WIT contract during Phase 1 kickoff is the only open loop.
