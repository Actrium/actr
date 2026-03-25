# @actr/dom

**Actor-RTC DOM-side Fixed Forwarding Layer**

This package is the fixed JavaScript layer provided by the Actor-RTC framework. It acts as a hardware-abstraction-style bridge for DOM-side WebRTC management and data forwarding.

## Design Philosophy

> **DOM side = "network driver", Service Worker side = "application code"**

All user business logic lives in the Service Worker runtime, typically in WASM. The DOM side is a fixed framework-provided implementation that users are not expected to modify.

## Core Responsibilities

1. **WebRTC connection management**: create and manage `RTCPeerConnection` instances, which are only available in DOM contexts
2. **Fast Path data forwarding**: forward data received from WebRTC DataChannels to the Service Worker with minimal copying
3. **PostMessage bridge**: provide bidirectional communication between the DOM and the Service Worker

## Installation

```bash
npm install @actr/dom
```

## Usage

### Basic Usage

```typescript
import { initActrDom } from '@actr/dom';

// Initialize the DOM runtime
const runtime = await initActrDom({
  serviceWorkerUrl: '/my-actor.sw.js',  // Service Worker script path
  webrtcConfig: {
    iceServers: [{ urls: 'stun:stun.l.google.com:19302' }],
  },
});

console.log('Actor-RTC DOM runtime initialized');

// The runtime automatically:
// 1. Registers the Service Worker
// 2. Establishes PostMessage communication
// 3. Listens for WebRTC commands from the SW
// 4. Forwards Fast Path data back to the SW
```

### HTML Usage

```html
<!DOCTYPE html>
<html>
<head>
  <title>Actor-RTC App</title>
</head>
<body>
  <div id="app"></div>

  <!-- Load the DOM runtime -->
  <script type="module">
    import { initActrDom } from 'https://cdn.example.com/@actr/dom/dist/index.js';

    const runtime = await initActrDom({
      serviceWorkerUrl: '/worker.js',
    });

    // Your UI code...
  </script>
</body>
</html>
```

## API Reference

### `initActrDom(config)`

Initialize the Actor-RTC DOM runtime.

**Parameters**:
- `config.serviceWorkerUrl` (string): Service Worker script path
- `config.webrtcConfig` (object, optional): WebRTC configuration
  - `iceServers` (RTCIceServer[]): ICE server list
  - `iceTransportPolicy` (RTCIceTransportPolicy): ICE transport policy

**Returns**: `Promise<ActrDomRuntime>`

### `ActrDomRuntime`

DOM runtime instance.

**Methods**:
- `getSWBridge()`: return the Service Worker bridge
- `getForwarder()`: return the Fast Path forwarder
- `getCoordinator()`: return the WebRTC coordinator
- `dispose()`: release all resources

## Architecture

See: [WASM-DOM Integration Architecture](../../docs/architecture/wasm-dom-integration.zh.md)

### Data Flow

```
WebRTC data arrives in the DOM
  ↓
WebRtcCoordinator receives it
  ↓
FastPathForwarder forwards it with minimal copying using Transferable ArrayBuffer
  ↓
PostMessage → Service Worker WASM
  ↓
Fast Path Registry.dispatch()
  ↓
User callback in Rust
```

### Performance Characteristics

- **Zero-copy style transfer**: uses Transferable ArrayBuffer
- **Batch forwarding**: configurable batching reduces PostMessage overhead
- **Target latency**: about `6-13ms` versus `30-40ms` for the State Path

## Components

### ServiceWorkerBridge

Handles PostMessage communication between the DOM and the Service Worker.

### FastPathForwarder

Forwards WebRTC DataChannel payloads to the Service Worker.

Supports two modes:
- `forward()`: immediately forward one payload
- `forwardBatch()`: batch forwarding for high-throughput scenarios

### WebRtcCoordinator

Manages WebRTC connections and DataChannels.

**Core functions**:
- create `RTCPeerConnection`
- create four negotiated DataChannels, one per payload type
- handle SDP offer/answer exchange
- handle ICE candidates
- automatically forward received data

## Development

```bash
# Install dependencies
npm install

# Build
npm run build

# Watch mode
npm run watch

# Clean
npm run clean
```

## Related Documents

- [Architecture Overview](../../docs/architecture/overview.zh.md)
- [WASM-DOM Integration Architecture](../../docs/architecture/wasm-dom-integration.zh.md) (core)
- [Dual-Layer Architecture](../../docs/architecture/dual-layer.zh.md)

## License

Apache-2.0

---

**Maintainer**: Actor-RTC Team
**Version**: 0.1.0
**Last updated**: 2025-11-11
