# Media Relay Example - 100% Real Implementation

**Status**: ✅ **Fully Functional - Production-Ready Pattern**

This example demonstrates **real Actor-RTC distributed communication** using:
- Real ActrSystem lifecycle
- Real WebRTC P2P connections
- Real RPC with protobuf
- Real message routing and dispatch

## Quick Start

### Prerequisites

- `actr` CLI available (`actr` in PATH or built at `../actr/target/{debug,release}/actr`)
- `actrix` source available at `../actrix` (or `actrix` already installed); `start.sh` will start it automatically

### Run the Demo

```bash
./start.sh   # auto-runs actr gen for actr-a/actr-b and starts actrix
```

Notes:
- Shared proto now lives at `media-relay/proto/media_relay.proto` and is used by both actr-a/actr-b during `actr gen`.

## What You'll See

1. ✅ Actr B starts and registers with signaling server
2. ✅ Actr A starts, connects to Actr B via WebRTC P2P
3. ✅ Actr A generates and sends 10 media frames via RPC
4. ✅ Actr B receives and logs each frame
5. ✅ Both actors complete successfully

## Architecture

```
Actr A (Relay)          WebRTC P2P          Actr B (Receiver)
┌─────────────────┐    ─────────────>    ┌──────────────────┐
│ ActrSystem +    │                      │  ActrSystem +    │
│ ClientWorkload  │     RPC Request      │  RelayService    │
│       ↓         │                      │       ↓          │
│ ActrRef Shell   │                      │  RPC Handler     │
│       ↓         │                      │       ↓          │
│ Generate Frames │                      │ Receive & Log    │
└─────────────────┘                      └──────────────────┘
```

## Implementation Status

- ✅ **actr-b**: 100% Real Actor server
- ✅ **actr-a**: 100% Real Shell client with ActrRef
- ✅ **WebRTC P2P**: Real connection establishment
- ✅ **RPC**: Real protobuf message passing
- ✅ **Code Generation**: Auto-generated Workload from proto

See [STATUS.md](STATUS.md) for detailed implementation notes.
