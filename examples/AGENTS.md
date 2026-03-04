# Repository Agents Guide

This document summarizes the expectations for anyone operating or extending the ACTR example workspace in `/Users/kaito/Project/ar/actr-examples`.

## Scope & Goals

- Keep the examples aligned with the upstream `../actr` crates (framework, protocol, runtime, config).
- Maintain runnable demos that illustrate data streaming, media relay, and shell echo flows.
- Ensure every addition preserves the developer experience: `cargo check`, `cargo fmt`, and `cargo run` must succeed from the workspace root.

## Workspace Layout

- `data-stream/` – bidirectional file-transfer example. Contains `sender/` and `receiver/` crates plus shared protobuf files.
- `media-relay/` – two-app relay scenario. Includes `common/` utilities, `actr-a/` client, and `actr-b/` server-plus-relay components.
- `shell-actr-echo/` – CLI oriented echo demo with `server/` and `client/` crates and helper configs.

Each crate is a workspace member (see root `Cargo.toml`) and targets Rust edition 2024.

## Daily Workflow

1. **Sync Dependencies** – verify paths pointing to `../../../actr/crates/...` still resolve after upstream changes. Run `cargo metadata` to confirm.
2. **Format & Lint** – execute `cargo fmt --all` and `cargo clippy --all-targets` before sending patches.
3. **Check & Test** – use `cargo check` for quick validation, `cargo test -p <crate>` for focused coverage, and `cargo test --workspace` for full verification when touching shared code.
4. **Run Examples** – Use `cargo run --bin <name>` or `cargo run -p <package>` to run individual examples:
   - `cargo run --bin data-stream-sender` or `cargo run -p data-stream-sender`
   - `cargo run --bin echo-real-server` or `cargo run -p echo-real-server`
   - `cargo run --bin echo-real-client-app` or `cargo run -p echo-real-client-app`
   - Both `--bin` and `-p` work identically in a workspace context.

## Tooling Prerequisites

- Required CLIs for codegen: `protoc-gen-prost`, `protoc-gen-actrframework`, and `actr` (from the `actr-cli` crate).
- Always install/update to the latest versions via `cargo install --force protoc-gen-prost actr-framework-protoc-codegen actr-cli`.
- The `start.sh` scripts call `scripts/ensure-tools.sh` to preflight these tools on every run and auto-install/update them if missing or outdated (install logs land in `logs/cargo-install-*.log`).

## Service Discovery

- **Use proper discovery APIs** instead of sleep workarounds. The examples now use the service discovery API to find remote actors via the signaling server.
- **Context-based discovery** (in `on_start`): Use `ctx.discover_route_candidate(&target_type).await?` to discover a single candidate.
  - Example: `data-stream/sender` uses this in `SenderWorkload::on_start` to find the receiver.
- **ActrRef-based discovery** (after node start): Use `actr_ref.discover_route_candidates(&target_type, count).await?` to discover multiple candidates.
  - Example: `shell-actr-echo/client` and `media-relay/actr-a` use this to find server instances.
- **No sleep workarounds**: The discovery API provides proper ready notification, eliminating the need for temporary sleep delays.

## Protobuf & Code Generation

- Protos live under each crate's `proto/` directory; generated Rust code is checked into `src/generated/`.
- When protos change, re-run the upstream tooling:
  ```
  actr gen --input=proto --output=src/generated --clean
  ```
  (Invoke from the specific crate directory; adjust paths if the generator binary resides elsewhere.)
- After regeneration, rerun `cargo fmt` and ensure git diffs only include intended updates.

## Documentation Expectations

- Keep documentation English-only inside repo files, even though conversations may use Chinese.
- Update `README.md` and example-specific docs whenever interfaces, commands, or prerequisites change.
- When adding public APIs, include doc comments plus short usage examples in the relevant crate.

## Actrix Configuration & Start Scripts

- Use only the workspace-root `actrix-config.toml`; do not add per-example copies.
- When modifying any `start.sh`, start actrix from the workspace root with the root config path.
- Distinguish the three examples in docs and scripts: `data-stream`, `shell-actr-echo`, and `media-relay` (each has its own `start.sh` entrypoint; all share the root config).

## Release & Review Tips

- Prefer small, focused PRs grouped by feature or fix.
- Reference affected workspace members explicitly in commit messages and PR descriptions.
- Attach reproduction steps (commands or logs) for behavior-oriented changes.
- Coordinate with the upstream `../actr` repository before bumping dependency paths or expecting new generator behavior.
