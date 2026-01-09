# AGENTS.md - AI Assistant Guide

This document is a concise guide for AI coding assistants working on Actrix.

## Critical invariants
- Key serialization: all secp256k1 public keys must be stored/sent as the 33-byte compressed output of `public_key.serialize_compressed()` (Base64-encoded). AIS/KS write paths and clients enforce this length; any 65-byte uncompressed key will be rejected.
- HTTP port sharing: all HTTP-based services (AIS, KS HTTP API, Signaling) share the same instance-level port defined by `bind.http` or `bind.https`.

## Project summary
- Repository: https://github.com/actor-rtc/actrix
- License: Apache 2.0
- Rust: 1.88+ (Edition 2024)
- Layout: workspace with multiple crates

## Core conventions
- Naming: use descriptive names; avoid abbreviations.
- Errors: use `Result` and custom error types; no `panic!`/`unwrap()` for expected failures.
- Logging: pick the right level, include context, prefer structured fields, never log secrets.
- Tests: public APIs should have unit tests; global state tests must use `serial_test`.
- Performance: avoid unnecessary clones; prefer non-blocking log writers; use caches for hot paths.
- Config: hierarchy is `config.toml` > `config.example.toml` > `ActrixConfig::default()`. Adding a config field requires a struct field + default, example config, default impl update, and tests.

## Architecture notes
- Startup flow: load config → init observability → create shutdown broadcast → register services → start all → attach KS gRPC if enabled → await handles → on error broadcast shutdown → stop all.
- Runtime semantics: any task wired to `shutdown_tx` should send on exit to converge the system.
- Error propagation: use `?` and return typed errors.

## Dependencies and features
- Prefer workspace dependencies to keep versions aligned.
- Optional features must be gated with `features` and `#[cfg(feature = "...")]`.

## Security and input handling
- Never log secrets; use `zeroize` for sensitive buffers.
- Validate all external inputs (length, charset, bounds).
- Client-facing errors should be generic; log details internally.

## Local workflow
- Format: `cargo fmt`
- Lint: `cargo clippy -- -D warnings`
- Test: `cargo test`
- Make shortcuts: `make fmt`, `make clippy`, `make test`, `make all`
