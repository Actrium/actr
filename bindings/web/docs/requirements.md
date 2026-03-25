# Actor-RTC Web Requirements

## Document Status

- Version: `v0.1.0-draft`
- Created: `2025-01-10`
- Last updated: `2025-01-10`
- Status: draft requirements

## Overview

Actor-RTC Web adapts the Actor-RTC runtime to browser environments. The goal is to reuse as much of the Rust runtime as practical while exposing a JavaScript and TypeScript-friendly surface for browser applications.

## Goals

- Bring core Actor-RTC capabilities to web browsers through WebAssembly.
- Reuse the existing Rust implementation where possible.
- Provide a type-safe API for JavaScript and TypeScript consumers.
- Preserve the same mental model across native and browser runtimes.

## Non-Goals

- Perfect parity with every native capability in the first release
- Full browser-side service hosting on day one
- Background refresh or retry loops that diverge from the native design principles

## High-Level Architecture

The browser runtime is split into two cooperating execution domains:

- DOM-side runtime for browser API interaction, UI adjacency, and user-facing coordination
- Service Worker-side runtime for transport management, mailbox persistence, and serialized actor execution

Shared logic is compiled to WASM and wrapped by a thin JavaScript bridge layer.

## Primary Usage Modes

### Client Mode

The browser acts as a client that discovers and calls remote actors. This is the main target for early delivery and should cover most product use cases.

### Runtime Mode

The browser hosts more of the actor runtime locally, including mailbox-backed inbound processing and richer transport coordination. This mode is more advanced and should evolve incrementally.

## Functional Requirements

- Remote actor discovery from the browser
- Request-response RPC calls
- Stream subscription support
- Typed code generation for browser clients
- WebRTC-based peer connectivity
- Signaling integration over WebSocket
- IndexedDB-backed mailbox support where persistence is required
- Separation between State Path and Fast Path processing

## Developer Experience Requirements

- A straightforward SDK entry point such as `createActor`
- Generated TypeScript types and ActorRef wrappers from proto definitions
- Vite-friendly setup for WASM-based projects
- React integration support where relevant
- Documentation for client mode, runtime mode, and debugging

## Runtime and Platform Constraints

- Browser security policies apply, including worker and cross-origin restrictions
- IndexedDB behavior varies across browsers and browsing modes
- WebRTC setup depends on signaling and ICE infrastructure
- WASM size and startup time must remain practical for interactive apps

## Quality Targets

- High code reuse from the native Rust runtime
- Predictable type safety across Rust and TypeScript boundaries
- Practical runtime performance suitable for interactive real-time features
- A debug path that makes transport and lifecycle failures diagnosable

## Delivery Priorities

1. Stable client mode with generated type-safe APIs
2. Reliable signaling and WebRTC transport integration
3. Browser storage and mailbox support where required
4. Runtime mode hardening and deeper native parity

## Related Reading

- [getting-started.md](./getting-started.md)
- [troubleshooting.md](./troubleshooting.md)
- [architecture/overview.zh.md](./architecture/overview.zh.md)
