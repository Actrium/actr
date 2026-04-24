//! Embedded web runtime assets for `actr run --web`.
//!
//! These files are compiled into the `actr` binary so that `actr run --web`
//! can serve a fully self-contained web actor host without requiring any
//! external runtime WASM files, JS glue, or HTML pages.
//!
//! Assets:
//! - `actr_sw_host_bg.wasm` — shared SW host WASM (wasm-pack from sw-host)
//! - `actr_sw_host.js`      — wasm-bindgen JS glue for the SW host
//! - `actor.sw.js`          — Service Worker entry point (Component Model
//!                            + jco path; default)
//! - `actor-wbg.sw.js`      — Service Worker entry point (Option U, wasm-
//!                            bindgen guest path; selected when
//!                            `ACTR_WEB_GUEST_MODE=wbg`)
//! - `actr-host.html`       — self-contained host page with inline @actr/dom

/// Shared SW host WASM binary (compiled from actr-sw-host via wasm-pack).
pub const RUNTIME_WASM: &[u8] = include_bytes!("../assets/web-runtime/actr_sw_host_bg.wasm");

/// wasm-bindgen JS glue for the shared SW host.
pub const RUNTIME_JS: &str = include_str!("../assets/web-runtime/actr_sw_host.js");

/// Generic Service Worker entry point (actor.sw.js) — Component Model + jco
/// bridge. Default.
pub const ACTOR_SW_JS: &str = include_str!("../assets/web-runtime/actor.sw.js");

/// Service Worker entry point for the wasm-bindgen guest path (Option U).
/// Served when `ACTR_WEB_GUEST_MODE=wbg`. Runs alongside `actor.sw.js` so
/// the Component Model pipeline stays intact for the examples that still
/// use it.
pub const ACTOR_WBG_SW_JS: &str =
    include_str!("../assets/web-runtime/actor-wbg.sw.js");

/// Self-contained HTML host page with inline @actr/dom (WebRTC coordinator).
pub const HOST_HTML: &str = include_str!("../assets/web-runtime/actr-host.html");
