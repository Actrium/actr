# `wit-lint`

Drift guard between the WIT contract at `core/framework/wit/actr-workload.wit` and the hand-rolled DynClib C ABI at `core/framework/src/guest/dynclib_abi.rs`.

DynClib (mobile static-link / server `dlopen`) cannot consume `wit-bindgen`-generated bindings — `wit-bindgen c` emits wasm-targeted glue, not a portable C ABI — so its ABI is hand-written. This tool cross-checks the two surfaces at CI time and refuses to let them drift.

```bash
cargo run -p wit-lint
```

Drift triggers a non-zero exit; the wired CI step is in `.github/workflows/ci-rust.yml`.

## Relationship to `actr-wit-compile-web`

The two tools live side-by-side under `tools/` and read the same WIT file but solve **different** drift problems. Both pin `wit-parser = 0.247.0` so they cannot disagree about how the WIT parses.

| Tool | Drives what | Drift it catches |
|---|---|---|
| `wit-lint` | nothing — pure read-side check | WIT vs **DynClib C ABI** (hand-written, `dynclib_abi.rs`) |
| `actr-wit-compile-web` | generates `bindings/web/crates/actr-web-abi/src/{types,guest,host}.rs` | WIT vs **wasm-bindgen browser ABI** (generated, regenerate with `cargo run -p actr-wit-compile-web`) |

CI invokes both as drift gates; they don't import each other and don't need to.

The boundary is permanent: `wit-bindgen` codegen targets only wasm, so any C ABI consumer (DynClib, future Linked variant, anything `dlopen`-style) needs its own hand-written ABI plus a lint like this. See `bindings/web/docs/option-u-wit-compile-web.zh.md` §0 for the architectural reasoning behind splitting browser codegen out of `wit-bindgen`.
