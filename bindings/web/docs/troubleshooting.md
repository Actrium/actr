# Troubleshooting

This document covers common issues in the current Actor-RTC Web Option U / wasm-bindgen path.

## Build Failures

### Workspace dependencies are missing

Install from the web workspace root:

```bash
cd bindings/web
pnpm install
```

### Service Worker host build fails

Check the browser wasm toolchain:

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
```

Then rebuild:

```bash
cd bindings/web
bash crates/sw-host/build.sh
```

If you need the CLI-embedded runtime assets, run:

```bash
cd bindings/web
bash scripts/sync-cli-assets.sh --build
```

### `.wbg` guest bundle is missing

The current Service Worker resolves the browser guest from a sibling `.wbg/` directory. For a package named `acme-demo-0.1.0-wasm32-wasip2.actr`, the browser path should include:

```text
acme-demo-0.1.0-wasm32-wasip2.actr
acme-demo-0.1.0-wasm32-wasip2.wbg/guest.js
acme-demo-0.1.0-wasm32-wasip2.wbg/guest_bg.wasm
```

The echo script lays this out automatically:

```bash
cd bindings/web/examples/echo
bash start-mock.sh
```

### CLI serves stale web runtime assets

Refresh the CLI asset copy:

```bash
cd bindings/web
bash scripts/sync-cli-assets.sh --build
```

Then rebuild the CLI binary that embeds `cli/assets/web-runtime/`.

## Runtime Failures

### Service Worker does not register

Check:

- the page is served from `http://127.0.0.1`, `http://localhost`, or HTTPS
- `actor.sw.js` is reachable at the configured Service Worker path
- the browser devtools Application panel does not show an old waiting worker
- the page and Service Worker are under the same origin and compatible scope

### Guest registration fails

The current path should call `register_guest_workload` from `actor.sw.js`. If registration fails, confirm:

- `guest.js` loads without a JavaScript exception
- `guest_bg.wasm` is reachable next to `guest.js`
- the `.actr` package URL and `.wbg/` sibling URL use the same package stem
- browser devtools show Service Worker logs, not only DOM page logs

### Cannot connect to signaling

For local example runs, prefer the mock actrix scripts:

```bash
cd bindings/web/examples/echo
bash start-mock.sh
```

```bash
cd bindings/web/examples/data-stream-peer-concurrent
bash start.sh
```

If connecting manually, verify:

- the WebSocket URL is correct
- the HTTP health endpoint is reachable
- the realm and package identity are registered
- local firewalls or proxies are not blocking WebSocket traffic

### IndexedDB errors

Common causes:

- private browsing mode disables or restricts IndexedDB
- browser storage quota is exhausted
- the page origin changed and the expected database is under a different origin

Basic guard:

```ts
if (!window.indexedDB) {
  console.error('IndexedDB is not supported');
}
```

### WebRTC setup fails

Debug in this order:

1. Signaling WebSocket connects.
2. Offer/answer exchange completes.
3. ICE candidates are exchanged.
4. Data channels open.
5. State-path RPC works before fast-path flows are tested.

## Debugging Strategy

1. Start with `examples/echo/start-mock.sh`.
2. Confirm `actor.sw.js`, `guest.js`, and `guest_bg.wasm` return HTTP 200.
3. Compare DOM-side console logs with Service Worker logs.
4. Clear old Service Worker registrations and reload if the browser keeps stale assets.
5. Move to `examples/data-stream-peer-concurrent/start.sh` only after the echo path works.
