# Actor-RTC Web

Actor-RTC Web is the browser runtime and SDK surface for Actrium. The current browser path is the Option U / wasm-bindgen guest path:

- browser workloads are loaded through a signed `.actr` package plus a sibling `<package-stem>.wbg/` directory
- the sibling directory contains `guest.js` and `guest_bg.wasm`
- the Service Worker entry is `actor.sw.js`
- the Service Worker loads the wasm-bindgen guest bundle and calls `register_guest_workload`

Component Model and jco material are historical context for the old browser bridge. The current browser runtime does not consume jco-transpiled bundles.

[中文说明](./README.zh.md)

## Quick Start

Install the web workspace dependencies:

```bash
cd bindings/web
pnpm install
```

Run the browser echo example against the in-repo mock actrix service:

```bash
cd bindings/web/examples/echo
bash start-mock.sh
```

Run the browser-hosted data-stream peer example:

```bash
cd bindings/web/examples/data-stream-peer-concurrent
bash start.sh
```

Rebuild and sync the web runtime assets embedded by `actr-cli`:

```bash
cd bindings/web
bash crates/sw-host/build.sh
bash scripts/sync-cli-assets.sh --build
```

`crates/sw-host/build.sh` builds the Service Worker host wasm-bindgen output. `scripts/sync-cli-assets.sh --build` rebuilds and copies the canonical web assets into `cli/assets/web-runtime/` so `actr run --web` serves the current runtime.

## Current Runtime Shape

```
Browser tab
  DOM application
  @actrium/actr-web SDK
  @actrium/actr-dom bridge
      |
      | MessagePort / postMessage
      v
Shared Service Worker: actor.sw.js
  sw-host wasm-bindgen runtime
  register_guest_workload(dispatchFn)
      |
      v
<package-stem>.wbg/
  guest.js
  guest_bg.wasm
```

Each browser tab owns its own DOM-side client identity. The Service Worker is shared by the origin and keeps per-client runtime state.

## Packages

- `packages/actr-dom`: `@actrium/actr-dom`, the DOM-side bridge for Service Worker, WebRTC, and browser APIs.
- `packages/web-sdk`: `@actrium/actr-web`, the browser SDK and `actor.sw.js` source.
- `packages/web-react`: `@actrium/actr-web-react`, React hooks. The public exports are `useActorClient`, `useServiceCall`, and `useSubscription`.
- `crates/sw-host`: Service Worker runtime compiled with wasm-bindgen.
- `crates/dom-bridge`: Rust-side DOM bridge support.
- `crates/mailbox-web`: IndexedDB-backed mailbox support.

## Publishing

The web npm packages are published through the `Publish Web Packages` GitHub
Actions workflow. The local equivalent is:

```bash
cd bindings/web
pnpm install --frozen-lockfile
bash scripts/publish.sh --dry-run --expected-version 0.1.0
```

The script publishes in dependency order: `@actrium/actr-dom`,
`@actrium/actr-web`, then `@actrium/actr-web-react`.

## Documentation

Start with the current user-facing docs:

- [Documentation index](./docs/README.md)
- [Getting started](./docs/getting-started.md)
- [Troubleshooting](./docs/troubleshooting.md)
- [Error handling](./docs/error-handling.md)
- [Requirements](./docs/requirements.md)

Historical deep dives are useful for understanding how the current path was reached, but they should not be read as current setup instructions:

- [Option U WIT compile web notes](./docs/option-u-wit-compile-web.zh.md)
- [2026-04 architecture change notes](./docs/architecture-changes-2026-04.zh.md)
- [Historical jco async-lift investigation](./docs/t18-jco-async-lift-hang.zh.md)
- [Architecture notes](./docs/architecture/README.zh.md)

## Development Notes

- Use `pnpm install` in `bindings/web` for workspace package dependencies.
- Use `bash crates/sw-host/build.sh` when editing `crates/sw-host`.
- Use `bash scripts/sync-cli-assets.sh --build` when the CLI-embedded web runtime assets must be refreshed.
- Use `bash examples/echo/start-mock.sh` for the primary echo smoke path.
- Use `bash examples/data-stream-peer-concurrent/start.sh` for browser-hosted peer/service checks.

## License

This project is licensed under Apache License 2.0. See [LICENSE](LICENSE) for details.
