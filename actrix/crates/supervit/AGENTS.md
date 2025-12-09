# Supervit - AI Development Guide

## Overview

Supervit provides gRPC communication between actrix nodes and the central supervisor platform.

## Directory Structure

```
crates/supervit/
├── bin/
│   └── gen_credential.rs      # Credential generator tool
├── examples/
│   └── supervisord_server.rs  # Test gRPC server
├── scripts/
│   └── test_supervised.sh     # grpcurl test script
├── proto/                     # Protocol definitions
└── src/                       # Implementation
```

## Testing

### Start Test Server

```bash
cargo run -p supervit --example supervisord_server
```

Server listens on `0.0.0.0:50055`.

### Run Tests with Shell Script (Recommended)

```bash
./crates/supervit/scripts/test_supervised.sh list          # List services
./crates/supervit/scripts/test_supervised.sh node_info     # Get node info
./crates/supervit/scripts/test_supervised.sh list_realms   # List realms
./crates/supervit/scripts/test_supervised.sh create_realm --realm-id my-realm
./crates/supervit/scripts/test_supervised.sh --help        # Show all options
```

### Generate Credentials Manually

```bash
# Human-readable output
cargo run -p supervit --bin gen_credential -- --action node_info

# JSON output (for scripts)
cargo run -p supervit --bin gen_credential -- --action node_info --output json

# With custom parameters
cargo run -p supervit --bin gen_credential -- \
  --action create_realm \
  --subject test-realm-01 \
  --node-id test-node-01 \
  --shared-secret 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
```

## Test Shared Secret

```
0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
```

Credentials expire after ~5 minutes.

## Authentication

Payload format: `{action}:{node_id}` or `{action}:{node_id}:{subject}`

Examples:
- `node_info:example-node-01`
- `create_realm:example-node-01:my-realm`
