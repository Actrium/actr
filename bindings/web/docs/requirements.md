# Actor-RTC Web Requirements

## Document Status

- Version: `v0.2.0-draft`
- Created: `2025-01-10`
- Last updated: `2026-05-21`
- Status: current user-facing requirements draft

## Overview

Actor-RTC Web adapts the Actrium runtime to browser environments. The current browser path is Option U / wasm-bindgen: a signed `.actr` package is served with a sibling `.wbg/` guest bundle, and `actor.sw.js` loads that guest through `register_guest_workload`.

Component Model and jco remain useful historical context, but they are not the current browser runtime consumption path.

## Goals

- Bring core Actor-RTC capabilities to web browsers through WebAssembly.
- Reuse existing Rust runtime logic where practical.
- Provide JavaScript and TypeScript-friendly SDK surfaces.
- Preserve the same actor mental model across native and browser runtimes.
- Support browser-hosted actor workloads where the current examples and runtime path allow it.

## Non-Goals

- Perfect parity with every native capability in the first browser release.
- Reintroducing periodic credential refresh loops or background retry loops that diverge from current repository policy.
- Treating historical Component Model / jco browser bridge documents as current user setup instructions.

## High-Level Architecture

The browser runtime is split into cooperating execution domains:

- DOM-side application and bridge code for browser APIs, UI adjacency, WebRTC coordination, and user-facing events.
- Service Worker-side runtime for shared worker execution, package loading, transport coordination, mailbox-backed processing, and guest dispatch.
- wasm-bindgen guest workload bundle loaded from `<package-stem>.wbg/guest.js` and `<package-stem>.wbg/guest_bg.wasm`.

## Primary Usage Modes

### Client Mode

The browser discovers and calls remote actors. This remains the simplest starting point for application developers.

### Browser-Hosted Workload Mode

The browser can host actor workload behavior through the current wasm-bindgen guest path. The data-stream peer concurrent example exercises browser-hosted service behavior.

### Runtime/Contributor Mode

Framework contributors may work directly with `crates/sw-host`, `packages/web-sdk/src/actor.sw.js`, `packages/actr-dom`, and the CLI asset sync path.

## Functional Requirements

- Remote actor discovery from the browser.
- Request-response RPC calls.
- Stream subscription support.
- Typed code generation for browser clients where generated bindings exist.
- WebRTC-based peer connectivity.
- Signaling and AIS integration through local or remote actrix-compatible services.
- IndexedDB-backed mailbox support where persistence is required.
- Separation between State Path and Fast Path processing.
- Service Worker based guest loading through `actor.sw.js`.
- wasm-bindgen browser guest dispatch through `register_guest_workload`.

## Developer Experience Requirements

- Workspace dependency installation through `pnpm install`.
- Current web runtime asset builds through `bash crates/sw-host/build.sh`.
- CLI asset refresh through `bash scripts/sync-cli-assets.sh --build`.
- Example smoke paths through `bash examples/echo/start-mock.sh` and `bash examples/data-stream-peer-concurrent/start.sh`.
- React integration through the currently exported hooks: `useActorClient`, `useServiceCall`, and `useSubscription`.
- Documentation that clearly separates current setup steps from historical architecture notes.

## Runtime and Platform Constraints

- Browser security policies apply, including Service Worker scope, origin, and HTTPS restrictions.
- IndexedDB behavior varies across browsers and browsing modes.
- WebRTC setup depends on signaling, ICE infrastructure, and browser policy.
- WASM size and startup time must remain practical for interactive apps.
- `.actr` packages and `.wbg/` sibling bundles must be served from paths that `actor.sw.js` can resolve consistently.

## Quality Targets

- High code reuse from the native Rust runtime where it fits the browser execution model.
- Predictable type safety across Rust and TypeScript boundaries.
- Practical runtime performance suitable for interactive real-time features.
- A debug path that makes Service Worker, guest loading, transport, and lifecycle failures diagnosable.

## Delivery Priorities

1. Keep the Option U / wasm-bindgen browser path stable and documented.
2. Keep echo and data-stream peer examples runnable through current scripts.
3. Keep CLI-embedded web assets synchronized with canonical web sources.
4. Continue improving generated TypeScript and React integration without documenting unsupported public APIs.

## Related Reading

- [getting-started.md](./getting-started.md)
- [troubleshooting.md](./troubleshooting.md)
- [error-handling.md](./error-handling.md)
- [docs/README.md](./README.md)
- [architecture/README.zh.md](./architecture/README.zh.md)
