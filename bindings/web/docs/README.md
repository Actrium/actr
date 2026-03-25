# Actor-RTC Web Documentation

**Actor-RTC Web** is the browser implementation of the Actor-RTC framework. It builds on WebAssembly and Service Worker primitives to provide an Actor-model programming experience that matches the native runtime closely.

---

## 📖 User Documentation

These documents target developers building applications with Actor-RTC Web.

### Getting Started

- **[Getting Started Guide](./getting-started.md)** ⭐ recommended first
  - Client mode for calling remote actors
  - Runtime mode for running an Actor runtime in the browser
  - React integration and end-to-end examples

### Troubleshooting

- **[Troubleshooting Guide](./troubleshooting.md)**
  - Common issues and fixes
  - Debugging techniques
  - Performance recommendations

---

## 📋 Requirements and Planning

Project requirements, goals, and planning documents:

- **[Web Requirements](./requirements.md)** - full functional requirements and architecture notes

---

## 🏗️ Architecture

These documents target framework contributors and readers who want to understand the internal design.

### Core Design

See **[architecture/](./architecture/)** for the full architecture set:

1. **[Architecture Overview](./architecture/overview.zh.md)** - dual-process model and core components
2. **[Dual-Layer Architecture](./architecture/dual-layer.zh.md)** - State Path vs Fast Path
3. **[API Layer Design](./architecture/api-layer.zh.md)** - Gate, Context, and ActrRef
4. **[Technical Decisions](./architecture/decisions.zh.md)** - nine key TDRs
5. **[Completion Status](./architecture/completion-status.zh.md)** - parity against native actr

---

## 🚀 Quick Preview

### Basic Usage

```typescript
import { createActor } from '@actr/web';

// Create an actor
const actor = await createActor({
  signalingUrl: 'wss://signal.example.com',
  realm: 'demo',
});

// Call a remote actor
const response = await actor.call('echo-service', 'sendEcho', {
  message: 'Hello, Actor-RTC!',
});
```

### Runtime Mode (Advanced)

Run the full Actor runtime inside the browser with a Service Worker plus DOM split-process architecture:

```rust
// Service Worker side
use actr_runtime_sw::*;

let manager = Arc::new(PeerTransport::new(...));
let mailbox = Arc::new(IndexedDbMailbox::new().await?);
let dispatcher = Arc::new(InboundPacketDispatcher::new(mailbox));
```

```rust
// DOM side
use actr_runtime_dom::*;

let registry = Arc::new(StreamHandlerRegistry::new());
let receiver = Arc::new(WebRtcDataChannelReceiver::new(registry));
```

---

## 📊 Current Status

| Area | Completion | Notes |
|------|------------|-------|
| Core architecture | 85% | Transport, Message, and full transport stack |
| Persistence and scheduling | 95% | Mailbox complete, Scheduler implemented |
| Fast Path support | 50% | Framework complete, integration still in progress |
| Overall | **78%** | Close to MVP |

See [Completion Status](./architecture/completion-status.zh.md) for details.

---

## 🔗 Related Resources

- **Examples**: `../examples/`
- **Crates**: `../crates/`
- **Native prototype**: `/d/actor-rtc/actr/`
- **GitHub**: https://github.com/actor-rtc/actor-rtc

---

**Maintainer**: Actor-RTC Team
**Last updated**: 2026-02-28
