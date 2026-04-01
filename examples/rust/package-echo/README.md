# package-echo

End-to-end test for the Actr package-driven execution flow, demonstrating how to build, sign, publish, and run `.actr` packages with WebRTC-based service discovery.

## Overview

This example demonstrates:

1. **Build**: Compile echo-actr WASM package and optimize with wasm-opt
2. **Sign**: Create signed `.actr` archive using `actr pkg build`
3. **Verify**: Validate package signature with `actr pkg verify`
4. **Publish**: Register package with MFR (Manufacturer Registry) via `actr pkg publish`
5. **Run**: Host server loads the package and exposes the echo service
6. **Discover**: Client discovers the service via Actrix signaling server
7. **Connect**: Establish WebRTC connection and exchange messages

## Architecture

```
┌─────────────────┐
│   echo-actr     │  WASM package (guest actor)
│   (WASM)        │
└────────┬────────┘
         │ packaged into
         ▼
┌─────────────────┐
│  .actr package  │  Signed archive
│  (signed)       │
└────────┬────────┘
         │ published to
         ▼
┌─────────────────┐       ┌──────────────────┐
│     Actrix      │◄─────►│ package-echo     │
│  (signaling +   │       │    server        │
│   MFR + AIS)    │       │  (loads .actr)   │
└────────┬────────┘       └──────────────────┘
         │                         ▲
         │ WebRTC signaling        │ WebRTC data
         ▼                         │
┌─────────────────┐                │
│ package-echo    │────────────────┘
│    client       │
└─────────────────┘
```

## Platform Support

| Platform | Support | Notes |
|----------|---------|-------|
| **macOS** | ✅ Full | Native support |
| **Linux** | ✅ Full | Native support |
| **Windows** | ⚠️ WSL 2 | Requires WSL 2 (see below) |

### Windows Requirements

Windows users must use **WSL 2** (Windows Subsystem for Linux 2):

- ❌ **WSL 1**: Not supported (incomplete network stack, UDP issues)
- ✅ **WSL 2**: Fully supported (complete Linux kernel)

**Why WSL 2?**
- The test script uses Unix-specific tools (`bash`, `lsof`, `sqlite3`, etc.)
- WebRTC requires proper UDP support (TURN/ICE on port 3478)
- Better file system performance for Rust compilation

## Prerequisites

### All Platforms

1. **Rust toolchain** (1.70+)
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   rustup target add wasm32-unknown-unknown
   ```

2. **jq** (JSON processor)
   - macOS: Auto-installed via Homebrew if missing
   - Linux: Auto-installed via apt/yum/dnf if missing
   - Manual install: https://jqlang.github.io/jq/download/

3. **wasm-opt** (WASM optimizer)
   ```bash
   cargo install wasm-opt
   ```

4. **System tools**
   - `sqlite3` - Database operations
   - `lsof` - Port checking
   - `curl` - HTTP requests
   - `nc` (netcat) - Network testing

### macOS

```bash
# Install Homebrew if not present
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"

# Install dependencies (jq auto-installs if missing)
brew install sqlite3
```

### Linux (Ubuntu/Debian)

```bash
sudo apt-get update
sudo apt-get install -y \
    build-essential \
    pkg-config \
    libssl-dev \
    sqlite3 \
    lsof \
    netcat \
    curl \
    jq
```

### Linux (RHEL/CentOS/Fedora)

```bash
sudo yum install -y \
    gcc \
    openssl-devel \
    sqlite \
    lsof \
    nc \
    curl \
    jq
```

### Windows (WSL 2 Setup)

**Step 1: Install WSL 2**

```powershell
# PowerShell (Administrator)
wsl --install -d Ubuntu
wsl --set-default-version 2

# Verify WSL 2 is active
wsl -l -v
# Should show VERSION 2
```

**Step 2: Enter WSL and Install Dependencies**

```bash
# Enter WSL
wsl

# Install dependencies
sudo apt-get update
sudo apt-get install -y \
    build-essential \
    pkg-config \
    libssl-dev \
    sqlite3 \
    lsof \
    netcat \
    curl \
    jq \
    git

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
rustup target add wasm32-unknown-unknown
```

**Step 3: Clone Project to WSL Filesystem**

⚠️ **Important**: Clone to WSL filesystem, NOT Windows filesystem (`/mnt/c`)

```bash
# ✅ Good: WSL filesystem (fast)
cd ~
git clone <repo-url> actr

# ❌ Bad: Windows filesystem (3-5x slower compilation)
cd /mnt/c/Users/username/actr
```

**Performance Comparison:**
- WSL filesystem (`~`): Normal compilation speed
- Windows filesystem (`/mnt/c`): 3-5x slower

## Quick Start

### 1. Navigate to Example Directory

```bash
cd examples/rust/package-echo
```

### 2. Run the Test

```bash
# Use default test message "TestMsg"
./start.sh

# Or send custom message
./start.sh "Hello World"
```

### 3. Expected Output

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
🧪 Testing package-echo (local echo-actr package loader)
    Using Actrix as signaling server
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
✅ jq found: jq-1.8.1
📦 Step 0: Compiling echo-actr WASM...
✅ WASM compiled: 2.1M
✅ wasm-opt done: 1.8M
📦 Step 0.5: Packing signed .actr package...
✅ .actr package built: 1.8M
✅ Package signature verified
...
✅ Test PASSED: package-backed echo server response received
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
🎉 Echo package test completed successfully!
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## What the Script Does

The `start.sh` script performs a complete end-to-end test:

### Step 0: Compile WASM Package
- Compiles `echo-actr` to WASM (`wasm32-unknown-unknown`)
- Optimizes with `wasm-opt --asyncify`

### Step 0.5: Build and Sign Package
- Creates signed `.actr` archive using `actr pkg build`
- Verifies signature with `actr pkg verify`
- Builds client-guest cdylib package

### Step 1-2: Start Actrix
- Checks for actrix binary (builds if needed)
- Starts actrix signaling server on port 8081
- Waits for HTTP (8081/tcp) and ICE/TURN (3478/udp) to be ready

### Step 2.5-2.7: Setup Infrastructure
- Creates realms in actrix database
- Registers MFR manufacturer identity with public key
- Publishes package via `actr pkg publish` (challenge-response flow)
- Seeds client package metadata

### Step 3-4: Start Server
- Builds package-echo-server binary
- Loads `.actr` package in production trust mode
- Registers with AIS and obtains credential

### Step 5: Run Client
- Builds package-echo-client binary
- Discovers echo service via Actrix
- Establishes WebRTC connection
- Sends test message and verifies echo response

### Step 6: Verify Results
- Checks client logs for expected echo response
- Reports success or failure

## Logs

All logs are stored in `logs/` directory:

```bash
# View logs
cat logs/actrix.log              # Actrix signaling server
cat logs/package-echo-server.log # Echo server
cat logs/package-echo-client.log # Echo client

# Follow logs in real-time
tail -f logs/actrix.log
```

## Network Ports

| Port | Protocol | Service | Purpose |
|------|----------|---------|---------|
| 8081 | TCP | Actrix HTTP | Signaling, AIS, MFR |
| 3478 | UDP | Actrix ICE/TURN | WebRTC connectivity |
| 49152-65535 | UDP | TURN relay | WebRTC data channels |

### WSL 2 Network Notes

- ✅ All ports work within WSL
- ⚠️ To access from Windows host, use port forwarding:

```powershell
# PowerShell (Administrator)
$wslIP = (wsl hostname -I).Trim()
netsh interface portproxy add v4tov4 `
    listenport=8081 `
    listenaddress=0.0.0.0 `
    connectport=8081 `
    connectaddress=$wslIP
```

## Troubleshooting

### "jq not found"

The script auto-installs `jq` on macOS (via Homebrew) and Linux (via package manager). If auto-install fails:

```bash
# macOS
brew install jq

# Linux
sudo apt-get install jq  # Debian/Ubuntu
sudo yum install jq      # RHEL/CentOS
```

### "Failed to extract public_key"

Ensure `jq` is installed and the public key file exists:

```bash
jq --version
ls -la ../echo-actr/public-key.json
```

### "Actrix not available"

Build actrix manually:

```bash
cd ../../../actrix  # Navigate to actrix directory
cargo build
```

### "Port 8081 already in use"

Kill existing process:

```bash
# macOS/Linux
lsof -ti:8081 | xargs kill

# WSL
sudo lsof -ti:8081 | xargs sudo kill
```

### WSL 2: "command not found: lsof"

Install missing tools:

```bash
sudo apt-get install lsof netcat sqlite3
```

### WSL 2: Slow compilation

Ensure project is on WSL filesystem, not Windows filesystem:

```bash
pwd
# ✅ Should be: /home/username/...
# ❌ Not: /mnt/c/Users/...
```

### WebRTC connection fails

Check UDP port 3478 is available:

```bash
lsof -iUDP:3478
# Should show actrix listening
```

## Using `actr run` Directly

Instead of the full test script, you can run components individually:

```bash
# Start actrix manually
cd ../../../actrix
cargo run -- --config examples/rust/package-echo/actrix-config.toml

# Run server with actr run
cd examples/rust/package-echo
cargo run -p actr-cli -- run \
    --config server/actr.toml \
    --package dist/actrium-EchoService-0.1.0-wasm32-unknown-unknown.actr \
    --trust-mode production \
    --ais-endpoint http://localhost:8081/ais

# Run client
cargo run --bin package-echo-client
```

## Project Structure

```
package-echo/
├── README.md              # This file
├── start.sh               # End-to-end test script
├── actrix-config.toml     # Actrix server configuration
├── dev-key.json           # Development signing key
├── server/                # Echo server (loads .actr package)
│   ├── src/
│   ├── actr.toml          # Runtime configuration
│   └── manifest.toml      # Package manifest
├── client/                # Echo client (native)
│   ├── src/
│   ├── actr.toml
│   └── manifest.toml
├── client-guest/          # Echo client (cdylib package)
│   ├── src/
│   ├── actr.toml
│   └── manifest.toml
└── logs/                  # Generated logs (gitignored)
    ├── actrix.log
    ├── package-echo-server.log
    └── package-echo-client.log
```

## Related Examples

- `echo-actr/` - The WASM guest actor package
- `shell-actr-echo/` - Shell-based echo example
- `ws-actr-echo/` - WebSocket-based echo example

## License

Apache-2.0
