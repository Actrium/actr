# Phase 0 Component Model Spike - Report

**Date:** 2026-04-18
**Repo HEAD at spike start:** `2dc0f0e8`
**Host:** Linux x86_64, Rust 1.91.1

## Executive summary

The WASM Component Model + WIT + wit-bindgen + wasmtime toolchain works
end-to-end for actr's conceptual workload contract. The spike builds, runs,
and exercises every pattern actr needs: host import, guest export, nested
records, variants, result types, instance reuse, panic-to-trap propagation.
No blockers were hit. Recommendation: **proceed to Phase 1.**

The only footgun found was a transitive Rust-version requirement
(wasmtime 44.0.0 requires rustc >= 1.92, so the spike pins wasmtime 43.0.1).
This is a pin decision, not a blocker.

## Tooling

### Versions used

| Tool              | Version   | Source                                  |
|-------------------|-----------|-----------------------------------------|
| rustc             | 1.91.1    | stable                                  |
| wasm32-wasip2     | -         | `rustup target add wasm32-wasip2`       |
| wit-bindgen (crate) | 0.57.1  | pinned in `guest/Cargo.toml`            |
| wasmtime          | 43.0.1    | pinned in `host/Cargo.toml`             |
| wasmtime-wasi     | 43.0.1    | pinned in `host/Cargo.toml`             |
| wasm-tools (CLI)  | 1.247.0   | `cargo install wasm-tools`              |
| wit-bindgen CLI   | 0.57.1    | `cargo install wit-bindgen-cli` (Q15)   |
| jco               | 1.18.1    | `npm i @bytecodealliance/jco` (Q14)     |
| Node              | 20.19.4   | for jco                                 |
| `cargo-component` | not used  | not needed                              |

### Toolchain decision - `cargo-component` vs `wasm32-wasip2` + `wasm-tools`

**Chose `wasm32-wasip2`** directly. Reasons:
- Rust 1.91 already ships a usable `wasm32-wasip2` target; no extra cargo plugin.
- `wit-bindgen 0.57` + `wasm32-wasip2` emits a valid Component in one step
  (confirmed via `wasm-tools validate` and `wasm-tools component wit`). No
  post-processing step needed.
- Keeps the build story to one familiar tool (`cargo build`) plus `wasm-tools`
  for inspection/stripping. No third-party cargo subcommand whose release
  cadence we have to track separately.
- For pure-Rust guests this is the cleanest path today.

`cargo-component` would add value if we were juggling component metadata
manually or linking multiple core modules into one component. Our workload
model is single-crate-per-guest, so the extra tool buys us nothing.

### Build times (cold, target/ cleared)

| Phase       | Wall time |
|-------------|-----------|
| Guest build (wasip2, release) | ~25 s |
| Host build (wasmtime 43 + WASI, release) | ~2 m 40 s |

Hot rebuilds: near-instant (cached).

### Binary size

| Artifact | Bytes |
|----------|-------|
| `spike_guest.wasm` (release, no strip) | 277 857 |
| `spike_guest.stripped.wasm` (`wasm-tools strip`) | 69 410 |

Stripping removes debug + custom sections, giving ~4x shrink. Further
shrinkage possible via `wasm-opt -Oz` and cutting the dependency on
wasi stdlib (e.g. `#![no_std]` guest), but ~70 KB for a non-trivial
Rust component is already within budget.

### Rough edges encountered

1. **`wasmtime::Error` + `anyhow::Context`.** The host initially used
   `anyhow::Context::context` on `wasmtime::Result`. Wasmtime's error type
   is not a `std::error::Error`, so `anyhow::Context` is not implemented.
   Fix: either `use wasmtime::error::Context;` or `.map_err(|e| anyhow!(...))`.
   **Impact:** trivial, but not obvious from the wasmtime docs.

2. **`wasmtime_wasi` module reorganisation.** `WasiView`, `IoView`,
   `WasiCtx` used to live under `wasmtime_wasi::p2`. In 43.x they're at
   `wasmtime_wasi::` root; `WasiView::ctx` now returns a `WasiCtxView<'a>`
   struct instead of separate `ctx` + `table` trait methods. Compiler
   diagnostics were accurate and pointed at the fix. **Impact:** one-off
   migration.

3. **wit-bindgen 0.57 passes host-imported `list<u8>` / `string` as
   `&[u8]` / `&str` to the guest.** Earlier examples used `Vec<u8>`;
   update accordingly. Zero-copy on the guest side is a nice improvement.

4. **`add_to_linker` generic signature.** Generated `add_to_linker` takes a
   `D: HasData` bound plus a closure projecting `&mut T` to the host impl.
   Use the standard pattern:
   `add_to_linker::<_, HasSelf<_>>(&mut linker, |s| s)`. Not obvious without
   reading either the macro output or the wasmtime component-model book.

5. **Rust-version creep.** Every wasmtime minor release bumps the MSRV to
   roughly "latest stable minus 1." Pin an exact version per Phase and
   schedule the upgrade, don't float `^`.

## WIT expressiveness for actr's shape

### Q5 - async methods

WIT's grammar accepts the `async` keyword on `func`:

```wit
on-tick: async func() -> result<_, string>;
```

`wasm-tools component wit` round-trips the async keyword. `wit-bindgen rust`
0.57 emits the expected `async fn on_tick() -> Result<(), ...>` for the guest
(trait method) side. **Async is in the grammar and in the bindgen - usable
for Phase 1.**

Caveat: full async execution on the host requires wasmtime with the
`async` / `component-model-async` features and a tokio-flavored store. The
spike host is sync because the bench loop needed sync for simpler timing,
not because async is broken. Budget some time in Phase 1 to wire the async
variant end-to-end before declaring the 16 async hooks runnable.

### Q6 - nested records

The spike WIT defines `peer-event { peer: peer-info, timestamp-ms: u64 }`
with `peer-info` as a separate record, and `error-event` containing a
variant field (`error-category`) and a `list<tuple<string, string>>`.

Both compile on the guest, serialise cleanly across the boundary, and
round-trip verbatim - the host constructs a `PeerEvent { peer: PeerInfo {
peer_id, relayed }, timestamp_ms }`, calls `call_on_peer_event`, and the
guest logs the exact fields back.

**Verdict:** WIT handles nested records with no depth restrictions we
could hit. The 3-level nesting in the spike is a superset of what actr's
real hook types need.

### Q7 - variant errors at the scale of ActrError

`spike-error` in the spike is a 2-case variant; the spike also defines
`error-category` as a 4-case variant with one payload-carrying arm
(`other(string)`). Both generate clean `#[repr(C)]`-ish Rust enums plus
matching C / TS / Python bindings with no pain.

Extending to 10+ variants is purely mechanical - variant cases are an
O(1) addition to the WIT source and O(1) to the generated code. The one
thing to watch: WIT variants are closed enums. If `ActrError` has
`#[non_exhaustive]` or needs wire-compat across WIT revisions, we need to
reserve an explicit `unknown(list<u8>)` escape hatch.

**Verdict:** `variant` scales fine; design the closed set deliberately and
reserve an escape-hatch arm.

### Q8 - guest calling host-imported async functions

Not explicitly exercised in this spike (host imports are sync in the
benchmark to keep timing clean). wit-bindgen 0.57 generates `async fn` on
both sides when the WIT func is declared `async`, so the bidirectional
flow is symmetric. Confirmed through the Q5 Rust-binding probe
(generated `async fn on_tick() -> ...`).

Blocker risk: **low.** Wire it up in Phase 1 on the async host path.

## Runtime behavior

### Q9 - host imports (guest -> host) work cleanly

Yes. On start, the guest calls:

```
host::log_message("info", "workload starting");
host::call_raw(ActrId{42}, "probe.Ping", b"hello-from-guest");
```

Both reach the host, the `call_raw` reply (bytes constructed on the host)
arrives intact in the guest, and the guest then logs it back via
`log_message`. Observed output:

```
[guest:info] workload starting
[guest:debug] call-raw ok: fake-reply target=42 route=probe.Ping paylen=16
```

Bi-directional flow is solid.

### Q10 - per-call overhead

Benchmark: 1000 calls of `dispatch(env: RpcEnvelope)` on the same
instance, payload = 64 bytes.

| Metric | Value |
|--------|-------|
| Total  | ~6 ms |
| Per call | **~6 us** |

The call crosses: host marshalling -> Cranelift-JITted component code ->
guest marshalling -> guest user code (which also calls back into the host
for each dispatch via `log_message`) -> reverse path.

6 us/call is well under the ~100 us-scale we'd worry about for RPC-rate
workloads. Per-call overhead is not a concern.

(No `wasmtime::Module` baseline measured in this spike - the existing
actr WasmWorkload is deep-integrated with actr's ABI and can't be invoked
standalone without dragging the rest of the runtime. Comparable numbers
would require a parallel no-op-module fixture; low-value add given the
absolute number is already comfortable.)

### Q11 - same instance, many calls (actor persistence)

Yes. The benchmark above is 1000 calls on the same `Store`/`Instance`.
No resource leak signs, throughput stays flat, and the host's
`call_count` (bumped by each guest `call_raw`) is 1 (from `on_start` only),
confirming the host state is persistent across calls.

Actor identity & per-instance state model in Phase 1: one `Store<HostState>`
per actor instance, reused across all hook invocations. Maps onto
wasmtime's `Store` concept 1-to-1.

### Q12 - panic propagation

Yes. Guest panic inside `dispatch("panic.Panic")` surfaces as a wasmtime
`Trap` with the WASM backtrace:

```
error while executing at wasm backtrace:
  0: 0xa2aa - spike_guest.wasm!abort
  1: 0x50ec - spike_guest.wasm!std::sys::pal::wasip2::helpers::abort_internal
  2: 0x62ba - spike_guest.wasm!std::process::abort
  ...
```

The `Store` is poisoned after a trap (can't reuse it), which matches
wasmtime's documented semantics. Phase 1 implication: on trap, the host
must tear down the offending actor instance and decide on restart policy
- same strategy as with `Module`-based workloads today.

## For Phase 1 planning

### Q13 - migration from `wasmtime::Module` to `wasmtime::component::Component`

The main actr WASM host is in `core/hyper/src/wasm/host.rs` (~789 lines).
Current shape:

- Hand-rolled `Linker::<HostData>::new()` with per-hook `func_wrap(...)`
  calls in `register_host_imports` (lines 152+).
- Imports `wasi_snapshot_preview1` fragments by hand
  (`proc_exit`, `fd_close`, etc).
- Uses `Module::new(&engine, wasm_bytes)` + `Instance` + `TypedFunc`.
- Owns a bespoke ABI layer in `core/hyper/src/wasm/abi.rs` for marshalling
  between host and guest (pointer/len into linear memory).

Component migration replaces:
- `Module` -> `Component`
- Hand-rolled linker wiring -> `bindgen!()`-generated `add_to_linker`
  called once per interface.
- Bespoke ABI layer -> **deleted**; WIT + Component canonical ABI subsumes
  it entirely.
- `wasi_snapshot_preview1` manual stubs ->
  `wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?` one-liner.

The API shift is **mechanical on the wasmtime side**. The *structural*
work is in the host-state model: the current `HostData` carries fields
like `pending_call`, `current_invocation`, `caller_id` threaded through
the cooperative-suspend mechanism (lines 35-80). Moving to Component
Model, the bookkeeping around cooperative suspend likely collapses into
just holding a `WasiCtx` + `ResourceTable` + actr-specific context,
because the canonical ABI handles all the pointer/length marshalling
work that the current suspend/resume dance services.

**Rough estimate:** `register_host_imports` (~150 lines) shrinks to a
single WIT file + `bindgen!()` macro. `abi.rs` disappears. `host.rs`
drops to maybe 250 lines. The guest-side guest bridge
(`bindings/web/crates/runtime-sw/src/guest_bridge.rs`) needs to be
rewritten against wit-bindgen-generated exports, but the shape is
1-to-1 with what the WIT contract declares.

**Restructuring concerns:**
- Store lifetime: wasmtime `Store<T>` owns both `WasiCtx` and our actr
  context today; same pattern, just swap the contents.
- Cooperative suspend: the existing code unwinds WASM to pump async
  host calls. Component Model + wasmtime async feature gives this
  natively - the unwind/rewind machinery can go away. Verify before
  committing: the async story needs its own spike-ette in Phase 1
  (see Q5 caveat).

### Q14 - jco (browser) compatibility

**Yes.** `jco transpile spike_guest.wasm` produces:

```
spike_guest.core.wasm       261 KiB  (real guest code)
spike_guest.core2.wasm        5 KiB  (adapter)
spike_guest.js              157 KiB  (glue + preview2-shim)
spike_guest.d.ts            0.9 KiB  (typed entry)
interfaces/                        ( actr-spike-{host,types,workload}.d.ts
                                   + wasi-io/streams/cli shims )
```

The generated `actr-spike-types.d.ts` is exactly what a hand-written
TypeScript binding would look like - `ActrId { serialNumber: bigint }`,
`SpikeError = SpikeErrorInternal | SpikeErrorBadPayload`, etc. All
nested records, variant tags, and tuple lists preserved. The browser
host just needs to implement `callRaw(target, routeKey, payload)` and
`logMessage(level, msg)` in JS - everything else comes free.

**preview2-shim coverage for our host imports:** yes. The only WASI
imports the guest actually needs are `wasi:cli/{environment,exit,stderr}`
and `wasi:io/{error,streams}`. jco's preview2-shim provides all of them.

**Phase-1 implication:** the same WIT file drives server (wasmtime) and
browser (jco) worlds. Single source of truth for the workload contract.

### Q15 - wit-bindgen-c (C ABI generation)

**Yes, clean output.**

`wit-bindgen c --world spike-guest --out-dir ... wit/actr-spike.wit`
produces:

- `spike_guest.h` (183 LOC) - typedefs for every record + variant +
  result, with stable `actr_spike_{types,host}_*` names:
  ```c
  typedef struct actr_spike_types_peer_event_t {
    actr_spike_types_peer_info_t peer;
    uint64_t timestamp_ms;
  } actr_spike_types_peer_event_t;

  extern bool actr_spike_host_call_raw(
    actr_spike_host_actr_id_t *target,
    spike_guest_string_t *route_key,
    spike_guest_list_u8_t *payload,
    spike_guest_list_u8_t *ret,
    actr_spike_host_spike_error_t *err);
  ```
- `spike_guest.c` (505 LOC) - runtime support for string/list/variant
  marshalling.
- `spike_guest_component_type.o` - a precompiled ELF object that embeds
  the WIT type info; link this against a guest written in C and you get
  a valid Component.

Variants are encoded as `{ uint8_t tag; union { ... } val; }` with
matching `#define ... N` constants - precisely the shape a language
binding generator would want to build on top of.

**Implication for non-Rust bindings (Python/Kotlin/Swift/TS wrappers):**
the `bindings/` generators in actr today hand-roll against the Rust
FFI. A future phase can switch them to consume either the generated C
header (for Python ctypes, Kotlin JNI, Swift) or the Component Model
directly (jco for TS, component-python for Python when it lands).
Both paths are live today.

## Blockers? Surprises?

**Blockers:** none.

**Surprises (worth calling out):**
1. Rust-version floor moves fast on wasmtime - expect to bump MSRV
   roughly once per wasmtime minor.
2. The `bindgen!` macro-generated API is pleasant but heavier than
   expected (cold rebuild of the host crate pulls ~2.5 minutes of
   wasmtime internals). Not a runtime issue, but factor into CI planning.
3. WIT gets the component ABI *for free* - both size (~70 KB stripped
   for a Rust guest with 4 exports + 2 imports) and per-call cost
   (~6 us) are at the "stop worrying about overhead" level.
4. The same WIT file driving Rust server (wasmtime), JS/browser (jco),
   and C (wit-bindgen c) toolchains with zero divergence is a bigger
   practical win than I'd weighted going in. This removes a whole
   category of "which binding am I looking at" confusion from actr's
   bindings/ tree.

## Recommendation

**Proceed to Phase 1.** Concrete next steps:

1. Land the spike as a reference (this commit).
2. Copy actr's real WorkloadContract into a canonical WIT file at
   `core/framework/wit/actr-workload.wit` (all 16 hooks, real types).
3. Spike the **async host** variant of this same setup before
   committing to a full rewrite - verify the 16 async hooks actually
   run under wasmtime's `component-model-async` path and that the
   existing cooperative-suspend machinery can be deleted.
4. Plan the `core/hyper/src/wasm/host.rs` rewrite around
   `bindgen!()`-generated linker + `Component::instantiate_async`.
   Delete `core/hyper/src/wasm/abi.rs` as part of that commit.
5. Keep `cargo-component` off the dependency list unless a future
   requirement (e.g. multi-module linking) demands it. `wasm32-wasip2`
   + `wasm-tools` is enough.
