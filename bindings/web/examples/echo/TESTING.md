# Echo Example — Automated Test Guide

## Overview

`test-auto.js` is a comprehensive Puppeteer-based test suite (26 suites, 80+ tests) covering:
- **A-Category**: Basic RPC, page refresh, SW lifecycle, WebRTC, multi-client, concurrency
- **B-Category**: CDP-enhanced tests (hard refresh, network emulation, WASM loading)
- **C-Category**: Process orchestration (actrix restart, Rust server lifecycle)
- **X-Category**: Cross-platform (Web Client ↔ Rust Server) integration

## Prerequisites

1. **Node.js** 18+ with `puppeteer`:
   ```bash
   mkdir -p /tmp/echo-test && cd /tmp/echo-test && npm init -y && npm i puppeteer
   ```
2. **Actrix signaling server** running on port 8081
3. **Echo server** Vite dev server (default: `http://localhost:5174`)
4. **Echo client** Vite dev server (default: `https://localhost:5173`)

## Quick Start

```bash
# Run all tests (services must already be running)
cd examples/echo
NODE_PATH=/tmp/echo-test/node_modules \
  CLIENT_URL=https://localhost:5173 \
  SERVER_URL=http://localhost:5174 \
  node test-auto.js
```

## Selective Execution

Run specific suites by name (case-insensitive, partial match supported):

```bash
# Single suite
node test-auto.js MultiTab

# Multiple suites
node test-auto.js MultiTab Concurrency Webrtc

# By category
node test-auto.js A    # All A-category (fast) suites
node test-auto.js B    # All B-category (CDP) suites
node test-auto.js C    # All C-category (orchestration) suites
node test-auto.js X    # All cross-platform suites
```

### Available Suites

| Cat | Suite Name | Tests | Description |
|-----|-----------|-------|-------------|
| A | `BasicFunction` | 1-1 ~ 1-6 | manual, empty, rapid, large messages, special characters, Enter key |
| A | `PageRefresh` | 2-1 ~ 2-4 | recovery after client, server, or dual refresh |
| A | `SwLifecycle` | 3-1, 3-4 | idle SW termination, keep-alive, SW update |
| A | `Webrtc` | 5-1, 5-4 | four DataChannel lanes, peer state change logs |
| A | `MultiTab` | 6-1 ~ 6-6 | multiple clients, concurrent sends, closing or refreshing one client |
| A | `PageClose` | 7-1 ~ 7-5 | page close and `beforeunload` behavior |
| A | `IdleRecovery` | 9-1 ~ 9-3 | idle recovery |
| A | `BrowserCompat` | 10-1, 10-4, 10-5 | Chrome, Edge, private browsing |
| A | `Concurrency` | 12-1 ~ 12-4 | 100 sequential sends, 5-client concurrency, log overflow, memory leak checks |
| A | `ErrorRecovery` | 13-x | error recovery and degradation |
| A | `SignalingConfig` | 14-x | signaling configuration edge cases |
| B | `CdpHardRefresh` | 15-x | hard refresh via CDP |
| B | `CdpSwControl` | 16-x | Service Worker control via CDP |
| B | `CdpNetwork` | 17-x | network simulation via CDP |
| B | `CdpWasmLoading` | 18-x | WASM loading via CDP |
| B | `CdpSignalingRecovery` | 19-x | signaling reconnect via CDP |
| B | `CdpIdleRecovery` | 20-x | idle recovery via CDP |
| C | `CActrixRestart` | C1-x | Actrix server lifecycle |
| C | `CSignalingEdgeCases` | C2-x | signaling edge cases |
| C | `CRustServerLifecycle` | C3-x | Rust server lifecycle |
| X | `CrossplatformEnv` | X-0-x | cross-platform environment checks |
| X | `CrossplatformBasic` | X-1-x | cross-platform basic functionality |
| X | `CrossplatformWebrtc` | X-2-x | cross-platform WebRTC |
| X | `CrossplatformClientLifecycle` | X-3-x | cross-platform client lifecycle |
| X | `CrossplatformNetwork` | X-4-x | cross-platform networking |
| X | `CrossplatformProtocol` | X-5-x | cross-platform protocol coverage |

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `CLIENT_URL` | `https://localhost:5173` | Echo client URL |
| `SERVER_URL` | `http://localhost:5174` | Echo server URL |
| `SLOW` | `0` | Set `1` to enable slow tests (idle, stress, memory leak) |
| `RUN_C` | `0` | Set `1` to enable C-category orchestration tests |
| `NODE_PATH` | — | Path to puppeteer install (e.g., `/tmp/echo-test/node_modules`) |

## Using test.sh

```bash
# Run all tests
./test.sh

# Run specific suites
./test.sh MultiTab Concurrency

# With options
SLOW=1 RUN_C=1 ./test.sh Webrtc
```

## Key Test Helpers

| Helper | Purpose |
|--------|---------|
| `waitForEchoWorking(page, timeout)` | Active retry: tries auto-echo first, then manual echo sends |
| `waitForClientLog(page, pattern, timeout)` | Passive wait for a log pattern to appear |
| `sendEchoMessage(page, msg, timeout)` | Type message + click Send, wait for RPC completion |
| `openClientReady(browser)` | Open client page, wait for ✅ status |
| `openServerReady(browser)` | Open server page, wait for ✅ status |

## Troubleshooting

### Tests timeout at 60s
- Check services: `lsof -iTCP -sTCP:LISTEN -nP | grep -E '5173|5174|8081'`
- Possible stale SW state; hard-refresh client in real browser or restart Vite dev servers
- View Actrix logs: `tail -f ../../../../actrix/logs/actrix.log`

### Multi-client notes
- Multi-client routing works correctly. Each client gets its own `SwRuntime` with
  an independent `dom_port` (MessagePort). RPC responses are routed per-client via
  the `CLIENTS` thread_local map in `client_runtime.rs`.
- Tests 6-2 and 12-2 verify multi-client echo (2 clients and 5 clients respectively).
- `sendEchoMessage` uses explicit `page.evaluate` polling instead of `waitForFunction`
  to reliably detect RPC completion under CDP load with multiple pages.

### Server reconnection tests skipped (7-5, 13-1)
- These tests require the client to reconnect after the server goes away and comes back.
- Requires WASM rebuild with ICE restart / reconnection fixes from `client_runtime.rs`.
- Changes exist in the codebase but WASM has not been rebuilt.

### Simultaneous refresh test skipped (2-4)
- Both client and server refreshing at the same time is inherently race-condition-prone.
- Signaling re-registration timing makes this unreliable.

### WebRTC connection slow
- Auto-echo in client retries up to 8 times (3s apart ~ 29s window)
- `waitForEchoWorking()` adds another manual retry layer (up to 60s total)
- If both fail, check ICE/TURN configuration in actrix config
