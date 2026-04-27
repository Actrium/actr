//! Embedded web runtime assets for `actr run --web`.
//!
//! These files are compiled into the `actr` binary so that `actr run --web`
//! can serve a fully self-contained web actor host without requiring any
//! external runtime WASM files, JS glue, or HTML pages.
//!
//! Assets:
//! - `actr_sw_host_bg.wasm` — shared SW host WASM (wasm-pack from sw-host)
//! - `actr_sw_host.js`      — wasm-bindgen JS glue for the SW host
//! - `actor.sw.js`          — Service Worker entry point (Option U /
//!   wasm-bindgen guest path; sole browser path after Phase 8)
//! - `actr-host.html`       — self-contained host page with inline @actr/dom

/// Shared SW host WASM binary (compiled from actr-sw-host via wasm-pack).
pub const RUNTIME_WASM: &[u8] = include_bytes!("../assets/web-runtime/actr_sw_host_bg.wasm");

/// wasm-bindgen JS glue for the shared SW host.
pub const RUNTIME_JS: &str = include_str!("../assets/web-runtime/actr_sw_host.js");

/// Service Worker entry point — wasm-bindgen guest bridge (Option U).
pub const ACTOR_SW_JS: &str = include_str!("../assets/web-runtime/actor.sw.js");

/// Self-contained HTML host page with inline @actr/dom (WebRTC coordinator).
pub const HOST_HTML: &str = include_str!("../assets/web-runtime/actr-host.html");
