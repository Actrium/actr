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
├── admin::v1    # Admin service definitions
│   ├── ControlService (Node → Admin)
│   ├── NodeAdminService (Admin → Node)
│   └── Common types (NonceCredential, RealmInfo, etc.)
└── ks::v1            # Key Server service definitions
    └── KeyServer service
```

## Proto Files

| File | Package | Description |
|------|---------|-------------|
| `common.proto` | `admin.v1` | Shared types: NonceCredential, RealmInfo, SystemMetrics, etc. |
| `admin.proto` | `admin.v1` | ControlService - Node registration and reporting |
| `node_admin.proto` | `admin.v1` | NodeAdminService - Realm/config management from Admin |
| `keyserver.proto` | `ks.v1` | KeyServer - Key generation and retrieval |

## Usage

### Direct module access

```rust
use actrix_proto::admin::v1::{RegisterNodeRequest, ReportRequest};
use actrix_proto::ks::v1::{GenerateKeyRequest, KeyServerClient};
```

### Convenience re-exports

```rust
// Common types re-exported at crate root
use actrix_proto::{
    NonceCredential, RealmInfo, ResourceType,
    ControlServiceClient, NodeAdminServiceServer,
};
```

## Design Notes

### Cross-Package References

The `ks.v1` package imports types from `admin.v1` (specifically `NonceCredential` for authentication). This creates a dependency between packages but allows consistent authentication across all services.

### Proto2 vs Proto3

All proto files use **proto2** syntax with `required` fields for stronger type guarantees in generated Rust code.
