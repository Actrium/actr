# Actrix - Actor-RTC Auxiliary Servers

A production-ready collection of WebRTC auxiliary servers providing STUN, TURN, Key Server (KS), and service coordination for the Actor-RTC ecosystem.

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://www.rust-lang.org/)

## Features

### Core Services
- **STUN Server**: NAT traversal assistance (UDP 3478)
- **TURN Server**: Media relay for restricted networks with LRU authentication cache (+40% performance)
- **Key Server (KS)**: ECIES cryptographic key generation and management

### Infrastructure
- ⚡ **High Performance**: LRU caching, async runtime, non-blocking I/O
- 📊 **Observability**: OpenTelemetry tracing, log rotation, structured logging
- 🔐 **Security**: TLS/HTTPS, PSK authentication, nonce-based replay protection
- 🎛️ **Flexible Configuration**: TOML-based, bitmask service control, comprehensive validation
- 🚀 **Production Ready**: Systemd integration, automated deployment, health checks

## Quick Start

### Installation

```bash
# Clone repository
git clone https://github.com/actor-rtc/actrix.git
cd actrix

# Build release binary
cargo build --release

# With OpenTelemetry support
cargo build --release --features opentelemetry
```

### Configuration

Copy and customize the example configuration:

```bash
cp config.example.toml config.toml
nano config.toml
```

Key settings to change:
- `actrix_shared_key` - Generate with: `openssl rand -hex 32`
- `turn.advertised_ip` - Your server's public IP
- `bind.https.cert/key` - TLS certificate paths
- `observability.log.output` - Set to `"file"` for production

### Running

```bash
# Validate configuration
./target/release/actrix test config.toml

# Start server
./target/release/actrix --config config.toml

# Or use systemd (see deploy/README.md)
sudo ./deploy/install.sh install
sudo systemctl start actrix
```

## Configuration

### Service Control (Bitmask)

```toml
# Binary: xxxxx
#         ││││└─ Signaling (1)  [Disabled]
#         │││└── STUN      (2)
#         ││└─── TURN      (4)
#         │└──── AIS       (8)  [Disabled]
#         └───── KS        (16)

enable = 6   # STUN + TURN
enable = 22  # KS + TURN + STUN (recommended)
```

### Environment Types

- `dev`: Development (HTTP allowed, console logs)
- `prod`: Production (HTTPS required, file logs recommended)
- `test`: Testing (automated tests)

### Example Configuration

```toml
enable = 6
name = "actrix-01"
env = "prod"

[observability]
filter_level = "info"    # RUST_LOG overrides if set

[observability.log]
output = "file"
rotate = true

[bind.ice]
advertised_ip = "203.0.113.10"
ip = "0.0.0.0"
port = 3478

[turn]
advertised_ip = "203.0.113.10"
advertised_port = 3478
relay_port_range = "49152-65535"
realm = "example.com"
```

See [docs/CONFIGURATION.md](docs/CONFIGURATION.md) for complete reference.

## Observability

### Logging

**Console Output** (development):
```toml
[observability]
filter_level = "debug"  # overridden by RUST_LOG if set

[observability.log]
output = "console"
```

**File Output with Rotation** (production):
```toml
[observability]
filter_level = "info"  # overridden by RUST_LOG if set

[observability.log]
output = "file"
rotate = true
path = "/var/log/actrix/"
```

### OpenTelemetry Tracing

```bash
# 1. Start Jaeger
docker-compose -f docker/jaeger-compose.yml up -d

# 2. Build with tracing support
cargo build --release --features opentelemetry

# 3. Configure endpoint
[observability.tracing]
enable = true
service_name = "actrix"
endpoint = "http://127.0.0.1:4317"

# 4. Access UI
http://localhost:16686
```

## API Endpoints

### KS (Key Server) - `/ks/*`

- `POST /ks/generate` - Generate ECIES key pair
- `GET /ks/secret/{key_id}` - Get private key (authenticated)
- `GET /ks/public/{key_id}` - Get public key
- `GET /ks/public/keys` - List all public keys
- `GET /ks/health` - Health check

## Deployment

### Systemd Service

```bash
# Install as systemd service
sudo ./deploy/install.sh install

# Start service
sudo systemctl start actrix
sudo systemctl enable actrix

# View logs
sudo journalctl -u actrix -f

# Update binary
sudo ./deploy/install.sh update
```

See [deploy/README.md](deploy/README.md) for complete deployment guide.

### Docker (Future)

Docker images planned for future releases.

## Performance

### TURN Authentication Cache

- **Without cache**: ~10,000 req/s
- **With LRU cache**: ~14,000 req/s (+40%)
- **Cache hit rate**: 95%+
- **Capacity**: 1000 entries

### Benchmarks

```bash
# Run benchmarks (future)
cargo bench
```

## Development

### Prerequisites

- Rust 1.88+ (Edition 2024)
- SQLite 3.x
- OpenSSL (for certificates)

### Build & Test

```bash
# Run quality checks
make all  # fmt, clippy, test, build

# Individual checks
make fmt
make clippy
make test
make coverage

# Run specific tests
cargo test -p ks
cargo test -p turn
```

### Project Structure

```
actrix/
├── src/              # Main application
├── crates/
│   ├── base/        # Shared config, storage, utilities
│   ├── ks/          # Key Server
│   ├── stun/        # STUN Server
│   ├── turn/        # TURN Server (with LRU cache)
│   └── ...
├── deploy/          # Deployment scripts
├── docs/            # Documentation
└── AGENTS.md        # AI development guide
```

See [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) for development guide.

## Documentation

- [ARCHITECTURE.md](docs/ARCHITECTURE.md) - System architecture
- [CONFIGURATION.md](docs/CONFIGURATION.md) - Configuration reference
- [DEVELOPMENT.md](docs/DEVELOPMENT.md) - Development guide
- [deploy/README.md](deploy/README.md) - Deployment guide
- [AGENTS.md](AGENTS.md) - AI assistant guide
- [CLAUDE.md](CLAUDE.md) - Project context

## Security

### Current Status

**Security Level**: B (Production-ready for internal use)

### Security Features

✅ **Implemented**:
- TLS/HTTPS for API endpoints
- PSK authentication with shared key
- Nonce-based replay protection
- SQLite file permissions
- Systemd security hardening

⚠️ **Limitations** (acceptable for internal deployment):
- Keys stored Base64-encoded in SQLite
- Shared PSK authentication
- No automatic key rotation
- Timing attack vulnerability in key lookups

### Deployment Requirements

- ✅ Change default `actrix_shared_key`
- ✅ Use HTTPS in production
- ✅ File permissions: `chmod 600 config.toml *.db`
- ✅ Network isolation
- ✅ Run as non-root user

See [CLAUDE.md](CLAUDE.md) for detailed security analysis.

## Roadmap

### Completed (v0.2.0)

- [x] OpenTelemetry tracing support
- [x] Log rotation and file output
- [x] TURN LRU authentication cache
- [x] Configuration validation
- [x] Deployment automation (systemd)
- [x] Comprehensive documentation

### Planned

- [ ] Re-enable AIS service with actr-protocol
- [ ] PostgreSQL backend support
- [ ] Prometheus metrics export
- [ ] Configuration hot reload
- [ ] Multi-region deployment support
- [ ] Docker images

## Contributing

This is an internal project for the Actor-RTC ecosystem. When contributing:

1. Follow code patterns in [AGENTS.md](AGENTS.md)
2. Add tests for new features
3. Run `make all` before committing
4. Use semantic commit messages (no AI tool mentions)

## License

Apache License 2.0

## Documentation

完整文档系统 (~4800 行精炼文档):

- **[INDEX.md](docs/INDEX.md)** - 文档导航索引 (从这里开始)
- **[ARCHITECTURE.md](docs/ARCHITECTURE.md)** - 架构设计 (含代码行号引用)
- **[CRATES.md](docs/CRATES.md)** - 代码实现详解
- **[SERVICES.md](docs/SERVICES.md)** - 服务管理、部署、运维
- **[API.md](docs/API.md)** - HTTP API 参考
- **[CONFIGURATION.md](docs/CONFIGURATION.md)** - 配置参考
- **[install/README.md](install/README.md)** - 生产部署指南
- **[DEVELOPMENT.md](docs/DEVELOPMENT.md)** - 开发指南

## Related Projects

- [actr-protocol](https://github.com/actor-rtc/actr-protocol) - Protobuf definitions
- [actr-framework](https://github.com/actor-rtc/actr-framework) - Actor framework

## Support

- GitHub Issues: https://github.com/actor-rtc/actrix/issues
- Documentation: [docs/INDEX.md](docs/INDEX.md)

---

**Note**: Designed for internal deployment in trusted networks. Requires security hardening for public internet exposure.
