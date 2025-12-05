# actrix-proto

Protocol buffer definitions for Actrix services.

## Overview

This crate consolidates all protobuf definitions used across Actrix components into a single location, providing:

- Centralized proto file management
- Unified build configuration
- Consistent type re-exports for consumers

## Module Structure

```
actrix_proto
├── supervisor::v1    # Supervisor service definitions
│   ├── SupervisorService (Node → Supervisor)
│   ├── SupervisedService (Supervisor → Node)
│   └── Common types (NonceCredential, TenantInfo, etc.)
└── ks::v1            # Key Server service definitions
    └── KeyServer service
```

## Proto Files

| File | Package | Description |
|------|---------|-------------|
| `common.proto` | `supervisor.v1` | Shared types: NonceCredential, TenantInfo, SystemMetrics, etc. |
| `supervisor.proto` | `supervisor.v1` | SupervisorService - Node registration and reporting |
| `supervised.proto` | `supervisor.v1` | SupervisedService - Realm/config management from Supervisor |
| `keyserver.proto` | `ks.v1` | KeyServer - Key generation and retrieval |

## Usage

### Direct module access

```rust
use actrix_proto::supervisor::v1::{RegisterNodeRequest, ReportRequest};
use actrix_proto::ks::v1::{GenerateKeyRequest, KeyServerClient};
```

### Convenience re-exports

```rust
// Common types re-exported at crate root
use actrix_proto::{
    NonceCredential, TenantInfo, ResourceType,
    SupervisorServiceClient, SupervisedServiceServer,
};
```

## Design Notes

### Cross-Package References

The `ks.v1` package imports types from `supervisor.v1` (specifically `NonceCredential` for authentication). This creates a dependency between packages but allows consistent authentication across all services.

### Proto2 vs Proto3

All proto files use **proto2** syntax with `required` fields for stronger type guarantees in generated Rust code.
