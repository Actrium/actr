# Actor-RTC Framework Demo

![Rust](https://img.shields.io/badge/rust-1.88+-orange.svg)
![Node.js](https://img.shields.io/badge/node.js-16+-green.svg)
![WebRTC](https://img.shields.io/badge/webrtc-enabled-blue.svg)
![License](https://img.shields.io/badge/license-MIT-green.svg)

A distributed real-time communication framework demo built on WebRTC and the Actor model.

## 📖 Overview

This project demonstrates a distributed system architecture that combines the classic Actor model with modern WebRTC transport. Each process is treated as a coarse-grained actor, communicates peer-to-peer over WebRTC, and uses a dual-path processing model to optimize different kinds of traffic.

### 🎯 Key Features

- **Coarse-grained Actor model**: Process-level actor abstraction for simpler distributed design
- **Native WebRTC support**: Built-in NAT traversal and peer-to-peer connectivity
- **Dual-path processing**:
  - **State Path**: Reliable ordered control-message handling
  - **Fast Path**: Low-latency streaming data handling
- **Type safety**: Contract-driven development based on Protobuf
- **ACL-aware discovery**: Secure discovery with access control list support

---

## 📚 Documentation

The repository includes a complete set of design and development documents. A practical reading order is:

#### **Part 1: Concepts & Architecture**
*Start here to build a shared mental model of the framework.*

1.  **[Ecosystem Overview](./docs/0-Ecosystem-Overview.zh.md)** (recommended first)
2.  **[Concepts and Architecture](./docs/1-Concepts-and-Architecture.zh.md)**
3.  **[ActorSystem and Actor](./docs/1.1-ActorSystem-and-Actor.zh.md)**
4.  **[Interacting with the Outside World](./docs/1.3-Interacting-with-the-Outside-World.zh.md)**
5.  **[Inter-Actor Communication Patterns](./docs/1.4-Inter-Actor-Communication-Patterns.zh.md)**
6.  **[Framework Internal Protocols](./docs/1.2-Framework-Internal-Protocols-zh.md)**

#### **Part 2: Guides & Practices**
*Hands-on material for building with the framework.*

1.  **[Developer Guide](./docs/2-Developer-Guide.zh.md)** (quick start)
2.  **[Project Manifest and CLI](./docs/2.4-Project-Manifest-and-CLI.zh.md)** (CLI reference)
3.  **[Actor Cookbook](./docs/2.2-Actor-Cookbook.zh.md)** (advanced patterns)
4.  **[Testing Your Actors](./docs/2.3-Testing-Your-Actors.zh.md)**
5.  **[Media Sources and Tracks](./docs/2.1-Media-Sources-and-Tracks.zh.md)** (scenario-specific guidance)

#### **Part 3: How It Works**
*For contributors and readers who want implementation-level details.*

1.  **[How It Works](./docs/3-How-it-works.zh.md)** (implementation overview)
2.  **[Deep-dive Topics](./docs/)** (includes the detailed `3.1` to `3.13` documents)

#### **Appendix**

*   **[Glossary](./docs/appendix-a-glossary.zh.md)**

---

## 🚀 Quick Start

### Prerequisites

- **Rust**: 1.88+ ([install guide](https://rustup.rs/))
- **Node.js**: 16+ ([download](https://nodejs.org/))
- **protoc**: Protocol Buffer compiler
  ```bash
  # Ubuntu/Debian
  sudo apt install protobuf-compiler
  
  # macOS
  brew install protobuf
  ```

### One-Command Demo

```bash
# 1. Set up the project (install dependencies and build)
./run_demo.sh setup

# 2. Run the full demo
./run_demo.sh demo
```

## 📁 Project Structure

```
actor-rtc/
├── docs/                          # framework design documents
├── proto/                         # Protobuf definitions
├── actor-rtc-framework/          # core framework crate
├── signaling-server/             # Node.js signaling server
├── examples/                     # example applications
└── run_demo.sh                   # automation script
```

## 🤝 Contributing

Contributions of all kinds are welcome.

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/AmazingFeature`)
3. Commit your changes (`git commit -m 'Add some AmazingFeature'`)
4. Push the branch (`git push origin feature/AmazingFeature`)
5. Open a pull request

## 📄 License

This project is released under the MIT License.
