# Actor-RTC Web Documentation

Actor-RTC Web currently uses the Option U / wasm-bindgen browser path. User workloads are loaded from a signed `.actr` package plus a sibling `.wbg/` bundle (`guest.js` and `guest_bg.wasm`). The browser Service Worker entry is `actor.sw.js`, which installs the guest dispatch function through `register_guest_workload`.

Component Model and jco documents are retained as historical design context only.

## Current User Documentation

- [Getting started](./getting-started.md): setup, current commands, example entry points, and React hooks.
- [Troubleshooting](./troubleshooting.md): build, Service Worker, `.wbg/`, signaling, WebRTC, and storage checks.
- [Error handling](./error-handling.md): how browser runtime errors should be categorized and reported.
- [Requirements](./requirements.md): current goals, non-goals, usage modes, and platform constraints.

## Current Examples

- [Echo example](../examples/echo/README.md): primary browser smoke path. Run with `bash examples/echo/start-mock.sh` from `bindings/web`.
- [Data-stream peer concurrent example](../examples/data-stream-peer-concurrent/README.zh.md): browser-hosted service and peer scenario. Run with `bash examples/data-stream-peer-concurrent/start.sh` from `bindings/web`.

## Current Build and Asset Commands

```bash
cd bindings/web
pnpm install
```

```bash
cd bindings/web
bash crates/sw-host/build.sh
bash scripts/sync-cli-assets.sh --build
```

## Architecture and Historical Notes

Use these when you need implementation background. They are not the canonical user setup path.

- [Architecture notes](./architecture/README.zh.md)
- [Option U WIT compile web notes](./option-u-wit-compile-web.zh.md)
- [2026-04 architecture change notes](./architecture-changes-2026-04.zh.md)
- [Historical jco async-lift investigation](./t18-jco-async-lift-hang.zh.md)
- [Tech debt notes](./tech-debt.zh.md)

## Source-of-Truth Pointers

- `packages/web-sdk/src/actor.sw.js`: Service Worker entry used by the current wasm-bindgen browser path.
- `crates/sw-host/src/guest_bridge.rs`: `register_guest_workload` and guest dispatch bridge.
- `crates/sw-host/build.sh`: builds the Service Worker host wasm-bindgen artifacts.
- `scripts/sync-cli-assets.sh`: syncs canonical web assets into `cli/assets/web-runtime/`.
- `packages/web-react/src/index.ts`: public React hook exports.
