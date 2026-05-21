# Getting Started

This guide covers the current Actor-RTC Web browser path: Option U / wasm-bindgen guests loaded by `actor.sw.js`.

## Prerequisites

- Rust toolchain with `wasm32-unknown-unknown` for wasm-bindgen builds.
- `wasm-pack` for browser wasm packages.
- Node.js and `pnpm`.
- A modern browser with Service Worker, WebRTC, WebSocket, and IndexedDB support.

Install dependencies:

```bash
cd bindings/web
pnpm install
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
```

Some example packaging flows still build and sign `.actr` packages from `wasm32-wasip2` components before serving the wasm-bindgen browser guest. If you run those flows, also install the repository-level Component Model toolchain described in the root `AGENTS.md`.

## Runtime Artifacts

The current browser runtime expects:

- a signed `.actr` package
- a sibling `<package-stem>.wbg/` directory
- `<package-stem>.wbg/guest.js`
- `<package-stem>.wbg/guest_bg.wasm`
- `actor.sw.js`

`actor.sw.js` loads the wasm-bindgen guest bundle and calls `register_guest_workload`. jco-transpiled bundles are not part of the current browser runtime path.

## Run the Echo Example

```bash
cd bindings/web
pnpm install
cd examples/echo
bash start-mock.sh
```

The script builds the browser guest bundle, signs the `.actr` packages, starts the in-repo mock actrix service, serves the web client/server through `actr run --web`, and drives the browser smoke suite.

## Run the Browser-Hosted Peer Example

```bash
cd bindings/web
pnpm install
cd examples/data-stream-peer-concurrent
bash start.sh
```

This example includes browser-hosted service behavior and concurrent peer/client flows.

## Rebuild CLI-Embedded Web Runtime Assets

When `crates/sw-host` or `packages/web-sdk/src/actor.sw.js` changes, refresh the assets embedded by `actr-cli`:

```bash
cd bindings/web
bash crates/sw-host/build.sh
bash scripts/sync-cli-assets.sh --build
```

The sync script copies canonical web runtime assets into `cli/assets/web-runtime/`. Rebuild `actr-cli` after syncing if you need the CLI binary to embed the new bytes.

## React Integration

`@actr/web-react` currently re-exports the hooks defined in `bindings/web/packages/web-react/src/index.ts`:

- `useActorClient`
- `useServiceCall`
- `useSubscription`

Use those hooks directly or wrap them in application-specific context. Do not rely on a generic actor hook unless your application defines it locally.

## Next Steps

- Read [troubleshooting.md](./troubleshooting.md) for environment and runtime checks.
- Read [error-handling.md](./error-handling.md) for error categories and reporting guidance.
- Read [requirements.md](./requirements.md) for current goals and platform constraints.
- Use [docs/README.md](./README.md) to find historical architecture notes.
