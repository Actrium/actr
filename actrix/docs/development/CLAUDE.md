# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is **Actrix** - a collection of WebRTC auxiliary servers providing Signaling, STUN, TURN, and identity services. It's designed as an internal backend service collection for the Actor-RTC ecosystem.

## Build and Development Commands

```bash
# Build the project (development)
cargo build

# Build optimized release
cargo build --release

# Run all quality checks
make all                    # Runs fmt, clippy, test, build, coverage

# Individual checks
make fmt                    # Format check
make clippy                 # Linting
make test                   # Run all tests
make coverage               # Generate coverage report

# Run the main service
cargo run --bin actrix -- --config path/to/config.toml

# Validate config only
cargo run --bin actrix -- test --config path/to/config.toml

# Run specific crate tests
cargo test -p ks            # Test KS (Key Server) crate
cargo test -p ais           # Test AIS (Actor Identity Service) crate
cargo test -p signaling     # Test signaling crate

# Build individual crates
cargo build -p base
cargo build -p ks
cargo build -p ais
```

## Architecture Overview

### Service Architecture
The system uses a **modular service architecture** with fine-grained control:

- **ICE Services**: STUN and TURN servers (independent UDP servers)
- **HTTP Router Services**: Multiple HTTP services sharing one axum server
  - AIS (Actor Identity Service) - `/ais`
  - KS (Key Server) - `/ks` 
  - Signaling - `/signaling`
  - Status/Health - `/status`

### Workspace Structure
```
├── crates/
│   ├── base/           # Shared configuration, storage, utilities
│   ├── ais/            # Actor Identity Service (token issuing/validation)
│   ├── ks/             # Key Server (cryptographic key management)
│   ├── signaling/      # WebRTC signaling service  
│   ├── stun/           # STUN server implementation
│   ├── turn/           # TURN server implementation
│   └── supervit/       # Supervisor/monitoring service
├── src/                # Main application orchestrator
└── deploy/             # Deployment tooling and wizards
```

### Key Components

1. **ServiceManager** (`src/service/manager.rs`): Orchestrates all services with unified shutdown handling

2. **Configuration System** (`crates/common/src/config/`): Single source of truth using TOML
   - Uses bitmask for service enable/disable (位掩码控制)
   - Supports environment-specific settings (dev/prod/test)

3. **Authentication**: Uses `nonce-auth` library for replay-attack prevention
   - PSK-based authentication between internal services
   - SQLite-backed nonce storage

4. **Storage**: SQLite-based persistent storage
   - Database path configurable via `sqlite` field in config
   - Separate databases for different services (KS keys, nonces, etc.)

## Critical Security Issues (Internal Service Context)

### P0 - Functional Breaking Issues (✅ FIXED)

**✅ FIXED - Token Validation Now Works:**
```rust
// crates/common/src/aid/credential/validator.rs:132-169
async fn get_secret_key_by_id(&self, key_id: u32) -> Result<SecretKey, AidError> {
    // 1. Try cache first
    match self.key_cache.get_cached_key(key_id).await? {
        Some(secret_key) => return Ok(secret_key),
        None => { /* Fetch from KS */ }
    }

    // 2. Fetch from KS service
    let (secret_key, expires_at) = self.ks_client.fetch_secret_key(key_id).await?;

    // 3. Update cache
    self.key_cache.cache_key(key_id, &secret_key, expires_at).await?;
    Ok(secret_key)
}
```
- **Status**: ✅ Implemented and tested
- **Solution**: Validator now retrieves keys from KS service with caching
- **Verification**: End-to-end integration tests confirm Issuer and Validator use matching keys
- **Tests**: `cargo test -p ais` (17/17 passed)

### P1 - Security Issues (Important for Internal Deployment)

**🔐 Private Key Storage:**
```rust
// crates/ks/src/storage.rs:91 - Plaintext storage
"INSERT INTO keys (public_key, secret_key, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)"
```
- **Risk**: Private keys stored as Base64 plaintext in SQLite
- **Mitigation**: Acceptable for internal use with proper file permissions (600)

**🔑 Shared PSK Authentication:**
```rust
// crates/common/src/config/mod.rs:137
actrix_shared_key: "default-auxes-shared-key-change-in-production"
```
- **Risk**: All services share same PSK, default value is publicly known
- **Solution**: Must change default PSK before deployment

**⏰ Key Lifecycle Management:**
```rust
// crates/ks/src/storage.rs:88 - Fixed expiration
let expires_at = now + 3600; // Hardcoded 1 hour
```
- **Issues**: No key rotation, no expired key cleanup, no configurable TTL
- **Impact**: Keys accumulate indefinitely, no rotation capability

**🎯 Access Control:**
```rust
// crates/ks/src/handlers.rs:159 - Overly permissive
match app_state.storage.get_secret_key(key_id)? {
    Some(secret_key) => { /* Any authenticated service gets any key */ }
}
```
- **Risk**: Any service with valid PSK can access any private key
- **Improvement**: Implement service-specific key access controls

### P2 - Information Disclosure & Operational Issues

**📝 Information Leakage:**
```rust
// crates/ks/src/handlers.rs:169 & 199
info!("Found secret key for key_id: {}", key_id);
warn!("Secret key not found for key_id: {}", key_id);
```
- **Risk**: Logs reveal key existence/access patterns
- **Fix**: Log access without revealing key_id details

**⚡ Timing Attack Vulnerability:**
```rust
// Different response times reveal valid key_ids
Some(secret_key) => { /* Fast path */ }
None => { /* Database query + error, slower */ }
```
- **Risk**: Attackers can enumerate valid key_ids via response timing
- **Fix**: Implement constant-time responses

**🔧 Hardcoded Token Key ID:**
```rust
// crates/ais/src/issuer.rs:83
token_key_id: 1, // Fixed value, prevents key rotation
```
- **Impact**: Cannot rotate encryption keys, single point of failure
- **Solution**: Implement dynamic key_id selection

### Security Assessment Summary

**Current Security Level**: D (Non-functional + Medium Risk)
**Post-Fix Security Level**: B (Functional + Acceptable Risk for Internal Use)

**Risk Context for Internal Deployment:**
- ✅ No public internet exposure
- ✅ Internal network isolation
- ✅ Physical access controls
- ⚠️ Still vulnerable to insider threats and lateral movement attacks

**📋 Pre-deployment Security Checklist:**
```bash
# 1. Fix P0 functional issue
# Ensure validator uses actual keys from KS, not random generation

# 2. Secure file permissions
chmod 600 *.db config.toml
chown service-user:service-user *.db config.toml

# 3. Change default credentials
grep -v "default-auxes-shared-key" config.toml  # Should return nothing

# 4. Verify network binding
netstat -tlnp | grep auxes  # Should bind to internal IPs only

# 5. Database security
ls -la *.db  # Should show 600 permissions, correct ownership

# 6. Enable audit logging
tail -f logs/auxes.log | grep -E "(Secret key|Found.*key)"
```

**Deployment Recommendation**: 
- ✅ **Safe for internal use** after fixing P0 issue and changing default PSK
- ⚠️ **Requires operational controls** for file permissions and network isolation  
- 🚫 **Not suitable for public deployment** without major security hardening

## Configuration

Uses `config.toml` in root directory. Key sections:

```toml
enable = 31              # Bitmask: all services (1+2+4+8+16)
name = "actrix-01"        # Instance identifier
env = "prod"             # Environment: dev/prod/test
sqlite_path = "database"   # SQLite database storage directory path
actrix_shared_key = "your-strong-key-here"  # MUST change from default

[bind.https]
ip = "0.0.0.0"
port = 8443
cert = "certificates/server.crt"
key = "certificates/server.key"

[bind.ice]  
ip = "0.0.0.0"
port = 3478

[turn]
public_ip = "127.0.0.1"
realm = "webrtc.rs"
```

## Service-Specific Notes

### KS (Key Server)
- **Purpose**: Generate ECIES key pairs for AIS token encryption
- **Flow**: Issue service gets public keys, validation services get private keys
- **Database**: Stores key_id, public_key, secret_key, timestamps
- **Routes**: `POST /generate` (returns public key), `GET /secret/{key_id}` (returns private key)

### AIS (Actor Identity Service) 
- **Purpose**: Issues and validates encrypted Actor ID tokens
- **Protocol**: Strict protobuf binary API using `AIdAllocationResult` oneof pattern
- **Encryption**: Uses ECIES for token encryption with keys from KS
- **Routes**: `POST /allocate` (protobuf binary in/out)

### Base Crate
- **Config**: Unified configuration system with TOML support
- **Storage**: SQLite abstractions and nonce storage for replay protection
- **AID**: Token claims, validation, and credential management utilities

## Development Patterns

### Error Handling
- Use `anyhow::Result` for application errors
- Service-specific error types (e.g., `KsError`, `AisError`) 
- Structured error responses in HTTP handlers

### Testing
- Use `tempfile` for temporary databases in tests
- `#[tokio::test]` for async tests
- Integration tests validate HTTP endpoints with real credentials

### Logging
- `tracing` for structured logging
- Log levels: error, warn, info, debug
- Security: Avoid logging sensitive data (keys, credentials)

## Important Implementation Details

- **nonce-auth**: v0.6.1 with SQLite storage for replay attack prevention
- **ECIES**: Elliptic curve encryption for token security  
- **Axum**: Web framework with JSON and binary protobuf support
- **Rustls**: TLS implementation, no OpenSSL dependency
- **SQLite**: Bundled SQLite with rusqlite crate

## Common Issues and Solutions

1. **Compilation errors with nonce-auth**: Update error handling calls from `NonceError::DatabaseError` to `NonceError::from_storage_error`

2. **Key validation failures**: Ensure validator uses correct key from KS service, not randomly generated keys

3. **Port conflicts**: HTTPS and WSS share the same port, STUN/TURN can share UDP port

4. **Certificate issues**: Ensure cert/key paths are correct and accessible

5. **Database permissions**: SQLite files need proper read/write permissions for service user

This is an internal service system designed for trusted network environments. Security measures are appropriate for internal deployment but would need significant hardening for public exposure.