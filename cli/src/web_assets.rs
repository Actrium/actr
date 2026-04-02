//! Embedded web runtime assets for `actr run --web`.
//!
//! These files are compiled into the `actr` binary so that `actr run --web`
//! can serve a fully self-contained web actor host without requiring any
//! external runtime WASM files, JS glue, or HTML pages.
//!
//! Assets:
//! - `actr_runtime_sw_bg.wasm` — shared runtime WASM (wasm-pack from runtime-sw)
//! - `actr_runtime_sw.js`      — wasm-bindgen JS glue for the runtime
//! - `actor.sw.js`             — Service Worker entry point
//! - `actr-host.html`          — self-contained host page with inline @actr/dom

/// Shared runtime WASM binary (compiled from actr-runtime-sw via wasm-pack).
pub const RUNTIME_WASM: &[u8] = include_bytes!("../assets/web-runtime/actr_runtime_sw_bg.wasm");

/// wasm-bindgen JS glue for the shared runtime.
pub const RUNTIME_JS: &str = include_str!("../assets/web-runtime/actr_runtime_sw.js");

/// Generic Service Worker entry point (actor.sw.js).
pub const ACTOR_SW_JS: &str = include_str!("../assets/web-runtime/actor.sw.js");

/// Self-contained HTML host page with inline @actr/dom (WebRTC coordinator).
pub const HOST_HTML: &str = include_str!("../assets/web-runtime/actr-host.html");
