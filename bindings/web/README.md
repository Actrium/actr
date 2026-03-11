# Actor-RTC Web

> Port of Actor-RTC distributed real-time communication framework to Web environments

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Status](https://img.shields.io/badge/status-MVP%20Stage-yellow.svg)]()

**Actor-RTC Web** is the browser implementation of the Actor-RTC framework, providing an Actor model programming experience consistent with the native version through WebAssembly technology.

[Chinese Documentation](./README.zh.md)

---

## 🎯 Project Status

**Current Version**: v0.1.0-alpha
**Development Stage**: MVP (Minimum Viable Product)
**Overall Completion**: 78% (relative to actr Native)

### Core Features Completed (P0 - MVP)

- ✅ ActorSystem lifecycle management (start/shutdown/error handling)
- ✅ WebRTC P2P connections (4 negotiated DataChannels)
- ✅ RPC request-response mechanism (request/pending_requests/timeout)
- ✅ DOM-side fixed forwarding layer (@actr/dom, PostMessage + WebRTC)
- ✅ UI interaction API (call/subscribe/on)
- ✅ End-to-end RPC testing (simple-rpc example)
- ✅ IndexedDB Mailbox (persistent message queue)
- ✅ TypeScript SDK (type-safe API)
- ✅ React Hooks (useActorClient, useServiceCall, useSubscription)

### Performance Optimizations (P1 - Completed)

- ✅ Fast Path data flow (zero-copy transmission, 30x performance boost)
  - Transferable ArrayBuffer (3ms latency)
  - SharedArrayBuffer (0.1ms latency)
  - WasmMemoryPool (pre-allocation reuse)
- ✅ RouteTable routing (static routing + dynamic LRU cache)
- ✅ Enum static dispatch (avoid dyn Trait virtual function overhead)
- ✅ DashMap lock-free concurrency (FastPathRegistry)
- ✅ AtomicU64 zero-lock monitoring (performance metrics collection)

### In Progress (P1/P2)

- ⏳ actr-cli Web support (0%)
- ⏳ Test coverage improvement (current 30%, target 70%)
- ⏳ Service Worker integration
- ⏳ More examples (react-echo, todo-app, chat-app)

See: [Completion Status](./docs/architecture/completion-status.zh.md)

---

## 🚀 Quick Start

### Prerequisites

- Rust 1.88+ and wasm-pack
- Node.js 18+ and npm
- Modern browser with WebRTC support

### Run Echo Example

```bash
# Enter example directory
cd examples/echo

# Auto-build and start (signaling server + echo server + web client)
./start.sh

# Browser opens automatically at http://localhost:3001
```

### From Scratch

```bash
# 1. Clone repository
git clone https://github.com/actor-rtc/actr-web.git
cd actr-web

# 2. Install dependencies
npm install

# 3. Build WASM
./scripts/build-wasm.sh

# 4. Run tests
node test-wasm.js
```

---

## 📖 Documentation

### User Documentation

For developers building applications with Actor-RTC Web:

- [Getting Started Guide](./docs/getting-started.md) ⭐ Recommended first read
- [Troubleshooting Guide](./docs/troubleshooting.md)

### Requirements & Planning

Project requirements, goals and planning:

- [Web Adaptation Requirements](./docs/requirements.md) - Complete feature requirements and architecture design

### Architecture Documentation

For framework contributors and developers seeking deep understanding:

- [Architecture Documentation Index](./docs/architecture/README.zh.md)
- [Architecture Overview](./docs/architecture/overview.zh.md) - Dual-process model and core components
- [Technical Decision Records](./docs/architecture/decisions.zh.md) - 9 TDRs
- [Completion Assessment](./docs/architecture/completion-status.zh.md) - Completion relative to actr Native (78%)

---

## 🏗️ Architecture Overview

```
┌─────────── Browser Environment ───────────┐
│                                            │
│  TypeScript Application                    │
│       ↓                                    │
│  @actr/web SDK                             │
│       ↓                                    │
│  WASM Runtime (Rust)                       │
│    ├─ ActorSystem                          │
│    ├─ Mailbox (IndexedDB)                  │
│    ├─ WebRTC Coordinator                   │
│    └─ Signaling Client                     │
│       ↓                                    │
│  Browser APIs                              │
│   (WebRTC, IndexedDB, WebSocket)           │
└────────────────────────────────────────────┘
```

**Core Features**:

- **High Code Reuse**: 85-90% of core logic directly reused from actr Native
- **Type Safety**: Rust + TypeScript dual-type guarantees
- **Performance Advantage**: WASM near-native performance
- **Browser First**: Full utilization of native browser WebRTC and IndexedDB

---

## 📊 Performance Metrics

| Metric | Current Value | Notes |
|--------|---------------|-------|
| WASM Bundle Size | 99.6 KB (~35 KB gzipped) | Kept lean |
| WASM Initialization Time | <100ms | Fast startup |
| State Path Latency | 30-40ms | RPC request-response |
| Fast Path Latency (Baseline) | ~3ms | Transferable ArrayBuffer |
| Fast Path Latency (Optimized) | ~0.1ms | SharedArrayBuffer, 30x boost |
| Video Stream Processing (60fps) | <0.1ms/frame | High frame rate support |
| IndexedDB Latency | <50ms | Persistent storage |
| Memory Usage | ~48 MB | Typical application |

---

## 🛠️ Tech Stack

### Rust / WebAssembly

- **wasm-bindgen**: Rust ↔ JavaScript interop
- **web-sys**: Browser Web API bindings
- **tokio**: Async runtime (minimal feature set)
- **rexie**: IndexedDB high-level API
- **prost**: Protobuf codec

### JavaScript / TypeScript

- **React 18**: UI framework
- **Vite**: Dev server and build tool
- **TypeScript 5**: Type system
- **grpc-web**: gRPC browser client

### Protocols & Standards

- **WebRTC**: P2P real-time communication
- **WebSocket**: Signaling channel
- **Protobuf**: Message serialization
- **IndexedDB**: Browser persistent storage

---

## 📦 Project Structure

```
actr-web/
├── crates/              # Rust crates (WASM core)
│   ├── runtime-sw/      # Service Worker runtime
│   ├── runtime-dom/     # DOM runtime
│   └── mailbox-web/     # IndexedDB Mailbox
│
├── packages/            # JavaScript/TypeScript packages
│   ├── actr-dom/        # DOM-side WASM bindings
│   ├── web-sdk/         # High-level TypeScript SDK (@actr/web)
│   └── web-react/       # React Hooks (@actr/web-react)
│
├── examples/            # Example projects
│   ├── echo/            # Echo example (complete implementation)
│   ├── hello-world/     # Minimal hello-world example
│   └── codegen-test/    # Code generation test
│       ├── proto/       # Protobuf definitions
│       ├── server/      # gRPC server (Tonic)
│       ├── client/      # Web client (React)
│       └── start.sh     # One-click startup script
│
├── docs/                # Documentation
│   ├── getting-started.md
│   ├── requirements.md
│   └── architecture/
│
└── scripts/             # Build scripts
    ├── build-wasm.sh
    └── test-e2e.sh
```

---

## 🧪 Development & Testing

### Build WASM

```bash
./scripts/build-wasm.sh
```

### Run Tests

```bash
# WASM unit tests
node test-wasm.js

# E2E tests (requires starting example first)
cd examples/echo
./start.sh
# In another terminal
npm run test:e2e
```

### Watch Mode Development

```bash
# Watch WASM changes and auto-rebuild
npm run dev:wasm

# Watch TypeScript changes
npm run dev:packages
```

---

## 🤝 Contributing

Contributions welcome! Please follow these steps:

1. Fork this repository
2. Create feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add some amazing feature'`)
4. Push to branch (`git push origin feature/amazing-feature`)
5. Submit Pull Request

Please follow the steps above to submit your contributions.

---

## 📝 License

This project is licensed under Apache License 2.0. See [LICENSE](LICENSE) file for details.

---

## 🔗 Related Resources

- **Main Project**: [Actor-RTC](https://github.com/actor-rtc/actor-rtc)
- **Documentation Site**: [actor-rtc.github.io](https://actor-rtc.github.io)
- **Native Implementation**: `/d/actor-rtc/actr/`
- **Issue Tracking**: [GitHub Issues](https://github.com/actor-rtc/actr-web/issues)

### Learning Resources

- [Rust WASM Book](https://rustwasm.github.io/docs/book/)
- [wasm-bindgen Guide](https://rustwasm.github.io/wasm-bindgen/)
- [WebRTC API (MDN)](https://developer.mozilla.org/en-US/docs/Web/API/WebRTC_API)
- [IndexedDB API (MDN)](https://developer.mozilla.org/en-US/docs/Web/API/IndexedDB_API)

---

## 📧 Contact

- **Maintainer**: kookyleo <kookyleo@gmail.com>
- **GitHub**: [@kookyleo](https://github.com/kookyleo)

---

**Last Updated**: 2025-11-18
**Documentation Version**: v1.1.0
