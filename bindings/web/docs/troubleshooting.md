# Troubleshooting

This document covers the most common problems when working with Actor-RTC Web.

## Build Failures

### WASM compilation fails

Check that the WASM target and toolchain are installed:

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
rustc --version
```

Then rebuild from a clean state:

```bash
cargo clean
npm run build:wasm
```

### Vite cannot load WASM

Install and configure the required plugins:

```bash
npm install --save-dev vite-plugin-wasm vite-plugin-top-level-await
```

Make sure `vite.config.ts` includes both plugins and the required COOP/COEP headers when SharedArrayBuffer-style behavior is needed.

### CORS or isolation errors

Development servers must emit the right headers:

```ts
export default defineConfig({
  server: {
    headers: {
      'Cross-Origin-Opener-Policy': 'same-origin',
      'Cross-Origin-Embedder-Policy': 'require-corp',
    },
  },
});
```

## Runtime Failures

### Cannot connect to the signaling server

Check:

- The signaling URL is correct.
- The server is reachable from the browser.
- Firewalls or proxies are not blocking WebSocket traffic.

Enable debug logging if the SDK supports it in the current example or app setup.

### IndexedDB errors

Common causes:

- Private browsing mode disables or restricts IndexedDB.
- The browser storage quota is exhausted.
- The browser environment does not support IndexedDB.

Basic guard:

```ts
if (!window.indexedDB) {
  console.error('IndexedDB is not supported');
}
```

### WebRTC connection setup fails

Check:

- STUN and TURN configuration
- NAT and firewall restrictions
- Whether the signaling flow completed successfully

When debugging, verify signaling first, then ICE gathering, then data-channel establishment.

## Debugging Strategy

1. Confirm the build output exists and loads.
2. Confirm the signaling endpoint is reachable.
3. Confirm browser storage and worker registration succeed.
4. Confirm WebRTC data channels or media channels transition to a connected state.

## When in Doubt

- Reduce the example to a single client and a single service.
- Prefer the simplest echo scenario before testing fast-path or runtime-mode features.
- Keep browser devtools open and compare DOM-side logs with Service Worker logs.
